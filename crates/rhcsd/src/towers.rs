//! Tower map: every LTE cell the router or the phone has been served by, how
//! long and how well, the handovers between them, and where each cell's tower
//! is relative to home.
//!
//! Tower positions come from public databases, asked one cell at a time:
//! beaconDB first (no key needed), then OpenCellID when `OPENCELLID_KEY` is
//! set and beaconDB doesn't know the cell. Only the cell IDs are sent, never
//! a position (`considerIp: false` stops beaconDB guessing from our address).
//! `TOWER_LOOKUP=off` in .env turns all outside lookups off. Nothing is
//! guessed: a cell no database knows has no position.
//!
//! Home location: `HOME_LAT` / `HOME_LON` in .env at startup, or
//! `POST /api/location` (browser). The phone's own position (location.rs) is
//! kept separately as the "current" position and used for distances when no
//! home is set; a manual home always wins.
//!
//! Everything is kept in `data/towers.json` (git-ignored: it contains tower IDs
//! and your home location).

use crate::state::{now_ms, Shared};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::RwLock;

const HANDOVERS_MAX: usize = 300;
/// Gaps longer than this between readings don't count as time on a cell.
const MAX_GAP_MS: i64 = 30_000;
const SAVE_EVERY: Duration = Duration::from_secs(60);
const LOOKUP_GAP: Duration = Duration::from_secs(5);
const RETRY_ERROR_MS: i64 = 10 * 60_000;
const RETRY_NOT_FOUND_MS: i64 = 7 * 24 * 3_600_000;
pub const BEACONDB_URL: &str = "https://api.beacondb.net/v1/geolocate";
pub const USER_AGENT: &str = "rhcs/0.1 (+https://github.com/awaaladin/control)";

// ------------------------------------------------------------------ geo maths

const EARTH_R_M: f64 = 6_371_008.8;

/// Great-circle distance in metres (haversine).
pub fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_R_M * a.sqrt().asin()
}

/// Initial bearing from point 1 to point 2, degrees clockwise from north (0-360).
pub fn bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dl = (lon2 - lon1).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

pub fn compass(deg: f64) -> &'static str {
    const N: [&str; 8] = ["north", "north-east", "east", "south-east", "south", "south-west", "west", "north-west"];
    N[(((deg % 360.0) + 22.5) / 45.0) as usize % 8]
}

// ------------------------------------------------------------------ records

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: Option<f64>,
    /// "env", "phone", "browser", …
    pub source: String,
    pub ts: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CellLocation {
    pub lat: f64,
    pub lon: f64,
    /// The database's estimate of the cell's coverage radius / accuracy.
    pub range_m: Option<f64>,
    /// How many measurements the position is based on (OpenCellID only).
    pub samples: Option<u64>,
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Lookup {
    /// "found" | "not_found" | "error" (the API adds "off", "incomplete", "pending")
    pub status: String,
    pub at: i64,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceStats {
    pub first_seen: i64,
    pub last_seen: i64,
    /// Time spent on this cell (seconds), summed over readings.
    pub secs: f64,
    pub samples: u64,
    pub rsrp_sum: f64,
    pub rsrp_n: u64,
    pub best_rsrp: Option<f32>,
    pub sinr_sum: f64,
    pub sinr_n: u64,
    pub best_sinr: Option<f32>,
}

impl SourceStats {
    pub fn avg_rsrp(&self) -> Option<f64> {
        (self.rsrp_n > 0).then(|| self.rsrp_sum / self.rsrp_n as f64)
    }
    pub fn avg_sinr(&self) -> Option<f64> {
        (self.sinr_n > 0).then(|| self.sinr_sum / self.sinr_n as f64)
    }
}

fn max_opt(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, y) => x.or(y),
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cell {
    pub key: String,
    pub mcc: Option<String>,
    pub mnc: Option<String>,
    pub tac: Option<u32>,
    pub cell_id: u32,
    pub pci: Option<u32>,
    pub bands: Vec<u16>,
    pub earfcn: Option<u32>,
    pub first_seen: i64,
    pub last_seen: i64,
    pub router: Option<SourceStats>,
    pub phone: Option<SourceStats>,
    pub location: Option<CellLocation>,
    pub lookup: Option<Lookup>,
}

impl Cell {
    /// Has everything OpenCellID needs.
    pub fn lookupable(&self) -> bool {
        self.mcc.is_some() && self.mnc.is_some() && self.tac.is_some()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HandoverEnd {
    pub key: String,
    pub cell_id: u32,
    pub pci: Option<u32>,
    pub band: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Handover {
    pub ts: i64,
    pub source: String,
    pub from: HandoverEnd,
    pub to: HandoverEnd,
    /// Seconds spent on the previous cell before switching.
    pub stayed_s: f64,
}

/// One serving-cell reading from the router or the phone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub mcc: Option<String>,
    pub mnc: Option<String>,
    pub tac: Option<u32>,
    pub cell_id: u32,
    pub pci: Option<u32>,
    pub band: Option<u16>,
    pub earfcn: Option<u32>,
    pub rsrp: Option<f32>,
    pub sinr: Option<f32>,
}

pub fn cell_key(mcc: Option<&str>, mnc: Option<&str>, tac: Option<u32>, cell_id: u32) -> String {
    format!(
        "{}-{}-{}-{}",
        mcc.unwrap_or("x"),
        mnc.unwrap_or("x"),
        tac.map(|t| t.to_string()).unwrap_or_else(|| "x".into()),
        cell_id
    )
}

impl Reading {
    pub fn key(&self) -> String {
        cell_key(self.mcc.as_deref(), self.mnc.as_deref(), self.tac, self.cell_id)
    }
}

#[derive(Clone, Debug)]
struct Serving {
    key: String,
    since: i64,
    last: i64,
    end: HandoverEnd,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Book {
    pub version: u32,
    pub cells: BTreeMap<String, Cell>,
    pub handovers: VecDeque<Handover>,
    pub home: Option<Location>,
    /// The phone's latest position (not saved: it is re-read every 30 s).
    #[serde(skip)]
    pub current: Option<Location>,
    #[serde(skip)]
    serving: HashMap<String, Serving>,
    #[serde(skip)]
    dirty: bool,
}

impl Book {
    /// Records one reading. Returns the handover if the serving cell changed.
    pub fn observe(&mut self, source: &str, ts: i64, r: &Reading) -> Option<Handover> {
        let key = r.key();
        let cell = self.cells.entry(key.clone()).or_insert_with(|| Cell {
            key: key.clone(),
            mcc: r.mcc.clone(),
            mnc: r.mnc.clone(),
            tac: r.tac,
            cell_id: r.cell_id,
            first_seen: ts,
            ..Default::default()
        });
        cell.last_seen = ts;
        cell.pci = r.pci.or(cell.pci);
        cell.earfcn = r.earfcn.or(cell.earfcn);
        if let Some(b) = r.band {
            if !cell.bands.contains(&b) {
                cell.bands.push(b);
                cell.bands.sort_unstable();
            }
        }

        let prev = self.serving.get(source).cloned();
        let gap = prev.as_ref().map(|p| ts - p.last);
        let same = prev.as_ref().is_some_and(|p| p.key == key);

        let st = match source {
            "phone" => cell.phone.get_or_insert_with(|| SourceStats { first_seen: ts, ..Default::default() }),
            _ => cell.router.get_or_insert_with(|| SourceStats { first_seen: ts, ..Default::default() }),
        };
        st.last_seen = ts;
        st.samples += 1;
        if same {
            if let Some(g) = gap.filter(|g| (0..=MAX_GAP_MS).contains(g)) {
                st.secs += g as f64 / 1000.0;
            }
        }
        if let Some(v) = r.rsrp {
            st.rsrp_sum += v as f64;
            st.rsrp_n += 1;
            st.best_rsrp = max_opt(st.best_rsrp, Some(v));
        }
        if let Some(v) = r.sinr {
            st.sinr_sum += v as f64;
            st.sinr_n += 1;
            st.best_sinr = max_opt(st.best_sinr, Some(v));
        }

        let end = HandoverEnd { key: key.clone(), cell_id: r.cell_id, pci: r.pci, band: r.band };
        let mut handover = None;
        match prev {
            Some(p) if p.key != key => {
                let h = Handover {
                    ts,
                    source: source.to_string(),
                    from: p.end.clone(),
                    to: end.clone(),
                    stayed_s: (p.last - p.since) as f64 / 1000.0,
                };
                self.handovers.push_back(h.clone());
                while self.handovers.len() > HANDOVERS_MAX {
                    self.handovers.pop_front();
                }
                handover = Some(h);
                self.serving.insert(source.into(), Serving { key, since: ts, last: ts, end });
            }
            Some(p) => {
                // Same cell; if the source was gone for a long time, restart the stay.
                let since = if gap.is_some_and(|g| g > MAX_GAP_MS) { ts } else { p.since };
                self.serving.insert(source.into(), Serving { key, since, last: ts, end });
            }
            None => {
                self.serving.insert(source.into(), Serving { key, since: ts, last: ts, end });
            }
        }
        self.dirty = true;
        handover
    }

    /// Where distances are measured from: the manual home, else the phone's
    /// current position. The bool says whether it is the manual home.
    pub fn origin(&self) -> Option<(&Location, bool)> {
        self.home.as_ref().map(|h| (h, true)).or_else(|| self.current.as_ref().map(|c| (c, false)))
    }

    /// The next cell the databases should be asked about, if any.
    pub fn next_lookup(&self, now: i64) -> Option<Cell> {
        self.cells
            .values()
            .filter(|c| c.lookupable())
            .find(|c| match &c.lookup {
                None => true,
                Some(l) if l.status == "error" => now - l.at > RETRY_ERROR_MS,
                Some(l) if l.status == "not_found" => now - l.at > RETRY_NOT_FOUND_MS,
                _ => false,
            })
            .cloned()
    }
}

/// Serving-cell reading from a WebSocket broadcast message, if it is one.
/// Router TAC arrives as hex text (ZTE `lac_code`), phone TAC as a number.
pub fn reading_from_message(v: &Value) -> Option<(&'static str, i64, Reading)> {
    let source = match v.get("type")?.as_str()? {
        "telemetry" => "router",
        "phone" => "phone",
        _ => return None,
    };
    let ts = v.get("ts")?.as_i64()?;
    let d = v.get("data")?;
    let u32_of = |k: &str| d.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let f32_of = |k: &str| d.get(k).and_then(Value::as_f64).map(|n| n as f32);
    let text = |k: &str| d.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let cell_id = u32_of("cell_id").filter(|c| *c > 0)?;
    let tac = match d.get("tac") {
        Some(Value::String(s)) => u32::from_str_radix(s.trim(), 16).ok(),
        Some(Value::Number(n)) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
    .filter(|t| *t > 0 && *t != 0xFFFF && *t != 0xFFFFFF);
    Some((
        source,
        ts,
        Reading {
            mcc: text("mcc"),
            mnc: text("mnc"),
            tac,
            cell_id,
            pci: u32_of("pci"),
            band: u32_of("band").and_then(|b| u16::try_from(b).ok()),
            earfcn: u32_of("earfcn"),
            rsrp: f32_of("rsrp"),
            sinr: f32_of("sinr"),
        },
    ))
}

/// OpenCellID `cell/get` answer: Ok(Some) found, Ok(None) not in the database,
/// Err for anything else (bad key, quota, network).
pub fn parse_opencellid(body: &str) -> Result<Option<CellLocation>, String> {
    let v: Value = serde_json::from_str(body.trim()).map_err(|_| {
        let snippet: String = body.trim().chars().take(80).collect();
        format!("unexpected answer from OpenCellID: {snippet}")
    })?;
    if let (Some(lat), Some(lon)) = (v.get("lat").and_then(Value::as_f64), v.get("lon").and_then(Value::as_f64)) {
        if lat == 0.0 && lon == 0.0 {
            return Ok(None);
        }
        return Ok(Some(CellLocation {
            lat,
            lon,
            range_m: v.get("range").and_then(Value::as_f64),
            samples: v.get("samples").and_then(Value::as_u64),
            source: "opencellid".into(),
        }));
    }
    let err = v.get("error").and_then(Value::as_str).unwrap_or("no position in answer");
    let code = v.get("code").and_then(Value::as_i64);
    if code == Some(1) || err.to_ascii_lowercase().contains("not found") {
        Ok(None)
    } else {
        Err(format!("OpenCellID: {err}"))
    }
}

/// JSON body for beaconDB's geolocate API (Ichnaea / Google format) for one
/// LTE cell. None when the cell lacks a numeric MCC/MNC or the TAC.
pub fn beacondb_request(c: &Cell) -> Option<String> {
    let mcc: u16 = c.mcc.as_deref()?.trim().parse().ok()?;
    let mnc: u16 = c.mnc.as_deref()?.trim().parse().ok()?;
    let tac = c.tac?;
    Some(
        json!({
            "considerIp": false,
            "cellTowers": [{
                "radioType": "lte",
                "mobileCountryCode": mcc,
                "mobileNetworkCode": mnc,
                "locationAreaCode": tac,
                "cellId": c.cell_id,
            }]
        })
        .to_string(),
    )
}

/// beaconDB answer: Ok(Some) = the cell's estimated position, Ok(None) = not
/// known (HTTP 404, or only an area/IP fallback, which is not the cell's
/// position), Err = anything else.
pub fn parse_beacondb(status: u16, body: &str) -> Result<Option<CellLocation>, String> {
    let v: Option<Value> = serde_json::from_str(body.trim()).ok();
    match status {
        200 => {
            let v = v.ok_or_else(|| "unexpected answer from beaconDB".to_string())?;
            if v.get("fallback").is_some_and(|f| !f.is_null()) {
                return Ok(None);
            }
            let loc = v.get("location");
            let lat = loc.and_then(|l| l.get("lat")).and_then(Value::as_f64);
            let lon = loc.and_then(|l| l.get("lng")).and_then(Value::as_f64);
            match (lat, lon) {
                (Some(lat), Some(lon)) if valid_coords(lat, lon) => Ok(Some(CellLocation {
                    lat,
                    lon,
                    range_m: v.get("accuracy").and_then(Value::as_f64),
                    samples: None,
                    source: "beacondb".into(),
                })),
                _ => Err("beaconDB answered without a position".into()),
            }
        }
        404 => Ok(None),
        429 => Err("beaconDB: too many requests, retrying later".into()),
        0 => Err("beaconDB could not be reached".into()),
        code => {
            let msg = v
                .as_ref()
                .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            Err(format!("beaconDB: HTTP {code} {msg}").trim().to_string())
        }
    }
}

/// Outcome of asking every database in turn: the first position found wins;
/// otherwise "error" if any database failed (retried soon), else "not_found".
pub fn combine(answers: Vec<(&str, Result<Option<CellLocation>, String>)>, now: i64) -> (Lookup, Option<CellLocation>) {
    let mut notes = Vec::new();
    let mut failed = false;
    for (name, a) in answers {
        match a {
            Ok(Some(loc)) => {
                let msg = format!("position from {name}");
                return (Lookup { status: "found".into(), at: now, message: Some(msg) }, Some(loc));
            }
            Ok(None) => notes.push(format!("not in {name}")),
            Err(e) => {
                failed = true;
                notes.push(e);
            }
        }
    }
    let status = if failed { "error" } else { "not_found" };
    let message = if notes.is_empty() { None } else { Some(notes.join("; ")) };
    (Lookup { status: status.into(), at: now, message }, None)
}

pub fn parse_home(lat: Option<&str>, lon: Option<&str>) -> Option<(f64, f64)> {
    let lat: f64 = lat?.trim().parse().ok()?;
    let lon: f64 = lon?.trim().parse().ok()?;
    valid_coords(lat, lon).then_some((lat, lon))
}

fn valid_coords(lat: f64, lon: f64) -> bool {
    lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) && !(lat == 0.0 && lon == 0.0)
}

// ------------------------------------------------------------------ state

pub struct Towers {
    pub book: RwLock<Book>,
    key: Option<String>,
    /// Outside lookups allowed (`TOWER_LOOKUP`, default on).
    lookup_on: bool,
    path: PathBuf,
}

/// Which outside lookups are configured (for the API view).
#[derive(Clone, Copy, Debug)]
pub struct LookupCfg {
    pub enabled: bool,
    pub opencellid: bool,
}

/// `TOWER_LOOKUP` value: on unless it says off.
pub fn lookup_switch(v: Option<&str>) -> bool {
    !matches!(v.map(|v| v.trim().to_ascii_lowercase()).as_deref(), Some("off" | "0" | "false" | "no"))
}

impl Towers {
    pub fn new() -> Self {
        let e = rhcs_core::env::load();
        let path = rhcs_core::env::project_dir().join("data").join("towers.json");
        let mut book: Book = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        book.version = 1;
        if let Some((lat, lon)) = parse_home(e.get("HOME_LAT").map(String::as_str), e.get("HOME_LON").map(String::as_str)) {
            book.home = Some(Location { lat, lon, accuracy_m: None, source: "env".into(), ts: now_ms() });
        }
        Self {
            book: RwLock::new(book),
            key: e.get("OPENCELLID_KEY").map(|k| k.trim().to_string()).filter(|k| !k.is_empty()),
            lookup_on: lookup_switch(e.get("TOWER_LOOKUP").map(String::as_str)),
            path,
        }
    }

    pub fn cfg(&self) -> LookupCfg {
        LookupCfg { enabled: self.lookup_on, opencellid: self.key.is_some() }
    }

    /// The phone's latest position (used for distances when no home is set).
    pub async fn set_current(&self, loc: Location) {
        self.book.write().await.current = Some(loc);
    }

    /// Plug-in point for any location source (phone GPS, browser, …).
    pub async fn set_home(&self, loc: Location) {
        let mut b = self.book.write().await;
        b.home = Some(loc);
        b.dirty = true;
    }

    async fn save(&self) {
        let text = {
            let mut b = self.book.write().await;
            if !b.dirty {
                return;
            }
            b.dirty = false;
            match serde_json::to_string(&*b) {
                Ok(t) => t,
                Err(_) => return,
            }
        };
        let path = self.path.clone();
        let res = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, &path)
        })
        .await;
        if !matches!(res, Ok(Ok(()))) {
            eprintln!("towers: could not save {}", self.path.display());
            self.book.write().await.dirty = true;
        }
    }
}

/// Follows the router and phone readings on the broadcast channel.
pub async fn run(s: Shared) {
    let mut rx = s.tx.subscribe();
    let mut save = tokio::time::interval(SAVE_EVERY);
    save.tick().await;
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(text) => {
                    // Cheap pre-filter: most messages are not cell readings.
                    if !(text.contains("\"type\":\"telemetry\"") || text.contains("\"type\":\"phone\"")) {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                    if let Some((source, ts, r)) = reading_from_message(&v) {
                        s.towers.book.write().await.observe(source, ts, &r);
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            _ = save.tick() => s.towers.save().await,
        }
    }
}

async fn fetch_beacondb(c: &Cell) -> Result<Option<CellLocation>, String> {
    let Some(body) = beacondb_request(c) else { return Err("the network codes are not numbers".into()) };
    let mut cmd = tokio::process::Command::new("curl");
    cmd.args([
        "-sS",
        "-m",
        "20",
        "-A",
        USER_AGENT,
        "-H",
        "content-type: application/json",
        "--data-binary",
        &body,
        "-w",
        "\n%{http_code}",
        BEACONDB_URL,
    ])
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = cmd.output().await.map_err(|e| format!("could not run curl: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", text.as_str()));
    let code: u16 = code.trim().parse().unwrap_or(0);
    if code == 0 {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "beaconDB could not be reached".into() } else { format!("beaconDB: {err}") });
    }
    parse_beacondb(code, body)
}

async fn fetch_opencellid(key: &str, c: &Cell) -> Result<Option<CellLocation>, String> {
    let url = format!(
        "https://opencellid.org/cell/get?key={key}&mcc={}&mnc={}&lac={}&cellid={}&radio=LTE&format=json",
        c.mcc.as_deref().unwrap_or_default(),
        c.mnc.as_deref().unwrap_or_default(),
        c.tac.unwrap_or_default(),
        c.cell_id
    );
    let mut cmd = tokio::process::Command::new("curl");
    cmd.args(["-sS", "-m", "20", &url]).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = cmd.output().await.map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().replace(key, "***");
        return Err(if err.is_empty() { "OpenCellID request failed".into() } else { err });
    }
    parse_opencellid(&String::from_utf8_lossy(&out.stdout))
}

/// Looks up tower positions (beaconDB, then OpenCellID if a key is set), one
/// cell every few seconds at most, cached on disk.
pub async fn geolocate(s: Shared) {
    if !s.towers.lookup_on {
        println!("  towers  outside lookups off (TOWER_LOOKUP=off)");
        return;
    }
    let key = s.towers.key.clone();
    loop {
        tokio::time::sleep(LOOKUP_GAP).await;
        let Some(cell) = s.towers.book.read().await.next_lookup(now_ms()) else { continue };
        let mut answers = vec![("beaconDB", fetch_beacondb(&cell).await)];
        let found = matches!(answers[0].1, Ok(Some(_)));
        if let (false, Some(k)) = (found, key.as_deref()) {
            tokio::time::sleep(Duration::from_secs(1)).await;
            answers.push(("OpenCellID", fetch_opencellid(k, &cell).await));
        }
        let (lookup, loc) = combine(answers, now_ms());
        {
            let mut b = s.towers.book.write().await;
            if let Some(c) = b.cells.get_mut(&cell.key) {
                if loc.is_some() {
                    c.location = loc;
                }
                c.lookup = Some(lookup);
            }
            b.dirty = true;
        }
        s.towers.save().await;
    }
}

// ------------------------------------------------------------------ API

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/towers", get(get_towers))
        .route("/api/location", get(get_location).post(set_location))
}

fn source_view(st: &Option<SourceStats>) -> Value {
    match st {
        None => Value::Null,
        Some(st) => json!({
            "first_seen": st.first_seen,
            "last_seen": st.last_seen,
            "secs": st.secs,
            "samples": st.samples,
            "best_rsrp": st.best_rsrp,
            "avg_rsrp": st.avg_rsrp(),
            "best_sinr": st.best_sinr,
            "avg_sinr": st.avg_sinr(),
        }),
    }
}

/// The full `/api/towers` answer (pure, so it is tested).
pub fn view(b: &Book, lc: LookupCfg) -> Value {
    let origin = b.origin();
    let home = origin.map(|(l, _)| l);
    let serving = |src: &str| b.serving.get(src).map(|s| s.key.clone());
    let (router_now, phone_now) = (serving("router"), serving("phone"));
    let mut best: Option<(f64, &Cell)> = None;
    let cells: Vec<Value> = b
        .cells
        .values()
        .map(|c| {
            let geo = match (home, &c.location) {
                (Some(h), Some(l)) => {
                    let d = distance_m(h.lat, h.lon, l.lat, l.lon);
                    let br = bearing_deg(h.lat, h.lon, l.lat, l.lon);
                    json!({ "distance_m": d, "bearing_deg": br, "compass": compass(br) })
                }
                _ => json!({ "distance_m": null, "bearing_deg": null, "compass": null }),
            };
            // "Best" = highest average SINR the router saw, else the phone.
            let sinr = c.router.as_ref().and_then(SourceStats::avg_sinr).or_else(|| c.phone.as_ref().and_then(SourceStats::avg_sinr));
            if let Some(v) = sinr {
                if best.is_none_or(|(b, _)| v > b) {
                    best = Some((v, c));
                }
            }
            let lookup = if c.location.is_some() && c.lookup.is_none() {
                json!({ "status": "found", "message": null })
            } else if !lc.enabled {
                json!({ "status": "off", "message": "outside lookups are off (TOWER_LOOKUP=off)" })
            } else if !c.lookupable() {
                json!({ "status": "incomplete", "message": "the TAC or network code is missing, so it can't be looked up" })
            } else {
                match &c.lookup {
                    Some(l) => json!(l),
                    None => json!({ "status": "pending", "message": null }),
                }
            };
            let mut seen_by = Vec::new();
            if c.router.is_some() {
                seen_by.push("router");
            }
            if c.phone.is_some() {
                seen_by.push("phone");
            }
            let mut serving_now = Vec::new();
            if router_now.as_deref() == Some(c.key.as_str()) {
                serving_now.push("router");
            }
            if phone_now.as_deref() == Some(c.key.as_str()) {
                serving_now.push("phone");
            }
            let mut v = json!({
                "key": c.key,
                "mcc": c.mcc, "mnc": c.mnc, "tac": c.tac,
                "cell_id": c.cell_id, "enb": c.cell_id >> 8, "sector": c.cell_id & 0xff,
                "pci": c.pci, "bands": c.bands, "earfcn": c.earfcn,
                "first_seen": c.first_seen, "last_seen": c.last_seen,
                "seen_by": seen_by,
                "serving": serving_now,
                "router": source_view(&c.router),
                "phone": source_view(&c.phone),
                "location": c.location,
                "lookup": lookup,
            });
            if let (Some(o), Some(g)) = (v.as_object_mut(), geo.as_object()) {
                o.extend(g.clone());
            }
            v
        })
        .collect();

    let best = best.map(|(sinr, c)| {
        let place = match (origin, &c.location) {
            (Some((h, manual)), Some(l)) => {
                let br = bearing_deg(h.lat, h.lon, l.lat, l.lon);
                let d = distance_m(h.lat, h.lon, l.lat, l.lon);
                Some(format!(
                    " Its tower is about {:.1} km to the {} ({:.0}°{}): placing the router on the side of the house facing {} usually helps.",
                    d / 1000.0,
                    compass(br),
                    br,
                    if manual { "" } else { ", measured from your phone's position" },
                    compass(br)
                ))
            }
            _ => None,
        };
        json!({
            "key": c.key,
            "avg_sinr": sinr,
            "message": format!(
                "Cleanest signal so far: eNB {} sector {} (average SINR {:.1} dB).{}",
                c.cell_id >> 8,
                c.cell_id & 0xff,
                sinr,
                place.unwrap_or_default()
            ),
        })
    });

    let message = if !lc.enabled {
        Some("Tower lookups are off (TOWER_LOOKUP=off in .env): no tower IDs are sent anywhere, so tower positions are not shown.")
    } else if home.is_none() {
        Some("No position yet for distance and direction: connect the phone (its location is read automatically), press \"Use this device's location\", or set HOME_LAT and HOME_LON in .env.")
    } else {
        None
    };
    let mut sources = Vec::new();
    if lc.enabled {
        sources.push("beacondb");
        if lc.opencellid {
            sources.push("opencellid");
        }
    }

    json!({
        "key_configured": lc.opencellid,
        "lookup": { "enabled": lc.enabled, "sources": sources },
        "message": message,
        "home": home,
        "home_manual": origin.is_some_and(|(_, m)| m),
        "current": b.current,
        "serving": { "router": router_now, "phone": phone_now },
        "cells": cells,
        "handovers": b.handovers.iter().rev().collect::<Vec<_>>(),
        "best": best,
    })
}

async fn get_towers(State(s): State<Shared>) -> Json<Value> {
    let b = s.towers.book.read().await;
    Json(view(&b, s.towers.cfg()))
}

async fn get_location(State(s): State<Shared>) -> Json<Value> {
    Json(json!(s.towers.book.read().await.home))
}

#[derive(Deserialize)]
struct LocationUpdate {
    lat: f64,
    lon: f64,
    accuracy_m: Option<f64>,
    source: Option<String>,
}

async fn set_location(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LocationUpdate>,
) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    if !valid_coords(req.lat, req.lon) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "message": "lat must be -90..90 and lon -180..180" })))
            .into_response();
    }
    let source: String = req.source.unwrap_or_else(|| "api".into()).chars().take(24).collect();
    s.towers
        .set_home(Location { lat: req.lat, lon: req.lon, accuracy_m: req.accuracy_m, source, ts: now_ms() })
        .await;
    s.towers.save().await;
    Json(json!({ "ok": true, "message": "home location updated" })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test-network codes (MCC 001 / MNC 01) and made-up cell numbers only.
    fn rd(cell: u32, sinr: f32) -> Reading {
        Reading {
            mcc: Some("001".into()),
            mnc: Some("01".into()),
            tac: Some(100),
            cell_id: cell,
            pci: Some(7),
            band: Some(3),
            earfcn: Some(1300),
            rsrp: Some(-90.0),
            sinr: Some(sinr),
        }
    }

    #[test]
    fn distance_and_bearing() {
        // One degree of latitude ≈ 111.2 km, due north.
        let d = distance_m(0.0, 0.0, 1.0, 0.0);
        assert!((d - 111_195.0).abs() < 50.0, "{d}");
        assert!((bearing_deg(0.0, 0.0, 1.0, 0.0) - 0.0).abs() < 1e-9);
        assert!((bearing_deg(0.0, 0.0, 0.0, 1.0) - 90.0).abs() < 1e-9);
        assert!((bearing_deg(0.0, 0.0, -1.0, 0.0) - 180.0).abs() < 1e-9);
        assert!((bearing_deg(0.0, 0.0, 0.0, -1.0) - 270.0).abs() < 1e-9);
        // Short hop to the north-east at mid latitude.
        let (lat, lon) = (10.0, 10.0);
        let b = bearing_deg(lat, lon, lat + 0.01, lon + 0.01 / lat.to_radians().cos());
        assert!((b - 45.0).abs() < 0.2, "{b}");
        assert_eq!(compass(b), "north-east");
        assert_eq!(compass(359.0), "north");
        assert_eq!(compass(200.0), "south");
        assert_eq!(compass(292.0), "west");
        assert_eq!(compass(293.0), "north-west");
        assert!(distance_m(5.0, 5.0, 5.0, 5.0).abs() < 1e-6);
    }

    #[test]
    fn readings_from_router_and_phone_messages() {
        let router = json!({"type":"telemetry","ts":1000,"data":{"cell_id":4660,"tac":"1F","mcc":"001","mnc":"01","band":7,"pci":9,"earfcn":2850,"rsrp":-95.0,"sinr":12.5}});
        let (src, ts, r) = reading_from_message(&router).unwrap();
        assert_eq!((src, ts), ("router", 1000));
        assert_eq!(r.tac, Some(31));
        assert_eq!(r.cell_id, 4660);
        assert_eq!(r.band, Some(7));
        assert_eq!(r.sinr, Some(12.5));
        assert_eq!(r.key(), "001-01-31-4660");

        let phone = json!({"type":"phone","ts":2000,"data":{"cell_id":4661,"tac":31,"mcc":"001","mnc":"01","band":3,"rsrp":-85.0,"sinr":null}});
        let (src, _, r) = reading_from_message(&phone).unwrap();
        assert_eq!(src, "phone");
        assert_eq!(r.tac, Some(31));
        assert_eq!(r.sinr, None);

        assert!(reading_from_message(&json!({"type":"phone","ts":1,"data":{"cell_id":null}})).is_none());
        assert!(reading_from_message(&json!({"type":"status","ts":1,"data":{}})).is_none());
        let no_tac = json!({"type":"telemetry","ts":1,"data":{"cell_id":5,"tac":""}});
        assert_eq!(reading_from_message(&no_tac).unwrap().2.key(), "x-x-x-5");
    }

    #[test]
    fn observe_counts_time_and_handovers() {
        let mut b = Book::default();
        assert!(b.observe("router", 0, &rd(0x1201, 10.0)).is_none());
        assert!(b.observe("router", 1000, &rd(0x1201, 14.0)).is_none());
        assert!(b.observe("router", 2000, &rd(0x1201, 12.0)).is_none());
        let h = b.observe("router", 3000, &rd(0x1302, 20.0)).unwrap();
        assert_eq!(h.from.cell_id, 0x1201);
        assert_eq!(h.to.cell_id, 0x1302);
        assert_eq!(h.stayed_s, 2.0);
        // A long gap on the same cell doesn't count as time on it.
        b.observe("router", 3000 + MAX_GAP_MS + 1, &rd(0x1302, 20.0));
        // The phone is tracked separately.
        b.observe("phone", 500, &rd(0x1201, 5.0));

        let c = &b.cells[&rd(0x1201, 0.0).key()];
        let r = c.router.as_ref().unwrap();
        assert_eq!(r.samples, 3);
        assert_eq!(r.secs, 2.0);
        assert_eq!(r.avg_sinr(), Some(12.0));
        assert_eq!(r.best_sinr, Some(14.0));
        assert_eq!(c.phone.as_ref().unwrap().samples, 1);
        assert_eq!(b.cells[&rd(0x1302, 0.0).key()].router.as_ref().unwrap().secs, 0.0);
        assert_eq!(b.handovers.len(), 1);

        let v = view(&b, LookupCfg { enabled: false, opencellid: false });
        assert_eq!(v["key_configured"], json!(false));
        assert!(v["message"].as_str().unwrap().contains("TOWER_LOOKUP=off"));
        assert_eq!(v["cells"].as_array().unwrap().len(), 2);
        assert_eq!(v["serving"]["router"], json!(rd(0x1302, 0.0).key()));
        assert_eq!(v["best"]["key"], json!(rd(0x1302, 0.0).key()));
        assert_eq!(v["cells"][0]["lookup"]["status"], json!("off"));
        // Lookups on (the default), no key needed: cells wait for beaconDB.
        let v = view(&b, LookupCfg { enabled: true, opencellid: false });
        assert_eq!(v["cells"][0]["lookup"]["status"], json!("pending"));
        assert_eq!(v["lookup"]["sources"], json!(["beacondb"]));
        assert!(v["message"].as_str().unwrap().contains("No position yet"));
    }

    #[test]
    fn view_adds_distance_and_direction() {
        let mut b = Book::default();
        b.observe("router", 0, &rd(0x2001, 18.0));
        b.home = Some(Location { lat: 10.0, lon: 10.0, accuracy_m: None, source: "env".into(), ts: 0 });
        let key = rd(0x2001, 0.0).key();
        b.cells.get_mut(&key).unwrap().location =
            Some(CellLocation { lat: 10.009, lon: 10.0, range_m: Some(1000.0), samples: Some(3), source: "opencellid".into() });
        let v = view(&b, LookupCfg { enabled: true, opencellid: true });
        let c = &v["cells"][0];
        assert!((c["distance_m"].as_f64().unwrap() - 1000.8).abs() < 2.0);
        assert_eq!(c["compass"], json!("north"));
        assert_eq!(c["lookup"]["status"], json!("found"));
        assert!(v["message"].is_null());
        assert_eq!(v["home_manual"], json!(true));
        assert_eq!(v["lookup"]["sources"], json!(["beacondb", "opencellid"]));
        assert!(v["best"]["message"].as_str().unwrap().contains("to the north"));

        // Without a manual home, the phone's current position is used, and said so.
        b.home = None;
        b.current = Some(Location { lat: 10.0, lon: 10.0, accuracy_m: Some(12.0), source: "phone".into(), ts: 0 });
        let v = view(&b, LookupCfg { enabled: true, opencellid: false });
        assert_eq!(v["home_manual"], json!(false));
        assert_eq!(v["home"]["source"], json!("phone"));
        assert!(v["best"]["message"].as_str().unwrap().contains("phone's position"));
        // A manual home wins over the phone position.
        b.home = Some(Location { lat: 10.0, lon: 10.001, accuracy_m: None, source: "env".into(), ts: 0 });
        let v = view(&b, LookupCfg { enabled: true, opencellid: false });
        assert_eq!(v["home"]["source"], json!("env"));
        assert_eq!(v["current"]["source"], json!("phone"));
    }

    fn lookup_cell() -> Cell {
        Cell { key: "k".into(), mcc: Some("001".into()), mnc: Some("01".into()), tac: Some(100), cell_id: 4660, ..Default::default() }
    }

    #[test]
    fn beacondb_request_body() {
        let body: Value = serde_json::from_str(&beacondb_request(&lookup_cell()).unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"considerIp": false, "cellTowers": [{"radioType": "lte", "mobileCountryCode": 1,
                   "mobileNetworkCode": 1, "locationAreaCode": 100, "cellId": 4660}]})
        );
        let mut no_tac = lookup_cell();
        no_tac.tac = None;
        assert!(beacondb_request(&no_tac).is_none());
        let mut bad = lookup_cell();
        bad.mnc = Some("x".into());
        assert!(beacondb_request(&bad).is_none());
    }

    #[test]
    fn beacondb_answers() {
        // Shapes as returned by api.beacondb.net (made-up position).
        let ok = parse_beacondb(200, r#"{"location":{"lat":10.5,"lng":7.25},"accuracy":850}"#).unwrap().unwrap();
        assert_eq!((ok.lat, ok.lon, ok.range_m, ok.source.as_str()), (10.5, 7.25, Some(850.0), "beacondb"));
        // An IP / area fallback is not the cell's position.
        let ipf = r#"{"location":{"lat":10.5,"lng":7.25},"accuracy":25000,"fallback":"ipf","license":"x"}"#;
        assert_eq!(parse_beacondb(200, ipf), Ok(None));
        let nf = r#"{"error":{"code":404,"errors":[{"domain":"geolocation","message":"No location could be estimated based on the data provided","reason":"notFound"}],"message":"Not found"}}"#;
        assert_eq!(parse_beacondb(404, nf), Ok(None));
        assert!(parse_beacondb(429, "").unwrap_err().contains("too many"));
        assert!(parse_beacondb(500, r#"{"error":{"message":"boom"}}"#).unwrap_err().contains("HTTP 500 boom"));
        assert!(parse_beacondb(200, "<html>").is_err());
        assert!(parse_beacondb(0, "").is_err());
    }

    #[test]
    fn databases_combined_in_order() {
        let loc = CellLocation { lat: 1.0, lon: 2.0, range_m: None, samples: None, source: "opencellid".into() };
        let (l, found) = combine(vec![("beaconDB", Ok(None)), ("OpenCellID", Ok(Some(loc.clone())))], 5);
        assert_eq!((l.status.as_str(), l.at), ("found", 5));
        assert_eq!(found, Some(loc));
        let (l, found) = combine(vec![("beaconDB", Ok(None)), ("OpenCellID", Ok(None))], 5);
        assert_eq!(l.status, "not_found");
        assert_eq!(l.message.as_deref(), Some("not in beaconDB; not in OpenCellID"));
        assert!(found.is_none());
        let (l, _) = combine(vec![("beaconDB", Err("beaconDB could not be reached".into()))], 5);
        assert_eq!(l.status, "error");
        assert!(lookup_switch(None) && lookup_switch(Some("on")) && !lookup_switch(Some(" OFF ")) && !lookup_switch(Some("0")));
    }

    #[test]
    fn opencellid_answers() {
        let ok = r#"{"lat":10.5,"lon":7.25,"mcc":1,"mnc":1,"lac":100,"cellid":4660,"averageSignalStrength":0,"range":1500,"samples":12,"changeable":1,"radio":"LTE"}"#;
        let loc = parse_opencellid(ok).unwrap().unwrap();
        assert_eq!((loc.lat, loc.lon, loc.range_m, loc.samples), (10.5, 7.25, Some(1500.0), Some(12)));
        assert_eq!(parse_opencellid(r#"{"error":"Cell not found","code":1}"#), Ok(None));
        assert!(parse_opencellid(r#"{"error":"Invalid token","code":2}"#).unwrap_err().contains("Invalid token"));
        assert!(parse_opencellid("<html>502</html>").is_err());
    }

    #[test]
    fn next_lookup_respects_retry_windows() {
        let mut b = Book::default();
        b.observe("router", 0, &rd(0x3001, 1.0));
        let key = rd(0x3001, 0.0).key();
        assert!(b.next_lookup(0).is_some());
        b.cells.get_mut(&key).unwrap().lookup = Some(Lookup { status: "error".into(), at: 0, message: None });
        assert!(b.next_lookup(1000).is_none());
        assert!(b.next_lookup(RETRY_ERROR_MS + 1).is_some());
        b.cells.get_mut(&key).unwrap().lookup = Some(Lookup { status: "found".into(), at: 0, message: None });
        assert!(b.next_lookup(i64::MAX / 2).is_none());
    }

    #[test]
    fn home_from_env_values() {
        assert_eq!(parse_home(Some(" 6.5 "), Some("3.4")), Some((6.5, 3.4)));
        assert_eq!(parse_home(Some("95"), Some("3")), None);
        assert_eq!(parse_home(Some("0"), Some("0")), None);
        assert_eq!(parse_home(None, Some("3")), None);
        assert_eq!(parse_home(Some("abc"), Some("3")), None);
    }
}
