//! The phone's position, read every 30 s:
//! * phone hub (rhcsd inside Termux): `termux-location -p network -r last`,
//!   falling back to `-p gps -r last`;
//! * laptop hub: `adb shell dumpsys location`, the most recent last-known
//!   fused / gps / network fix (no app needed on the phone).
//!
//! A fresh fix becomes the "current position" on the tower map (a manual home
//! still wins for "home") and, with the signal at that moment, a sample for
//! the automatic places (places.rs). Nothing here asks the phone to start GPS;
//! it reads the position Android already has.

use crate::state::{now_ms, Shared};
use crate::towers::Location;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(30);
/// Fixes older than this are shown but not used for tower distances.
pub const FRESH_MS: i64 = 10 * 60_000;
/// Fixes older than this don't count as "the phone is here now" for places.
pub const PLACE_MAX_AGE_MS: i64 = 5 * 60_000;

/// One position of the phone.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: Option<f64>,
    /// Android provider: "fused", "gps", "network".
    pub provider: String,
    /// "termux" or "adb".
    pub via: String,
    /// When the phone took the fix (ms since epoch, from its age).
    pub fix_ts: i64,
    /// When rhcsd read it.
    pub read_ts: i64,
}

impl Fix {
    pub fn age_ms(&self, now: i64) -> i64 {
        (now - self.fix_ts).max(0)
    }
}

/// A fix as parsed, before it is dated: (lat, lon, accuracy, provider, age in ms).
#[derive(Clone, Debug, PartialEq)]
pub struct RawFix {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: Option<f64>,
    pub provider: String,
    pub age_ms: i64,
}

fn valid(lat: f64, lon: f64) -> bool {
    lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) && !(lat == 0.0 && lon == 0.0)
}

/// `termux-location` JSON. `elapsedMs` is the fix's age. Errors come back as
/// `{"API_ERROR": "..."}` and are returned as Err with that text.
pub fn parse_termux_location(text: &str) -> Result<Option<RawFix>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let v: Value = serde_json::from_str(t).map_err(|_| format!("unexpected answer from termux-location: {}", t.chars().take(80).collect::<String>()))?;
    if let Some(e) = v.get("API_ERROR").and_then(Value::as_str) {
        return Err(e.to_string());
    }
    let (Some(lat), Some(lon)) = (v.get("latitude").and_then(Value::as_f64), v.get("longitude").and_then(Value::as_f64)) else {
        return Ok(None);
    };
    if !valid(lat, lon) {
        return Ok(None);
    }
    Ok(Some(RawFix {
        lat,
        lon,
        accuracy_m: v.get("accuracy").and_then(Value::as_f64).filter(|a| *a > 0.0),
        provider: v.get("provider").and_then(Value::as_str).unwrap_or("unknown").to_string(),
        age_ms: v.get("elapsedMs").and_then(Value::as_f64).map(|a| a.max(0.0) as i64).unwrap_or(0),
    }))
}

/// Android `TimeUtils.formatDuration` text, e.g. `+1d2h3m4s567ms`, in ms.
pub fn parse_android_duration(s: &str) -> Option<i64> {
    let s = s.trim().trim_start_matches('+');
    if s.is_empty() {
        return None;
    }
    let (mut total, mut num, mut unit) = (0i64, String::new(), String::new());
    let mut flush = |num: &mut String, unit: &mut String| -> Option<()> {
        if num.is_empty() {
            return None;
        }
        let n: i64 = num.parse().ok()?;
        total += n * match unit.as_str() {
            "d" => 86_400_000,
            "h" => 3_600_000,
            "m" => 60_000,
            "s" => 1000,
            "ms" => 1,
            _ => return None,
        };
        num.clear();
        unit.clear();
        Some(())
    };
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !unit.is_empty() {
                flush(&mut num, &mut unit)?;
            }
            num.push(c);
        } else if c.is_ascii_alphabetic() {
            unit.push(c);
        } else {
            return None;
        }
    }
    flush(&mut num, &mut unit)?;
    Some(total)
}

/// Text from `cat /proc/uptime; echo @@LOC; dumpsys location`. Every
/// `Location[provider lat,lon hAcc=… et=+…]` in it is a candidate; the most
/// recent one wins (ties: the more accurate). Age = uptime − et.
pub fn parse_dumpsys_location(text: &str) -> Option<RawFix> {
    let (uptime, dump) = text.split_once("@@LOC").unwrap_or(("", text));
    let uptime_ms = uptime.split_whitespace().next().and_then(|u| u.parse::<f64>().ok()).map(|u| (u * 1000.0) as i64);
    let mut best: Option<(i64, RawFix)> = None;
    let mut rest = dump;
    while let Some(i) = rest.find("Location[") {
        rest = &rest[i + "Location[".len()..];
        let body = &rest[..rest.find(']').unwrap_or(rest.len())];
        let mut tok = body.split_whitespace();
        let provider = tok.next().unwrap_or("").to_string();
        let Some((lat, lon)) = tok.next().and_then(|c| c.split_once(',')) else { continue };
        let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) else { continue };
        if !valid(lat, lon) {
            continue;
        }
        let mut acc = None;
        let mut et = None;
        for t in tok {
            if let Some(v) = t.strip_prefix("hAcc=").or_else(|| t.strip_prefix("acc=")) {
                acc = v.parse::<f64>().ok().filter(|a| *a > 0.0);
            } else if let Some(v) = t.strip_prefix("et=") {
                et = parse_android_duration(v);
            }
        }
        let et = et.unwrap_or(0);
        let age_ms = match uptime_ms {
            Some(u) if et > 0 => (u - et).max(0),
            _ => i64::MAX / 4, // unknown age: treated as old
        };
        let cand = RawFix { lat, lon, accuracy_m: acc, provider, age_ms };
        let better = match &best {
            None => true,
            Some((bet, b)) => et > *bet || (et == *bet && acc.unwrap_or(f64::MAX) < b.accuracy_m.unwrap_or(f64::MAX)),
        };
        if better {
            best = Some((et, cand));
        }
    }
    best.map(|(_, f)| f)
}

fn date(raw: RawFix, via: &str, now: i64) -> Fix {
    Fix {
        lat: raw.lat,
        lon: raw.lon,
        accuracy_m: raw.accuracy_m,
        provider: raw.provider,
        via: via.to_string(),
        fix_ts: now - raw.age_ms.min(now),
        read_ts: now,
    }
}

async fn read_termux() -> Result<Option<RawFix>, String> {
    let mut last_err = None;
    for provider in ["network", "gps"] {
        match crate::phone::local("termux-location", &["-p", provider, "-r", "last"]).await {
            Ok(text) => match parse_termux_location(&text) {
                Ok(Some(f)) => return Ok(Some(f)),
                Ok(None) => {}
                Err(e) => last_err = Some(e),
            },
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    match last_err {
        Some(e) => Err(format!("{e} (allow Location for Termux:API and turn on the phone's Location)")),
        None => Ok(None),
    }
}

async fn read_adb(s: &Shared) -> Result<Option<RawFix>, String> {
    let transport = {
        let i = s.inner.read().await;
        if !i.sources.phone.online {
            return Err("the phone is not connected over ADB".into());
        }
        i.phone.as_ref().and_then(|p| p.transport.clone())
    };
    let target = match transport.as_deref() {
        Some("wifi") => s.cfg.phone_adb_addr.clone(),
        _ => s.cfg.phone_serial.clone(),
    };
    let text = crate::phone::adb(&s.cfg, target.as_deref(), &["shell", "cat /proc/uptime; echo @@LOC; dumpsys location"])
        .await
        .map_err(|e| e.to_string())?;
    if text.contains("Permission Denial") {
        return Err("Android refused `dumpsys location` over ADB".into());
    }
    Ok(parse_dumpsys_location(&text))
}

/// Router and phone signal right now, for a place sample.
async fn signal_now(s: &Shared) -> crate::places::Signal {
    let i = s.inner.read().await;
    let router = i.telemetry.as_ref().filter(|_| i.sources.router.online).map(|t| (t.sinr, t.rsrp));
    let phone = i.phone.as_ref().filter(|_| i.sources.phone.online).map(|p| (p.sinr, p.rsrp));
    crate::places::Signal {
        router_sinr: router.and_then(|r| r.0),
        router_rsrp: router.and_then(|r| r.1),
        phone_sinr: phone.and_then(|p| p.0),
        phone_rsrp: phone.and_then(|p| p.1),
    }
}

/// Makes a fix the phone's current position: the source state, the WebSocket feed, the tower
/// map's origin and (if fresh and precise) a sample for the automatic places. Used by the
/// ADB/Termux poller below and by the app's `POST /api/location/push`.
pub async fn publish_fix(s: &Shared, fix: Fix, now: i64) {
    {
        let mut i = s.inner.write().await;
        i.sources.location.online = true;
        i.sources.location.error = None;
        i.sources.location.last_ok = Some(now);
    }
    *s.location.write().await = Some(fix.clone());
    s.send(json!({ "type": "location", "ts": now, "data": fix }));
    let age = fix.age_ms(now);
    if age <= FRESH_MS {
        s.towers
            .set_current(Location { lat: fix.lat, lon: fix.lon, accuracy_m: fix.accuracy_m, source: "phone".into(), ts: fix.fix_ts })
            .await;
    }
    if age <= PLACE_MAX_AGE_MS {
        let sig = signal_now(s).await;
        s.places.observe(fix.lat, fix.lon, fix.accuracy_m, now, sig).await;
    }
}

/// Validates the app's `{lat, lon, accuracy_m, ts, source}`. `ts` (ms since epoch) defaults to
/// now; fixes from the future (beyond clock skew) or older than a day are refused.
pub fn parse_push(v: &Value, now: i64) -> Result<Fix, String> {
    let o = v.as_object().ok_or("the body must be a JSON object")?;
    let lat = o.get("lat").and_then(Value::as_f64).ok_or("lat is required (a number)")?;
    let lon = o.get("lon").and_then(Value::as_f64).ok_or("lon is required (a number)")?;
    if !valid(lat, lon) {
        return Err("lat must be -90..90 and lon -180..180 (and not 0,0)".into());
    }
    let accuracy_m = match o.get("accuracy_m") {
        None | Some(Value::Null) => None,
        Some(a) => {
            let a = a.as_f64().filter(|a| a.is_finite() && *a >= 0.0 && *a <= 100_000.0).ok_or("accuracy_m must be 0..100000")?;
            (a > 0.0).then_some(a)
        }
    };
    let ts = match o.get("ts") {
        None | Some(Value::Null) => now,
        Some(t) => t.as_i64().ok_or("ts must be whole milliseconds since 1970")?,
    };
    if ts > now + 5 * 60_000 || ts < now - 24 * 3_600_000 {
        return Err("ts is in the future or more than a day old".into());
    }
    let provider: String = o
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(16)
        .collect();
    Ok(Fix {
        lat,
        lon,
        accuracy_m,
        provider: if provider.is_empty() { "unknown".into() } else { provider },
        via: "app".into(),
        fix_ts: ts.min(now),
        read_ts: now,
    })
}

pub async fn run(s: Shared) {
    let termux = crate::phone::in_termux();
    let mut last_err: Option<String> = None;
    // Give the phone poller a moment to connect first.
    tokio::time::sleep(Duration::from_secs(8)).await;
    loop {
        let res = if termux { read_termux().await } else { read_adb(&s).await };
        let now = now_ms();
        let res = match res {
            Ok(None) => Err("the phone has no recent position: turn on Location on the phone (and open a map app once)".to_string()),
            other => other,
        };
        match res {
            Ok(Some(raw)) => {
                let fix = date(raw, if termux { "termux" } else { "adb" }, now);
                if last_err.take().is_some() {
                    s.event("location", format!("phone position available ({}, about {:.0} m)", fix.provider, fix.accuracy_m.unwrap_or(0.0)), json!({})).await;
                }
                publish_fix(&s, fix, now).await;
            }
            Ok(None) => unreachable!(),
            Err(e) => {
                if last_err.as_deref() != Some(e.as_str()) {
                    // Only worth an event when the phone itself is there.
                    if termux || s.inner.read().await.sources.phone.online {
                        s.event("location", format!("phone position not available: {e}"), json!({})).await;
                    }
                    last_err = Some(e.clone());
                }
                let mut i = s.inner.write().await;
                i.sources.location.online = false;
                i.sources.location.error = Some(e);
            }
        }
        s.places.save().await;
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Positions below are made up (open ocean); only the text layout is real.

    #[test]
    fn termux_location_json() {
        let ok = r#"{"latitude": 1.234567, "longitude": 2.345678, "altitude": 0.0, "accuracy": 18.5,
                     "vertical_accuracy": 0.0, "bearing": 0.0, "speed": 0.0, "elapsedMs": 4210, "provider": "network"}"#;
        let f = parse_termux_location(ok).unwrap().unwrap();
        assert_eq!((f.lat, f.lon, f.accuracy_m, f.age_ms), (1.234567, 2.345678, Some(18.5), 4210));
        assert_eq!(f.provider, "network");
        assert_eq!(parse_termux_location(""), Ok(None));
        assert_eq!(parse_termux_location("{}"), Ok(None));
        assert!(parse_termux_location(r#"{"API_ERROR":"Location permission not granted"}"#).unwrap_err().contains("permission"));
        assert!(parse_termux_location("garbage").is_err());
    }

    #[test]
    fn android_durations() {
        assert_eq!(parse_android_duration("+1d2h3m4s567ms"), Some(93_784_567));
        assert_eq!(parse_android_duration("+512ms"), Some(512));
        assert_eq!(parse_android_duration("+3m0s"), Some(180_000));
        assert_eq!(parse_android_duration(""), None);
        assert_eq!(parse_android_duration("+3x"), None);
    }

    #[test]
    fn dumpsys_location_newest_fix_wins() {
        // Android 12+ layout ("hAcc"), plus the older "Last Known Locations" one ("acc").
        let text = "\
7300.50 14000.00
@@LOC
Location Manager State:
  Location Providers:
    network provider:
      last location=Location[network 1.000100,2.000100 hAcc=40.0 et=+1h55m0s0ms alt=10.0 {Bundle[mParcelledData.dataSize=96]}]
      last coarse location=Location[network 1.000000,2.000000 hAcc=2000.0 et=+1h55m0s0ms]
    gps provider:
      last location=null
    fused provider:
      last location=Location[fused 1.000200,2.000200 hAcc=12.5 et=+2h0m0s500ms alt=11.0 vel=0.0 bear=0.0]
  Last Known Locations:
    passive: Location[network 1.000300,2.000300 acc=60 et=+1h0m0s0ms]
";
        let f = parse_dumpsys_location(text).unwrap();
        assert_eq!(f.provider, "fused");
        assert_eq!((f.lat, f.lon, f.accuracy_m), (1.0002, 2.0002, Some(12.5)));
        // uptime 7300.5 s, fix at 7200.5 s -> 100 s old.
        assert_eq!(f.age_ms, 100_000);

        assert!(parse_dumpsys_location("0.0 0.0\n@@LOC\n  gps provider:\n    last location=null\n").is_none());
        // Without an uptime the age is unknown and treated as old.
        let f = parse_dumpsys_location("Location[gps 1.5,2.5 acc=5 et=+10s0ms]").unwrap();
        assert!(f.age_ms > PLACE_MAX_AGE_MS);
    }

    #[test]
    fn push_validation() {
        let now = 1_700_000_000_000i64;
        let ok = json!({"lat": 1.5, "lon": 2.5, "accuracy_m": 12.5, "ts": now - 4000, "source": "gps"});
        let f = parse_push(&ok, now).unwrap();
        assert_eq!((f.lat, f.lon, f.accuracy_m, f.fix_ts, f.via.as_str(), f.provider.as_str()), (1.5, 2.5, Some(12.5), now - 4000, "app", "gps"));
        // ts defaults to now; a slightly-future ts is clamped, a far-future one refused
        assert_eq!(parse_push(&json!({"lat": 1.5, "lon": 2.5}), now).unwrap().fix_ts, now);
        assert_eq!(parse_push(&json!({"lat": 1.5, "lon": 2.5, "ts": now + 60_000}), now).unwrap().fix_ts, now);
        for bad in [
            json!({"lat": 91.0, "lon": 2.5}),
            json!({"lat": 1.5, "lon": 181.0}),
            json!({"lat": 0.0, "lon": 0.0}),
            json!({"lat": "1.5", "lon": 2.5}),
            json!({"lon": 2.5}),
            json!({"lat": 1.5, "lon": 2.5, "accuracy_m": -3.0}),
            json!({"lat": 1.5, "lon": 2.5, "accuracy_m": "x"}),
            json!({"lat": 1.5, "lon": 2.5, "ts": now + 3_600_000}),
            json!({"lat": 1.5, "lon": 2.5, "ts": now - 2 * 86_400_000}),
            json!({"lat": 1.5, "lon": 2.5, "ts": "now"}),
            json!([1, 2]),
            json!("x"),
            json!(null),
        ] {
            assert!(parse_push(&bad, now).is_err(), "{bad}");
        }
        // odd provider text is cleaned, not trusted
        let f = parse_push(&json!({"lat": 1.5, "lon": 2.5, "source": "<b>gps</b>\n"}), now).unwrap();
        assert_eq!(f.provider, "bgpsb");
    }

    #[test]
    fn fix_dates_from_age() {
        let raw = RawFix { lat: 1.0, lon: 2.0, accuracy_m: None, provider: "gps".into(), age_ms: 5000 };
        let f = date(raw, "adb", 100_000);
        assert_eq!((f.fix_ts, f.read_ts, f.age_ms(110_000)), (95_000, 100_000, 15_000));
    }
}
