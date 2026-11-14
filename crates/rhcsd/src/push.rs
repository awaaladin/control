//! App mode: the Android app pushes the phone's own readings here instead of rhcsd
//! polling ADB/Termux.
//!
//! * `POST /api/phone/push`    the phone's cell/battery status (+ neighbouring cells)
//! * `POST /api/location/push` the phone's position
//!
//! Both need the token (auth.rs gates all of /api in app mode) and answer 409 outside app
//! mode. A push becomes the phone source exactly like a poll would: it is stored in
//! the state and broadcast as a `phone` WebSocket message, so towers.rs, places.rs and
//! connections.rs work unchanged. Without pushes the source goes offline after 15 s.

use crate::phone::{Neighbor, PhoneStatus};
use crate::state::{now_ms, Shared};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

/// No phone push for this long = the phone source is offline.
pub const PHONE_TIMEOUT_MS: i64 = 15_000;
/// The app re-sends the last position every ~30 s even when standing still.
pub const LOCATION_TIMEOUT_MS: i64 = 120_000;
const MAX_NEIGHBORS: usize = 32;
const MAX_NEIGHBORS_ACCEPTED: usize = 64;

pub fn routes() -> Router<crate::state::Shared> {
    Router::new()
        .route("/api/phone/push", post(phone_push))
        .route("/api/location/push", post(location_push))
}

// ------------------------------------------------------------------ validation

type Obj = Map<String, Value>;

/// A number field in [lo, hi]; missing or null = None; anything else is an error.
fn num(o: &Obj, k: &str, lo: f64, hi: f64) -> Result<Option<f64>, String> {
    match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && (lo..=hi).contains(n))
            .map(Some)
            .ok_or_else(|| format!("{k} must be a number between {lo} and {hi}")),
    }
}

fn whole(o: &Obj, k: &str, lo: u64, hi: u64) -> Result<Option<u64>, String> {
    match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| (lo..=hi).contains(n))
            .map(Some)
            .ok_or_else(|| format!("{k} must be a whole number between {lo} and {hi}")),
    }
}

/// Free text: control characters dropped, at most 64 characters, empty = None.
fn text(o: &Obj, k: &str) -> Result<Option<String>, String> {
    match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let t: String = s.chars().filter(|c| !c.is_control()).take(64).collect();
            let t = t.trim().to_string();
            Ok((!t.is_empty()).then_some(t))
        }
        Some(_) => Err(format!("{k} must be text")),
    }
}

/// MCC (3 digits) / MNC (2 or 3 digits) as text.
fn code(o: &Obj, k: &str, digits: std::ops::RangeInclusive<usize>) -> Result<Option<String>, String> {
    match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let s = s.trim();
            if digits.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) {
                Ok(Some(s.to_string()))
            } else {
                Err(format!("{k} must be {}-{} digits", digits.start(), digits.end()))
            }
        }
        Some(_) => Err(format!("{k} must be text of digits")),
    }
}

/// Signal fields shared by the serving cell and the neighbours.
struct Radio {
    ci: Option<u32>,
    pci: Option<u32>,
    tac: Option<u32>,
    earfcn: Option<u32>,
    band: Option<u16>,
    mcc: Option<String>,
    mnc: Option<String>,
    rsrp: Option<f32>,
    rsrq: Option<f32>,
    sinr: Option<f32>,
    rat: Option<String>,
}

fn radio(o: &Obj, ci_key: &str) -> Result<Radio, String> {
    let rat = text(o, "rat")?.map(|r| r.to_ascii_uppercase());
    if let Some(r) = &rat {
        if !matches!(r.as_str(), "LTE" | "NR" | "WCDMA" | "GSM" | "CDMA" | "TDSCDMA") {
            return Err("rat must be LTE, NR, WCDMA, GSM, CDMA or TDSCDMA".into());
        }
    }
    Ok(Radio {
        // 36 bits is the NR cell identity; it does not fit the table, which only keeps u32 ones
        ci: whole(o, ci_key, 0, 0xF_FFFF_FFFF)?.and_then(|c| u32::try_from(c).ok()),
        pci: whole(o, "pci", 0, 1007)?.map(|n| n as u32),
        tac: whole(o, "tac", 0, 0xFF_FFFF)?.map(|n| n as u32),
        earfcn: whole(o, "earfcn", 0, 4_000_000)?.map(|n| n as u32),
        band: whole(o, "band", 1, 512)?.map(|n| n as u16),
        mcc: code(o, "mcc", 3..=3)?,
        mnc: code(o, "mnc", 2..=3)?,
        rsrp: num(o, "rsrp", -160.0, 0.0)?.map(|n| n as f32),
        rsrq: num(o, "rsrq", -50.0, 20.0)?.map(|n| n as f32),
        sinr: num(o, "sinr", -50.0, 60.0)?.map(|n| n as f32),
        rat,
    })
}

/// Validates `POST /api/phone/push` and turns it into the phone source's status. A single
/// out-of-range value rejects the whole push (the app never sends one; garbage should be loud).
/// Android's "unavailable" sentinels must already be null.
pub fn parse_phone_push(v: &Value) -> Result<PhoneStatus, String> {
    let o = v.as_object().ok_or("the body must be a JSON object")?;
    let r = radio(o, "cell_id")?;
    let mut neighbors = Vec::new();
    match o.get("neighbors") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            if list.len() > MAX_NEIGHBORS_ACCEPTED {
                return Err(format!("at most {MAX_NEIGHBORS_ACCEPTED} neighbors"));
            }
            for (i, item) in list.iter().take(MAX_NEIGHBORS).enumerate() {
                let n = item.as_object().ok_or_else(|| format!("neighbors[{i}] must be an object"))?;
                // the issue statement calls the identity `ci`; `cell_id` is accepted too
                let key = if n.contains_key("ci") { "ci" } else { "cell_id" };
                let nr = radio(n, key).map_err(|e| format!("neighbors[{i}]: {e}"))?;
                neighbors.push(Neighbor {
                    key: None, // never taken from the sender
                    ci: nr.ci,
                    pci: nr.pci,
                    tac: nr.tac,
                    earfcn: nr.earfcn,
                    band: nr.band,
                    mcc: nr.mcc,
                    mnc: nr.mnc,
                    rsrp: nr.rsrp,
                    rsrq: nr.rsrq,
                    sinr: nr.sinr,
                    rat: nr.rat,
                });
            }
        }
        Some(_) => return Err("neighbors must be a list".into()),
    }
    Ok(PhoneStatus {
        model: text(o, "model")?,
        android: text(o, "android")?,
        transport: Some("app".into()),
        battery_level: whole(o, "battery_level", 0, 100)?.map(|n| n as u8),
        battery_temp_c: num(o, "battery_temp_c", -40.0, 150.0)?.map(|n| n as f32),
        charging: o.get("charging").and_then(Value::as_bool),
        usb_powered: o.get("usb_powered").and_then(Value::as_bool),
        rsrp: r.rsrp,
        rsrq: r.rsrq,
        sinr: r.sinr,
        rssi: num(o, "rssi", -160.0, 0.0)?.map(|n| n as f32),
        pci: r.pci,
        cell_id: r.ci,
        tac: r.tac,
        earfcn: r.earfcn,
        band: r.band,
        mcc: r.mcc,
        mnc: r.mnc,
        hotspot_clients: Vec::new(),
        operator: text(o, "operator")?,
        network_type: text(o, "network_type")?,
        sim_slot: whole(o, "sim_slot", 0, 7)?.map(|n| n as u8),
        rat: r.rat,
        neighbors,
    })
}

// ------------------------------------------------------------------ applying

/// Stores a validated phone push as the phone source and broadcasts it like the poller does.
pub async fn apply_phone_push(s: &Shared, mut st: PhoneStatus, ts: i64) {
    // The key is derived by towers.rs from the identity fields; nothing to copy here.
    let was_online = {
        let i = s.inner.read().await;
        i.sources.phone.online
    };
    if !was_online {
        s.event(
            "phone_online",
            format!("phone connected through the app: {}", st.model.clone().unwrap_or_else(|| "android".into())),
            json!({ "transport": "app" }),
        )
        .await;
    }
    st.transport = Some("app".into());
    {
        let mut i = s.inner.write().await;
        i.sources.phone.online = true;
        i.sources.phone.error = None;
        i.sources.phone.last_ok = Some(ts);
        i.phone = Some(st.clone());
    }
    s.send(json!({ "type": "phone", "ts": ts, "data": st }));
    if !was_online {
        s.send_status().await;
    }
}

/// Marks pushed sources offline when the app stopped sending. One call per tick.
pub async fn expire(s: &Shared, now: i64) {
    let mut changed = false;
    let mut phone_gone = false;
    {
        let mut i = s.inner.write().await;
        if i.sources.phone.online && i.sources.phone.last_ok.is_some_and(|t| now - t > PHONE_TIMEOUT_MS) {
            i.sources.phone.online = false;
            i.sources.phone.error = Some("the app has stopped sending the phone's readings".into());
            changed = true;
            phone_gone = true;
        }
        if i.sources.location.online && i.sources.location.last_ok.is_some_and(|t| now - t > LOCATION_TIMEOUT_MS) {
            i.sources.location.online = false;
            i.sources.location.error = Some("the app has stopped sending the phone's position".into());
            changed = true;
        }
    }
    if phone_gone {
        s.event("phone_offline", "no readings from the app for 15 s", json!({})).await;
    }
    if changed {
        s.send_status().await;
    }
}

/// Background task (app mode): applies the timeouts above.
pub async fn watch(s: Shared) {
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        expire(&s, now_ms()).await;
    }
}

// ------------------------------------------------------------------ handlers

fn not_app() -> Response {
    (StatusCode::CONFLICT, Json(json!({ "ok": false, "message": "push is only accepted in app mode (RHCSD_APP=1)" }))).into_response()
}

fn bad(m: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "message": m }))).into_response()
}

async fn phone_push(State(s): State<Shared>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    if !s.cfg.app_mode {
        return not_app();
    }
    match parse_phone_push(&body) {
        Ok(st) => {
            apply_phone_push(&s, st, now_ms()).await;
            Json(json!({ "ok": true })).into_response()
        }
        Err(m) => bad(m),
    }
}

static LAST_PLACES_SAVE: AtomicI64 = AtomicI64::new(0);

async fn location_push(State(s): State<Shared>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    if !s.cfg.app_mode {
        return not_app();
    }
    let now = now_ms();
    match crate::location::parse_push(&body, now) {
        Ok(fix) => {
            crate::location::publish_fix(&s, fix, now).await;
            // places.json is rewritten at most every 30 s, however often fixes arrive
            if now - LAST_PLACES_SAVE.load(Ordering::Relaxed) > 30_000 {
                LAST_PLACES_SAVE.store(now, Ordering::Relaxed);
                s.places.save().await;
            }
            Json(json!({ "ok": true })).into_response()
        }
        Err(m) => bad(m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, Config};
    use std::collections::HashMap;

    /// Made-up codes only (test network 001/01).
    fn sample() -> Value {
        json!({
            "model": "Test phone", "android": "14", "battery_level": 80, "battery_temp_c": 31.5, "charging": false,
            "rsrp": -95, "rsrq": -11, "sinr": 7, "rssi": -70, "pci": 120, "cell_id": 4660, "tac": 100, "earfcn": 1300,
            "band": 3, "mcc": "001", "mnc": "01", "rat": "LTE", "operator": "Test Net", "network_type": "LTE", "sim_slot": 0,
            "neighbors": [
                { "key": "forged", "ci": 4661, "pci": 121, "tac": 100, "earfcn": 1300, "band": 3, "mcc": "001", "mnc": "01", "rsrp": -80, "rsrq": -9, "sinr": 12, "rat": "LTE" },
                { "pci": 300, "earfcn": 6300, "band": 20, "rsrp": -110, "rat": "LTE" }
            ]
        })
    }

    #[test]
    fn accepts_a_full_push() {
        let st = parse_phone_push(&sample()).unwrap();
        assert_eq!((st.rsrp, st.sinr, st.pci, st.cell_id, st.tac, st.band), (Some(-95.0), Some(7.0), Some(120), Some(4660), Some(100), Some(3)));
        assert_eq!((st.operator.as_deref(), st.network_type.as_deref(), st.sim_slot), (Some("Test Net"), Some("LTE"), Some(0)));
        assert_eq!(st.neighbors.len(), 2);
        assert_eq!(st.neighbors[0].key, None, "the sender's key is never trusted");
        assert_eq!(st.neighbors[0].ci, Some(4661));
        assert_eq!(st.neighbors[1].ci, None);
        // everything is optional
        let empty = parse_phone_push(&json!({})).unwrap();
        assert!(empty.rsrp.is_none() && empty.neighbors.is_empty());
        let nulls = parse_phone_push(&json!({"rsrp": null, "cell_id": null, "neighbors": null})).unwrap();
        assert!(nulls.rsrp.is_none());
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        let mut cases: Vec<Value> = vec![json!(null), json!("x"), json!(5), json!([1]), json!({"rsrp": "-90"}), json!({"neighbors": 3})];
        for (field, value) in [
            ("rsrp", json!(5)), ("rsrp", json!(-300)), ("rsrp", json!(2147483647)), ("rsrq", json!(99)), ("sinr", json!(-100)),
            ("pci", json!(2000)), ("pci", json!(-1)), ("pci", json!(1.5)), ("battery_level", json!(101)), ("tac", json!(99_999_999)),
            ("band", json!(0)), ("mcc", json!("12")), ("mcc", json!(1)), ("mnc", json!("abcd")), ("mnc", json!("1234")),
            ("sim_slot", json!(9)), ("rat", json!("6G")), ("model", json!(5)), ("operator", json!({})),
        ] {
            cases.push(json!({ field: value }));
        }
        cases.push(json!({"neighbors": [1]}));
        cases.push(json!({"neighbors": [{"rsrp": 50}]}));
        cases.push(json!({"neighbors": vec![json!({}); 65]}));
        for c in cases {
            assert!(parse_phone_push(&c).is_err(), "{c}");
        }
        // more than 32 (but under the limit) neighbours: keep the first 32
        let many = parse_phone_push(&json!({"neighbors": vec![json!({"pci": 1}); 40]})).unwrap();
        assert_eq!(many.neighbors.len(), MAX_NEIGHBORS);
        // text is cleaned
        let st = parse_phone_push(&json!({"model": "  Phone\u{7}\n X  "})).unwrap();
        assert_eq!(st.model.as_deref(), Some("Phone X"));
    }

    fn app_state(name: &str) -> Shared {
        let dir = std::env::temp_dir().join(format!("rhcsd-test-{name}-{}", std::process::id()));
        let mut m = HashMap::new();
        m.insert("RHCSD_APP".to_string(), "1".to_string());
        m.insert("RHCSD_TOKEN".to_string(), "t".repeat(32));
        m.insert("RHCSD_DATA_DIR".to_string(), dir.display().to_string());
        let cfg = Config::from_map(&m);
        assert!(cfg.app_mode && !cfg.router_configured && cfg.bind == "127.0.0.1:7878");
        AppState::new(cfg)
    }

    #[tokio::test]
    async fn push_becomes_the_phone_source_and_expires() {
        let s = app_state("push");
        let mut rx = s.tx.subscribe();
        let st = parse_phone_push(&sample()).unwrap();
        apply_phone_push(&s, st, 1_000_000).await;
        {
            let i = s.inner.read().await;
            assert!(i.sources.phone.online);
            assert!(!i.sources.router_configured);
            assert_eq!(i.phone.as_ref().unwrap().transport.as_deref(), Some("app"));
        }
        // the broadcast is a `phone` message that towers.rs understands
        let mut saw_phone = false;
        while let Ok(m) = rx.try_recv() {
            let v: Value = serde_json::from_str(&m).unwrap();
            if v["type"] == "phone" {
                saw_phone = true;
                let (src, ts, r) = crate::towers::reading_from_message(&v).unwrap();
                assert_eq!((src, ts, r.cell_id), ("phone", 1_000_000, 4660));
            }
        }
        assert!(saw_phone);
        // still online just before the timeout, offline just after
        expire(&s, 1_000_000 + PHONE_TIMEOUT_MS).await;
        assert!(s.inner.read().await.sources.phone.online);
        expire(&s, 1_000_000 + PHONE_TIMEOUT_MS + 1).await;
        let i = s.inner.read().await;
        assert!(!i.sources.phone.online && i.sources.phone.error.is_some());
        let _ = std::fs::remove_dir_all(&s.cfg.data_dir);
    }
}
