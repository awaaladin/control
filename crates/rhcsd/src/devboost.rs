//! Game Boost for the PC itself: reversible changes that free this computer for a game, plus
//! a measured before / after of the ping. `GET /api/boost/device`, `POST /api/boost/device/start`
//! and `/stop`.
//!
//! What it never does: kill a process, stop a service, touch Defender or Windows Update, write
//! outside HKCU, or promise a stronger radio signal. Every change records its original value
//! in `<data dir>/devboost.json` BEFORE it is applied, so a crash (or a power cut) cannot leave
//! the PC altered: the next start of rhcsd puts everything back. Everything is also restored
//! when the game exits, on "Restore everything" and on a clean shutdown.
//!
//! The Windows operations sit behind the [`OsOps`] trait: [`RealOps`] runs powercfg, reg.exe,
//! PowerShell and Win32 priority calls on Windows (stubs elsewhere: `supported: false`), the
//! tests use a fake that touches nothing.

use crate::state::{now_ms, Shared};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

// ------------------------------------------------------------------ constants

/// Windows' own "High performance" power scheme.
pub const HIGH_PERF_GUID: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
/// powercfg: Wireless Adapter Settings -> Power Saving Mode (0 = Maximum Performance).
pub const WIFI_SUBGROUP: &str = "19cbb8fa-5279-450e-9fac-8a3d5fedd0c1";
pub const WIFI_SETTING: &str = "12bbebe6-58d6-4636-95bb-3217ef867c1a";
pub const GAMEBAR_KEY: &str = r"Software\Microsoft\GameBar";
pub const GAME_MODE_VALUE: &str = "AutoGameModeEnabled";
pub const TOAST_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\PushNotifications";
pub const TOAST_VALUE: &str = "ToastEnabled";
pub const QOS_POLICY: &str = "rhcs-gameboost";
pub const DSCP_EXPEDITED: u32 = 46;
pub const PRIO_HIGH: u32 = 0x80;
pub const PRIO_BELOW_NORMAL: u32 = 0x4000;
/// Probes per link in the before / after measurement (about 2.5 s) and in the live readings.
pub const MEASURE_PROBES: u32 = 12;
pub const LIVE_PROBES: u32 = 6;
/// The "after" measurement is taken this long after the changes were applied.
pub const SETTLE: Duration = Duration::from_secs(10);
pub const LIVE_GAP: Duration = Duration::from_secs(20);

/// Programs that must never get a lowered priority from this feature.
const PROTECTED: &[&str] = &[
    "system", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe", "dwm.exe", "explorer.exe",
    "rhcsd.exe", "rhcsd-keepalive.exe", "svchost.exe", "msmpeng.exe", "nissrv.exe", "securityhealthservice.exe",
    "securityhealthsystray.exe", "audiodg.exe", "fontdrvhost.exe", "registry", "memory compression",
];

static PAUSED: AtomicBool = AtomicBool::new(false);

/// True while Game Boost for the PC is active and rhcsd's own LAN scans / auto-switch pause.
pub fn scans_paused() -> bool {
    PAUSED.load(Ordering::SeqCst)
}

// ------------------------------------------------------------------ data types

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RegValue {
    Dword { value: u32 },
    Str { value: String },
}

/// How to put one change back. Written to disk before the change is applied.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Undo {
    PowerPlan { original: String },
    /// `original: None` = the value did not exist (restore deletes it).
    Registry { key: String, name: String, original: Option<RegValue> },
    /// `process` guards against a reused pid: only restored if the same program still runs there.
    Priority { pid: u32, process: String, original: u32 },
    WifiPower { scheme: String, ac: u32, dc: u32 },
    QosPolicy { policy: String },
    NetworkSwitch,
    PauseScans,
    /// Cannot be undone (flush DNS).
    Irreversible,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub id: String,
    pub title: String,
    /// What was changed, in plain words ("Power plan: Balanced → High performance").
    pub detail: String,
    pub undo: Undo,
    pub applied_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GameRef {
    pub pid: u32,
    pub name: String,
}

/// One ping reading of the link in use.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Snap {
    pub link_id: String,
    pub link_name: String,
    pub ping_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: Option<f64>,
    pub at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Verdict {
    /// Some(true) = measurably better, Some(false) = measurably worse, None = no measurable change / unknown.
    pub improved: Option<bool>,
    pub text: String,
    /// after - before (negative = lower = better). None when either side has no reading.
    pub ping_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BoostResult {
    pub before: Option<Snap>,
    pub after: Option<Snap>,
    pub verdict: Verdict,
    pub finished_at: i64,
}

/// What is persisted in devboost.json and shown by the API.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Book {
    /// Changes are applied right now (and listed in `changes`).
    pub active: bool,
    /// idle | measuring_before | applying | active | restoring | restore_failed
    pub phase: String,
    pub started_at: Option<i64>,
    pub game: Option<GameRef>,
    pub changes: Vec<Change>,
    pub before: Option<Snap>,
    pub live: Option<Snap>,
    pub last_result: Option<BoostResult>,
    /// Why the last boost ended ("restored by you", "the game closed", ...).
    pub message: Option<String>,
}

impl Book {
    fn busy(&self) -> bool {
        matches!(self.phase.as_str(), "measuring_before" | "applying" | "active" | "restoring" | "restore_failed")
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Candidate {
    pub pid: u32,
    pub name: String,
    /// Only for programs that look like games or emulators (window titles of other programs
    /// can hold private text, so they are not listed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    pub mem_mb: u64,
    pub likely_game: bool,
    /// "Android emulator" / "Steam game": a plain name heuristic, not a claim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// What the OS looks like right now (read-only), for the action list and the plan.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    pub power_plan: Option<String>,
    pub power_plan_is_high: bool,
    pub high_perf_available: bool,
    pub game_mode: Option<u32>,
    pub toast_enabled: Option<u32>,
    pub wifi_power_available: bool,
}

// ------------------------------------------------------------------ the OS seam

pub trait OsOps: Send + Sync {
    fn supported(&self) -> bool;
    fn admin(&self) -> bool;
    fn probe(&self, fresh: bool) -> Probe;
    fn candidates(&self) -> Vec<Candidate>;
    /// Image file name of a running process ("HD-Player.exe"); None if no such process.
    fn process_name(&self, pid: u32) -> Option<String>;

    fn power_active(&self) -> Result<String, String>;
    fn power_set(&self, guid: &str) -> Result<(), String>;
    fn reg_get(&self, key: &str, name: &str) -> Result<Option<RegValue>, String>;
    fn reg_set(&self, key: &str, name: &str, v: &RegValue) -> Result<(), String>;
    fn reg_delete(&self, key: &str, name: &str) -> Result<(), String>;
    fn priority_get(&self, pid: u32) -> Result<u32, String>;
    fn priority_set(&self, pid: u32, class: u32) -> Result<(), String>;
    fn wifi_power_get(&self, scheme: &str) -> Result<(u32, u32), String>;
    fn wifi_power_set(&self, scheme: &str, ac: u32, dc: u32) -> Result<(), String>;
    fn qos_add(&self, policy: &str, exe: &str) -> Result<(), String>;
    fn qos_remove(&self, policy: &str) -> Result<(), String>;
    fn flush_dns(&self) -> Result<(), String>;

    /// rhcsd's own loops (LAN scan, auto-switch) pause / resume.
    fn pause_scans(&self, on: bool);
    /// Prefer the best measured link. Ok(None) = already on it (nothing changed).
    fn switch_best(&self) -> Result<Option<String>, String>;
    fn switch_restore(&self) -> Result<(), String>;
}

// ------------------------------------------------------------------ actions

pub struct ActionDef {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub needs_admin: bool,
    pub default_on: bool,
}

/// In the order they are applied (restored in reverse).
pub const ACTIONS: &[ActionDef] = &[
    ActionDef {
        id: "power_plan_high",
        title: "High performance power plan",
        description: "Switches Windows to its High performance power plan so the CPU does not slow down while you play. Your current plan is put back afterwards. Uses more power and runs warmer.",
        needs_admin: false,
        default_on: true,
    },
    ActionDef {
        id: "game_mode_on",
        title: "Windows Game Mode",
        description: "Turns Game Mode on for your account (a per-user setting): Windows then keeps background tasks from taking CPU from the game. Put back to its old value afterwards.",
        needs_admin: false,
        default_on: true,
    },
    ActionDef {
        id: "game_priority_high",
        title: "Raise the game's priority",
        description: "Gives the program you pick a higher CPU priority (High) so the system serves it before ordinary programs. Reset to the old priority afterwards.",
        needs_admin: false,
        default_on: true,
    },
    ActionDef {
        id: "background_low_priority",
        title: "Lower the priority of programs you pick",
        description: "Gives the background programs you tick a lower CPU priority (Below normal). Nothing is closed or stopped. Reset afterwards.",
        needs_admin: false,
        default_on: true,
    },
    ActionDef {
        id: "quiet_notifications",
        title: "Quiet notifications",
        description: "Turns off pop-up notifications from apps for your account, so nothing interrupts the game. Put back to its old value afterwards.",
        needs_admin: false,
        default_on: true,
    },
    ActionDef {
        id: "pause_pulse_scans",
        title: "Pause PULSE's own scans",
        description: "PULSE's LAN scans and automatic link switching wait while the boost is on, so they cannot add traffic or move the connection during a match. They resume afterwards.",
        needs_admin: false,
        default_on: true,
    },
    ActionDef {
        id: "wifi_max_performance",
        title: "Wi-Fi adapter: maximum performance",
        description: "Sets the Wi-Fi adapter's power saving to Maximum Performance on the active power plan, which avoids small delays from the adapter dozing between packets. Best effort: some adapters ignore it. Put back afterwards.",
        needs_admin: false,
        default_on: true,
    },
    ActionDef {
        id: "network_best_link",
        title: "Use the best connection",
        description: "After measuring every connection, makes the one with the lowest, steadiest ping the one Windows uses for new connections (the same switch as Game Boost's Switch now). Your running game keeps its current connection. Undone with the rest.",
        needs_admin: true,
        default_on: true,
    },
    ActionDef {
        id: "network_dscp",
        title: "Mark the game's traffic as high priority (DSCP 46)",
        description: "Adds a temporary Windows QoS policy that tags the game's network packets as expedited. It helps only if your router honours the tag (many home routers and mobile networks ignore it), so it is off by default. Removed afterwards, and it disappears on reboot anyway.",
        needs_admin: true,
        default_on: false,
    },
    ActionDef {
        id: "flush_dns",
        title: "Clear the DNS cache",
        description: "Empties Windows' cache of website addresses. Harmless, but it cannot be undone (Windows refills it by itself).",
        needs_admin: false,
        default_on: false,
    },
];

fn action_def(id: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|a| a.id == id)
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct StartRequest {
    #[serde(default)]
    pub actions: Vec<String>,
    pub game_pid: Option<u32>,
    #[serde(default)]
    pub background_pids: Vec<u32>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PlanItem {
    pub id: String,
    pub title: String,
    /// What would change, in plain words.
    pub detail: String,
    #[serde(skip)]
    pub targets: Vec<GameRef>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Skip {
    pub id: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub items: Vec<PlanItem>,
    pub skipped: Vec<Skip>,
    pub game: Option<GameRef>,
    /// The `started_at` of the boost this plan belongs to (set by `Core::begin`): a plan whose
    /// boost was cancelled meanwhile is not applied.
    pub token: Option<i64>,
}

fn pid_ok(pid: u32) -> bool {
    pid > 4
}

fn is_protected(name: &str) -> bool {
    PROTECTED.iter().any(|p| p.eq_ignore_ascii_case(name))
}

/// Turns a request into the exact list of changes that WOULD be made. Read-only.
pub fn make_plan(req: &StartRequest, ops: &dyn OsOps) -> Result<Plan, String> {
    if !ops.supported() {
        return Err(UNSUPPORTED.into());
    }
    for a in &req.actions {
        if action_def(a).is_none() {
            return Err(format!("unknown action '{a}'"));
        }
    }
    let game = match req.game_pid {
        Some(pid) if pid_ok(pid) => {
            let name = ops.process_name(pid).ok_or("that program is not running any more: pick the game again")?;
            if is_protected(&name) {
                return Err(format!("{name} is a system program: pick the game instead"));
            }
            Some(GameRef { pid, name })
        }
        Some(_) => return Err("not a valid program".into()),
        None => None,
    };
    let mut background: Vec<GameRef> = Vec::new();
    let mut bg_skipped: Vec<String> = Vec::new();
    for &pid in &req.background_pids {
        if !pid_ok(pid) || Some(pid) == req.game_pid || pid == std::process::id() || background.iter().any(|b| b.pid == pid) {
            continue;
        }
        match ops.process_name(pid) {
            Some(name) if !is_protected(&name) => background.push(GameRef { pid, name }),
            Some(name) => bg_skipped.push(format!("{name} is a system program")),
            None => bg_skipped.push(format!("process {pid} has already closed")),
        }
    }
    let probe = ops.probe(true);
    let admin = ops.admin();
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for def in ACTIONS.iter().filter(|d| req.actions.iter().any(|a| a == d.id)) {
        let skip = |reason: &str| Skip { id: def.id.into(), reason: reason.into() };
        if def.needs_admin && !admin {
            skipped.push(skip("needs rhcsd running as administrator"));
            continue;
        }
        let item = |detail: String, targets: Vec<GameRef>| PlanItem { id: def.id.into(), title: def.title.into(), detail, targets };
        match def.id {
            "power_plan_high" => {
                if !probe.high_perf_available {
                    skipped.push(skip("this PC has no High performance power plan"));
                } else if probe.power_plan_is_high {
                    skipped.push(skip("already on High performance"));
                } else {
                    let now = probe.power_plan.clone().unwrap_or_else(|| "current plan".into());
                    items.push(item(format!("Power plan: {now} → High performance"), vec![]));
                }
            }
            "game_mode_on" => {
                if probe.game_mode == Some(1) {
                    skipped.push(skip("Game Mode is already on"));
                } else {
                    let now = match probe.game_mode {
                        Some(0) => "off",
                        _ => "not set",
                    };
                    items.push(item(format!("Game Mode: {now} → on"), vec![]));
                }
            }
            "game_priority_high" => match &game {
                None => skipped.push(skip("pick the game first")),
                Some(g) => items.push(item(format!("{} priority: current → High", g.name), vec![g.clone()])),
            },
            "background_low_priority" => {
                if background.is_empty() {
                    let why = if bg_skipped.is_empty() { "no programs ticked".to_string() } else { bg_skipped.join("; ") };
                    skipped.push(skip(&why));
                } else {
                    let names: Vec<&str> = background.iter().map(|b| b.name.as_str()).collect();
                    items.push(item(format!("Priority → Below normal: {}", names.join(", ")), background.clone()));
                }
            }
            "quiet_notifications" => {
                if probe.toast_enabled == Some(0) {
                    skipped.push(skip("app notifications are already off"));
                } else {
                    items.push(item("App notifications: on → off".into(), vec![]));
                }
            }
            "pause_pulse_scans" => items.push(item("PULSE LAN scans and auto-switch: running → paused".into(), vec![])),
            "wifi_max_performance" => {
                if !probe.wifi_power_available {
                    skipped.push(skip("this PC has no Wi-Fi power saving setting"));
                } else {
                    items.push(item("Wi-Fi adapter power saving → Maximum Performance".into(), vec![]));
                }
            }
            "network_best_link" => items.push(item("Prefer the connection with the best measured ping (only if it is not already in use)".into(), vec![])),
            "network_dscp" => match &game {
                None => skipped.push(skip("pick the game first")),
                Some(g) => items.push(item(format!("QoS policy: mark {} traffic DSCP {DSCP_EXPEDITED}", g.name), vec![g.clone()])),
            },
            "flush_dns" => items.push(item("DNS cache cleared (cannot be undone)".into(), vec![])),
            _ => {}
        }
    }
    Ok(Plan { items, skipped, game, token: None })
}

pub const UNSUPPORTED: &str = "Boosting the PC works on Windows only. On Android an app cannot change how another app performs, so nothing here can be applied from the phone.";

// ------------------------------------------------------------------ verdict

/// Compares two readings in plain words. A change counts only when it is bigger than normal
/// run-to-run noise: ping 3 ms and 10 %, jitter 2 ms and 25 %, loss 1 point.
pub fn verdict(before: &Snap, after: &Snap) -> Verdict {
    let d = |b: Option<f64>, a: Option<f64>| match (b, a) {
        (Some(b), Some(a)) => Some(a - b),
        _ => None,
    };
    let (dp, dj, dl) = (d(before.ping_ms, after.ping_ms), d(before.jitter_ms, after.jitter_ms), d(before.loss_pct, after.loss_pct));
    let mut v = Verdict { improved: None, text: String::new(), ping_ms: dp, jitter_ms: dj, loss_pct: dl };
    let (Some(bp), Some(ap)) = (before.ping_ms, after.ping_ms) else {
        v.text = "Could not compare: the connection test got no answers before or after the boost, so no before / after numbers can be given.".into();
        return v;
    };
    let moved = if before.link_id != after.link_id {
        format!("The boost moved new connections from {} to {}. ", before.link_name, after.link_name)
    } else {
        String::new()
    };
    // (delta, noise floor): negative delta beyond the floor = better.
    let ping_floor = f64::max(3.0, 0.10 * bp);
    let jit_floor = f64::max(2.0, 0.25 * before.jitter_ms.unwrap_or(0.0));
    let sig = |delta: Option<f64>, floor: f64| delta.filter(|x| x.abs() >= floor);
    let (sp, sj, sl) = (sig(dp, ping_floor), sig(dj, jit_floor), sig(dl, 1.0));
    let better = [sp, sj, sl].iter().any(|x| x.is_some_and(|x| x < 0.0));
    let worse = [sp, sj, sl].iter().any(|x| x.is_some_and(|x| x > 0.0));
    let ms = |x: f64| format!("{x:.0}");
    let ms1 = |x: f64| format!("{x:.1}");
    if better && !worse {
        let mut parts = Vec::new();
        if sp.is_some() {
            parts.push(format!("ping went from {} ms to {} ms", ms(bp), ms(ap)));
        }
        if let (Some(_), Some(bj), Some(aj)) = (sj, before.jitter_ms, after.jitter_ms) {
            parts.push(format!("jitter from {} ms to {} ms", ms1(bj), ms1(aj)));
        }
        if let (Some(_), Some(bl), Some(al)) = (sl, before.loss_pct, after.loss_pct) {
            parts.push(format!("loss from {} % to {} %", ms1(bl), ms1(al)));
        }
        v.improved = Some(true);
        v.text = format!("{moved}Measured on {}: {}. This is a short test; another run can differ by a few ms.", after.link_name, parts.join(", "));
    } else if worse && !better {
        v.improved = Some(false);
        v.text = format!(
            "{moved}The ping was higher after the boost ({} ms to {} ms). Ping varies from run to run; measure again before drawing a conclusion.",
            ms(bp),
            ms(ap)
        );
    } else if better && worse {
        v.text = format!("{moved}Mixed result: some readings improved and others got worse, so there is no clear change. Measure again to check.");
    } else {
        v.text = format!("{moved}No measurable change: your game was limited by the connection, not the PC.");
    }
    v
}

// ------------------------------------------------------------------ the journal + engine

pub struct Core {
    path: Option<PathBuf>,
    book: Mutex<Book>,
}

#[derive(Debug, Default, Serialize)]
pub struct ApplyReport {
    pub applied: Vec<Value>,
    pub failed: Vec<Value>,
    pub notes: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub failed: Vec<Value>,
}

impl Core {
    /// Loads devboost.json from `dir` if there is one (changes listed there are restored by [`Core::recover`]).
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("devboost.json");
        let book = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Book>(&t).ok()).unwrap_or_default();
        Self { path: Some(path), book: Mutex::new(book) }
    }

    pub fn in_memory() -> Self {
        Self { path: None, book: Mutex::new(Book::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Writes the journal. A failed write is reported: the caller must not apply a change
    /// whose original value could not be saved.
    fn persist(&self, b: &Book) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(b).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| format!("cannot save {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("cannot save {}: {e}", path.display()))
    }

    pub fn snapshot(&self) -> Book {
        self.lock().clone()
    }

    /// Claims the "one boost at a time" slot. Err = a boost is already running.
    pub fn begin(&self, game: Option<GameRef>) -> Result<i64, String> {
        let mut b = self.lock();
        if b.busy() {
            return Err(if b.phase == "restore_failed" {
                "A previous boost could not be fully restored. Press \"Restore everything\" first.".into()
            } else {
                "A boost is already running. Restore everything first.".into()
            });
        }
        let last = b.last_result.clone();
        let token = now_ms();
        *b = Book { phase: "measuring_before".into(), game, started_at: Some(token), last_result: last, ..Default::default() };
        self.persist(&b)?;
        Ok(token)
    }

    pub fn set_before(&self, snap: Option<Snap>) {
        let mut b = self.lock();
        b.before = snap;
    }

    pub fn set_live(&self, snap: Option<Snap>) {
        let mut b = self.lock();
        if b.active {
            b.live = snap;
        }
    }

    pub fn set_result(&self, r: BoostResult) {
        let mut b = self.lock();
        b.last_result = Some(r);
        let _ = self.persist(&b);
    }

    /// Ends a boost that never applied anything (cancelled while measuring).
    #[allow(dead_code)]
    pub fn abandon(&self, why: &str) {
        let mut b = self.lock();
        if !b.active && b.changes.is_empty() && b.phase != "restore_failed" {
            b.phase = "idle".into();
            b.message = Some(why.into());
            b.game = None;
            let _ = self.persist(&b);
        }
    }

    /// Applies the plan. Each change's original value is read and saved to the journal first.
    /// A change that fails is undone on the spot and reported; the others still apply.
    pub fn apply(&self, ops: &dyn OsOps, plan: &Plan) -> ApplyReport {
        let mut rep = ApplyReport::default();
        let mut b = self.lock();
        if plan.token.is_some() && (b.started_at != plan.token || b.phase != "measuring_before") {
            rep.notes.push("Cancelled before anything was changed.".into());
            return rep;
        }
        b.phase = "applying".into();
        b.game = plan.game.clone();
        for item in &plan.items {
            let prepared = match self.prepare(ops, item) {
                Ok(p) => p,
                Err(e) => {
                    rep.failed.push(json!({ "id": item.id, "title": item.title, "error": e }));
                    continue;
                }
            };
            let undos = match prepared {
                Prepared::Do(u, notes) => {
                    rep.notes.extend(notes);
                    u
                }
                Prepared::Nothing(note) => {
                    rep.notes.push(note);
                    continue;
                }
            };
            if undos.is_empty() {
                continue;
            }
            // 1. journal, 2. apply.
            let first = b.changes.len();
            for (undo, detail) in &undos {
                b.changes.push(Change {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    detail: detail.clone(),
                    undo: undo.clone(),
                    applied_at: now_ms(),
                });
            }
            b.active = true;
            if let Err(e) = self.persist(&b) {
                // Nothing was changed yet: drop the entries and refuse this change.
                b.changes.truncate(first);
                rep.failed.push(json!({ "id": item.id, "title": item.title, "error": e }));
                continue;
            }
            for k in first..first + undos.len() {
                let change = b.changes[k].clone();
                match self.do_apply(ops, &change, plan.game.as_ref()) {
                    Err(e) if e.starts_with(NOOP) => {
                        // Nothing was changed, so there is nothing to put back: forget the entry.
                        b.changes[k].id = String::new();
                        rep.notes.push(e[NOOP.len()..].to_string());
                    }
                    Ok(note) => {
                        if let Some(n) = note {
                            b.changes[k].detail = n;
                        }
                        rep.applied.push(json!({ "id": change.id, "title": change.title, "detail": b.changes[k].detail }));
                    }
                    Err(e) => {
                        // Put back whatever may have changed, then forget the entry.
                        let _ = self.do_undo(ops, &change);
                        b.changes[k].id = String::new(); // marks "drop"
                        rep.failed.push(json!({ "id": change.id, "title": change.title, "error": e }));
                    }
                }
            }
            b.changes.retain(|c| !c.id.is_empty());
            let _ = self.persist(&b);
        }
        b.active = !b.changes.is_empty();
        b.phase = if b.active { "active".into() } else { "idle".into() };
        if !b.active {
            b.message = Some("Nothing could be changed.".into());
            b.game = None;
        }
        let _ = self.persist(&b);
        rep
    }

    /// Reads the original value(s) of what an action will change.
    fn prepare(&self, ops: &dyn OsOps, item: &PlanItem) -> Result<Prepared, String> {
        let one = |u: Undo, d: String| Ok(Prepared::Do(vec![(u, d)], vec![]));
        match item.id.as_str() {
            "power_plan_high" => {
                let original = ops.power_active()?;
                one(Undo::PowerPlan { original }, item.detail.clone())
            }
            "game_mode_on" => {
                let original = ops.reg_get(GAMEBAR_KEY, GAME_MODE_VALUE)?;
                one(Undo::Registry { key: GAMEBAR_KEY.into(), name: GAME_MODE_VALUE.into(), original }, item.detail.clone())
            }
            "quiet_notifications" => {
                let original = ops.reg_get(TOAST_KEY, TOAST_VALUE)?;
                one(Undo::Registry { key: TOAST_KEY.into(), name: TOAST_VALUE.into(), original }, item.detail.clone())
            }
            "game_priority_high" | "background_low_priority" => {
                let (mut v, mut notes) = (Vec::new(), Vec::new());
                for t in &item.targets {
                    match ops.priority_get(t.pid) {
                        Ok(original) => {
                            let what = if item.id == "game_priority_high" { "High" } else { "Below normal" };
                            v.push((Undo::Priority { pid: t.pid, process: t.name.clone(), original }, format!("{} priority → {what}", t.name)));
                        }
                        // One program we cannot touch (another user's, elevated) must not block the rest.
                        Err(e) if item.id == "background_low_priority" => notes.push(format!("{}: {e}", t.name)),
                        Err(e) => return Err(format!("{}: {e}", t.name)),
                    }
                }
                if v.is_empty() && !notes.is_empty() {
                    return Err(notes.join("; "));
                }
                Ok(Prepared::Do(v, notes))
            }
            "pause_pulse_scans" => one(Undo::PauseScans, item.detail.clone()),
            "wifi_max_performance" => {
                let scheme = ops.power_active()?;
                let (ac, dc) = ops.wifi_power_get(&scheme)?;
                if ac == 0 && dc == 0 {
                    return Ok(Prepared::Nothing("Wi-Fi power saving was already Maximum Performance".into()));
                }
                one(Undo::WifiPower { scheme, ac, dc }, item.detail.clone())
            }
            "network_best_link" => one(Undo::NetworkSwitch, item.detail.clone()),
            "network_dscp" => one(Undo::QosPolicy { policy: QOS_POLICY.into() }, item.detail.clone()),
            "flush_dns" => one(Undo::Irreversible, item.detail.clone()),
            other => Err(format!("unknown action '{other}'")),
        }
    }

    /// Performs one journalled change. May return a more precise description.
    fn do_apply(&self, ops: &dyn OsOps, c: &Change, game: Option<&GameRef>) -> Result<Option<String>, String> {
        match (&c.undo, c.id.as_str()) {
            (Undo::PowerPlan { .. }, _) => ops.power_set(HIGH_PERF_GUID).map(|_| None),
            (Undo::Registry { key, name, .. }, "game_mode_on") => ops.reg_set(key, name, &RegValue::Dword { value: 1 }).map(|_| None),
            (Undo::Registry { key, name, .. }, _) => ops.reg_set(key, name, &RegValue::Dword { value: 0 }).map(|_| None),
            (Undo::Priority { pid, .. }, "game_priority_high") => ops.priority_set(*pid, PRIO_HIGH).map(|_| None),
            (Undo::Priority { pid, .. }, _) => ops.priority_set(*pid, PRIO_BELOW_NORMAL).map(|_| None),
            (Undo::WifiPower { scheme, .. }, _) => ops.wifi_power_set(scheme, 0, 0).map(|_| None),
            (Undo::QosPolicy { policy }, _) => {
                let exe = game.and_then(|g| ops.process_name(g.pid)).ok_or("the game is not running any more")?;
                ops.qos_add(policy, &exe).map(|_| None)
            }
            (Undo::NetworkSwitch, _) => match ops.switch_best()? {
                Some(msg) => Ok(Some(msg)),
                None => Err(format!("{NOOP}No connection switch was needed (the best connection is already in use)")),
            },
            (Undo::PauseScans, _) => {
                ops.pause_scans(true);
                Ok(None)
            }
            (Undo::Irreversible, _) => ops.flush_dns().map(|_| None),
        }
    }

    fn do_undo(&self, ops: &dyn OsOps, c: &Change) -> Result<(), String> {
        match &c.undo {
            Undo::PowerPlan { original } => ops.power_set(original),
            Undo::Registry { key, name, original } => match original {
                Some(v) => ops.reg_set(key, name, v),
                None => ops.reg_delete(key, name),
            },
            Undo::Priority { pid, process, original } => match ops.process_name(*pid) {
                Some(now) if now.eq_ignore_ascii_case(process) => ops.priority_set(*pid, *original),
                // Closed (or the pid now belongs to another program): nothing of ours to restore.
                _ => Ok(()),
            },
            Undo::WifiPower { scheme, ac, dc } => ops.wifi_power_set(scheme, *ac, *dc),
            Undo::QosPolicy { policy } => ops.qos_remove(policy),
            Undo::NetworkSwitch => ops.switch_restore(),
            Undo::PauseScans => {
                ops.pause_scans(false);
                Ok(())
            }
            Undo::Irreversible => Ok(()),
        }
    }

    /// Puts everything back, newest change first. Entries that restore leave the journal at once;
    /// ones that fail stay (and the phase becomes `restore_failed`) so a later try can finish.
    pub fn restore_all(&self, ops: &dyn OsOps, why: &str) -> RestoreReport {
        let mut rep = RestoreReport::default();
        let mut b = self.lock();
        if b.changes.is_empty() && !b.active {
            if b.phase != "idle" {
                b.phase = "idle".into();
                b.game = None;
                let _ = self.persist(&b);
            }
            ops.pause_scans(false);
            return rep;
        }
        b.phase = "restoring".into();
        let _ = self.persist(&b);
        let list: Vec<Change> = b.changes.iter().rev().cloned().collect();
        for c in list {
            match self.do_undo(ops, &c) {
                Ok(()) => {
                    if let Some(i) = b.changes.iter().position(|x| x == &c) {
                        b.changes.remove(i);
                    }
                    if !matches!(c.undo, Undo::Irreversible) {
                        rep.restored.push(c.title.clone());
                    }
                    let _ = self.persist(&b);
                }
                Err(e) => rep.failed.push(json!({ "id": c.id, "title": c.title, "error": e })),
            }
        }
        if b.changes.is_empty() {
            b.active = false;
            b.phase = "idle".into();
            b.game = None;
            b.live = None;
            b.message = Some(why.into());
            ops.pause_scans(false);
        } else {
            b.phase = "restore_failed".into();
            b.message = Some("Some changes could not be restored. Try \"Restore everything\" again.".into());
        }
        let _ = self.persist(&b);
        rep
    }

    /// On rhcsd start: if the journal lists changes, a previous run ended without restoring
    /// (crash, power cut, killed): put everything back now.
    pub fn recover(&self, ops: &dyn OsOps) -> Option<RestoreReport> {
        let pending = {
            let b = self.lock();
            !b.changes.is_empty() || b.active || b.phase == "restore_failed" || b.phase == "active"
        };
        if !pending {
            return None;
        }
        Some(self.restore_all(ops, "Restored after rhcsd restarted (the previous boost had not been undone)."))
    }

    /// One look at a running boost: restores everything when the game has exited.
    /// Returns true when it did.
    pub fn check_game(&self, ops: &dyn OsOps) -> bool {
        let game = {
            let b = self.lock();
            if !b.active || b.phase != "active" {
                return false;
            }
            b.game.clone()
        };
        let Some(g) = game else { return false };
        let still = ops.process_name(g.pid).is_some_and(|n| n.eq_ignore_ascii_case(&g.name));
        if still {
            return false;
        }
        self.restore_all(ops, &format!("{} closed, so everything was put back.", g.name));
        true
    }
}

/// Marks an "error" that really means "nothing to do" (no change was made).
const NOOP: &str = "noop:";

enum Prepared {
    /// (undo + description) per change, and notes about targets that were skipped.
    Do(Vec<(Undo, String)>, Vec<String>),
    Nothing(String),
}

// ------------------------------------------------------------------ parsing helpers (pure)

/// First GUID (8-4-4-4-12 hex) in a text, lower-cased. Works in any Windows language.
pub fn find_guid(text: &str) -> Option<String> {
    text.split(|c: char| !(c.is_ascii_hexdigit() || c == '-')).find_map(|t| {
        let b = t.as_bytes();
        (t.len() == 36 && [8, 13, 18, 23].iter().all(|&i| b[i] == b'-') && t.chars().filter(|c| *c != '-').all(|c| c.is_ascii_hexdigit()))
            .then(|| t.to_ascii_lowercase())
    })
}

/// `powercfg /getactivescheme` -> (guid, plan name).
pub fn parse_active_scheme(text: &str) -> Option<(String, Option<String>)> {
    let guid = find_guid(text)?;
    let name = text.rfind('(').and_then(|a| text[a + 1..].find(')').map(|b| text[a + 1..a + 1 + b].trim().to_string())).filter(|n| !n.is_empty());
    Some((guid, name))
}

/// `powercfg /query scheme sub setting`: the last two 0x… numbers are the AC and DC values.
pub fn parse_power_indexes(text: &str) -> Option<(u32, u32)> {
    let nums: Vec<u32> = text
        .split_whitespace()
        .filter_map(|t| t.strip_prefix("0x").and_then(|h| u32::from_str_radix(h.trim_end_matches(','), 16).ok()))
        .collect();
    (nums.len() >= 2).then(|| (nums[nums.len() - 2], nums[nums.len() - 1]))
}

/// `reg query HKCU\key /v name`: the value line is `    name    REG_DWORD    0x1`.
pub fn parse_reg_query(text: &str, name: &str) -> Option<RegValue> {
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() >= 3 && t[0].eq_ignore_ascii_case(name) {
            return match t[1] {
                "REG_DWORD" => {
                    let v = t[2];
                    let n = v.strip_prefix("0x").map(|h| u32::from_str_radix(h, 16)).unwrap_or_else(|| v.parse());
                    n.ok().map(|value| RegValue::Dword { value })
                }
                "REG_SZ" => Some(RegValue::Str { value: t[2..].join(" ") }),
                _ => None,
            };
        }
    }
    None
}

/// Name heuristic for "this looks like a game or an emulator". A plain list, no claim:
/// the user picks the program.
pub fn classify_game(process: &str, path: &str) -> Option<&'static str> {
    let n = process.to_ascii_lowercase();
    let p = path.to_ascii_lowercase().replace('/', "\\");
    const NAMES: &[(&str, &str)] = &[
        ("hd-player", "Android emulator (BlueStacks / MSI App Player)"),
        ("bluestacks", "Android emulator (BlueStacks)"),
        ("msiapp", "Android emulator (MSI App Player)"),
        ("dnplayer", "Android emulator (LDPlayer)"),
        ("ldplayer", "Android emulator (LDPlayer)"),
        ("aow_exe", "Android emulator (Gameloop)"),
        ("gameloop", "Android emulator (Gameloop)"),
        ("androidemulator", "Android emulator (Gameloop)"),
        ("memu", "Android emulator (MEmu)"),
        ("nox", "Android emulator (Nox)"),
        ("freefire", "Free Fire"),
    ];
    // "nox" and "memu" are short: only match them at the start of the name.
    if let Some((_, h)) = NAMES.iter().find(|(k, _)| if matches!(*k, "nox" | "memu") { n.starts_with(k) } else { n.contains(k) }) {
        return Some(h);
    }
    if p.contains("\\steamapps\\common\\") {
        return Some("Steam game");
    }
    if p.contains("\\epic games\\") {
        return Some("Epic Games game");
    }
    None
}

#[derive(Deserialize)]
struct RawProc {
    pid: u32,
    name: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    mem: u64,
    #[serde(default)]
    path: String,
}

const SHELL_PROCESSES: &[&str] = &[
    "explorer", "applicationframehost", "shellexperiencehost", "searchhost", "searchapp", "startmenuexperiencehost", "textinputhost",
    "systemsettings", "lockapp", "rhcsd", "rhcsd-keepalive", "powershell", "conhost", "windowsterminal", "cmd",
];

/// Running programs with a visible window, biggest memory first. Window titles are kept only
/// for likely games / emulators.
pub fn parse_candidates(json: &str) -> Vec<Candidate> {
    let json = json.trim();
    let raw: Vec<RawProc> = serde_json::from_str::<Vec<RawProc>>(json)
        .or_else(|_| serde_json::from_str::<RawProc>(json).map(|r| vec![r]))
        .unwrap_or_default();
    let mut out: Vec<Candidate> = raw
        .into_iter()
        .filter(|r| r.pid > 4 && r.pid != std::process::id() && !SHELL_PROCESSES.iter().any(|s| s.eq_ignore_ascii_case(&r.name)))
        .map(|r| {
            let hint = classify_game(&r.name, &r.path);
            Candidate {
                pid: r.pid,
                window_title: hint.map(|_| r.title.clone()).filter(|t| !t.is_empty()),
                name: r.name,
                mem_mb: r.mem,
                likely_game: hint.is_some(),
                hint: hint.map(str::to_string),
            }
        })
        .collect();
    out.sort_by(|a, b| b.mem_mb.cmp(&a.mem_mb));
    out
}

pub const CANDIDATES_SCRIPT: &str = r#"
$r = @(Get-Process | Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle } | ForEach-Object {
  $p = ''; try { $p = [string]$_.Path } catch {}
  [pscustomobject]@{ pid = [int]$_.Id; name = [string]$_.ProcessName; title = [string]$_.MainWindowTitle; mem = [int]($_.WorkingSet64 / 1MB); path = $p }
})
ConvertTo-Json -InputObject $r -Compress
"#;

// ------------------------------------------------------------------ the real OS (Windows) and the stub

#[cfg(windows)]
mod sys {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetPriorityClass, OpenProcess, QueryFullProcessImageNameW, SetPriorityClass, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SET_INFORMATION,
    };

    const STILL_ACTIVE: u32 = 259;
    const ERROR_INVALID_PARAMETER: u32 = 87;

    pub fn cmd(program: &str, args: &[&str]) -> Result<String, String> {
        let out = Command::new(program)
            .args(args)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .map_err(|e| format!("could not run {program}: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() {
            Ok(text)
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(crate::winps::first_line(&format!("{}\n{}", err.trim(), text.trim())))
        }
    }

    fn guard_key(key: &str) -> Result<(), String> {
        // Only the current user's Software branch; never anything outside HKCU.
        if key.starts_with("Software\\") && !key.contains("..") {
            Ok(())
        } else {
            Err(format!("refusing to touch registry key {key}"))
        }
    }

    pub fn power_active() -> Result<String, String> {
        let t = cmd("powercfg", &["/getactivescheme"])?;
        parse_active_scheme(&t).map(|(g, _)| g).ok_or_else(|| "could not read the active power plan".into())
    }
    pub fn power_set(guid: &str) -> Result<(), String> {
        if find_guid(guid).as_deref() != Some(guid) {
            return Err("not a power plan id".into());
        }
        cmd("powercfg", &["/setactive", guid]).map(|_| ())
    }
    pub fn reg_get(key: &str, name: &str) -> Result<Option<RegValue>, String> {
        guard_key(key)?;
        // A missing key / value makes reg.exe exit 1: that is "not set", not an error.
        Ok(cmd("reg", &["query", &format!("HKCU\\{key}"), "/v", name]).ok().and_then(|t| parse_reg_query(&t, name)))
    }
    pub fn reg_set(key: &str, name: &str, v: &RegValue) -> Result<(), String> {
        guard_key(key)?;
        let full = format!("HKCU\\{key}");
        match v {
            RegValue::Dword { value } => cmd("reg", &["add", &full, "/v", name, "/t", "REG_DWORD", "/d", &value.to_string(), "/f"]),
            RegValue::Str { value } => cmd("reg", &["add", &full, "/v", name, "/t", "REG_SZ", "/d", value, "/f"]),
        }
        .map(|_| ())
    }
    pub fn reg_delete(key: &str, name: &str) -> Result<(), String> {
        guard_key(key)?;
        if reg_get(key, name)?.is_none() {
            return Ok(());
        }
        cmd("reg", &["delete", &format!("HKCU\\{key}"), "/v", name, "/f"]).map(|_| ())
    }

    fn open(pid: u32, access: u32) -> Result<HANDLE, u32> {
        let h = unsafe { OpenProcess(access, 0, pid) };
        if h.is_null() {
            Err(unsafe { GetLastError() })
        } else {
            Ok(h)
        }
    }

    /// None = no such process (or it has exited). A program we may not open still exists:
    /// it gets a stable stand-in name.
    pub fn process_name(pid: u32) -> Option<String> {
        match open(pid, PROCESS_QUERY_LIMITED_INFORMATION) {
            Err(ERROR_INVALID_PARAMETER) => None,
            Err(_) => Some(format!("pid {pid}")),
            Ok(h) => {
                let mut code = 0u32;
                let alive = unsafe { GetExitCodeProcess(h, &mut code) } != 0 && code == STILL_ACTIVE;
                let mut name = None;
                if alive {
                    let mut buf = vec![0u16; 1024];
                    let mut len = buf.len() as u32;
                    if unsafe { QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) } != 0 {
                        let full = String::from_utf16_lossy(&buf[..len as usize]);
                        name = full.rsplit(['\\', '/']).next().map(str::to_string);
                    }
                }
                unsafe { CloseHandle(h) };
                alive.then(|| name.unwrap_or_else(|| format!("pid {pid}")))
            }
        }
    }
    pub fn priority_get(pid: u32) -> Result<u32, String> {
        let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).map_err(|e| format!("cannot open the program (Windows error {e})"))?;
        let c = unsafe { GetPriorityClass(h) };
        unsafe { CloseHandle(h) };
        if c == 0 {
            Err("cannot read its priority".into())
        } else {
            Ok(c)
        }
    }
    pub fn priority_set(pid: u32, class: u32) -> Result<(), String> {
        let h = open(pid, PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION)
            .map_err(|e| format!("not allowed to change this program's priority (Windows error {e}); it may be running as administrator"))?;
        let ok = unsafe { SetPriorityClass(h, class) } != 0;
        unsafe { CloseHandle(h) };
        if ok {
            Ok(())
        } else {
            Err("Windows refused the priority change".into())
        }
    }

    pub fn wifi_power_get(scheme: &str) -> Result<(u32, u32), String> {
        let t = cmd("powercfg", &["/query", scheme, WIFI_SUBGROUP, WIFI_SETTING])?;
        parse_power_indexes(&t).ok_or_else(|| "this PC has no Wi-Fi power saving setting".into())
    }
    pub fn wifi_power_set(scheme: &str, ac: u32, dc: u32) -> Result<(), String> {
        cmd("powercfg", &["/setacvalueindex", scheme, WIFI_SUBGROUP, WIFI_SETTING, &ac.to_string()])?;
        cmd("powercfg", &["/setdcvalueindex", scheme, WIFI_SUBGROUP, WIFI_SETTING, &dc.to_string()])?;
        // The scheme must be re-activated for the new index to apply.
        cmd("powercfg", &["/setactive", scheme]).map(|_| ())
    }
    pub fn qos_add(policy: &str, exe: &str) -> Result<(), String> {
        let script = format!(
            "New-NetQosPolicy -Name {} -AppPathNameMatchCondition {} -DSCPAction {DSCP_EXPEDITED} -PolicyStore ActiveStore | Out-Null",
            crate::winps::quote(policy),
            crate::winps::quote(exe)
        );
        crate::winps::run_blocking(&script, Duration::from_secs(20)).map(|_| ())
    }
    pub fn qos_remove(policy: &str) -> Result<(), String> {
        let script = format!(
            "Remove-NetQosPolicy -Name {} -PolicyStore ActiveStore -Confirm:$false -ErrorAction SilentlyContinue",
            crate::winps::quote(policy)
        );
        crate::winps::run_blocking(&script, Duration::from_secs(20)).map(|_| ())
    }
    pub fn flush_dns() -> Result<(), String> {
        cmd("ipconfig", &["/flushdns"]).map(|_| ())
    }

    pub fn probe() -> Probe {
        let active = cmd("powercfg", &["/getactivescheme"]).ok().and_then(|t| parse_active_scheme(&t));
        let list = cmd("powercfg", &["/list"]).unwrap_or_default();
        let high = list.to_ascii_lowercase().contains(HIGH_PERF_GUID);
        let dword = |key: &str, name: &str| match reg_get(key, name) {
            Ok(Some(RegValue::Dword { value })) => Some(value),
            _ => None,
        };
        Probe {
            power_plan_is_high: active.as_ref().is_some_and(|(g, _)| g == HIGH_PERF_GUID),
            power_plan: active.as_ref().and_then(|(_, n)| n.clone()),
            high_perf_available: high,
            game_mode: dword(GAMEBAR_KEY, GAME_MODE_VALUE),
            toast_enabled: dword(TOAST_KEY, TOAST_VALUE),
            wifi_power_available: active.as_ref().is_some_and(|(g, _)| wifi_power_get(g).is_ok()),
        }
    }

    static PROBE_CACHE: Mutex<Option<(Instant, Probe)>> = Mutex::new(None);
    static CAND_CACHE: Mutex<Option<(Instant, Vec<Candidate>)>> = Mutex::new(None);

    pub fn probe_cached(fresh: bool) -> Probe {
        let mut g = PROBE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if !fresh {
            if let Some((at, p)) = g.as_ref() {
                if at.elapsed() < Duration::from_secs(8) {
                    return p.clone();
                }
            }
        }
        let p = probe();
        *g = Some((Instant::now(), p.clone()));
        p
    }

    pub fn candidates() -> Vec<Candidate> {
        let mut g = CAND_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, v)) = g.as_ref() {
            if at.elapsed() < Duration::from_secs(8) {
                return v.clone();
            }
        }
        let v = crate::winps::run_blocking(CANDIDATES_SCRIPT, Duration::from_secs(20)).map(|j| parse_candidates(&j)).unwrap_or_default();
        *g = Some((Instant::now(), v.clone()));
        v
    }
}

#[cfg(not(windows))]
mod sys {
    use super::*;
    fn no<T>() -> Result<T, String> {
        Err(UNSUPPORTED.into())
    }
    pub fn power_active() -> Result<String, String> {
        no()
    }
    pub fn power_set(_: &str) -> Result<(), String> {
        no()
    }
    pub fn reg_get(_: &str, _: &str) -> Result<Option<RegValue>, String> {
        no()
    }
    pub fn reg_set(_: &str, _: &str, _: &RegValue) -> Result<(), String> {
        no()
    }
    pub fn reg_delete(_: &str, _: &str) -> Result<(), String> {
        no()
    }
    pub fn process_name(_: u32) -> Option<String> {
        None
    }
    pub fn priority_get(_: u32) -> Result<u32, String> {
        no()
    }
    pub fn priority_set(_: u32, _: u32) -> Result<(), String> {
        no()
    }
    pub fn wifi_power_get(_: &str) -> Result<(u32, u32), String> {
        no()
    }
    pub fn wifi_power_set(_: &str, _: u32, _: u32) -> Result<(), String> {
        no()
    }
    pub fn qos_add(_: &str, _: &str) -> Result<(), String> {
        no()
    }
    pub fn qos_remove(_: &str) -> Result<(), String> {
        no()
    }
    pub fn flush_dns() -> Result<(), String> {
        no()
    }
    pub fn probe_cached(_: bool) -> Probe {
        Probe::default()
    }
    pub fn candidates() -> Vec<Candidate> {
        Vec::new()
    }
}

/// The real thing. Built inside the async runtime (it keeps a handle to it so the switch calls,
/// which are async, can run from the blocking threads the engine uses).
pub struct RealOps {
    s: Shared,
    handle: tokio::runtime::Handle,
}

impl RealOps {
    pub fn new(s: &Shared) -> Self {
        Self { s: s.clone(), handle: tokio::runtime::Handle::current() }
    }
}

impl OsOps for RealOps {
    fn supported(&self) -> bool {
        cfg!(windows)
    }
    fn admin(&self) -> bool {
        self.s.admin
    }
    fn probe(&self, fresh: bool) -> Probe {
        sys::probe_cached(fresh)
    }
    fn candidates(&self) -> Vec<Candidate> {
        sys::candidates()
    }
    fn process_name(&self, pid: u32) -> Option<String> {
        sys::process_name(pid)
    }
    fn power_active(&self) -> Result<String, String> {
        sys::power_active()
    }
    fn power_set(&self, guid: &str) -> Result<(), String> {
        sys::power_set(guid)
    }
    fn reg_get(&self, key: &str, name: &str) -> Result<Option<RegValue>, String> {
        sys::reg_get(key, name)
    }
    fn reg_set(&self, key: &str, name: &str, v: &RegValue) -> Result<(), String> {
        sys::reg_set(key, name, v)
    }
    fn reg_delete(&self, key: &str, name: &str) -> Result<(), String> {
        sys::reg_delete(key, name)
    }
    fn priority_get(&self, pid: u32) -> Result<u32, String> {
        sys::priority_get(pid)
    }
    fn priority_set(&self, pid: u32, class: u32) -> Result<(), String> {
        sys::priority_set(pid, class)
    }
    fn wifi_power_get(&self, scheme: &str) -> Result<(u32, u32), String> {
        sys::wifi_power_get(scheme)
    }
    fn wifi_power_set(&self, scheme: &str, ac: u32, dc: u32) -> Result<(), String> {
        sys::wifi_power_set(scheme, ac, dc)
    }
    fn qos_add(&self, policy: &str, exe: &str) -> Result<(), String> {
        sys::qos_add(policy, exe)
    }
    fn qos_remove(&self, policy: &str) -> Result<(), String> {
        sys::qos_remove(policy)
    }
    fn flush_dns(&self) -> Result<(), String> {
        sys::flush_dns()
    }
    fn pause_scans(&self, on: bool) {
        PAUSED.store(on, Ordering::SeqCst);
    }
    fn switch_best(&self) -> Result<Option<String>, String> {
        let s = self.s.clone();
        self.handle.block_on(async move {
            let (rec, links) = {
                let st = s.boost.state.read().await;
                (st.recommendation.clone(), st.links.clone())
            };
            let Some(rec) = rec else { return Ok(None) };
            if rec.already_default {
                return Ok(None);
            }
            let alias = links
                .iter()
                .find(|l| l.link.id == rec.best)
                .map(|l| l.link.adapter.clone())
                .ok_or("the best connection is gone")?;
            crate::switch::switch_to(&s, &alias).await.map(Some)
        })
    }
    fn switch_restore(&self) -> Result<(), String> {
        let s = self.s.clone();
        self.handle.block_on(async move { crate::switch::restore(&s).await.map(|_| ()) })
    }
}

// ------------------------------------------------------------------ AppState part

pub struct Devboost {
    pub core: Core,
}

impl Devboost {
    pub fn open(dir: &Path) -> Self {
        Self { core: Core::open(dir) }
    }
}

// ------------------------------------------------------------------ views

/// Latest reading of the link in use (the default one; else the first with answers).
pub fn snap_of(links: &[crate::boost::LinkResult]) -> Option<Snap> {
    let l = links.iter().find(|l| l.link.is_default && l.stats.sent > 0).or_else(|| links.iter().find(|l| l.stats.received > 0))?;
    Some(Snap {
        link_id: l.link.id.clone(),
        link_name: l.link.name.clone(),
        ping_ms: l.stats.ping_ms,
        jitter_ms: l.stats.jitter_ms,
        loss_pct: l.stats.loss_pct,
        at: now_ms(),
    })
}

fn probe_detail(id: &str, p: &Probe) -> String {
    match id {
        "power_plan_high" => match (&p.power_plan, p.high_perf_available) {
            (_, false) => "this PC has no High performance plan".into(),
            (Some(n), _) => format!("now: {n}"),
            _ => String::new(),
        },
        "game_mode_on" => format!("now: {}", match p.game_mode { Some(1) => "on", Some(0) => "off", _ => "not set" }),
        "quiet_notifications" => format!("now: {}", if p.toast_enabled == Some(0) { "off" } else { "on" }),
        "wifi_max_performance" if !p.wifi_power_available => "this PC has no Wi-Fi power saving setting".into(),
        _ => String::new(),
    }
}

/// The `GET /api/boost/device` answer. Pure, so the shape is unit-tested.
pub fn device_view(supported: bool, admin: bool, probe: &Probe, candidates: &[Candidate], book: &Book) -> Value {
    let busy = book.busy();
    let actions: Vec<Value> = ACTIONS
        .iter()
        .map(|d| {
            let active = book.changes.iter().any(|c| c.id == d.id);
            let admin_ok = !d.needs_admin || admin;
            let special_ok = match d.id {
                "power_plan_high" => busy || probe.high_perf_available,
                "wifi_max_performance" => busy || probe.wifi_power_available,
                _ => true,
            };
            let available = supported && admin_ok && special_ok;
            let detail = if active {
                book.changes.iter().filter(|c| c.id == d.id).map(|c| c.detail.clone()).collect::<Vec<_>>().join("; ")
            } else if !supported {
                "Windows only".to_string()
            } else if !admin_ok {
                "needs rhcsd running as administrator".to_string()
            } else {
                probe_detail(d.id, probe)
            };
            json!({
                "id": d.id, "title": d.title, "description": d.description, "needs_admin": d.needs_admin,
                "available": available, "default_on": d.default_on, "active": active, "detail": detail,
            })
        })
        .collect();
    json!({
        "supported": supported,
        "unsupported_reason": if supported { Value::Null } else { json!(UNSUPPORTED) },
        "admin": admin,
        "actions": actions,
        "candidates": candidates,
        "state": {
            "active": book.active,
            "phase": if book.phase.is_empty() { "idle" } else { book.phase.as_str() },
            "started_at": book.started_at,
            "game": book.game,
            "changes": book.changes.iter().map(|c| json!({
                "id": c.id, "title": c.title, "detail": c.detail, "reversible": !matches!(c.undo, Undo::Irreversible),
            })).collect::<Vec<_>>(),
            "before": book.before,
            "live": book.live,
            "message": book.message,
        },
        "last_result": book.last_result,
    })
}

// ------------------------------------------------------------------ running a boost

async fn blocking<T: Send + 'static>(s: &Shared, f: impl FnOnce(&Core, &RealOps) -> T + Send + 'static) -> T {
    let ops = RealOps::new(s);
    let s2 = s.clone();
    tokio::task::spawn_blocking(move || f(&s2.devboost.core, &ops)).await.expect("devboost task panicked")
}

/// Measure, apply, watch the game, measure again; ends when everything has been restored.
async fn run_boost(s: Shared, plan: Plan) {
    let token = plan.token;
    let current = |s: &Shared| {
        let b = s.devboost.core.snapshot();
        b.started_at == token && b.phase == "measuring_before"
    };
    let before = crate::boost::measure_once(&s, MEASURE_PROBES).await.and_then(|l| snap_of(&l));
    if !current(&s) {
        return; // restored / cancelled while measuring
    }
    s.devboost.core.set_before(before.clone());
    let rep = blocking(&s, move |c, ops| c.apply(ops, &plan)).await;
    for f in &rep.failed {
        s.event("devboost", format!("could not apply {}: {}", f["title"].as_str().unwrap_or("?"), f["error"].as_str().unwrap_or("?")), json!({})).await;
    }
    let book = s.devboost.core.snapshot();
    if !book.active || book.started_at != token {
        return;
    }
    s.event("devboost", format!("Game Boost for this PC on: {} change(s)", book.changes.len()), json!({})).await;

    let applied = std::time::Instant::now();
    let mut last_live = std::time::Instant::now();
    let mut result_done = false;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let b = s.devboost.core.snapshot();
        if b.started_at != token || !b.active || b.phase != "active" {
            break;
        }
        if blocking(&s, |c, ops| c.check_game(ops)).await {
            s.event("devboost", "The game closed: everything was put back", json!({})).await;
            break;
        }
        let due_after = !result_done && applied.elapsed() >= SETTLE;
        let due_live = result_done && last_live.elapsed() >= LIVE_GAP;
        if due_after || due_live {
            let probes = if due_after { MEASURE_PROBES } else { LIVE_PROBES };
            let snap = crate::boost::measure_once(&s, probes).await.and_then(|l| snap_of(&l));
            let b = s.devboost.core.snapshot();
            if b.started_at != token || !b.active {
                break; // restored while measuring: this reading no longer describes a boosted PC
            }
            s.devboost.core.set_live(snap.clone());
            if due_after {
                result_done = true;
                let verdict = match (&before, &snap) {
                    (Some(b0), Some(a0)) => verdict(b0, a0),
                    _ => Verdict {
                        improved: None,
                        text: "Could not compare: one of the two connection tests got no answers.".into(),
                        ping_ms: None,
                        jitter_ms: None,
                        loss_pct: None,
                    },
                };
                s.devboost.core.set_result(BoostResult { before: before.clone(), after: snap, verdict, finished_at: now_ms() });
            }
            last_live = std::time::Instant::now();
        }
    }
}

/// Called once when rhcsd starts: puts back anything a previous run left changed.
pub async fn startup(s: Shared) {
    if !cfg!(windows) {
        return;
    }
    if let Some(rep) = blocking(&s, |c, ops| c.recover(ops)).await {
        let msg = if rep.failed.is_empty() {
            format!("Game Boost for this PC: restored {} change(s) left by the previous run", rep.restored.len())
        } else {
            format!("Game Boost for this PC: could not restore {} change(s); open Boost and press Restore everything", rep.failed.len())
        };
        s.event("devboost", msg, json!({})).await;
    }
}

/// Called on a clean shutdown (Ctrl+C or the engine's stop): restores before the process ends.
pub async fn shutdown_restore(s: &Shared) {
    if !cfg!(windows) {
        return;
    }
    let b = s.devboost.core.snapshot();
    if b.changes.is_empty() && !b.active {
        return;
    }
    let rep = blocking(s, |c, ops| c.restore_all(ops, "Restored when rhcsd shut down.")).await;
    println!("devboost: restored {} change(s) on shutdown", rep.restored.len());
}

// ------------------------------------------------------------------ API

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/boost/device", get(get_device))
        .route("/api/boost/device/start", post(post_start))
        .route("/api/boost/device/stop", post(post_stop))
}

async fn get_device(State(s): State<Shared>) -> Json<Value> {
    let book = s.devboost.core.snapshot();
    let ops = RealOps::new(&s);
    let supported = ops.supported();
    let admin = s.admin;
    let (probe, cands) = if supported {
        tokio::task::spawn_blocking(move || (ops.probe(false), ops.candidates())).await.unwrap_or_default()
    } else {
        (Probe::default(), Vec::new())
    };
    Json(device_view(supported, admin, &probe, &cands, &book))
}

fn bad(status: StatusCode, message: &str, extra: Value) -> Response {
    let mut v = json!({ "ok": false, "message": message });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    (status, Json(v)).into_response()
}

async fn post_start(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<StartRequest>,
) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    if !cfg!(windows) {
        return bad(StatusCode::BAD_REQUEST, UNSUPPORTED, json!({ "supported": false }));
    }
    let r = req.clone();
    let mut plan = match blocking(&s, move |_, ops| make_plan(&r, ops)).await {
        Ok(p) => p,
        Err(e) => return bad(StatusCode::BAD_REQUEST, &e, json!({})),
    };
    if req.dry_run {
        // Read-only: report exactly what would change, change nothing.
        return Json(json!({
            "ok": true, "dry_run": true, "would_change": plan.items, "skipped": plan.skipped, "game": plan.game,
            "message": if plan.items.is_empty() { "Nothing would change." } else { "This is what would change. Nothing was changed." },
        }))
        .into_response();
    }
    if plan.items.is_empty() {
        return bad(StatusCode::BAD_REQUEST, "Nothing to change with these choices.", json!({ "skipped": plan.skipped }));
    }
    match s.devboost.core.begin(plan.game.clone()) {
        Err(e) => return bad(StatusCode::CONFLICT, &e, json!({})),
        Ok(token) => plan.token = Some(token),
    }
    let (items, skipped, game) = (plan.items.clone(), plan.skipped.clone(), plan.game.clone());
    tokio::spawn(run_boost(s.clone(), plan));
    Json(json!({
        "ok": true, "started": true, "would_change": items, "skipped": skipped, "game": game,
        "message": "Measuring your connection first (a few seconds), then boosting.",
    }))
    .into_response()
}

async fn post_stop(State(s): State<Shared>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    if !cfg!(windows) {
        return Json(json!({ "ok": true, "restored": [], "message": "Nothing to restore: boosting the PC is Windows only." })).into_response();
    }
    let rep = blocking(&s, |c, ops| c.restore_all(ops, "Restored by you.")).await;
    if rep.failed.is_empty() {
        s.event("devboost", format!("Restored everything ({} change(s))", rep.restored.len()), json!({})).await;
        Json(json!({
            "ok": true, "restored": rep.restored,
            "message": if rep.restored.is_empty() { "Nothing was changed.".to_string() } else { "Everything is back as it was.".to_string() },
        }))
        .into_response()
    } else {
        bad(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Some changes could not be restored. Try again, or restart rhcsd: it retries when it starts.",
            json!({ "restored": rep.restored, "failed": rep.failed }),
        )
    }
}

// ------------------------------------------------------------------ tests (a fake OS: nothing real is touched)

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";

    struct FakeState {
        supported: bool,
        admin: bool,
        plan: String,
        reg: HashMap<(String, String), RegValue>,
        /// pid -> (image name, priority class)
        procs: HashMap<u32, (String, u32)>,
        wifi: (u32, u32),
        qos: Vec<String>,
        paused: bool,
        switched: bool,
        switch_result: Result<Option<String>, String>,
        dns_flushed: bool,
        /// Every call that changes something, in order.
        mutations: Vec<String>,
        fail: Option<String>,
        /// Journal file to look at whenever something is about to change.
        journal: Option<PathBuf>,
        journaled_before_change: Vec<bool>,
    }

    struct Fake(Mutex<FakeState>);

    impl Fake {
        fn new() -> Self {
            let mut procs = HashMap::new();
            procs.insert(100, ("HD-Player.exe".to_string(), 0x20));
            procs.insert(200, ("Updater.exe".to_string(), 0x20));
            procs.insert(300, ("Backup.exe".to_string(), 0x20));
            let mut reg = HashMap::new();
            reg.insert((TOAST_KEY.to_string(), TOAST_VALUE.to_string()), RegValue::Dword { value: 1 });
            Fake(Mutex::new(FakeState {
                supported: true,
                admin: true,
                plan: BALANCED.into(),
                reg,
                procs,
                wifi: (3, 3),
                qos: vec![],
                paused: false,
                switched: false,
                switch_result: Ok(Some("Now using Cable".into())),
                dns_flushed: false,
                mutations: vec![],
                fail: None,
                journal: None,
                journaled_before_change: vec![],
            }))
        }
        fn st(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.0.lock().unwrap()
        }
        /// Records a mutation; Err if this one is set to fail.
        fn touch(&self, what: &str) -> Result<(), String> {
            let mut s = self.st();
            if let Some(path) = s.journal.clone() {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let in_journal = text.contains("\"changes\"") && !text.contains("\"changes\": []");
                s.journaled_before_change.push(in_journal);
            }
            if s.fail.as_deref().is_some_and(|f| what.starts_with(f)) {
                return Err(format!("{what} failed"));
            }
            s.mutations.push(what.to_string());
            Ok(())
        }
    }

    impl OsOps for Fake {
        fn supported(&self) -> bool {
            self.st().supported
        }
        fn admin(&self) -> bool {
            self.st().admin
        }
        fn probe(&self, _fresh: bool) -> Probe {
            let s = self.st();
            Probe {
                power_plan: Some("Balanced".into()),
                power_plan_is_high: s.plan == HIGH_PERF_GUID,
                high_perf_available: true,
                game_mode: match s.reg.get(&(GAMEBAR_KEY.to_string(), GAME_MODE_VALUE.to_string())) {
                    Some(RegValue::Dword { value }) => Some(*value),
                    _ => None,
                },
                toast_enabled: match s.reg.get(&(TOAST_KEY.to_string(), TOAST_VALUE.to_string())) {
                    Some(RegValue::Dword { value }) => Some(*value),
                    _ => None,
                },
                wifi_power_available: true,
            }
        }
        fn candidates(&self) -> Vec<Candidate> {
            vec![]
        }
        fn process_name(&self, pid: u32) -> Option<String> {
            self.st().procs.get(&pid).map(|p| p.0.clone())
        }
        fn power_active(&self) -> Result<String, String> {
            Ok(self.st().plan.clone())
        }
        fn power_set(&self, guid: &str) -> Result<(), String> {
            self.touch(&format!("power_set:{guid}"))?;
            self.st().plan = guid.into();
            Ok(())
        }
        fn reg_get(&self, key: &str, name: &str) -> Result<Option<RegValue>, String> {
            Ok(self.st().reg.get(&(key.to_string(), name.to_string())).cloned())
        }
        fn reg_set(&self, key: &str, name: &str, v: &RegValue) -> Result<(), String> {
            self.touch(&format!("reg_set:{name}"))?;
            self.st().reg.insert((key.to_string(), name.to_string()), v.clone());
            Ok(())
        }
        fn reg_delete(&self, key: &str, name: &str) -> Result<(), String> {
            self.touch(&format!("reg_delete:{name}"))?;
            self.st().reg.remove(&(key.to_string(), name.to_string()));
            Ok(())
        }
        fn priority_get(&self, pid: u32) -> Result<u32, String> {
            self.st().procs.get(&pid).map(|p| p.1).ok_or_else(|| "no such process".into())
        }
        fn priority_set(&self, pid: u32, class: u32) -> Result<(), String> {
            self.touch(&format!("priority_set:{pid}:{class:#x}"))?;
            self.st().procs.get_mut(&pid).ok_or("no such process")?.1 = class;
            Ok(())
        }
        fn wifi_power_get(&self, _scheme: &str) -> Result<(u32, u32), String> {
            Ok(self.st().wifi)
        }
        fn wifi_power_set(&self, _scheme: &str, ac: u32, dc: u32) -> Result<(), String> {
            self.touch(&format!("wifi_power_set:{ac},{dc}"))?;
            self.st().wifi = (ac, dc);
            Ok(())
        }
        fn qos_add(&self, policy: &str, exe: &str) -> Result<(), String> {
            self.touch(&format!("qos_add:{exe}"))?;
            self.st().qos.push(policy.into());
            Ok(())
        }
        fn qos_remove(&self, policy: &str) -> Result<(), String> {
            self.touch("qos_remove")?;
            self.st().qos.retain(|p| p != policy);
            Ok(())
        }
        fn flush_dns(&self) -> Result<(), String> {
            self.touch("flush_dns")?;
            self.st().dns_flushed = true;
            Ok(())
        }
        fn pause_scans(&self, on: bool) {
            let mut s = self.st();
            if s.paused != on {
                s.mutations.push(format!("pause_scans:{on}"));
            }
            s.paused = on;
        }
        fn switch_best(&self) -> Result<Option<String>, String> {
            self.touch("switch_best")?;
            let r = self.st().switch_result.clone();
            if matches!(r, Ok(Some(_))) {
                self.st().switched = true;
            }
            r
        }
        fn switch_restore(&self) -> Result<(), String> {
            self.touch("switch_restore")?;
            self.st().switched = false;
            Ok(())
        }
    }

    fn all_actions() -> Vec<String> {
        ACTIONS.iter().map(|a| a.id.to_string()).collect()
    }

    fn full_request() -> StartRequest {
        StartRequest { actions: all_actions(), game_pid: Some(100), background_pids: vec![200, 300], dry_run: false }
    }

    fn started(core: &Core, fake: &Fake, req: &StartRequest) -> (Plan, ApplyReport) {
        let mut plan = make_plan(req, fake).unwrap();
        plan.token = Some(core.begin(plan.game.clone()).unwrap());
        let rep = core.apply(fake, &plan);
        (plan, rep)
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rhcsd-test-devboost-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn dry_run_plans_exactly_and_changes_nothing() {
        let fake = Fake::new();
        let plan = make_plan(&full_request(), &fake).unwrap();
        let ids: Vec<&str> = plan.items.iter().map(|i| i.id.as_str()).collect();
        // Canonical order, whatever order the request lists them in; Game Mode is added, notifications turned off.
        assert_eq!(
            ids,
            [
                "power_plan_high", "game_mode_on", "game_priority_high", "background_low_priority", "quiet_notifications",
                "pause_pulse_scans", "wifi_max_performance", "network_best_link", "network_dscp", "flush_dns"
            ]
        );
        assert_eq!(plan.items[0].detail, "Power plan: Balanced → High performance");
        assert_eq!(plan.items[1].detail, "Game Mode: not set → on");
        assert_eq!(plan.items[3].targets.len(), 2);
        assert!(plan.skipped.is_empty());
        let s = fake.st();
        assert!(s.mutations.is_empty(), "a plan must not change anything: {:?}", s.mutations);
        assert_eq!(s.plan, BALANCED);
        assert!(!s.paused && !s.switched && s.qos.is_empty());
        drop(s);
        // The book is untouched too.
        assert!(!Core::in_memory().snapshot().busy());
    }

    #[test]
    fn plan_skips_what_does_not_apply() {
        let fake = Fake::new();
        fake.st().admin = false;
        let req = StartRequest { actions: all_actions(), game_pid: None, background_pids: vec![], dry_run: true };
        let plan = make_plan(&req, &fake).unwrap();
        let skip = |id: &str| plan.skipped.iter().find(|s| s.id == id).map(|s| s.reason.clone()).unwrap_or_default();
        assert!(skip("network_best_link").contains("administrator"));
        assert!(skip("network_dscp").contains("administrator"));
        assert!(skip("game_priority_high").contains("pick the game"));
        assert!(skip("background_low_priority").contains("no programs"));
        // Already-true settings are skipped rather than "changed".
        fake.st().plan = HIGH_PERF_GUID.into();
        let plan = make_plan(&StartRequest { actions: vec!["power_plan_high".into()], ..Default::default() }, &fake).unwrap();
        assert!(plan.items.is_empty() && plan.skipped[0].reason.contains("already"));
        // Bad input.
        assert!(make_plan(&StartRequest { actions: vec!["format_c".into()], ..Default::default() }, &fake).unwrap_err().contains("unknown action"));
        assert!(make_plan(&StartRequest { game_pid: Some(9999), ..Default::default() }, &fake).unwrap_err().contains("not running"));
        fake.st().procs.insert(500, ("lsass.exe".into(), 0x20));
        assert!(make_plan(&StartRequest { game_pid: Some(500), ..Default::default() }, &fake).unwrap_err().contains("system program"));
        let bg = StartRequest { actions: vec!["background_low_priority".into()], background_pids: vec![500, 4, 0], ..Default::default() };
        assert!(make_plan(&bg, &fake).unwrap().items.is_empty(), "system programs never get a lowered priority");
        fake.st().supported = false;
        assert!(make_plan(&full_request(), &fake).unwrap_err().contains("Windows only"));
    }

    #[test]
    fn apply_then_restore_in_reverse_order_puts_everything_back() {
        let fake = Fake::new();
        let core = Core::in_memory();
        let (_, rep) = started(&core, &fake, &full_request());
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        {
            let s = fake.st();
            assert_eq!(
                s.mutations,
                [
                    format!("power_set:{HIGH_PERF_GUID}"),
                    "reg_set:AutoGameModeEnabled".into(),
                    "priority_set:100:0x80".into(),
                    "priority_set:200:0x4000".into(),
                    "priority_set:300:0x4000".into(),
                    "reg_set:ToastEnabled".into(),
                    "pause_scans:true".into(),
                    "wifi_power_set:0,0".into(),
                    "switch_best".into(),
                    "qos_add:HD-Player.exe".into(),
                    "flush_dns".into(),
                ]
            );
            assert_eq!(s.plan, HIGH_PERF_GUID);
            assert!(s.paused && s.switched && s.dns_flushed);
            assert_eq!(s.procs[&100].1, 0x80);
            assert_eq!(s.reg[&(TOAST_KEY.to_string(), TOAST_VALUE.to_string())], RegValue::Dword { value: 0 });
        }
        let b = core.snapshot();
        assert!(b.active && b.phase == "active");
        assert_eq!(b.game, Some(GameRef { pid: 100, name: "HD-Player.exe".into() }));
        assert_eq!(b.changes.len(), 11, "one entry per change; the priority actions have one per program");

        fake.st().mutations.clear();
        let rr = core.restore_all(&fake, "Restored by you.");
        assert!(rr.failed.is_empty(), "{:?}", rr.failed);
        let s = fake.st();
        // Newest first.
        let expect: Vec<String> = [
            "qos_remove",
            "switch_restore",
            "wifi_power_set:3,3",
            "pause_scans:false",
            "reg_set:ToastEnabled",
            "priority_set:300:0x20",
            "priority_set:200:0x20",
            "priority_set:100:0x20",
            "reg_delete:AutoGameModeEnabled",
            &format!("power_set:{BALANCED}"),
        ]
        .iter()
        .map(|x| x.to_string())
        .collect();
        assert_eq!(s.mutations, expect);
        assert_eq!(s.plan, BALANCED);
        assert_eq!((s.wifi, s.paused, s.switched), ((3, 3), false, false));
        assert!(s.qos.is_empty());
        assert_eq!(s.procs[&100].1, 0x20);
        assert_eq!(s.reg[&(TOAST_KEY.to_string(), TOAST_VALUE.to_string())], RegValue::Dword { value: 1 });
        assert!(!s.reg.contains_key(&(GAMEBAR_KEY.to_string(), GAME_MODE_VALUE.to_string())), "a value that did not exist is deleted again");
        drop(s);
        let b = core.snapshot();
        assert!(!b.active && b.changes.is_empty() && b.phase == "idle");
    }

    #[test]
    fn journal_is_written_before_each_change() {
        let dir = tmp("journal");
        let fake = Fake::new();
        fake.st().journal = Some(dir.join("devboost.json"));
        let core = Core::open(&dir);
        started(&core, &fake, &StartRequest { actions: vec!["power_plan_high".into(), "quiet_notifications".into()], ..Default::default() });
        let seen = fake.st().journaled_before_change.clone();
        assert!(!seen.is_empty() && seen.iter().all(|b| *b), "every change was already in devboost.json when it happened: {seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_recovery_restores_from_the_file() {
        let dir = tmp("crash");
        let fake = Fake::new();
        {
            let core = Core::open(&dir);
            let (_, rep) = started(&core, &fake, &full_request());
            assert!(rep.failed.is_empty());
            // The process dies here: no restore, the Core is just dropped.
        }
        assert_eq!(fake.st().plan, HIGH_PERF_GUID, "the PC is still altered");
        let text = std::fs::read_to_string(dir.join("devboost.json")).unwrap();
        assert!(text.contains("power_plan") && text.contains(BALANCED));

        // rhcsd starts again.
        let core = Core::open(&dir);
        assert!(core.snapshot().active, "the file lists unrestored changes");
        let rep = core.recover(&fake).expect("pending changes are restored");
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        {
            let s = fake.st();
            assert_eq!(s.plan, BALANCED);
            assert!(!s.paused && !s.switched && s.qos.is_empty());
            assert_eq!(s.procs[&100].1, 0x20);
        }
        assert!(core.recover(&fake).is_none(), "nothing left to recover");
        let again = Core::open(&dir).snapshot();
        assert!(!again.active && again.changes.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recovery_leaves_a_reused_pid_alone() {
        let fake = Fake::new();
        let core = Core::in_memory();
        started(&core, &fake, &StartRequest { actions: vec!["game_priority_high".into()], game_pid: Some(100), ..Default::default() });
        // The game is gone and Windows gave pid 100 to another program.
        fake.st().procs.insert(100, ("Other.exe".into(), 0x20));
        fake.st().mutations.clear();
        core.restore_all(&fake, "x");
        assert!(fake.st().mutations.is_empty(), "never touch a program that is not the one we changed");
        assert!(core.snapshot().changes.is_empty());
    }

    #[test]
    fn restores_by_itself_when_the_game_exits() {
        let fake = Fake::new();
        let core = Core::in_memory();
        started(&core, &fake, &StartRequest { actions: vec!["power_plan_high".into(), "game_priority_high".into()], game_pid: Some(100), ..Default::default() });
        assert!(!core.check_game(&fake), "game still running: nothing happens");
        assert_eq!(fake.st().plan, HIGH_PERF_GUID);
        fake.st().procs.remove(&100);
        assert!(core.check_game(&fake), "game exited: restored");
        assert_eq!(fake.st().plan, BALANCED);
        let b = core.snapshot();
        assert!(!b.active && b.changes.is_empty());
        assert!(b.message.unwrap().contains("HD-Player.exe closed"));
        assert!(!core.check_game(&fake), "and only once");
    }

    #[test]
    fn one_failing_change_does_not_stop_the_rest_and_leaves_no_entry() {
        let fake = Fake::new();
        fake.st().fail = Some("wifi_power_set".into());
        let core = Core::in_memory();
        let req = StartRequest { actions: vec!["power_plan_high".into(), "wifi_max_performance".into(), "quiet_notifications".into()], ..Default::default() };
        let (_, rep) = started(&core, &fake, &req);
        assert_eq!(rep.failed.len(), 1);
        assert_eq!(rep.failed[0]["id"], "wifi_max_performance");
        let ids: Vec<String> = core.snapshot().changes.iter().map(|c| c.id.clone()).collect();
        assert_eq!(ids, ["power_plan_high", "quiet_notifications"]);
        assert_eq!(fake.st().wifi, (3, 3));
    }

    #[test]
    fn switching_nothing_is_a_note_not_a_change() {
        let fake = Fake::new();
        fake.st().switch_result = Ok(None);
        let core = Core::in_memory();
        let (_, rep) = started(&core, &fake, &StartRequest { actions: vec!["network_best_link".into()], ..Default::default() });
        assert!(rep.failed.is_empty() && rep.notes.iter().any(|n| n.contains("already in use")), "{:?}", rep.notes);
        assert!(core.snapshot().changes.is_empty() && !core.snapshot().active);
        core.restore_all(&fake, "x");
        assert!(!fake.st().mutations.iter().any(|m| m == "switch_restore"), "rhcs must not undo a switch it did not make");
    }

    #[test]
    fn only_one_boost_at_a_time_and_a_cancelled_plan_is_not_applied() {
        let fake = Fake::new();
        let core = Core::in_memory();
        let mut plan = make_plan(&StartRequest { actions: vec!["power_plan_high".into()], ..Default::default() }, &fake).unwrap();
        plan.token = Some(core.begin(None).unwrap());
        assert!(core.begin(None).unwrap_err().contains("already running"));
        // "Restore everything" pressed while still measuring.
        core.restore_all(&fake, "Restored by you.");
        let rep = core.apply(&fake, &plan);
        assert!(rep.notes[0].contains("Cancelled"));
        assert!(fake.st().mutations.is_empty());
        assert!(core.begin(None).is_ok(), "the slot is free again");
    }

    #[test]
    fn json_shapes() {
        let fake = Fake::new();
        let core = Core::in_memory();
        let cands = parse_candidates(r#"[{"pid":100,"name":"HD-Player","title":"Example Game","mem":900,"path":"C:\\Program Files\\Emu\\HD-Player.exe"},{"pid":7,"name":"notepad","title":"secret.txt - Notepad","mem":30,"path":""}]"#);
        let idle = device_view(true, true, &fake.probe(false), &cands, &core.snapshot());
        for k in ["supported", "admin", "actions", "candidates", "state", "last_result"] {
            assert!(idle.get(k).is_some(), "missing {k}");
        }
        let a = &idle["actions"][0];
        for k in ["id", "title", "description", "needs_admin", "available", "default_on", "active", "detail"] {
            assert!(a.get(k).is_some(), "action missing {k}");
        }
        assert_eq!(idle["actions"].as_array().unwrap().len(), ACTIONS.len());
        assert_eq!(a["detail"], "now: Balanced");
        assert_eq!(a["active"], false);
        assert_eq!(idle["state"]["phase"], "idle");
        for k in ["active", "started_at", "game", "changes"] {
            assert!(idle["state"].get(k).is_some(), "state missing {k}");
        }
        let c = &idle["candidates"][0];
        assert_eq!((c["pid"].as_u64(), c["name"].as_str(), c["mem_mb"].as_u64(), c["likely_game"].as_bool()), (Some(100), Some("HD-Player"), Some(900), Some(true)));
        assert_eq!(c["window_title"], "Example Game");
        assert!(idle["candidates"][1].get("window_title").is_none(), "titles of ordinary programs are not exposed");
        assert_eq!(idle["candidates"][1]["likely_game"], false);

        started(&core, &fake, &full_request());
        let on = device_view(true, true, &fake.probe(false), &[], &core.snapshot());
        assert_eq!(on["state"]["active"], true);
        assert_eq!(on["state"]["game"]["pid"], 100);
        assert_eq!(on["actions"][0]["active"], true);
        assert!(on["actions"][0]["detail"].as_str().unwrap().contains("High performance"));
        let flush = on["state"]["changes"].as_array().unwrap().iter().find(|c| c["id"] == "flush_dns").unwrap();
        assert_eq!(flush["reversible"], false);

        // Not Windows: plain reason, nothing available.
        let off = device_view(false, false, &Probe::default(), &[], &Book::default());
        assert_eq!(off["supported"], false);
        assert!(off["unsupported_reason"].as_str().unwrap().contains("Android"));
        assert!(off["actions"].as_array().unwrap().iter().all(|a| a["available"] == false));
        // Not admin: admin actions unavailable with the reason.
        let noadmin = device_view(true, false, &fake.probe(false), &[], &Book::default());
        let best = noadmin["actions"].as_array().unwrap().iter().find(|a| a["id"] == "network_best_link").unwrap();
        assert_eq!((best["available"].clone(), best["needs_admin"].clone()), (json!(false), json!(true)));
        assert!(best["detail"].as_str().unwrap().contains("administrator"));
    }

    fn snap(ping: Option<f64>, jitter: Option<f64>, loss: Option<f64>) -> Snap {
        Snap { link_id: "10.0.0.2".into(), link_name: "Cable".into(), ping_ms: ping, jitter_ms: jitter, loss_pct: loss, at: 0 }
    }

    #[test]
    fn verdict_is_honest_about_noise() {
        // Real improvement.
        let v = verdict(&snap(Some(80.0), Some(12.0), Some(0.0)), &snap(Some(55.0), Some(5.0), Some(0.0)));
        assert_eq!(v.improved, Some(true));
        assert!(v.text.contains("ping went from 80 ms to 55 ms") && v.text.contains("jitter from 12.0 ms to 5.0 ms"), "{}", v.text);
        assert_eq!(v.ping_ms, Some(-25.0));
        // Within the noise: say so, never claim a gain.
        let v = verdict(&snap(Some(80.0), Some(6.0), Some(0.0)), &snap(Some(77.0), Some(6.5), Some(0.0)));
        assert_eq!(v.improved, None);
        assert!(v.text.starts_with("No measurable change: your game was limited by the connection, not the PC."), "{}", v.text);
        // Worse.
        let v = verdict(&snap(Some(50.0), None, None), &snap(Some(70.0), None, None));
        assert_eq!(v.improved, Some(false));
        assert!(v.text.contains("higher after the boost"));
        // Mixed.
        let v = verdict(&snap(Some(80.0), Some(4.0), Some(0.0)), &snap(Some(50.0), Some(12.0), Some(0.0)));
        assert_eq!(v.improved, None);
        assert!(v.text.contains("Mixed"));
        // No answers: no numbers invented.
        let v = verdict(&snap(None, None, None), &snap(Some(50.0), None, None));
        assert_eq!((v.improved, v.ping_ms), (None, None));
        assert!(v.text.contains("Could not compare"));
        // A link change is reported.
        let mut after = snap(Some(40.0), Some(3.0), Some(0.0));
        after.link_id = "10.0.1.2".into();
        after.link_name = "Phone (USB)".into();
        let v = verdict(&snap(Some(90.0), Some(9.0), Some(0.0)), &after);
        assert!(v.text.starts_with("The boost moved new connections from Cable to Phone (USB)."), "{}", v.text);
    }

    #[test]
    fn parsers() {
        let active = "Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)";
        assert_eq!(parse_active_scheme(active), Some((BALANCED.to_string(), Some("Balanced".to_string()))));
        assert_eq!(find_guid("x 8C5E7FDA-E8BF-4A96-9A85-A6E23A8C635C y").as_deref(), Some(HIGH_PERF_GUID));
        assert_eq!(find_guid("no guid here"), None);
        let q = "  GUID Alias: x\n    Minimum Possible Setting: 0x00000000\n    Maximum Possible Setting: 0x00000003\n    Possible Settings increment: 0x00000001\n    Current AC Power Setting Index: 0x00000003\n    Current DC Power Setting Index: 0x00000002\n";
        assert_eq!(parse_power_indexes(q), Some((3, 2)));
        assert_eq!(parse_power_indexes("nothing"), None);
        let reg = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\GameBar\r\n    AutoGameModeEnabled    REG_DWORD    0x1\r\n";
        assert_eq!(parse_reg_query(reg, "AutoGameModeEnabled"), Some(RegValue::Dword { value: 1 }));
        assert_eq!(parse_reg_query("    Name    REG_SZ    two words", "Name"), Some(RegValue::Str { value: "two words".into() }));
        assert_eq!(parse_reg_query(reg, "Other"), None);
        // Candidates: shells and rhcsd itself are hidden, big first, hints from names/paths only.
        let j = r#"[{"pid":10,"name":"explorer","title":"x","mem":500,"path":""},
                    {"pid":11,"name":"LdPlayer","title":"Example","mem":2000,"path":""},
                    {"pid":12,"name":"Game","title":"Game","mem":4000,"path":"D:\\Steam\\steamapps\\common\\Game\\Game.exe"},
                    {"pid":13,"name":"browser","title":"private tab","mem":3000,"path":""}]"#;
        let c = parse_candidates(j);
        let names: Vec<&str> = c.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Game", "browser", "LdPlayer"]);
        assert_eq!(c[0].hint.as_deref(), Some("Steam game"));
        assert!(c[2].likely_game && c[2].hint.as_deref().unwrap().contains("LDPlayer"));
        assert!(!c[1].likely_game && c[1].window_title.is_none());
        assert_eq!(parse_candidates("").len() + parse_candidates("nope").len(), 0);
    }

    #[test]
    fn stub_os_reports_unsupported() {
        #[cfg(not(windows))]
        {
            assert!(sys::power_active().is_err());
            assert!(sys::candidates().is_empty());
            assert!(sys::process_name(100).is_none());
        }
        assert!(UNSUPPORTED.contains("Android"));
    }
}
