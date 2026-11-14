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
//! `TOWER_LOOKUP=external` (the default inside the Android app) never starts
//! curl: cells that need a position are listed by `GET /api/towers/lookups`, the
//! app asks the databases itself and posts each answer to `POST /api/towers/location`.
//!
//! Home location: `HOME_LAT` / `HOME_LON` in .env at startup, or
//! `POST /api/location` (browser). The phone's own position (location.rs) is
//! kept separately as the "current" position and used for distances when no
//! home is set; a manual home always wins.
//!
//! Everything is kept in `data/towers.json` (git-ignored: it contains tower IDs
//! and your home location).

use crate::phone::Neighbor;
use crate::state::{now_ms, Config, Shared};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
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
/// Neighbour cells are only added to the table while it is smaller than this.
const CELLS_MAX: usize = 2000;
/// A "stronger cell nearby" needs at least this much more RSRP than the serving cell.
const MIN_GAIN_DB: f32 = 6.0;
/// The phone's cell list older than this is not used for recommendations.
const AROUND_FRESH_MS: i64 = 30_000;
/// Cells handed to the app per `GET /api/towers/lookups`.
const LOOKUPS_PER_CALL: usize = 8;
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
    /// Last signal the phone measured on this cell without being served by it.
    #[serde(default)]
    pub nearby: Option<Measure>,
}

/// A signal measurement of a cell the phone listened to but was not connected to.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Measure {
    pub ts: i64,
    pub rsrp: Option<f32>,
    pub rsrq: Option<f32>,
    pub sinr: Option<f32>,
}

/// One entry of "cells around you": the serving cell first, then what else the phone hears.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Nearby {
    pub serving: bool,
    pub n: Neighbor,
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
    /// What the phone hears right now (not saved): serving cell + neighbours.
    #[serde(skip)]
    pub around: Vec<Nearby>,
    #[serde(skip)]
    pub around_ts: i64,
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
        self.cells.values().find(|c| lookup_due(c, now)).cloned()
    }

    /// Up to `limit` cells that need a position lookup (what the app is asked to do).
    pub fn due_lookups(&self, now: i64, limit: usize) -> Vec<Cell> {
        self.cells.values().filter(|c| lookup_due(c, now)).take(limit).cloned().collect()
    }

    /// Stores the outcome of a lookup (made here or by the app) exactly as an internal one.
    /// False when no such cell is known.
    pub fn store_lookup(&mut self, key: &str, lookup: Lookup, loc: Option<CellLocation>) -> bool {
        let Some(c) = self.cells.get_mut(key) else { return false };
        if loc.is_some() {
            c.location = loc;
        }
        c.lookup = Some(lookup);
        self.dirty = true;
        true
    }

    /// The cell with these (numeric) network codes, however the text was formatted ("01" = "1").
    pub fn find_cell(&self, mcc: u16, mnc: u16, tac: u32, cell_id: u32) -> Option<&Cell> {
        self.cells.values().find(|c| {
            c.cell_id == cell_id
                && c.tac == Some(tac)
                && c.mcc.as_deref().and_then(|m| m.trim().parse::<u16>().ok()) == Some(mcc)
                && c.mnc.as_deref().and_then(|m| m.trim().parse::<u16>().ok()) == Some(mnc)
        })
    }

    /// Records what the phone hears right now: the serving cell and the neighbours. Neighbours
    /// that reveal their cell identity also get an entry in the cells table (with the signal
    /// measured on them, "seen by the phone"), so their towers can be looked up and drawn.
    pub fn observe_around(&mut self, ts: i64, list: Vec<Nearby>) {
        let serving_key = list.iter().find(|n| n.serving).and_then(|n| n.n.key.clone());
        for n in list.iter().filter(|n| !n.serving) {
            let (Some(key), Some(ci)) = (n.n.key.clone(), n.n.ci) else { continue };
            if serving_key.as_deref() == Some(key.as_str()) {
                continue;
            }
            if !self.cells.contains_key(&key) && self.cells.len() >= CELLS_MAX {
                continue;
            }
            let cell = self.cells.entry(key.clone()).or_insert_with(|| Cell {
                key,
                mcc: n.n.mcc.clone(),
                mnc: n.n.mnc.clone(),
                tac: n.n.tac,
                cell_id: ci,
                first_seen: ts,
                ..Default::default()
            });
            cell.last_seen = ts;
            cell.pci = n.n.pci.or(cell.pci);
            cell.earfcn = n.n.earfcn.or(cell.earfcn);
            if let Some(b) = n.n.band {
                if !cell.bands.contains(&b) {
                    cell.bands.push(b);
                    cell.bands.sort_unstable();
                }
            }
            cell.nearby = Some(Measure { ts, rsrp: n.n.rsrp, rsrq: n.n.rsrq, sinr: n.n.sinr });
        }
        self.around = list;
        self.around_ts = ts;
        self.dirty = true;
    }
}

fn lookup_due(c: &Cell, now: i64) -> bool {
    c.lookupable()
        && match &c.lookup {
            None => true,
            Some(l) if l.status == "error" => now - l.at > RETRY_ERROR_MS,
            Some(l) if l.status == "not_found" => now - l.at > RETRY_NOT_FOUND_MS,
            _ => false,
        }
}

/// A cell key for a neighbour that reveals its identity (LTE only: the table is LTE-shaped).
fn neighbor_key(n: &Neighbor) -> Option<String> {
    let lte = n.rat.as_deref().is_none_or(|r| r.eq_ignore_ascii_case("lte"));
    match n.ci {
        Some(ci) if lte && ci > 0 => Some(cell_key(n.mcc.as_deref(), n.mnc.as_deref(), n.tac, ci)),
        _ => None,
    }
}

/// The phone's serving cell plus neighbours from a `phone` broadcast message.
pub fn nearby_from_message(v: &Value) -> Option<(i64, Vec<Nearby>)> {
    if v.get("type")?.as_str()? != "phone" {
        return None;
    }
    let ts = v.get("ts")?.as_i64()?;
    let d = v.get("data")?;
    let mut out = Vec::new();
    let serving = neighbor_of(d, "cell_id");
    if serving.rsrp.is_some() || serving.pci.is_some() || serving.ci.is_some() {
        out.push(Nearby { serving: true, n: serving });
    }
    for item in d.get("neighbors").and_then(Value::as_array).into_iter().flatten() {
        out.push(Nearby { serving: false, n: neighbor_of(item, "ci") });
    }
    for n in out.iter_mut() {
        n.n.key = neighbor_key(&n.n);
    }
    Some((ts, out))
}

fn neighbor_of(d: &Value, id_key: &str) -> Neighbor {
    let u32_of = |k: &str| d.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let f32_of = |k: &str| d.get(k).and_then(Value::as_f64).map(|n| n as f32);
    let text = |k: &str| d.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    Neighbor {
        key: None,
        ci: u32_of(id_key).filter(|c| *c > 0),
        pci: u32_of("pci"),
        tac: u32_of("tac").filter(|t| *t > 0 && *t != 0xFFFF && *t != 0xFFFFFF),
        earfcn: u32_of("earfcn"),
        band: u32_of("band").and_then(|b| u16::try_from(b).ok()),
        mcc: text("mcc"),
        mnc: text("mnc"),
        rsrp: f32_of("rsrp"),
        rsrq: f32_of("rsrq"),
        sinr: f32_of("sinr"),
        rat: text("rat"),
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
    /// Who looks up tower positions (`TOWER_LOOKUP`).
    pub mode: LookupMode,
    path: PathBuf,
}

/// Who asks the public databases where a tower stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupMode {
    /// rhcsd itself, through curl (laptop, Termux).
    Curl,
    /// The Android app: rhcsd lists what it needs and the app posts the answers. No curl.
    External,
    /// Nobody: no tower ID leaves the machine.
    Off,
}

/// Which outside lookups are configured (for the API view).
#[derive(Clone, Copy, Debug)]
pub struct LookupCfg {
    pub enabled: bool,
    pub opencellid: bool,
    pub external: bool,
}

/// `TOWER_LOOKUP` value. Unset means on (curl) normally and `external` inside the app, where
/// curl does not exist; there `on` also means external.
pub fn lookup_mode(v: Option<&str>, app: bool) -> LookupMode {
    match v.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("off" | "0" | "false" | "no") => LookupMode::Off,
        Some("external") => LookupMode::External,
        _ if app => LookupMode::External,
        _ => LookupMode::Curl,
    }
}

impl Towers {
    pub fn open(cfg: &Config) -> Self {
        let path = cfg.data_dir.join("towers.json");
        let mut book: Book = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        book.version = 1;
        if let Some((lat, lon)) = cfg.home {
            book.home = Some(Location { lat, lon, accuracy_m: None, source: "env".into(), ts: now_ms() });
        }
        Self { book: RwLock::new(book), key: cfg.opencellid_key.clone().filter(|k| !k.is_empty()), mode: cfg.tower_lookup, path }
    }

    pub fn cfg(&self) -> LookupCfg {
        LookupCfg {
            enabled: self.mode != LookupMode::Off,
            opencellid: self.key.is_some() && self.mode == LookupMode::Curl,
            external: self.mode == LookupMode::External,
        }
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
                    if let Some((ts, list)) = nearby_from_message(&v) {
                        s.towers.book.write().await.observe_around(ts, list);
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
    match s.towers.mode {
        LookupMode::Off => {
            println!("  towers  outside lookups off (TOWER_LOOKUP=off)");
            return;
        }
        LookupMode::External => {
            println!("  towers  positions are looked up by the app (TOWER_LOOKUP=external), never by curl");
            return;
        }
        LookupMode::Curl => {}
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
        s.towers.book.write().await.store_lookup(&cell.key, lookup, loc);
        s.towers.save().await;
    }
}

// ------------------------------------------------------------------ API

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/towers", get(get_towers))
        .route("/api/towers/lookups", get(get_lookups))
        .route("/api/towers/location", post(post_cell_location))
        .route("/api/location", get(get_location).post(set_location))
}

/// Cells whose position the app should look up (TOWER_LOOKUP=external). Empty in the other modes.
async fn get_lookups(State(s): State<Shared>) -> Json<Value> {
    let external = s.towers.mode == LookupMode::External;
    let list: Vec<Value> = if external {
        let b = s.towers.book.read().await;
        b.due_lookups(now_ms(), LOOKUPS_PER_CALL)
            .iter()
            .map(|c| {
                json!({
                    "mcc": c.mcc, "mnc": c.mnc, "tac": c.tac, "cell_id": c.cell_id,
                    "radio": "lte",
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    Json(json!({ "external": external, "lookups": list }))
}

/// An answer the app got for one cell.
#[derive(Debug, PartialEq)]
pub struct CellReport {
    pub mcc: u16,
    pub mnc: u16,
    pub tac: u32,
    pub cell_id: u32,
    pub outcome: ReportOutcome,
}

#[derive(Debug, PartialEq)]
pub enum ReportOutcome {
    Found(CellLocation),
    NotFound,
    /// The app could not ask (network down, rate limit): retried after a while.
    Failed(String),
}

fn code_of(v: Option<&Value>, max: u64) -> Result<u64, String> {
    let n = match v {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().parse::<u64>().ok(),
        _ => None,
    };
    n.filter(|n| *n <= max).ok_or_else(|| "mcc, mnc, tac and cell_id must be non-negative whole numbers in range".to_string())
}

/// Validates `POST /api/towers/location`. Never panics; every problem is a message.
pub fn parse_report(v: &Value) -> Result<CellReport, String> {
    let o = v.as_object().ok_or("the body must be a JSON object")?;
    let mcc = code_of(o.get("mcc"), 999)? as u16;
    let mnc = code_of(o.get("mnc"), 999)? as u16;
    let tac = code_of(o.get("tac"), 0xFF_FFFF)? as u32;
    let cell_id = code_of(o.get("cell_id"), u32::MAX as u64)? as u32;
    let outcome = if let Some(e) = o.get("error").and_then(Value::as_str).filter(|e| !e.trim().is_empty()) {
        ReportOutcome::Failed(e.trim().chars().filter(|c| !c.is_control()).take(120).collect())
    } else if o.get("found").and_then(Value::as_bool) == Some(false) {
        ReportOutcome::NotFound
    } else {
        let lat = o.get("lat").and_then(Value::as_f64);
        let lon = o.get("lon").and_then(Value::as_f64);
        let (Some(lat), Some(lon)) = (lat, lon) else { return Err("lat and lon are required unless found is false".into()) };
        if !valid_coords(lat, lon) {
            return Err("lat must be -90..90 and lon -180..180 (and not 0,0)".into());
        }
        let range_m = match o.get("accuracy_m") {
            None | Some(Value::Null) => None,
            Some(a) => Some(a.as_f64().filter(|a| a.is_finite() && (0.0..=1.0e7).contains(a)).ok_or("accuracy_m must be 0..10000000")?),
        };
        let source: String = o
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("app")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect();
        ReportOutcome::Found(CellLocation {
            lat,
            lon,
            range_m,
            samples: None,
            source: if source.is_empty() { "app".into() } else { source },
        })
    };
    Ok(CellReport { mcc, mnc, tac, cell_id, outcome })
}

/// Stores a report like an internal lookup. Err when the cell is not one rhcsd knows.
pub fn apply_report(b: &mut Book, r: CellReport, now: i64) -> Result<&'static str, String> {
    let key = b
        .find_cell(r.mcc, r.mnc, r.tac, r.cell_id)
        .map(|c| c.key.clone())
        .ok_or("no such cell: only cells rhcsd has seen can be located")?;
    let (lookup, loc, status) = match r.outcome {
        ReportOutcome::Found(l) => (Lookup { status: "found".into(), at: now, message: Some(format!("position from {}", l.source)) }, Some(l), "found"),
        ReportOutcome::NotFound => (Lookup { status: "not_found".into(), at: now, message: Some("not in the public databases".into()) }, None, "not_found"),
        ReportOutcome::Failed(e) => (Lookup { status: "error".into(), at: now, message: Some(e) }, None, "error"),
    };
    b.store_lookup(&key, lookup, loc);
    Ok(status)
}

async fn post_cell_location(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    if s.towers.mode != LookupMode::External {
        return (StatusCode::CONFLICT, Json(json!({ "ok": false, "message": "tower positions are not looked up by an app here (TOWER_LOOKUP is not external)" })))
            .into_response();
    }
    let report = match parse_report(&body) {
        Ok(r) => r,
        Err(m) => return (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "message": m }))).into_response(),
    };
    let res = apply_report(&mut *s.towers.book.write().await, report, now_ms());
    match res {
        Ok(status) => {
            s.towers.save().await;
            Json(json!({ "ok": true, "status": status })).into_response()
        }
        Err(m) => (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "message": m }))).into_response(),
    }
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

fn lookup_json(c: &Cell, lc: LookupCfg) -> Value {
    if c.location.is_some() && c.lookup.is_none() {
        json!({ "status": "found", "message": null })
    } else if !lc.enabled {
        json!({ "status": "off", "message": "outside lookups are off (TOWER_LOOKUP=off)" })
    } else if !c.lookupable() {
        json!({ "status": "incomplete", "message": "the TAC or network code is missing, so it can't be looked up" })
    } else {
        match &c.lookup {
            Some(l) => json!(l),
            None if lc.external => json!({ "status": "pending", "message": "waiting for the app to look it up" }),
            None => json!({ "status": "pending", "message": null }),
        }
    }
}

/// Carrier frequency of the common LTE bands, for words like "band 3 (1800 MHz)".
pub fn band_mhz(band: u16) -> Option<&'static str> {
    Some(match band {
        1 => "2100 MHz",
        3 => "1800 MHz",
        5 => "850 MHz",
        7 => "2600 MHz",
        8 => "900 MHz",
        20 => "800 MHz",
        28 => "700 MHz",
        38 => "2600 MHz TDD",
        40 => "2300 MHz TDD",
        41 => "2500 MHz TDD",
        _ => return None,
    })
}

fn band_words(band: Option<u16>) -> String {
    match band {
        Some(b) => match band_mhz(b) {
            Some(m) => format!("band {b} ({m})"),
            None => format!("band {b}"),
        },
        None => "an unknown band".to_string(),
    }
}

fn distance_words(m: f64) -> String {
    if m < 1000.0 {
        format!("about {} m", ((m / 10.0).round() * 10.0).max(10.0) as i64)
    } else {
        format!("about {:.1} km", m / 1000.0)
    }
}

/// "Is there a stronger cell than the one the phone is on?" From the phone's live cell list:
/// a same-technology neighbour with clearly more RSRP (and not much noisier) is worth saying.
/// The wording is honest: a phone picks its cell itself and cannot be pushed onto another.
pub fn recommend(b: &Book, origin: Option<&Location>, now: i64) -> Value {
    if b.around.is_empty() {
        return Value::Null;
    }
    if now - b.around_ts > AROUND_FRESH_MS {
        return json!({ "status": "stale", "message": "The phone has not reported its cells for a moment, so there is nothing to compare right now." });
    }
    let Some(serving) = b.around.iter().find(|n| n.serving) else {
        return json!({ "status": "unknown", "message": "The phone reports no serving cell right now." });
    };
    let Some(s_rsrp) = serving.n.rsrp else {
        return json!({ "status": "unknown", "message": "The phone gives no signal strength for its own cell, so cells can't be compared." });
    };
    let same_rat = |a: &Neighbor, c: &Neighbor| {
        a.rat.as_deref().unwrap_or("LTE").eq_ignore_ascii_case(c.rat.as_deref().unwrap_or("LTE"))
    };
    let better = b
        .around
        .iter()
        .filter(|n| !n.serving && same_rat(&serving.n, &n.n))
        .filter_map(|n| n.n.rsrp.map(|r| (r, n)))
        .filter(|(r, _)| *r >= s_rsrp + MIN_GAIN_DB)
        // a stronger but much noisier cell is not a better one
        .filter(|(_, n)| match (n.n.sinr, serving.n.sinr) {
            (Some(c), Some(s)) => c >= s - 3.0,
            _ => true,
        })
        .max_by(|a, c| a.0.total_cmp(&c.0));
    let Some((rsrp, n)) = better else {
        let others = b.around.iter().filter(|n| !n.serving).count();
        return json!({
            "status": "best",
            "message": if others == 0 {
                "The phone hears only its own cell right now.".to_string()
            } else {
                format!("You are on the strongest cell the phone can hear right now ({others} other cell{} heard).", if others == 1 { "" } else { "s" })
            },
        });
    };
    let gain = (rsrp - s_rsrp).round();
    let name = match n.n.ci {
        Some(ci) => format!("tower {} sector {}", ci >> 8, ci & 0xff),
        None => match n.n.pci {
            Some(p) => format!("a cell with PCI {p}"),
            None => "another cell".to_string(),
        },
    };
    let loc = n.n.key.as_ref().and_then(|k| b.cells.get(k)).and_then(|c| c.location.as_ref());
    let (distance_m, bearing, compass_w, place) = match (origin, loc) {
        (Some(o), Some(l)) => {
            let d = distance_m(o.lat, o.lon, l.lat, l.lon);
            let br = bearing_deg(o.lat, o.lon, l.lat, l.lon);
            (Some(d), Some(br), Some(compass(br)), format!(", {} to the {}", distance_words(d), compass(br)))
        }
        _ => (None, None, None, String::new()),
    };
    json!({
        "status": "better",
        "key": n.n.key,
        "ci": n.n.ci,
        "enb": n.n.ci.map(|c| c >> 8),
        "sector": n.n.ci.map(|c| c & 0xff),
        "pci": n.n.pci,
        "band": n.n.band,
        "delta_db": gain,
        "distance_m": distance_m,
        "bearing_deg": bearing,
        "compass": compass_w,
        "can_force": false,
        "message": format!(
            "A stronger cell is nearby: {name} on {}, about {gain:.0} dB stronger than the cell you are on{place}. Your phone chooses its cell by itself and cannot be forced onto this one; it can move over when you change position (near a window, higher up, outdoors).",
            band_words(n.n.band)
        ),
    })
}

/// The "cells around you" list: serving cell first, then neighbours by strength.
fn around_json(b: &Book, lc: LookupCfg, origin: Option<&Location>, now: i64) -> Value {
    if b.around.is_empty() {
        return Value::Null;
    }
    let mut list: Vec<&Nearby> = b.around.iter().collect();
    list.sort_by(|a, c| {
        c.serving
            .cmp(&a.serving)
            .then(c.n.rsrp.unwrap_or(f32::NEG_INFINITY).total_cmp(&a.n.rsrp.unwrap_or(f32::NEG_INFINITY)))
    });
    let cells: Vec<Value> = list
        .into_iter()
        .map(|n| {
            let cell = n.n.key.as_ref().and_then(|k| b.cells.get(k));
            let loc = cell.and_then(|c| c.location.as_ref());
            let geo = match (origin, loc) {
                (Some(o), Some(l)) => {
                    let br = bearing_deg(o.lat, o.lon, l.lat, l.lon);
                    json!({ "distance_m": distance_m(o.lat, o.lon, l.lat, l.lon), "bearing_deg": br, "compass": compass(br) })
                }
                _ => json!({ "distance_m": null, "bearing_deg": null, "compass": null }),
            };
            let mut v = json!({
                "serving": n.serving,
                "key": n.n.key,
                "ci": n.n.ci,
                "enb": n.n.ci.map(|c| c >> 8),
                "sector": n.n.ci.map(|c| c & 0xff),
                "pci": n.n.pci, "tac": n.n.tac, "earfcn": n.n.earfcn, "band": n.n.band,
                "mcc": n.n.mcc, "mnc": n.n.mnc,
                "rsrp": n.n.rsrp, "rsrq": n.n.rsrq, "sinr": n.n.sinr,
                "rat": n.n.rat,
                "mhz": n.n.band.and_then(band_mhz),
                "location": loc,
                "lookup": cell.map(|c| lookup_json(c, lc)),
            });
            if let (Some(o), Some(g)) = (v.as_object_mut(), geo.as_object()) {
                o.extend(g.clone());
            }
            v
        })
        .collect();
    json!({ "ts": b.around_ts, "age_s": ((now - b.around_ts).max(0) / 1000), "cells": cells })
}

/// The full `/api/towers` answer (pure, so it is tested).
pub fn view(b: &Book, lc: LookupCfg, now: i64) -> Value {
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
            let lookup = lookup_json(c, lc);
            let mut seen_by = Vec::new();
            if c.router.is_some() {
                seen_by.push("router");
            }
            if c.phone.is_some() || c.nearby.is_some() {
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
                "nearby": c.nearby,
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
        "lookup": { "enabled": lc.enabled, "sources": sources, "external": lc.external },
        "message": message,
        "home": home,
        "home_manual": origin.is_some_and(|(_, m)| m),
        "current": b.current,
        "serving": { "router": router_now, "phone": phone_now },
        "cells": cells,
        "handovers": b.handovers.iter().rev().collect::<Vec<_>>(),
        "best": best,
        "around": around_json(b, lc, home, now),
        "recommendation": recommend(b, home, now),
    })
}

async fn get_towers(State(s): State<Shared>) -> Json<Value> {
    let b = s.towers.book.read().await;
    Json(view(&b, s.towers.cfg(), now_ms()))
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

        let v = view(&b, LookupCfg { enabled: false, opencellid: false, external: false }, 0);
        assert_eq!(v["key_configured"], json!(false));
        assert!(v["message"].as_str().unwrap().contains("TOWER_LOOKUP=off"));
        assert_eq!(v["cells"].as_array().unwrap().len(), 2);
        assert_eq!(v["serving"]["router"], json!(rd(0x1302, 0.0).key()));
        assert_eq!(v["best"]["key"], json!(rd(0x1302, 0.0).key()));
        assert_eq!(v["cells"][0]["lookup"]["status"], json!("off"));
        // Lookups on (the default), no key needed: cells wait for beaconDB.
        let v = view(&b, LookupCfg { enabled: true, opencellid: false, external: false }, 0);
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
        let v = view(&b, LookupCfg { enabled: true, opencellid: true, external: false }, 0);
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
        let v = view(&b, LookupCfg { enabled: true, opencellid: false, external: false }, 0);
        assert_eq!(v["home_manual"], json!(false));
        assert_eq!(v["home"]["source"], json!("phone"));
        assert!(v["best"]["message"].as_str().unwrap().contains("phone's position"));
        // A manual home wins over the phone position.
        b.home = Some(Location { lat: 10.0, lon: 10.001, accuracy_m: None, source: "env".into(), ts: 0 });
        let v = view(&b, LookupCfg { enabled: true, opencellid: false, external: false }, 0);
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
        assert_eq!(lookup_mode(None, false), LookupMode::Curl);
        assert_eq!(lookup_mode(Some("on"), false), LookupMode::Curl);
        assert_eq!(lookup_mode(Some(" OFF "), false), LookupMode::Off);
        assert_eq!(lookup_mode(Some("0"), true), LookupMode::Off);
        assert_eq!(lookup_mode(Some("external"), false), LookupMode::External);
        // inside the app there is no curl: unset and "on" both mean external
        assert_eq!(lookup_mode(None, true), LookupMode::External);
        assert_eq!(lookup_mode(Some("on"), true), LookupMode::External);
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

    // ---- app mode: neighbours, external lookups, best cell (made-up codes only)

    fn phone_msg(ts: i64, serving_rsrp: f64, neighbors: Value) -> Value {
        json!({"type":"phone","ts":ts,"data":{
            "cell_id":4660,"tac":100,"mcc":"001","mnc":"01","pci":7,"band":3,"earfcn":1300,
            "rsrp":serving_rsrp,"rsrq":-10.0,"sinr":8.0,"rat":"LTE","neighbors":neighbors}})
    }

    fn feed(b: &mut Book, msg: &Value) {
        let (src, ts, r) = reading_from_message(msg).unwrap();
        b.observe(src, ts, &r);
        let (nts, list) = nearby_from_message(msg).unwrap();
        b.observe_around(nts, list);
    }

    #[test]
    fn neighbours_join_the_cells_table() {
        let mut b = Book::default();
        let msg = phone_msg(
            1000,
            -100.0,
            json!([
                {"ci":4661,"pci":8,"tac":100,"mcc":"001","mnc":"01","earfcn":1300,"band":3,"rsrp":-85.0,"rsrq":-8.0,"sinr":12.0,"rat":"LTE"},
                {"ci":4660,"pci":7,"tac":100,"mcc":"001","mnc":"01","rsrp":-100.0,"rat":"LTE"},
                {"pci":99,"earfcn":6300,"band":20,"rsrp":-110.0,"rat":"LTE"},
                {"ci":99999,"pci":5,"tac":100,"mcc":"001","mnc":"01","rsrp":-90.0,"rat":"NR"}
            ]),
        );
        feed(&mut b, &msg);
        // serving cell + the one LTE neighbour that reveals its identity; the duplicate of the
        // serving cell, the identity-less one and the NR one are not table entries
        assert_eq!(b.cells.len(), 2);
        let n = &b.cells[&cell_key(Some("001"), Some("01"), Some(100), 4661)];
        assert!(n.phone.is_none() && n.router.is_none(), "not served by the phone");
        assert_eq!(n.nearby.as_ref().unwrap().rsrp, Some(-85.0));
        assert_eq!((n.pci, n.bands.clone(), n.earfcn), (Some(8), vec![3], Some(1300)));
        // all four are in the live list, serving first
        assert_eq!(b.around.len(), 5);
        assert!(b.around[0].serving && b.around[0].n.key.as_deref() == Some("001-01-100-4660"));
        assert!(b.around[3].n.key.is_none());
        let v = view(&b, LookupCfg { enabled: true, opencellid: false, external: true }, 1500);
        let cell = v["cells"].as_array().unwrap().iter().find(|c| c["cell_id"] == json!(4661)).unwrap();
        assert_eq!(cell["seen_by"], json!(["phone"]));
        assert_eq!(cell["serving"], json!([]));
        assert_eq!(cell["lookup"]["status"], json!("pending"));
        assert_eq!(v["around"]["cells"].as_array().unwrap().len(), 5);
        assert_eq!(v["around"]["cells"][0]["serving"], json!(true));
        assert_eq!(v["around"]["cells"][1]["rsrp"], json!(-85.0)); // strongest neighbour next
        assert_eq!(v["lookup"]["external"], json!(true));
    }

    #[test]
    fn external_lookup_flow() {
        let mut b = Book::default();
        feed(&mut b, &phone_msg(1000, -90.0, json!([{"ci":4661,"tac":100,"mcc":"001","mnc":"01","rsrp":-80.0,"rat":"LTE"}])));
        let due = b.due_lookups(2000, 8);
        assert_eq!(due.len(), 2);
        assert_eq!(b.due_lookups(2000, 1).len(), 1);

        // the app answers "found" for one cell (codes as numbers, leading zeros irrelevant)
        let r = parse_report(&json!({"mcc":1,"mnc":"01","tac":100,"cell_id":4661,"lat":10.5,"lon":7.25,"accuracy_m":800,"source":"beacondb"})).unwrap();
        assert_eq!(apply_report(&mut b, r, 5000), Ok("found"));
        let c = &b.cells["001-01-100-4661"];
        assert_eq!(c.location.as_ref().map(|l| (l.lat, l.lon, l.range_m, l.source.as_str())), Some((10.5, 7.25, Some(800.0), "beacondb")));
        assert_eq!(c.lookup.as_ref().unwrap().status, "found");
        assert_eq!(b.due_lookups(6000, 8).len(), 1);

        // "not found" is retried after a week, a failure after ten minutes
        let r = parse_report(&json!({"mcc":"001","mnc":"01","tac":100,"cell_id":4660,"found":false})).unwrap();
        assert_eq!(apply_report(&mut b, r, 5000), Ok("not_found"));
        assert!(b.due_lookups(5000 + RETRY_ERROR_MS + 1, 8).is_empty());
        assert_eq!(b.due_lookups(5000 + RETRY_NOT_FOUND_MS + 1, 8).len(), 1);
        let r = parse_report(&json!({"mcc":"001","mnc":"01","tac":100,"cell_id":4660,"error":"no network"})).unwrap();
        assert_eq!(apply_report(&mut b, r, 9000), Ok("error"));
        assert_eq!(b.due_lookups(9000 + RETRY_ERROR_MS + 1, 8).len(), 1);
        assert!(b.due_lookups(9000 + 1000, 8).is_empty());

        // unknown cells cannot be located (no poisoning the book with arbitrary cells)
        let r = parse_report(&json!({"mcc":"001","mnc":"01","tac":100,"cell_id":1,"lat":1.0,"lon":2.0})).unwrap();
        assert!(apply_report(&mut b, r, 1).is_err());
    }

    #[test]
    fn report_validation() {
        let base = |extra: Value| {
            let mut v = json!({"mcc":"001","mnc":"01","tac":100,"cell_id":5,"lat":1.0,"lon":2.0});
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v
        };
        assert!(parse_report(&base(json!({}))).is_ok());
        for bad in [
            base(json!({"lat":95.0})), base(json!({"lon":-181.0})), base(json!({"lat":0.0,"lon":0.0})),
            base(json!({"accuracy_m":-1})), base(json!({"accuracy_m":"x"})), base(json!({"mcc":"abc"})),
            base(json!({"mcc":1000})), base(json!({"cell_id":-1})), base(json!({"cell_id":4294967296u64})),
            base(json!({"tac":16777216})), json!({"mcc":1,"mnc":1,"tac":1,"cell_id":1}), json!(null), json!([]),
        ] {
            assert!(parse_report(&bad).is_err(), "{bad}");
        }
        assert!(matches!(parse_report(&base(json!({"found":false}))).unwrap().outcome, ReportOutcome::NotFound));
    }

    #[test]
    fn best_cell_recommendation() {
        let now = 10_000;
        let mut b = Book::default();
        // nothing yet
        assert_eq!(recommend(&b, None, now), Value::Null);
        // serving -100; neighbour 11 dB stronger and clean; one stronger but far noisier; one weaker
        let msg = phone_msg(
            now,
            -100.0,
            json!([
                {"ci":4661,"pci":8,"tac":100,"mcc":"001","mnc":"01","band":7,"rsrp":-89.0,"sinr":10.0,"rat":"LTE"},
                {"ci":4662,"pci":9,"tac":100,"mcc":"001","mnc":"01","band":3,"rsrp":-80.0,"sinr":-5.0,"rat":"LTE"},
                {"ci":4663,"pci":10,"tac":100,"mcc":"001","mnc":"01","band":3,"rsrp":-104.0,"sinr":3.0,"rat":"LTE"}
            ]),
        );
        feed(&mut b, &msg);
        let r = recommend(&b, None, now);
        assert_eq!(r["status"], json!("better"));
        assert_eq!((r["enb"].as_u64(), r["sector"].as_u64(), r["band"].as_u64()), (Some(18), Some(53), Some(7)));
        assert_eq!(r["delta_db"], json!(11.0));
        assert_eq!(r["can_force"], json!(false));
        let m = r["message"].as_str().unwrap();
        assert!(m.contains("stronger cell is nearby") && m.contains("tower 18 sector 53") && m.contains("band 7 (2600 MHz)") && m.contains("11 dB"), "{m}");
        assert!(m.contains("cannot be forced"), "{m}");
        assert!(!m.contains("to the"), "no direction without a position: {m}");

        // with the tower's position and ours, the direction and distance appear
        b.cells.get_mut("001-01-100-4661").unwrap().location =
            Some(CellLocation { lat: 10.009, lon: 10.0, range_m: None, samples: None, source: "beacondb".into() });
        let here = Location { lat: 10.0, lon: 10.0, accuracy_m: None, source: "phone".into(), ts: 0 };
        let r = recommend(&b, Some(&here), now);
        assert_eq!(r["compass"], json!("north"));
        assert!(r["message"].as_str().unwrap().contains("about 1.0 km to the north"));
        let v = view(&b, LookupCfg { enabled: true, opencellid: false, external: true }, now);
        assert_eq!(v["recommendation"]["status"], json!("better"));

        // a gap below the hysteresis is not worth a move
        let mut b2 = Book::default();
        feed(&mut b2, &phone_msg(now, -95.0, json!([{"ci":4661,"tac":100,"mcc":"001","mnc":"01","rsrp":-91.0,"rat":"LTE"}])));
        assert_eq!(recommend(&b2, None, now)["status"], json!("best"));
        // no neighbours at all
        let mut b3 = Book::default();
        feed(&mut b3, &phone_msg(now, -95.0, json!([])));
        assert_eq!(recommend(&b3, None, now)["status"], json!("best"));
        // NR neighbour is not compared with an LTE serving cell
        let mut b4 = Book::default();
        feed(&mut b4, &phone_msg(now, -110.0, json!([{"pci":4,"rsrp":-70.0,"rat":"NR"}])));
        assert_eq!(recommend(&b4, None, now)["status"], json!("best"));
        // stale list, and a serving cell without RSRP
        assert_eq!(recommend(&b2, None, now + AROUND_FRESH_MS + 1)["status"], json!("stale"));
        let mut b5 = Book::default();
        b5.observe_around(now, vec![Nearby { serving: true, n: Neighbor { pci: Some(1), ..Default::default() } }]);
        assert_eq!(recommend(&b5, None, now)["status"], json!("unknown"));
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
