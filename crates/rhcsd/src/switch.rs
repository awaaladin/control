//! Switching which link carries the internet (Windows only, needs admin).
//!
//! Windows sends new connections over the interface with the lowest metric.
//! Switching sets the chosen interface's IPv4 metric to 10 (automatic metric
//! off) and every other internet link's to 500, through the IP Helper API
//! (`SetIpInterfaceEntry`). Sockets that already exist stay on their link, so
//! a running game keeps its connection; only new connections use the new link.
//!
//! The first time an interface is touched, its original setting (automatic, or
//! a fixed metric) is remembered in `data/switch.json`, so "restore" puts back
//! exactly what was there, even after rhcsd restarts.

use crate::links::Link;
use crate::state::{now_ms, Shared};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::sync::Mutex;

pub const PREFERRED_METRIC: u32 = 10;
pub const DEMOTED_METRIC: u32 = 500;

pub const ADMIN_HELP: &str = "Switching needs administrator rights. Close rhcsd (Ctrl+C in its window), then: Start menu → type PowerShell → right-click Windows PowerShell → Run as administrator, and run:  cd D:\\setup\\rhcs ; .\\target\\debug\\rhcsd.exe";

/// One interface's IPv4 metric setting.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricSetting {
    pub automatic: bool,
    pub metric: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Touched {
    pub alias: String,
    pub original: MetricSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SwitchBook {
    /// Interface index → original setting, recorded on first touch.
    pub touched: BTreeMap<u32, Touched>,
    /// Alias of the link chosen last, while switched.
    pub preferred: Option<String>,
    pub last_switch: Option<i64>,
    pub last_message: Option<String>,
}

/// One metric change to apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub if_index: u32,
    pub alias: String,
    pub set: MetricSetting,
}

/// The changes that make `chosen` the preferred link: it gets metric 10,
/// every other internet link 500. Pure (unit-tested). Each interface appears once.
pub fn plan(links: &[Link], chosen_alias: &str) -> Result<Vec<Change>, String> {
    if !links.iter().any(|l| l.adapter == chosen_alias) {
        return Err(format!("no internet link called \"{chosen_alias}\" right now"));
    }
    let mut out: Vec<Change> = Vec::new();
    for l in links {
        if l.if_index == 0 || out.iter().any(|c| c.if_index == l.if_index) {
            continue;
        }
        let metric = if l.adapter == chosen_alias { PREFERRED_METRIC } else { DEMOTED_METRIC };
        out.push(Change { if_index: l.if_index, alias: l.adapter.clone(), set: MetricSetting { automatic: false, metric } });
    }
    Ok(out)
}

pub struct Switcher {
    pub book: Mutex<SwitchBook>,
    path: PathBuf,
}

impl Switcher {
    pub fn open(dir: &std::path::Path) -> Self {
        let path = dir.join("switch.json");
        let book = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { book: Mutex::new(book), path }
    }

    fn save(&self, b: &SwitchBook) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if b.touched.is_empty() {
            let _ = std::fs::remove_file(&self.path);
        } else if let Ok(t) = serde_json::to_string_pretty(b) {
            if std::fs::write(&self.path, t).is_err() {
                eprintln!("switch: could not save {}", self.path.display());
            }
        }
    }
}

pub fn supported() -> bool {
    cfg!(windows)
}

// ------------------------------------------------------------------ OS calls

/// True when this process runs elevated (Windows administrator).
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut el = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut el as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && el.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(windows)]
fn read_metric(if_index: u32) -> Result<MetricSetting, String> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetIpInterfaceEntry, InitializeIpInterfaceEntry, MIB_IPINTERFACE_ROW};
    use windows_sys::Win32::Networking::WinSock::AF_INET;
    unsafe {
        let mut row: MIB_IPINTERFACE_ROW = std::mem::zeroed();
        InitializeIpInterfaceEntry(&mut row);
        row.Family = AF_INET;
        row.InterfaceIndex = if_index;
        let rc = GetIpInterfaceEntry(&mut row);
        if rc != 0 {
            return Err(format!("could not read interface {if_index} (error {rc})"));
        }
        Ok(MetricSetting { automatic: row.UseAutomaticMetric != 0, metric: row.Metric })
    }
}

#[cfg(windows)]
fn write_metric(if_index: u32, set: MetricSetting) -> Result<(), String> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetIpInterfaceEntry, InitializeIpInterfaceEntry, SetIpInterfaceEntry, MIB_IPINTERFACE_ROW,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;
    unsafe {
        let mut row: MIB_IPINTERFACE_ROW = std::mem::zeroed();
        InitializeIpInterfaceEntry(&mut row);
        row.Family = AF_INET;
        row.InterfaceIndex = if_index;
        let rc = GetIpInterfaceEntry(&mut row);
        if rc != 0 {
            return Err(format!("could not read interface {if_index} (error {rc})"));
        }
        row.SitePrefixLength = 0; // must be 0 for IPv4 when setting
        row.UseAutomaticMetric = u8::from(set.automatic);
        if !set.automatic {
            row.Metric = set.metric;
        }
        match SetIpInterfaceEntry(&mut row) {
            0 => Ok(()),
            5 => Err(format!("access denied changing interface {if_index}: {ADMIN_HELP}")),
            rc => Err(format!("could not change interface {if_index} (error {rc})")),
        }
    }
}

#[cfg(not(windows))]
fn read_metric(_if_index: u32) -> Result<MetricSetting, String> {
    Err("switching links is only supported on Windows".into())
}

#[cfg(not(windows))]
fn write_metric(_if_index: u32, _set: MetricSetting) -> Result<(), String> {
    Err("switching links is only supported on Windows".into())
}

// ------------------------------------------------------------------ actions

/// Makes the link with this adapter alias the one Windows uses for new
/// connections. Returns a plain-words message.
pub async fn switch_to(s: &Shared, alias: &str) -> Result<String, String> {
    if !supported() {
        return Err("switching links is only supported on Windows".into());
    }
    if !s.admin {
        return Err(ADMIN_HELP.into());
    }
    let links = crate::links::discover(s).await;
    let changes = plan(&links, alias)?;
    let name = links.iter().find(|l| l.adapter == alias).map(|l| l.name.clone()).unwrap_or_else(|| alias.to_string());
    let mut book = s.switch.book.lock().await;
    let mut done = Vec::new();
    for c in &changes {
        if !book.touched.contains_key(&c.if_index) {
            let original = tokio::task::spawn_blocking({
                let i = c.if_index;
                move || read_metric(i)
            })
            .await
            .map_err(|e| e.to_string())??;
            book.touched.insert(c.if_index, Touched { alias: c.alias.clone(), original });
            s.switch.save(&book);
        }
        let (i, set) = (c.if_index, c.set);
        tokio::task::spawn_blocking(move || write_metric(i, set)).await.map_err(|e| e.to_string())??;
        done.push(format!("{} → {}", c.alias, c.set.metric));
    }
    book.preferred = Some(alias.to_string());
    book.last_switch = Some(now_ms());
    let msg = format!("Now using {name} for new connections ({}). Your current game keeps its connection.", done.join(", "));
    book.last_message = Some(msg.clone());
    s.switch.save(&book);
    Ok(msg)
}

/// Puts every touched interface back exactly as it was.
pub async fn restore(s: &Shared) -> Result<String, String> {
    if !supported() {
        return Err("switching links is only supported on Windows".into());
    }
    if !s.admin {
        return Err(ADMIN_HELP.into());
    }
    let mut book = s.switch.book.lock().await;
    if book.touched.is_empty() {
        return Ok("Nothing to restore: rhcs hasn't changed any link.".into());
    }
    let mut errors = Vec::new();
    let mut done = Vec::new();
    let entries: Vec<(u32, Touched)> = book.touched.iter().map(|(k, v)| (*k, v.clone())).collect();
    for (i, t) in entries {
        let set = t.original;
        match tokio::task::spawn_blocking(move || write_metric(i, set)).await.map_err(|e| e.to_string()).and_then(|r| r) {
            Ok(()) => {
                book.touched.remove(&i);
                done.push(t.alias);
            }
            // The adapter is gone (unplugged): nothing to restore on it.
            Err(e) if e.contains("could not read") => {
                book.touched.remove(&i);
            }
            Err(e) => errors.push(format!("{}: {e}", t.alias)),
        }
    }
    let res = if errors.is_empty() {
        let msg = format!("Restored Windows' own choice on {}.", if done.is_empty() { "all links".into() } else { done.join(", ") });
        book.preferred = None;
        Ok(msg)
    } else {
        Err(errors.join("; "))
    };
    book.last_switch = Some(now_ms());
    book.last_message = Some(match &res {
        Ok(m) => m.clone(),
        Err(e) => format!("restore failed: {e}"),
    });
    s.switch.save(&book);
    res
}

// ------------------------------------------------------------------ API

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/links", get(get_links))
        .route("/api/links/switch", post(post_switch))
        .route("/api/links/restore", post(post_restore))
}

pub async fn status(s: &Shared) -> Value {
    let b = s.switch.book.lock().await;
    json!({
        "admin": s.admin,
        "supported": supported(),
        "admin_help": if s.admin || !supported() { Value::Null } else { json!(ADMIN_HELP) },
        "switched": !b.touched.is_empty(),
        "preferred": b.preferred,
        "touched": b.touched.values().map(|t| json!({ "alias": t.alias, "original": t.original })).collect::<Vec<_>>(),
        "last_switch": b.last_switch,
        "last_message": b.last_message,
    })
}

async fn get_links(State(s): State<Shared>) -> Json<Value> {
    let links = crate::links::discover(&s).await;
    let mut v = status(&s).await;
    v["links"] = json!(links);
    Json(v)
}

#[derive(Deserialize)]
struct SwitchRequest {
    alias: String,
}

fn refused(msg: String, admin: bool) -> Response {
    let code = if admin { StatusCode::BAD_REQUEST } else { StatusCode::FORBIDDEN };
    (code, Json(json!({ "ok": false, "message": msg, "admin": admin }))).into_response()
}

async fn post_switch(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<SwitchRequest>,
) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    match switch_to(&s, req.alias.trim()).await {
        Ok(msg) => {
            s.event("link_switch", msg.clone(), json!({ "alias": req.alias, "auto": false })).await;
            Json(json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => refused(e, s.admin),
    }
}

async fn post_restore(State(s): State<Shared>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    // Auto-switch would undo a restore, so it stops too.
    crate::boost::set_auto(&s, false).await;
    match restore(&s).await {
        Ok(msg) => {
            s.event("link_switch", msg.clone(), json!({ "restore": true })).await;
            Json(json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => refused(e, s.admin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::LinkKind;

    fn link(adapter: &str, if_index: u32, ip: &str) -> Link {
        Link {
            id: ip.into(),
            name: adapter.into(),
            kind: LinkKind::Other,
            medium: "Wi-Fi".into(),
            adapter: adapter.into(),
            description: String::new(),
            local_ip: ip.parse().unwrap(),
            gateway: None,
            metric: Some(25),
            is_default: false,
            if_index,
            ssid: None,
        }
    }

    #[test]
    fn plan_prefers_one_and_demotes_the_rest_once_each() {
        let links = vec![link("WiFi", 12, "10.1.0.2"), link("Ethernet 2", 20, "10.2.0.2"), link("Ethernet 2", 20, "10.2.0.3")];
        let p = plan(&links, "Ethernet 2").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], Change { if_index: 12, alias: "WiFi".into(), set: MetricSetting { automatic: false, metric: 500 } });
        assert_eq!(p[1].set, MetricSetting { automatic: false, metric: 10 });
        assert!(plan(&links, "Nope").unwrap_err().contains("Nope"));
    }
}
