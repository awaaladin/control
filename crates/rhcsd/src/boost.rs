//! Game Boost: measures every internet link of this machine (TCP connect time,
//! jitter, loss) and recommends the one with the lowest, steadiest ping.
//!
//! Switching the default link needs admin rights (switch.rs): with rhcsd
//! elevated, PULSE switches in one click, and Auto mode keeps measuring
//! gently and switches by itself when another link is clearly and steadily
//! better. Without admin, the recommendation carries the command to run.
//! It cannot make the radio signal stronger.
//!
//! No ICMP (that needs admin): a probe is one TCP handshake to a target,
//! sent from a socket bound to the link's local address.

use crate::links::{self, Link, LinkKind};
use crate::state::{now_ms, Shared};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub const DEFAULT_TARGETS: &str = "1.1.1.1:443,8.8.8.8:443,9.9.9.9:443,1.1.1.1:53,8.8.8.8:53";
pub const DEFAULT_PROBES: u32 = 20;
pub const PROBE_GAP: Duration = Duration::from_millis(200);
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(1200);
/// Pause between runs in watch mode.
pub const WATCH_GAP: Duration = Duration::from_secs(10);
/// Stop measuring a link after this many rounds in a row with no answer.
const DEAD_ROUNDS: usize = 4;
const RECENT_MAX: usize = 60;

// Auto-switch: gentle measuring and hysteresis so it never flaps.
/// Probes per link in one auto measurement (about 1 s of tiny TCP connects).
pub const AUTO_PROBES: u32 = 5;
/// Pause between auto measurements.
pub const AUTO_GAP: Duration = Duration::from_secs(12);
/// Rounds in a row another link must be clearly better before switching.
pub const AUTO_ROUNDS: u32 = 3;
/// "Clearly better": at least this much lower score (ms) …
pub const AUTO_MARGIN_MS: f64 = 15.0;
/// … and at least this fraction lower.
pub const AUTO_MARGIN_PCT: f64 = 0.20;
/// Never switch more often than this.
pub const AUTO_MIN_GAP_MS: i64 = 120_000;

#[derive(Clone, Debug, Serialize)]
pub struct TargetStat {
    pub target: String,
    pub sent: u32,
    pub received: u32,
    pub refused: u32,
    pub median_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct LinkStats {
    pub sent: u32,
    pub received: u32,
    pub loss_pct: Option<f64>,
    /// Typical ping: median over targets of each target's median RTT.
    pub ping_ms: Option<f64>,
    /// Mean change between consecutive pings to the same target, averaged over targets.
    pub jitter_ms: Option<f64>,
    pub best_ms: Option<f64>,
    pub p90_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Radio {
    /// "router" or "phone"
    pub source: String,
    pub rsrp: Option<f32>,
    pub sinr: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LinkResult {
    #[serde(flatten)]
    pub link: Link,
    #[serde(flatten)]
    pub stats: LinkStats,
    pub targets: Vec<TargetStat>,
    pub radio: Option<Radio>,
    /// Lower is better; ping-equivalent milliseconds. None = no answers.
    pub score: Option<f64>,
    /// excellent | good | fair | poor | none, for gaming.
    pub grade: String,
    pub verdict: String,
    /// Per-round ping (median across targets), oldest first; null = all lost.
    pub recent: Vec<Option<f64>>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Recommendation {
    pub best: String,
    pub best_name: String,
    pub headline: String,
    pub explanation: String,
    /// The best link is already the one Windows uses for the internet.
    pub already_default: bool,
    /// The top two links are within a few ms of each other.
    pub close_call: bool,
    /// Admin PowerShell commands that would make the best link the default.
    /// rhcsd never runs these.
    pub commands: Vec<String>,
}

/// One auto-switch decision, kept for the UI.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Decision {
    pub ts: i64,
    /// stay | wait | hold | switch | error
    pub action: String,
    pub message: String,
    /// Link id (local address) switched to / favoured.
    pub to: Option<String>,
    pub to_alias: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct AutoView {
    pub enabled: bool,
    pub last: Option<Decision>,
    pub last_switch: Option<i64>,
}

/// Hysteresis memory for auto-switch.
#[derive(Clone, Debug, Default)]
pub struct AutoMemory {
    pub streak_id: Option<String>,
    pub streak: u32,
    pub last_switch: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BoostState {
    /// idle | measuring | done | error
    pub phase: String,
    pub watch: bool,
    pub started: Option<i64>,
    pub finished: Option<i64>,
    pub done: u32,
    pub total: u32,
    pub probes: u32,
    pub interval_ms: u64,
    pub targets: Vec<String>,
    pub links: Vec<LinkResult>,
    pub recommendation: Option<Recommendation>,
    pub error: Option<String>,
    pub auto: AutoView,
}

pub struct Boost {
    pub state: RwLock<BoostState>,
    running: AtomicBool,
    watch: AtomicBool,
    auto: AtomicBool,
    auto_task: AtomicBool,
    auto_mem: tokio::sync::Mutex<AutoMemory>,
    targets: Vec<String>,
}

impl Boost {
    pub fn new() -> Self {
        let e = rhcs_core::env::load();
        let raw = e.get("BOOST_TARGETS").filter(|v| !v.trim().is_empty()).cloned();
        let targets = parse_targets(raw.as_deref().unwrap_or(DEFAULT_TARGETS));
        let targets = if targets.is_empty() { parse_targets(DEFAULT_TARGETS) } else { targets };
        Self {
            state: RwLock::new(BoostState {
                phase: "idle".into(),
                probes: DEFAULT_PROBES,
                interval_ms: PROBE_GAP.as_millis() as u64,
                targets: targets.clone(),
                ..Default::default()
            }),
            running: AtomicBool::new(false),
            watch: AtomicBool::new(false),
            auto: AtomicBool::new(false),
            auto_task: AtomicBool::new(false),
            auto_mem: tokio::sync::Mutex::new(AutoMemory::default()),
            targets,
        }
    }
}

// ------------------------------------------------------------------ parsing

/// `host:port` entries, comma separated. Bad entries are dropped.
pub fn parse_targets(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|t| {
            t.rsplit_once(':')
                .is_some_and(|(h, p)| !h.is_empty() && !h.contains(' ') && p.parse::<u16>().is_ok_and(|p| p > 0))
        })
        .map(str::to_string)
        .collect()
}

async fn resolve(target: &str) -> Option<SocketAddr> {
    if let Ok(sa) = target.parse::<SocketAddr>() {
        return sa.is_ipv4().then_some(sa);
    }
    tokio::net::lookup_host(target).await.ok()?.find(|a| a.is_ipv4())
}

// ------------------------------------------------------------------ statistics

pub fn median(v: &[f64]) -> Option<f64> {
    percentile(v, 50.0)
}

/// Linear-interpolated percentile (0-100).
pub fn percentile(v: &[f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let rank = (p / 100.0).clamp(0.0, 1.0) * (s.len() - 1) as f64;
    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
    Some(s[lo] + (s[hi] - s[lo]) * (rank - lo as f64))
}

/// Mean absolute change between consecutive samples (what a game feels as
/// "jumpy" ping). Needs at least two samples.
pub fn jitter(series: &[f64]) -> Option<f64> {
    if series.len() < 2 {
        return None;
    }
    let sum: f64 = series.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    Some(sum / (series.len() - 1) as f64)
}

/// `rounds[r][t]` = RTT of round r to target t (None = lost).
pub fn summarize(rounds: &[Vec<Option<f64>>], n_targets: usize) -> (LinkStats, Vec<(u32, Option<f64>, Option<f64>)>) {
    let mut per_target = Vec::new();
    let mut all = Vec::new();
    let mut medians = Vec::new();
    let mut jitters = Vec::new();
    let (mut sent, mut received) = (0u32, 0u32);
    for t in 0..n_targets {
        let series: Vec<f64> = rounds.iter().filter_map(|r| r.get(t).copied().flatten()).collect();
        let tsent = rounds.iter().filter(|r| r.len() > t).count() as u32;
        sent += tsent;
        received += series.len() as u32;
        let m = median(&series);
        let j = jitter(&series);
        if let Some(m) = m {
            medians.push(m);
        }
        if let Some(j) = j {
            jitters.push(j);
        }
        all.extend_from_slice(&series);
        per_target.push((series.len() as u32, m, j));
    }
    let stats = LinkStats {
        sent,
        received,
        loss_pct: (sent > 0).then(|| 100.0 * (sent - received) as f64 / sent as f64),
        ping_ms: median(&medians),
        jitter_ms: (!jitters.is_empty()).then(|| jitters.iter().sum::<f64>() / jitters.len() as f64),
        best_ms: all.iter().copied().reduce(f64::min),
        p90_ms: percentile(&all, 90.0),
    };
    (stats, per_target)
}

/// Ping-equivalent cost in ms: ping + 2×jitter + 10 ms per 1 % loss, plus a
/// small penalty when the radio is noisy (SINR below 13 dB), because a noisy
/// link tends to get worse under load even if it measured fine just now.
pub fn score(s: &LinkStats, sinr: Option<f32>) -> Option<f64> {
    let ping = s.ping_ms?;
    let jit = s.jitter_ms.unwrap_or(0.0);
    let loss = s.loss_pct.unwrap_or(0.0);
    let radio = sinr.map(|v| (13.0 - v as f64).clamp(0.0, 20.0)).unwrap_or(0.0);
    Some(ping + 2.0 * jit + 10.0 * loss + radio)
}

/// (grade, verdict) for online games.
pub fn game_grade(s: &LinkStats) -> (&'static str, &'static str) {
    let (Some(p), Some(loss)) = (s.ping_ms, s.loss_pct) else {
        return ("none", "No answers: no internet through this link");
    };
    let j = s.jitter_ms.unwrap_or(0.0);
    if p <= 50.0 && j <= 8.0 && loss < 1.0 {
        ("excellent", "Great for online games")
    } else if p <= 90.0 && j <= 15.0 && loss < 2.0 {
        ("good", "Good for online games")
    } else if p <= 150.0 && j <= 30.0 && loss < 5.0 {
        ("fair", "Playable, expect some lag")
    } else {
        ("poor", "Laggy: expect rubber-banding")
    }
}

fn ms(v: Option<f64>) -> String {
    v.map(|v| format!("{:.0} ms", v)).unwrap_or_else(|| "—".into())
}

fn lan_only(alias: &str, undo: bool) -> String {
    format!(
        "powershell -ExecutionPolicy Bypass -File scripts\\lan-only.ps1 -Interface \"{alias}\"{}",
        if undo { " -Undo" } else { "" }
    )
}

/// Picks the best link and explains why. None when nothing answered.
pub fn recommend(results: &[LinkResult]) -> Option<Recommendation> {
    let mut ranked: Vec<&LinkResult> = results.iter().filter(|r| r.score.is_some()).collect();
    ranked.sort_by(|a, b| a.score.unwrap_or(f64::MAX).total_cmp(&b.score.unwrap_or(f64::MAX)));
    let best = *ranked.first()?;
    let runner = ranked.get(1).copied();
    let b = &best.stats;

    let (headline, close_call) = match runner {
        Some(r) => {
            let s = &r.stats;
            let close = (r.score.unwrap_or(0.0) - best.score.unwrap_or(0.0)) < 5.0_f64.max(0.1 * best.score.unwrap_or(0.0));
            (
                format!(
                    "Use {}: {} vs {}, jitter {} vs {}",
                    best.link.name,
                    ms(b.ping_ms),
                    ms(s.ping_ms),
                    ms(b.jitter_ms),
                    ms(s.jitter_ms)
                ),
                close,
            )
        }
        None => (format!("Use {}: {} ping, jitter {}", best.link.name, ms(b.ping_ms), ms(b.jitter_ms)), false),
    };

    let mut why = Vec::new();
    match runner {
        None if results.len() <= 1 => why.push(format!(
            "{} is the only internet link this computer has right now, so there is nothing to compare. Connect another one too (router Wi-Fi, phone USB tethering or hotspot) and run again.",
            best.link.name
        )),
        None => why.push(format!("{} is the only link that answered. The others had no internet.", best.link.name)),
        Some(r) if close_call => why.push(format!(
            "{} and {} are about the same. Switching would not make a noticeable difference.",
            best.link.name, r.link.name
        )),
        Some(r) => {
            why.push(format!(
                "{} answered faster and more steadily than {}.",
                best.link.name, r.link.name
            ));
            if r.stats.loss_pct.unwrap_or(0.0) >= 1.0 && r.stats.loss_pct > b.loss_pct {
                why.push(format!("{} also lost {:.0} % of probes.", r.link.name, r.stats.loss_pct.unwrap_or(0.0)));
            }
        }
    }
    if let Some(sinr) = best.radio.as_ref().and_then(|r| r.sinr) {
        if sinr < 5.0 {
            why.push(format!(
                "Its radio signal is noisy (SINR {sinr:.0} dB), so ping may jump during busy hours. Aim mode can help find a better spot."
            ));
        }
    }
    let unanswered: Vec<&str> = results.iter().filter(|r| r.score.is_none()).map(|r| r.link.name.as_str()).collect();
    if !unanswered.is_empty() {
        why.push(format!("No internet through: {}.", unanswered.join(", ")));
    }

    let already_default = best.link.is_default;
    let mut commands = Vec::new();
    if already_default {
        why.push("Windows already sends your internet through this link: nothing to change.".into());
    } else if !close_call {
        why.push(
            "Windows is sending your internet through another link. To switch, run these once in an admin PowerShell in the rhcs folder (undo with -Undo). rhcsd does not run them for you."
                .to_string(),
        );
        for r in results {
            if r.link.id != best.link.id && r.link.adapter != best.link.adapter && r.link.gateway.is_some() {
                let cmd = lan_only(&r.link.adapter, false);
                if !commands.contains(&cmd) {
                    commands.push(cmd);
                }
            }
        }
        if best.link.metric.is_some_and(|m| m >= 500) {
            commands.push(lan_only(&best.link.adapter, true));
        }
    }

    Some(Recommendation {
        best: best.link.id.clone(),
        best_name: best.link.name.clone(),
        headline,
        explanation: why.join(" "),
        already_default,
        close_call,
        commands,
    })
}

/// Auto-switch decision after one measurement (pure, unit-tested). Switches
/// only when another link beats the one in use by ≥ 15 ms and ≥ 20 % score
/// for 3 measurements in a row, and not within 2 minutes of the last switch.
pub fn decide(mem: &mut AutoMemory, results: &[LinkResult], now: i64) -> Decision {
    let d = |action: &str, message: String, to: Option<&LinkResult>| Decision {
        ts: now,
        action: action.into(),
        message,
        to: to.map(|r| r.link.id.clone()),
        to_alias: to.map(|r| r.link.adapter.clone()),
    };
    let best = results
        .iter()
        .filter(|r| r.score.is_some())
        .min_by(|a, b| a.score.unwrap_or(f64::MAX).total_cmp(&b.score.unwrap_or(f64::MAX)));
    let Some(best) = best else {
        mem.streak = 0;
        mem.streak_id = None;
        return d("stay", "No link answered: nothing to switch to.".into(), None);
    };
    let Some(cur) = results.iter().find(|r| r.link.is_default) else {
        mem.streak = 0;
        mem.streak_id = None;
        return d("stay", "Can't tell which link Windows is using right now; not switching.".into(), None);
    };
    if cur.link.id == best.link.id {
        mem.streak = 0;
        mem.streak_id = None;
        return d("stay", format!("{} is in use and is the best link ({}).", cur.link.name, ms(cur.stats.ping_ms)), None);
    }
    let bs = best.score.unwrap_or(f64::MAX);
    let clearly = match cur.score {
        None => true, // the link in use has no internet
        Some(cs) => cs - bs >= AUTO_MARGIN_MS && bs <= cs * (1.0 - AUTO_MARGIN_PCT),
    };
    if !clearly {
        mem.streak = 0;
        mem.streak_id = None;
        return d(
            "stay",
            format!(
                "{} ({}) is only a little better than {} ({}): not worth switching.",
                best.link.name,
                ms(best.stats.ping_ms),
                cur.link.name,
                ms(cur.stats.ping_ms)
            ),
            None,
        );
    }
    if mem.streak_id.as_deref() == Some(best.link.id.as_str()) {
        mem.streak += 1;
    } else {
        mem.streak_id = Some(best.link.id.clone());
        mem.streak = 1;
    }
    let vs = format!("{} vs {}", ms(best.stats.ping_ms), if cur.score.is_some() { ms(cur.stats.ping_ms) } else { "no answer".into() });
    if mem.streak < AUTO_ROUNDS {
        return d("wait", format!("{} is better ({vs}), {}/{} checks in a row.", best.link.name, mem.streak, AUTO_ROUNDS), Some(best));
    }
    if let Some(last) = mem.last_switch.filter(|t| now - t < AUTO_MIN_GAP_MS) {
        let left = (AUTO_MIN_GAP_MS - (now - last)) / 1000;
        return d("hold", format!("{} is better ({vs}), but the last switch was less than 2 minutes ago ({left} s to wait).", best.link.name), Some(best));
    }
    mem.last_switch = Some(now);
    mem.streak = 0;
    mem.streak_id = None;
    d("switch", format!("switched to {}: {vs}", best.link.name), Some(best))
}

// ------------------------------------------------------------------ measuring

#[derive(Debug, Clone, Copy, PartialEq)]
enum Probe {
    Rtt(f64),
    Refused,
    Lost,
}

async fn probe(local: Ipv4Addr, target: SocketAddr) -> Probe {
    let Ok(sock) = tokio::net::TcpSocket::new_v4() else { return Probe::Lost };
    if sock.bind(SocketAddr::new(IpAddr::V4(local), 0)).is_err() {
        return Probe::Lost;
    }
    let t0 = Instant::now();
    match tokio::time::timeout(PROBE_TIMEOUT, sock.connect(target)).await {
        Ok(Ok(stream)) => {
            let rtt = t0.elapsed().as_secs_f64() * 1000.0;
            drop(stream);
            Probe::Rtt(rtt)
        }
        // Windows retries a refused SYN, so its timing is not a clean RTT.
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => Probe::Refused,
        _ => Probe::Lost,
    }
}

fn radio_for(link: &Link, i: &crate::state::Inner) -> Option<Radio> {
    match link.kind {
        LinkKind::Router if i.sources.router.online => i
            .telemetry
            .as_ref()
            .map(|t| Radio { source: "router".into(), rsrp: t.rsrp, sinr: t.sinr }),
        LinkKind::PhoneUsb | LinkKind::PhoneHotspot if i.sources.phone.online => i
            .phone
            .as_ref()
            .map(|p| Radio { source: "phone".into(), rsrp: p.rsrp, sinr: p.sinr }),
        _ => None,
    }
}


fn result_for(link: &Link, rounds: &[Vec<Probe>], targets: &[String], radio: Option<Radio>, error: Option<String>) -> LinkResult {
    let rtts: Vec<Vec<Option<f64>>> = rounds
        .iter()
        .map(|r| r.iter().map(|p| if let Probe::Rtt(v) = p { Some(*v) } else { None }).collect())
        .collect();
    let (stats, per) = summarize(&rtts, targets.len());
    let tstats = targets
        .iter()
        .enumerate()
        .map(|(t, name)| TargetStat {
            target: name.clone(),
            sent: rounds.iter().filter(|r| r.len() > t).count() as u32,
            received: per[t].0,
            refused: rounds.iter().filter(|r| r.get(t) == Some(&Probe::Refused)).count() as u32,
            median_ms: per[t].1,
            jitter_ms: per[t].2,
        })
        .collect();
    let recent: Vec<Option<f64>> = rtts
        .iter()
        .map(|r| median(&r.iter().flatten().copied().collect::<Vec<_>>()))
        .collect();
    let recent = recent[recent.len().saturating_sub(RECENT_MAX)..].to_vec();
    let sinr = radio.as_ref().and_then(|r| r.sinr);
    let (grade, verdict) = if rounds.is_empty() && error.is_none() { ("none", "Measuring…") } else { game_grade(&stats) };
    LinkResult {
        link: link.clone(),
        score: score(&stats, sinr),
        stats,
        targets: tstats,
        radio,
        grade: grade.into(),
        verdict: verdict.into(),
        recent,
        error,
    }
}

async fn publish(s: &Shared) {
    let mut v = json!(*s.boost.state.read().await);
    v["running"] = json!(s.boost.running.load(Ordering::SeqCst));
    v["admin"] = json!(s.admin);
    s.send(json!({ "type": "boost", "ts": now_ms(), "data": v }));
}

async fn measure_link(s: Shared, idx: usize, link: Link, addrs: Vec<Option<SocketAddr>>, names: Vec<String>, probes: u32) {
    let mut rounds: Vec<Vec<Probe>> = Vec::new();
    let mut dead = 0usize;
    for _ in 0..probes {
        let t0 = Instant::now();
        let mut set = tokio::task::JoinSet::new();
        for (t, a) in addrs.iter().enumerate() {
            let a = *a;
            let local = link.local_ip;
            set.spawn(async move { (t, if let Some(a) = a { probe(local, a).await } else { Probe::Lost }) });
        }
        let mut round = vec![Probe::Lost; addrs.len()];
        while let Some(Ok((t, p))) = set.join_next().await {
            round[t] = p;
        }
        dead = if round.iter().any(|p| matches!(p, Probe::Rtt(_))) { 0 } else { dead + 1 };
        rounds.push(round);

        let radio = radio_for(&link, &*s.inner.read().await);
        let stop = dead >= DEAD_ROUNDS;
        let err = stop.then(|| format!("no answer in {DEAD_ROUNDS} rounds: this link has no internet right now"));
        {
            let mut st = s.boost.state.write().await;
            st.done += 1;
            if stop {
                st.done += probes - rounds.len() as u32;
            }
            if let Some(slot) = st.links.get_mut(idx) {
                *slot = result_for(&link, &rounds, &names, radio, err);
            }
        }
        publish(&s).await;
        if stop {
            break;
        }
        if let Some(rest) = PROBE_GAP.checked_sub(t0.elapsed()) {
            tokio::time::sleep(rest).await;
        }
    }
}

/// One full measurement of every link. Assumes `running` is held.
async fn run_once(s: &Shared, probes: u32) {
    let targets = s.boost.targets.clone();
    let links = links::discover(s).await;
    let mut addrs = Vec::new();
    for t in &targets {
        addrs.push(resolve(t).await);
    }
    {
        let i = s.inner.read().await;
        let mut st = s.boost.state.write().await;
        st.phase = "measuring".into();
        st.started = Some(now_ms());
        st.finished = None;
        st.error = None;
        st.done = 0;
        st.probes = probes;
        st.total = probes * links.len() as u32;
        st.targets = targets.clone();
        st.recommendation = None;
        st.links = links.iter().map(|l| result_for(l, &[], &targets, radio_for(l, &i), None)).collect();
        if links.is_empty() {
            st.phase = "error".into();
            st.error = Some("No internet link found: this computer has no network adapter with a gateway.".into());
        } else if addrs.iter().all(Option::is_none) {
            st.phase = "error".into();
            st.error = Some("None of the test targets could be resolved. Check BOOST_TARGETS in .env (host:port, comma separated).".into());
        }
    }
    publish(s).await;
    if s.boost.state.read().await.phase == "error" {
        s.boost.state.write().await.finished = Some(now_ms());
        return;
    }

    let mut set = tokio::task::JoinSet::new();
    for (idx, link) in links.into_iter().enumerate() {
        set.spawn(measure_link(s.clone(), idx, link, addrs.clone(), targets.clone(), probes));
    }
    while set.join_next().await.is_some() {}

    {
        let mut st = s.boost.state.write().await;
        st.recommendation = recommend(&st.links);
        st.phase = "done".into();
        st.finished = Some(now_ms());
        st.done = st.total;
        if let Some(r) = &st.recommendation {
            println!("{} [boost] {}", chrono::Local::now().format("%H:%M:%S"), r.headline);
        }
    }
    publish(s).await;
}

/// Starts a measurement in the background (or watch mode). False if one is running.
pub fn start(s: &Shared, probes: u32, watch: bool) -> bool {
    s.boost.watch.store(watch, Ordering::SeqCst);
    if s.boost.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return false;
    }
    let s = s.clone();
    tokio::spawn(async move {
        loop {
            s.boost.state.write().await.watch = s.boost.watch.load(Ordering::SeqCst);
            run_once(&s, probes).await;
            if !s.boost.watch.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(WATCH_GAP).await;
            if !s.boost.watch.load(Ordering::SeqCst) {
                break;
            }
        }
        s.boost.state.write().await.watch = false;
        s.boost.running.store(false, Ordering::SeqCst);
        publish(&s).await;
    });
    true
}

/// Turns auto-switch on or off. On starts the gentle measuring loop.
pub async fn set_auto(s: &Shared, on: bool) {
    s.boost.auto.store(on, Ordering::SeqCst);
    s.boost.state.write().await.auto.enabled = on;
    if on && s.boost.auto_task.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        let s = s.clone();
        tokio::spawn(async move {
            auto_loop(&s).await;
            s.boost.auto_task.store(false, Ordering::SeqCst);
        });
    }
    publish(s).await;
}

async fn auto_loop(s: &Shared) {
    while s.boost.auto.load(Ordering::SeqCst) {
        // A manual run (or watch) in progress already measures; skip this turn.
        if s.boost.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            run_once(s, AUTO_PROBES).await;
            s.boost.running.store(false, Ordering::SeqCst);
            if !s.boost.auto.load(Ordering::SeqCst) {
                break;
            }
            let results = s.boost.state.read().await.links.clone();
            let mut decision = {
                let mut mem = s.boost.auto_mem.lock().await;
                decide(&mut mem, &results, now_ms())
            };
            if decision.action == "switch" {
                let alias = decision.to_alias.clone().unwrap_or_default();
                match crate::switch::switch_to(s, &alias).await {
                    Ok(_) => {
                        s.event("link_switch", decision.message.clone(), json!({ "alias": alias, "auto": true })).await;
                        s.boost.state.write().await.auto.last_switch = Some(decision.ts);
                    }
                    Err(e) => {
                        decision.action = "error".into();
                        decision.message = format!("could not switch: {e}");
                        s.event("error", format!("auto-switch: {e}"), json!({})).await;
                        if !s.admin {
                            s.boost.auto.store(false, Ordering::SeqCst);
                        }
                    }
                }
            }
            {
                let mut st = s.boost.state.write().await;
                st.auto.enabled = s.boost.auto.load(Ordering::SeqCst);
                st.auto.last = Some(decision);
            }
            publish(s).await;
        }
        tokio::time::sleep(AUTO_GAP).await;
    }
    s.boost.state.write().await.auto.enabled = false;
    publish(s).await;
}

// ------------------------------------------------------------------ API

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/boost", get(get_boost))
        .route("/api/boost/run", post(run))
        .route("/api/boost/stop", post(stop))
        .route("/api/boost/auto", post(auto))
        .route("/api/signal/hourly", get(hourly))
}

#[derive(Deserialize)]
struct AutoRequest {
    enabled: bool,
}

async fn auto(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<AutoRequest>,
) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    if req.enabled && !crate::switch::supported() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "message": "auto-switch is only supported on Windows", "admin": s.admin })),
        )
            .into_response();
    }
    if req.enabled && !s.admin {
        return (
            axum::http::StatusCode::FORBIDDEN,
            Json(json!({ "ok": false, "message": crate::switch::ADMIN_HELP, "admin": false })),
        )
            .into_response();
    }
    set_auto(&s, req.enabled).await;
    let message = if req.enabled {
        "auto-switch on: measuring every 12 s, switching only when another link is clearly better 3 times in a row"
    } else {
        "auto-switch off"
    };
    Json(json!({ "ok": true, "message": message })).into_response()
}

/// Router signal history (`ts`, SINR, RSRP, download B/s) averaged per local
/// hour of day. `utc_offset_s` shifts timestamps to local time.
pub fn hourly_buckets(rows: impl Iterator<Item = (i64, Option<f32>, Option<f32>, Option<u64>)>, utc_offset_s: i64) -> Value {
    #[derive(Default, Clone, Copy)]
    struct B {
        n: u64,
        sinr: (f64, u64),
        rsrp: (f64, u64),
        rx: (f64, u64),
    }
    let mut b = [B::default(); 24];
    let (mut first, mut last) = (i64::MAX, i64::MIN);
    for (ts, sinr, rsrp, rx) in rows {
        first = first.min(ts);
        last = last.max(ts);
        let h = ((ts.div_euclid(1000) + utc_offset_s).div_euclid(3600)).rem_euclid(24) as usize;
        let x = &mut b[h];
        x.n += 1;
        if let Some(v) = sinr {
            x.sinr = (x.sinr.0 + v as f64, x.sinr.1 + 1);
        }
        if let Some(v) = rsrp {
            x.rsrp = (x.rsrp.0 + v as f64, x.rsrp.1 + 1);
        }
        if let Some(v) = rx {
            x.rx = (x.rx.0 + v as f64, x.rx.1 + 1);
        }
    }
    let avg = |(sum, n): (f64, u64)| (n > 0).then(|| sum / n as f64);
    let hours: Vec<Value> = b
        .iter()
        .enumerate()
        .map(|(h, x)| json!({ "hour": h, "samples": x.n, "avg_sinr": avg(x.sinr), "avg_rsrp": avg(x.rsrp), "avg_rx_bps": avg(x.rx) }))
        .collect();
    let covered_h = if last >= first { (last - first) as f64 / 3_600_000.0 } else { 0.0 };
    json!({ "hours": hours, "covered_h": covered_h })
}

async fn hourly(State(s): State<Shared>) -> Json<Value> {
    let offset = chrono::Local::now().offset().local_minus_utc() as i64;
    let i = s.inner.read().await;
    Json(hourly_buckets(i.history.iter().map(|r| (r.ts, r.sinr, r.rsrp, r.rx_bytes_per_s)), offset))
}

async fn get_boost(State(s): State<Shared>) -> Json<Value> {
    let mut v = json!(*s.boost.state.read().await);
    v["running"] = json!(s.boost.running.load(Ordering::SeqCst));
    v["admin"] = json!(s.admin);
    v["switch"] = crate::switch::status(&s).await;
    Json(v)
}

#[derive(Deserialize, Default)]
struct RunRequest {
    probes: Option<u32>,
    watch: Option<bool>,
}

async fn run(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Option<Json<RunRequest>>,
) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    let req = body.map(|b| b.0).unwrap_or_default();
    let probes = req.probes.unwrap_or(DEFAULT_PROBES).clamp(5, 100);
    let watch = req.watch.unwrap_or(false);
    if start(&s, probes, watch) {
        Json(json!({ "ok": true, "message": if watch { "game boost watch started" } else { "game boost measurement started" } }))
            .into_response()
    } else {
        Json(json!({ "ok": true, "message": "a measurement is already running" })).into_response()
    }
}

async fn stop(State(s): State<Shared>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    s.boost.watch.store(false, Ordering::SeqCst);
    s.boost.state.write().await.watch = false;
    Json(json!({ "ok": true, "message": "watch mode off; the current measurement finishes on its own" })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str, ip: &str, default: bool, adapter: &str) -> Link {
        Link {
            id: ip.into(),
            name: name.into(),
            kind: LinkKind::Other,
            medium: "Wi-Fi".into(),
            adapter: adapter.into(),
            description: String::new(),
            local_ip: ip.parse().unwrap(),
            gateway: Some("10.9.9.1".parse().unwrap()),
            metric: Some(25),
            is_default: default,
            if_index: 3,
            ssid: None,
        }
    }

    fn res(l: Link, ping: f64, jit: f64, loss: f64) -> LinkResult {
        let stats = LinkStats {
            sent: 100,
            received: (100.0 - loss) as u32,
            loss_pct: Some(loss),
            ping_ms: Some(ping),
            jitter_ms: Some(jit),
            best_ms: Some(ping),
            p90_ms: Some(ping + jit),
        };
        let (g, v) = game_grade(&stats);
        LinkResult {
            link: l,
            score: score(&stats, None),
            stats,
            targets: vec![],
            radio: None,
            grade: g.into(),
            verdict: v.into(),
            recent: vec![],
            error: None,
        }
    }

    #[test]
    fn hourly_buckets_use_local_hour() {
        let h = 3_600_000i64;
        let rows = vec![
            (0, Some(10.0), Some(-90.0), Some(1000)),
            (1000, Some(20.0), None, None),
            (5 * h + 10, Some(4.0), Some(-100.0), Some(500)),
        ];
        let v = hourly_buckets(rows.clone().into_iter(), 0);
        assert_eq!(v["hours"][0]["samples"], json!(2));
        assert_eq!(v["hours"][0]["avg_sinr"], json!(15.0));
        assert_eq!(v["hours"][0]["avg_rsrp"], json!(-90.0));
        assert_eq!(v["hours"][5]["avg_rx_bps"], json!(500.0));
        assert!(v["hours"][3]["avg_sinr"].is_null());
        assert!((v["covered_h"].as_f64().unwrap() - 5.0).abs() < 0.01);
        // UTC+1: midnight UTC is 01:00 local.
        let v = hourly_buckets(rows.into_iter(), 3600);
        assert_eq!(v["hours"][1]["samples"], json!(2));
        assert_eq!(v["hours"][6]["samples"], json!(1));
        assert_eq!(hourly_buckets(std::iter::empty(), 0)["covered_h"], json!(0.0));
    }

    #[test]
    fn targets_parse() {
        assert_eq!(
            parse_targets(" 1.1.1.1:53, example.com:443 ,bad, :80, a b:1, x:0, x:99999"),
            vec!["1.1.1.1:53", "example.com:443"]
        );
        assert_eq!(parse_targets(DEFAULT_TARGETS).len(), 5);
    }

    #[test]
    fn stats_maths() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(median(&[]), None);
        assert_eq!(percentile(&[10.0, 20.0, 30.0, 40.0, 50.0], 90.0), Some(46.0));
        assert_eq!(jitter(&[10.0, 14.0, 12.0, 12.0]), Some(2.0));
        assert_eq!(jitter(&[10.0]), None);
    }

    #[test]
    fn summarize_two_targets_with_loss() {
        // target 0 ≈ 20 ms, target 1 ≈ 60 ms, one loss on target 1.
        let rounds = vec![
            vec![Some(20.0), Some(60.0)],
            vec![Some(22.0), None],
            vec![Some(20.0), Some(64.0)],
            vec![Some(22.0), Some(60.0)],
        ];
        let (s, per) = summarize(&rounds, 2);
        assert_eq!(s.sent, 8);
        assert_eq!(s.received, 7);
        assert!((s.loss_pct.unwrap() - 12.5).abs() < 1e-9);
        // per-target medians 21 and 60 -> link ping 40.5
        assert_eq!(per[0].1, Some(21.0));
        assert_eq!(per[1].1, Some(60.0));
        assert_eq!(s.ping_ms, Some(40.5));
        // jitter: t0 = (2+2+2)/3 = 2, t1 = (4+4)/2 = 4 -> mean 3. Not polluted by the 40 ms gap between targets.
        assert_eq!(s.jitter_ms, Some(3.0));
        assert_eq!(s.best_ms, Some(20.0));
    }

    #[test]
    fn all_lost_has_no_score() {
        let rounds = vec![vec![None, None]; 3];
        let (s, _) = summarize(&rounds, 2);
        assert_eq!(s.loss_pct, Some(100.0));
        assert_eq!(s.ping_ms, None);
        assert_eq!(score(&s, Some(20.0)), None);
        assert_eq!(game_grade(&s).0, "none");
    }

    #[test]
    fn scoring_penalises_jitter_loss_and_noise() {
        let calm = LinkStats { ping_ms: Some(40.0), jitter_ms: Some(2.0), loss_pct: Some(0.0), ..Default::default() };
        let jumpy = LinkStats { ping_ms: Some(35.0), jitter_ms: Some(15.0), loss_pct: Some(0.0), ..Default::default() };
        let lossy = LinkStats { ping_ms: Some(35.0), jitter_ms: Some(2.0), loss_pct: Some(3.0), ..Default::default() };
        assert!(score(&calm, None) < score(&jumpy, None));
        assert!(score(&calm, None) < score(&lossy, None));
        assert_eq!(score(&calm, Some(25.0)), Some(44.0));
        assert_eq!(score(&calm, Some(3.0)), Some(54.0));
        assert_eq!(game_grade(&calm).0, "excellent");
        assert_eq!(game_grade(&jumpy).0, "good");
        assert_eq!(game_grade(&lossy).0, "fair");
    }

    #[test]
    fn recommends_faster_link_with_command() {
        let a = res(link("Router (Wi-Fi)", "10.1.0.5", true, "Wi-Fi"), 71.0, 19.0, 0.0);
        let b = res(link("Phone (USB)", "10.2.0.5", false, "Ethernet 2"), 38.0, 4.0, 0.0);
        let r = recommend(&[a, b]).unwrap();
        assert_eq!(r.best_name, "Phone (USB)");
        assert_eq!(r.headline, "Use Phone (USB): 38 ms vs 71 ms, jitter 4 ms vs 19 ms");
        assert!(!r.already_default);
        assert!(!r.close_call);
        assert_eq!(r.commands, vec!["powershell -ExecutionPolicy Bypass -File scripts\\lan-only.ps1 -Interface \"Wi-Fi\""]);
    }

    #[test]
    fn auto_switch_hysteresis() {
        let slow = |default| res(link("Router (Wi-Fi)", "10.1.0.5", default, "WiFi"), 88.0, 5.0, 0.0);
        let fast = |default| res(link("Phone (USB)", "10.2.0.5", default, "Ethernet 2"), 41.0, 3.0, 0.0);
        let mut mem = AutoMemory::default();
        let t0 = 1_000_000;
        // Better 3 rounds in a row: wait, wait, switch.
        assert_eq!(decide(&mut mem, &[slow(true), fast(false)], t0).action, "wait");
        assert_eq!(decide(&mut mem, &[slow(true), fast(false)], t0 + 12_000).action, "wait");
        let d = decide(&mut mem, &[slow(true), fast(false)], t0 + 24_000);
        assert_eq!(d.action, "switch");
        assert_eq!(d.message, "switched to Phone (USB): 41 ms vs 88 ms");
        assert_eq!(d.to_alias.as_deref(), Some("Ethernet 2"));

        // Already on the best link: stay, streak cleared.
        assert_eq!(decide(&mut mem, &[slow(false), fast(true)], t0 + 36_000).action, "stay");

        // The other link becomes clearly better again right away: no flapping
        // within 2 minutes of the last switch, even after 3 rounds.
        let worse_fast = |default| res(link("Phone (USB)", "10.2.0.5", default, "Ethernet 2"), 120.0, 9.0, 0.0);
        let good_slow = |default| res(link("Router (Wi-Fi)", "10.1.0.5", default, "WiFi"), 40.0, 3.0, 0.0);
        for i in 0..2 {
            assert_eq!(decide(&mut mem, &[good_slow(false), worse_fast(true)], t0 + 48_000 + i * 12_000).action, "wait");
        }
        assert_eq!(decide(&mut mem, &[good_slow(false), worse_fast(true)], t0 + 72_000).action, "hold");
        assert_eq!(decide(&mut mem, &[good_slow(false), worse_fast(true)], t0 + 24_000 + AUTO_MIN_GAP_MS).action, "switch");

        // A small edge (≥15 ms but < 20 %, or ≥ 20 % but < 15 ms) never switches and resets the streak.
        let mut mem = AutoMemory::default();
        let a = res(link("A", "10.1.0.5", true, "WiFi"), 100.0, 0.0, 0.0);
        let b = res(link("B", "10.2.0.5", false, "Ethernet 2"), 84.0, 0.0, 0.0);
        assert_eq!(decide(&mut mem, &[a, b], 0).action, "stay");
        let a = res(link("A", "10.1.0.5", true, "WiFi"), 40.0, 0.0, 0.0);
        let b = res(link("B", "10.2.0.5", false, "Ethernet 2"), 28.0, 0.0, 0.0);
        assert_eq!(decide(&mut mem, &[a, b], 0).action, "stay");
        assert_eq!(mem.streak, 0);

        // The link in use has no internet: any answering link is clearly better.
        let mut dead = slow(true);
        dead.score = None;
        let mut mem = AutoMemory::default();
        let d = decide(&mut mem, &[dead, fast(false)], 0);
        assert_eq!(d.action, "wait");
        assert!(d.message.contains("no answer"));
    }

    #[test]
    fn close_call_and_already_default_need_no_command() {
        let a = res(link("Router (Wi-Fi)", "10.1.0.5", false, "Wi-Fi"), 41.0, 3.0, 0.0);
        let b = res(link("Phone (USB)", "10.2.0.5", true, "Ethernet 2"), 40.0, 3.0, 0.0);
        let r = recommend(&[a.clone(), b]).unwrap();
        assert!(r.close_call);
        assert!(r.already_default);
        assert!(r.commands.is_empty());

        let dead = res(link("Other", "10.3.0.5", false, "Wi-Fi 2"), 0.0, 0.0, 100.0);
        let mut dead = dead;
        dead.score = None;
        let r = recommend(&[a, dead.clone()]).unwrap();
        assert!(r.explanation.contains("No internet through: Other"));
        assert!(recommend(&[dead]).is_none());
    }
}
