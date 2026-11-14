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
    /// RHCSD_LOCK_READS=on: LAN callers need the token for reads too.
    pub lock_reads: bool,
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
    /// Wi-Fi names of the routers (ROUTER_WIFI_SSID, ROUTER2_WIFI_SSID), for naming links.
    pub router_ssids: Vec<String>,
    /// Wi-Fi name(s) of the phone's hotspot (PHONE_HOTSPOT_SSID, comma separated).
    pub phone_hotspot_ssids: Vec<String>,
    /// RHCSD_APP=1: rhcsd runs inside the Android app. The app pushes the phone's own
    /// cell/location data, there is no ADB/Termux/router, and every /api and /ws call
    /// needs the token, even from loopback (other apps on the phone can reach it).
    pub app_mode: bool,
    /// A router address was given (always true outside app mode, where it defaults).
    pub router_configured: bool,
    /// Where towers.json, places.json and switch.json live (RHCSD_DATA_DIR).
    pub data_dir: PathBuf,
    /// How tower positions are looked up (TOWER_LOOKUP): on = curl, external = the app, off.
    pub tower_lookup: crate::towers::LookupMode,
    /// OpenCellID API key (OPENCELLID_KEY), only used by the curl lookups.
    pub opencellid_key: Option<String>,
    /// HOME_LAT / HOME_LON
    pub home: Option<(f64, f64)>,
    /// Extra browser origins allowed to call the API (RHCSD_ALLOWED_ORIGINS, comma separated).
    pub allowed_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self::from_map(&rhcs_core::env::load())
    }

    /// Pure form of [`Config::from_env`] (tests and the app pass their own map).
    pub fn from_map(e: &HashMap<String, String>) -> Self {
        let get = |k: &str| e.get(k).cloned().filter(|v| !v.trim().is_empty());
        let app_mode = rhcs_core::env::truthy(get("RHCSD_APP").as_deref());

        let adb_path = get("ADB_PATH").unwrap_or_else(|| {
            let sdk = std::env::var("LOCALAPPDATA")
                .map(|l| PathBuf::from(l).join("Android/Sdk/platform-tools/adb.exe"))
                .ok()
                .filter(|p| p.is_file());
            sdk.map(|p| p.display().to_string()).unwrap_or_else(|| "adb".into())
        });

        let mut known_macs = HashMap::new();
        for (k, v) in e.iter() {
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
            bind: get("RHCSD_BIND").unwrap_or_else(|| if app_mode { "127.0.0.1:7878" } else { "0.0.0.0:7878" }.into()),
            token: get("RHCSD_TOKEN"),
            lock_reads: get("RHCSD_LOCK_READS").is_some_and(|v| matches!(v.to_lowercase().as_str(), "on" | "1" | "true" | "yes")),
            // App mode has no default router: unset or empty means "no router".
            router_ip: get("ROUTER_IP").map(|v| v.trim().to_string()).unwrap_or_else(|| if app_mode { String::new() } else { "192.168.0.1".into() }),
            router_configured: !app_mode || get("ROUTER_IP").is_some(),
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
            web_dir: get("RHCSD_WEB_DIR")
                .map(|d| PathBuf::from(d.trim()))
                .unwrap_or_else(|| rhcs_core::env::project_dir().join("web").join("dist")),
            app_mode,
            data_dir: rhcs_core::env::data_dir_from(e),
            tower_lookup: crate::towers::lookup_mode(get("TOWER_LOOKUP").as_deref(), app_mode),
            allowed_origins: get("RHCSD_ALLOWED_ORIGINS")
                .map(|s| s.split(',').map(|o| o.trim().trim_end_matches('/').to_string()).filter(|o| !o.is_empty()).collect())
                .unwrap_or_default(),
            opencellid_key: get("OPENCELLID_KEY").map(|k| k.trim().to_string()),
            home: crate::towers::parse_home(e.get("HOME_LAT").map(String::as_str), e.get("HOME_LON").map(String::as_str)),
            router_ssids: ["ROUTER_WIFI_SSID", "ROUTER2_WIFI_SSID"].iter().filter_map(|k| get(k)).collect(),
            phone_hotspot_ssids: get("PHONE_HOTSPOT_SSID")
                .map(|s| s.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect())
                .unwrap_or_default(),
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
    /// False when no router address is set (the app): PULSE then hides router-only UI and errors.
    pub router_configured: bool,
    pub router: RouterState,
    pub phone: SourceState,
    pub scan: ScanState,
    /// The phone's position (Termux:API or `adb shell dumpsys location`).
    pub location: SourceState,
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
    /// Asked to end the process cleanly (POST /api/engine/stop).
    pub shutdown: Notify,
    pub relogin: AtomicBool,
    pub oui: OuiDb,
    /// Game Boost measurements (see boost.rs).
    pub boost: crate::boost::Boost,
    /// Towers seen, handovers, tower locations and home location (see towers.rs).
    pub towers: crate::towers::Towers,
    /// The phone's latest position (see location.rs).
    pub location: RwLock<Option<crate::location::Fix>>,
    /// Places found automatically from positions + signal (see places.rs).
    pub places: crate::places::Places,
    /// Interface metrics changed by switching, for an exact restore (see switch.rs).
    pub switch: crate::switch::Switcher,
    /// rhcsd runs elevated (Windows administrator): switching links is possible.
    pub admin: bool,
    /// Wi-Fi names seen as the phone's hotspot this session.
    pub learned_hotspot_ssids: RwLock<Vec<String>>,
}

impl AppState {
    pub fn new(cfg: Config) -> Shared {
        let (tx, _) = broadcast::channel(512);
        let sources = Sources {
            router_configured: cfg.router_configured,
            router: RouterState { host: cfg.router_ip.clone(), model: cfg.router_model.clone(), ..Default::default() },
            ..Default::default()
        };
        let towers = crate::towers::Towers::open(&cfg);
        let places = crate::places::Places::open(&cfg.data_dir);
        let switch = crate::switch::Switcher::open(&cfg.data_dir);
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
            shutdown: Notify::new(),
            relogin: AtomicBool::new(false),
            oui: OuiDb::load(),
            boost: crate::boost::Boost::new(),
            towers,
            location: RwLock::new(None),
            places,
            switch,
            admin: crate::switch::is_elevated(),
            learned_hotspot_ssids: RwLock::new(Vec::new()),
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
        drop(i);
        if let Some(f) = &*self.location.read().await {
            out.push(json!({ "type": "location", "ts": ts, "data": f }));
        }
        let i = self.inner.read().await;
        let skip = i.events.len().saturating_sub(50);
        out.extend(i.events.iter().skip(skip).cloned());
        out
    }
}
