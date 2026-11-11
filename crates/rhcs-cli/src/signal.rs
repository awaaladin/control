//! `rhcs signal`: live LTE radio view for the ZTE router, with history,
//! handover detection and a CSV log (the raw data for Tower Intelligence).

use crate::{env, ui};
use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use zte::{band_mhz, grade_rsrp, grade_rsrq, grade_sinr, Grade, Telemetry, ZteClient};

const HISTORY: usize = 48;
const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

#[derive(Default)]
struct State {
    rsrp: VecDeque<f32>,
    sinr: VecDeque<f32>,
    rx: VecDeque<f32>,
    cell: Option<(u32, u32, u16)>, // (cell id, pci, band)
    events: VecDeque<String>,
    samples: u64,
    best: Option<f32>,
    worst: Option<f32>,
}

fn push(q: &mut VecDeque<f32>, v: f32) {
    if q.len() == HISTORY {
        q.pop_front();
    }
    q.push_back(v);
}

impl State {
    fn observe(&mut self, t: &Telemetry) {
        self.samples += 1;
        if let Some(v) = t.rsrp {
            push(&mut self.rsrp, v);
            self.best = Some(self.best.map_or(v, |b| b.max(v)));
            self.worst = Some(self.worst.map_or(v, |w| w.min(v)));
        }
        if let Some(v) = t.sinr {
            push(&mut self.sinr, v);
        }
        push(&mut self.rx, t.rx_bytes_per_s.unwrap_or(0) as f32 * 8.0 / 1e6);

        if let (Some(cell), Some(pci), Some(band)) = (t.cell_id, t.pci, t.band) {
            let now = (cell, pci, band);
            if let Some(prev) = self.cell.filter(|p| *p != now) {
                let time = chrono::Local::now().format("%H:%M:%S");
                self.events.push_front(format!(
                    "{time}  handover  PCI {}→{}  eNB {}→{}  B{}→B{}",
                    prev.1,
                    now.1,
                    prev.0 >> 8,
                    now.0 >> 8,
                    prev.2,
                    now.2
                ));
                self.events.truncate(6);
            }
            self.cell = Some(now);
        }
    }
}

fn bar(v: f32, lo: f32, hi: f32, width: usize) -> String {
    let n = (((v - lo) / (hi - lo)).clamp(0.0, 1.0) * width as f32).round() as usize;
    format!("{}{}", "█".repeat(n), "░".repeat(width - n))
}

fn spark(values: &VecDeque<f32>, lo: f32, hi: f32) -> String {
    values
        .iter()
        .map(|v| SPARK[(((v - lo) / (hi - lo)).clamp(0.0, 0.999) * 8.0) as usize])
        .collect()
}

fn paint_grade(g: Grade, s: &str) -> String {
    match g {
        Grade::Excellent => ui::green(s),
        Grade::Good => ui::cyan(s),
        Grade::Fair => ui::yellow(s),
        Grade::Poor => ui::red(s),
    }
}

fn human_bytes(b: u64) -> String {
    let b = b as f64;
    match b {
        x if x >= 1e9 => format!("{:.2} GB", x / 1e9),
        x if x >= 1e6 => format!("{:.1} MB", x / 1e6),
        x if x >= 1e3 => format!("{:.1} KB", x / 1e3),
        x => format!("{x:.0} B"),
    }
}

fn human_secs(s: u64) -> String {
    match s {
        s if s >= 3600 => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
        s if s >= 60 => format!("{}m {:02}s", s / 60, s % 60),
        s => format!("{s}s"),
    }
}

fn metric_line(name: &str, v: Option<f32>, unit: &str, lo: f32, hi: f32, g: fn(f32) -> Grade, hist: Option<&VecDeque<f32>>) -> String {
    match v {
        None => format!("  {}  {}\n", ui::dim(&format!("{name:<5}")), ui::dim("—")),
        Some(v) => {
            let grade = g(v);
            let value = format!("{v:>6.1} {unit:<3}");
            let sparkline = hist.map(|h| spark(h, lo, hi)).unwrap_or_default();
            format!(
                "  {}  {}  {}  {:<9}  {}\n",
                ui::dim(&format!("{name:<5}")),
                ui::bold(&value),
                paint_grade(grade, &bar(v, lo, hi, 22)),
                paint_grade(grade, grade.label()),
                ui::dim(&sparkline)
            )
        }
    }
}

fn render(model: &str, host: &str, t: &Telemetry, st: &State, login_problem: Option<&str>, log: Option<&str>) -> String {
    let mut o = String::new();
    let link = if t.connected() { ui::green("● connected") } else { ui::red(&format!("● {}", t.ppp_status)) };
    o += &format!(
        "  {} {}  {}  {}  {}  {}\n",
        ui::yellow("◆"),
        ui::bold(model),
        ui::dim(host),
        t.provider,
        t.network_type,
        link
    );
    o += &format!("  {}\n", ui::dim(&"─".repeat(72)));

    if !t.logged_in {
        o += &format!("  {}\n", ui::yellow("not logged in: showing public fields only"));
        if let Some(p) = login_problem {
            o += &format!("  {}\n", ui::red(p));
        }
    }

    let enb = t.enb().map(|e| e.to_string()).unwrap_or("—".into());
    let sector = t.sector().map(|s| s.to_string()).unwrap_or("—".into());
    let pci = t.pci.map(|p| p.to_string()).unwrap_or("—".into());
    o += &format!(
        "  {}  eNB {}  ·  sector {}  ·  PCI {}  ·  TAC {}  ·  {}/{}\n",
        ui::dim("TOWER"),
        ui::bold(&enb),
        sector,
        pci,
        t.tac.as_deref().unwrap_or("—"),
        t.mcc.as_deref().unwrap_or("—"),
        t.mnc.as_deref().unwrap_or("—")
    );
    let band = match t.band {
        Some(b) => format!("B{b} · {} MHz", band_mhz(b).unwrap_or("?")),
        None => "—".into(),
    };
    o += &format!(
        "  {}  {}  ·  EARFCN {}  ·  bars {}/5\n\n",
        ui::dim("BAND "),
        ui::bold(&band),
        t.earfcn.map(|e| e.to_string()).unwrap_or("—".into()),
        t.signal_bars.map(|b| b.to_string()).unwrap_or("—".into())
    );

    o += &metric_line("RSRP", t.rsrp, "dBm", -120.0, -70.0, grade_rsrp, Some(&st.rsrp));
    o += &metric_line("RSRQ", t.rsrq, "dB", -20.0, -3.0, grade_rsrq, None);
    o += &metric_line("SINR", t.sinr, "dB", -5.0, 30.0, grade_sinr, Some(&st.sinr));
    if let Some(r) = t.rssi {
        o += &format!("  {}  {:>6.1} dBm\n", ui::dim("RSSI "), r);
    }

    let mbps = |b: Option<u64>| b.map(|v| format!("{:.2} Mbit/s", v as f64 * 8.0 / 1e6)).unwrap_or("—".into());
    let peak = st.rx.iter().cloned().fold(0.1f32, f32::max);
    o += &format!(
        "\n  {}  {:<14}  {}\n  {}  {}\n",
        ui::dim("DOWN "),
        ui::bold(&mbps(t.rx_bytes_per_s)),
        ui::dim(&spark(&st.rx, 0.0, peak)),
        ui::dim("UP   "),
        ui::bold(&mbps(t.tx_bytes_per_s))
    );
    o += &format!(
        "  {}  {}  ·  ↓ {}  ↑ {}     {}  ↓ {}  ↑ {}\n",
        ui::dim("SESSION"),
        t.session_secs.map(human_secs).unwrap_or("—".into()),
        t.session_rx_bytes.map(human_bytes).unwrap_or("—".into()),
        t.session_tx_bytes.map(human_bytes).unwrap_or("—".into()),
        ui::dim("MONTH"),
        t.month_rx_bytes.map(human_bytes).unwrap_or("—".into()),
        t.month_tx_bytes.map(human_bytes).unwrap_or("—".into())
    );

    if !st.events.is_empty() {
        o += &format!("\n  {}\n", ui::dim("EVENTS"));
        for e in &st.events {
            o += &format!("    {}\n", ui::yellow(e));
        }
    }
    let range = match (st.best, st.worst) {
        (Some(b), Some(w)) => format!(" · RSRP best {b:.0} / worst {w:.0}"),
        _ => String::new(),
    };
    o += &format!(
        "\n  {}\n",
        ui::dim(&format!(
            "{} samples{range}{} · Ctrl+C to stop",
            st.samples,
            log.map(|l| format!(" · log {l}")).unwrap_or_default()
        ))
    );
    o
}

struct CsvLog {
    file: File,
    path: String,
}

impl CsvLog {
    fn open(dir: &Path) -> Result<Self> {
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs)?;
        let path = logs.join(format!("signal-{}.csv", chrono::Local::now().format("%Y-%m-%d")));
        let new = !path.exists();
        let mut file = OpenOptions::new().create(true).append(true).open(&path).context("opening signal log")?;
        if new {
            writeln!(file, "time,rsrp,rsrq,sinr,rssi,band,earfcn,pci,cell_id,enb,sector,rx_bytes_s,tx_bytes_s,connected")?;
        }
        Ok(Self { file, path: path.display().to_string() })
    }

    fn write(&mut self, t: &Telemetry) -> Result<()> {
        fn o<T: ToString>(v: Option<T>) -> String {
            v.map(|x| x.to_string()).unwrap_or_default()
        }
        writeln!(
            self.file,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"),
            o(t.rsrp),
            o(t.rsrq),
            o(t.sinr),
            o(t.rssi),
            o(t.band),
            o(t.earfcn),
            o(t.pci),
            t.cell_id.map(|c| format!("{c:x}")).unwrap_or_default(),
            o(t.enb()),
            o(t.sector()),
            o(t.rx_bytes_per_s),
            o(t.tx_bytes_per_s),
            t.connected()
        )?;
        Ok(())
    }
}

pub async fn run(host: Option<String>, interval: f64, once: bool, json: bool, log: bool) -> Result<()> {
    let cfg = env::load();
    let host = host
        .or_else(|| cfg.get("ROUTER_IP").cloned().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "192.168.0.1".into());
    let password = cfg.get("ROUTER_ADMIN_PASSWORD").cloned().filter(|s| !s.is_empty());
    let model = cfg.get("ROUTER_MODEL").cloned().unwrap_or_else(|| "ZTE router".into());
    let mut client = ZteClient::new(host.clone(), password);

    if once || json {
        let t = client.telemetry().await?;
        if json {
            println!("{}", serde_json::to_string_pretty(&t)?);
        } else {
            let mut st = State::default();
            st.observe(&t);
            print!("{}", render(&model, &host, &t, &st, client.login_problem(), None));
        }
        return Ok(());
    }

    let mut logger = if log { Some(CsvLog::open(&env::project_dir())?) } else { None };
    let mut st = State::default();
    let tick = Duration::from_secs_f64(interval.max(0.5));
    loop {
        match client.telemetry().await {
            Ok(t) => {
                st.observe(&t);
                if let Some(l) = logger.as_mut() {
                    l.write(&t)?;
                }
                let frame = render(&model, &host, &t, &st, client.login_problem(), logger.as_ref().map(|l| l.path.as_str()));
                if ui::color_enabled() {
                    print!("\x1b[H\x1b[J{frame}");
                } else {
                    println!("{frame}");
                }
                std::io::stdout().flush().ok();
            }
            Err(e) => eprintln!("{} {}", ui::red("router unreachable:"), e),
        }
        tokio::time::sleep(tick).await;
    }
}
