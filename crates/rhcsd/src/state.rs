//! Shared daemon state: latest readings, history, devices, events, and the
//! broadcast channel that feeds every WebSocket client.

use crate::phone::PhoneStatus;
use netscan::oui::OuiDb;
use rhcs_core::{Device, MacAddr};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, Notify, RwLock};
use zte::Telemetry;

/// 24 h of 1-second samples.
pub const HISTORY_MAX: usize = 86_400;
pub const EVENTS_MAX: usize = 500;

pub type Shared = Arc<AppState>;

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub token: Option<String>,
    pub router_ip: String,
    pub router_password: Option<String>,
    pub router_model: String,
    pub phone_serial: Option<String>,
    /// `ip:port` for ADB over Wi-Fi (e.g. the phone's hotspot address + 5555)
    pub phone_adb_addr: Option<String>,
    pub adb_path: String,
    pub scan_cidrs: Vec<String>,
    pub scan_interval_s: u64,
    /// normalized MAC -> friendly label, from any `*_MAC=` entry in .env
    pub known_macs: HashMap<String, String>,
    pub web_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let e = rhcs_core::env::load();
        let get = |k: &str| e.get(k).cloned().filter(|v| !v.is_empty());

        let adb_path = get("ADB_PATH").unwrap_or_else(|| {
            let sdk = std::env::var("LOCALAPPDATA")
                .map(|l| PathBuf::from(l).join("Android/Sdk/platform-tools/adb.exe"))
                .ok()
                .filter(|p| p.is_file());
            sdk.map(|p| p.display().to_string()).unwrap_or_else(|| "adb".into())
        });

        let mut known_macs = HashMap::new();
        for (k, v) in &e {
            if let (Some(prefix), Some(mac)) = (k.strip_suffix("_MAC"), MacAddr::parse(v)) {
                let label = prefix
                    .trim_start_matches("HOTSPOT_KNOWN_")
                    .trim_start_matches("KNOWN_")
                    .replace('_', " ")
                    .to_lowercase();
                known_macs.insert(mac.to_string(), label);
            }
        }

        Self {
            bind: get("RHCSD_BIND").unwrap_or_else(|| "0.0.0.0:7878".into()),
            token: get("RHCSD_TOKEN"),
            router_ip: get("ROUTER_IP").unwrap_or_else(|| "192.168.0.1".into()),
            router_password: get("ROUTER_ADMIN_PASSWORD"),
            router_model: get("ROUTER_MODEL").unwrap_or_else(|| "ZTE router".into()),
            phone_serial: get("PHONE_ADB_SERIAL"),
            phone_adb_addr: get("PHONE_ADB_ADDR"),
            adb_path,
            scan_cidrs: get("SCAN_CIDRS")
                .map(|s| s.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect())
                .unwrap_or_default(),
            scan_interval_s: get("SCAN_INTERVAL_S").and_then(|s| s.parse().ok()).unwrap_or(300),
            known_macs,
            web_dir: rhcs_core::env::project_dir().join("web").join("dist"),
        }
    }

    pub fn label_for(&self, mac: Option<&MacAddr>) -> Option<String> {
        mac.and_then(|m| self.known_macs.get(&m.to_string()).cloned())
    }
}

#[derive(Clone, Serialize)]
pub struct Sample {
    pub ts: i64,
    pub rsrp: Option<f32>,
    pub rsrq: Option<f32>,
    pub sinr: Option<f32>,
    pub rx_bytes_per_s: Option<u64>,
    pub tx_bytes_per_s: Option<u64>,
    pub pci: Option<u32>,
    pub cell_id: Option<u32>,
    pub band: Option<u16>,
}

impl Sample {
    pub fn from(ts: i64, t: &Telemetry) -> Self {
        Self {
            ts,
            rsrp: t.rsrp,
            rsrq: t.rsrq,
            sinr: t.sinr,
            rx_bytes_per_s: t.rx_bytes_per_s,
            tx_bytes_per_s: t.tx_bytes_per_s,
            pci: t.pci,
            cell_id: t.cell_id,
            band: t.band,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct DeviceView {
    pub network: String,
    pub label: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    #[serde(flatten)]
    pub device: Device,
}

#[derive(Clone, Default, Serialize)]
pub struct SourceState {
    pub online: bool,
    pub error: Option<String>,
    pub last_ok: Option<i64>,
}

#[derive(Clone, Default, Serialize)]
pub struct RouterState {
    pub online: bool,
    pub logged_in: bool,
    pub error: Option<String>,
    pub last_ok: Option<i64>,
    pub host: String,
    pub model: String,
}

#[derive(Clone, Default, Serialize)]
pub struct ScanState {
    pub running: bool,
    pub last_run: Option<i64>,
    pub networks: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
pub struct Sources {
    pub router: RouterState,
    pub phone: SourceState,
    pub scan: ScanState,
}

pub struct Inner {
    pub telemetry: Option<Telemetry>,
    pub history: VecDeque<Sample>,
    pub devices: Vec<DeviceView>,
    pub phone: Option<PhoneStatus>,
    pub events: VecDeque<Value>,
    pub sources: Sources,
    pub interval_s: f64,
}

pub struct AppState {
    pub cfg: Config,
    pub started: Instant,
    pub tx: broadcast::Sender<String>,
    pub inner: RwLock<Inner>,
    pub scan_now: Notify,
    pub relogin: AtomicBool,
    pub oui: OuiDb,
    /// Game Boost measurements (see boost.rs).
    pub boost: crate::boost::Boost,
    /// Towers seen, handovers, tower locations and home location (see towers.rs).
    pub towers: crate::towers::Towers,
}

impl AppState {
    pub fn new(cfg: Config) -> Shared {
        let (tx, _) = broadcast::channel(512);
        let sources = Sources {
            router: RouterState { host: cfg.router_ip.clone(), model: cfg.router_model.clone(), ..Default::default() },
            ..Default::default()
        };
        Arc::new(Self {
            cfg,
            started: Instant::now(),
            tx,
            inner: RwLock::new(Inner {
                telemetry: None,
                history: VecDeque::new(),
                devices: Vec::new(),
                phone: None,
                events: VecDeque::new(),
                sources,
                interval_s: 1.0,
            }),
            scan_now: Notify::new(),
            relogin: AtomicBool::new(false),
            oui: OuiDb::load(),
            boost: crate::boost::Boost::new(),
            towers: crate::towers::Towers::new(),
        })
    }

    pub fn send(&self, v: Value) {
        let _ = self.tx.send(v.to_string());
    }

    pub async fn event(&self, kind: &str, message: impl Into<String>, data: Value) {
        let message = message.into();
        println!("{} [{kind}] {message}", chrono::Local::now().format("%H:%M:%S"));
        let ev = json!({ "type": "event", "ts": now_ms(), "kind": kind, "message": message, "data": data });
        {
            let mut i = self.inner.write().await;
            i.events.push_back(ev.clone());
            while i.events.len() > EVENTS_MAX {
                i.events.pop_front();
            }
        }
        self.send(ev);
    }

    pub async fn send_status(&self) {
        let s = self.inner.read().await.sources.clone();
        self.send(json!({ "type": "status", "ts": now_ms(), "data": s }));
    }

    /// Everything a freshly connected WebSocket client needs to draw the UI.
    pub async fn snapshot(&self) -> Vec<Value> {
        let i = self.inner.read().await;
        let ts = now_ms();
        let mut out = vec![json!({ "type": "status", "ts": ts, "data": i.sources })];
        if let Some(t) = &i.telemetry {
            out.push(json!({ "type": "telemetry", "ts": ts, "data": t }));
        }
        out.push(json!({ "type": "devices", "ts": ts, "data": i.devices }));
        if let Some(p) = &i.phone {
            out.push(json!({ "type": "phone", "ts": ts, "data": p }));
        }
        let skip = i.events.len().saturating_sub(50);
        out.extend(i.events.iter().skip(skip).cloned());
        out
    }
}
