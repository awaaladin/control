//! Places found automatically: every ~30 s the phone's position (location.rs)
//! is stored with the router's and the phone's signal at that moment.
//! Positions within about 15 m of each other (more when the fix is fuzzy) are
//! one place; each place keeps sample counts and average/best SINR and RSRP
//! for router and phone. Ranked by signal, labelled in words relative to home
//! ("about 20 m north-east of home"); coordinates never leave rhcsd here.
//!
//! Saved in `data/places.json` (git-ignored: it holds positions).

use crate::state::{now_ms, Shared};
use crate::towers::{bearing_deg, compass, distance_m};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::sync::RwLock;

/// Base join radius (m): fixes this close are the same place.
pub const JOIN_M: f64 = 15.0;
/// Fuzzy fixes may join from further away, but never beyond this.
pub const JOIN_MAX_M: f64 = 40.0;
/// Fixes less accurate than this never start a new place.
pub const NEW_PLACE_MAX_ACC_M: f64 = 50.0;
/// Accuracy assumed when the phone doesn't say.
const DEFAULT_ACC_M: f64 = 30.0;

/// Signal at one moment (None = that source had no reading).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Signal {
    pub router_sinr: Option<f32>,
    pub router_rsrp: Option<f32>,
    pub phone_sinr: Option<f32>,
    pub phone_rsrp: Option<f32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Sig {
    pub sinr_sum: f64,
    pub sinr_n: u64,
    pub best_sinr: Option<f32>,
    pub rsrp_sum: f64,
    pub rsrp_n: u64,
    pub best_rsrp: Option<f32>,
}

impl Sig {
    fn add(&mut self, sinr: Option<f32>, rsrp: Option<f32>) {
        if let Some(v) = sinr {
            self.sinr_sum += v as f64;
            self.sinr_n += 1;
            self.best_sinr = Some(self.best_sinr.map_or(v, |b| b.max(v)));
        }
        if let Some(v) = rsrp {
            self.rsrp_sum += v as f64;
            self.rsrp_n += 1;
            self.best_rsrp = Some(self.best_rsrp.map_or(v, |b| b.max(v)));
        }
    }
    pub fn avg_sinr(&self) -> Option<f64> {
        (self.sinr_n > 0).then(|| self.sinr_sum / self.sinr_n as f64)
    }
    pub fn avg_rsrp(&self) -> Option<f64> {
        (self.rsrp_n > 0).then(|| self.rsrp_sum / self.rsrp_n as f64)
    }
    fn view(&self) -> Value {
        if self.sinr_n == 0 && self.rsrp_n == 0 {
            return Value::Null;
        }
        json!({
            "samples": self.sinr_n.max(self.rsrp_n),
            "avg_sinr": self.avg_sinr(), "best_sinr": self.best_sinr,
            "avg_rsrp": self.avg_rsrp(), "best_rsrp": self.best_rsrp,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Place {
    pub id: u32,
    pub lat: f64,
    pub lon: f64,
    /// Sum of the fix weights folded into the centre (1/accuracy²).
    pub weight: f64,
    /// Best fix accuracy seen here (m).
    pub best_acc_m: f64,
    pub samples: u64,
    pub router: Sig,
    pub phone: Sig,
    pub first_seen: i64,
    pub last_seen: i64,
    /// Set by the user (`POST /api/places/:id/name`).
    pub name: Option<String>,
}

impl Place {
    /// Ranking value: the router's average SINR, else the phone's.
    pub fn quality(&self) -> Option<f64> {
        self.router.avg_sinr().or_else(|| self.phone.avg_sinr())
    }
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("Place {}", self.id))
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct PlaceBook {
    pub version: u32,
    pub next_id: u32,
    pub places: Vec<Place>,
    /// Place the latest sample joined.
    #[serde(skip)]
    pub current: Option<u32>,
    #[serde(skip)]
    dirty: bool,
}

/// How far a fix may be from a place's centre and still join it.
pub fn join_radius(acc_m: f64) -> f64 {
    acc_m.clamp(JOIN_M, JOIN_MAX_M)
}

/// Distance in words: "about 20 m", "about 1.2 km".
pub fn distance_words(m: f64) -> String {
    if m < 1000.0 {
        format!("about {} m", ((m / 5.0).round() * 5.0).max(5.0) as i64)
    } else {
        format!("about {:.1} km", m / 1000.0)
    }
}

impl PlaceBook {
    /// Adds one sample. Returns the place it joined (or started), or None when
    /// the fix is too fuzzy to start a place and none is close enough.
    pub fn observe(&mut self, lat: f64, lon: f64, acc_m: Option<f64>, ts: i64, sig: Signal) -> Option<u32> {
        let acc = acc_m.filter(|a| a.is_finite() && *a > 0.0).unwrap_or(DEFAULT_ACC_M);
        let nearest = self
            .places
            .iter_mut()
            .map(|p| (distance_m(p.lat, p.lon, lat, lon), p))
            .filter(|(d, _)| *d <= join_radius(acc))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let w = 1.0 / acc.max(5.0).powi(2);
        let place = match nearest {
            Some((_, p)) => {
                // Accuracy-weighted running centre.
                let tw = p.weight + w;
                p.lat = (p.lat * p.weight + lat * w) / tw;
                p.lon = (p.lon * p.weight + lon * w) / tw;
                p.weight = tw;
                p.best_acc_m = p.best_acc_m.min(acc);
                p
            }
            None if acc <= NEW_PLACE_MAX_ACC_M => {
                self.next_id = self.next_id.max(self.places.iter().map(|p| p.id).max().unwrap_or(0)) + 1;
                self.places.push(Place {
                    id: self.next_id,
                    lat,
                    lon,
                    weight: w,
                    best_acc_m: acc,
                    first_seen: ts,
                    ..Default::default()
                });
                self.places.last_mut().unwrap()
            }
            None => {
                self.current = None;
                return None;
            }
        };
        place.samples += 1;
        place.last_seen = ts;
        place.router.add(sig.router_sinr, sig.router_rsrp);
        place.phone.add(sig.phone_sinr, sig.phone_rsrp);
        let id = place.id;
        self.current = Some(id);
        self.dirty = true;
        Some(id)
    }

    pub fn rename(&mut self, id: u32, name: Option<String>) -> bool {
        match self.places.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.name = name;
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    /// `/api/places`: ranked, labelled in words, without coordinates.
    /// `home`: the manual home, if any; otherwise the most-visited place
    /// counts as home.
    pub fn view(&self, home: Option<(f64, f64)>) -> Value {
        let most_visited = self.places.iter().max_by_key(|p| p.samples);
        let (origin, home_kind) = match (home, most_visited) {
            (Some(h), _) => (Some(h), "manual"),
            (None, Some(p)) => (Some((p.lat, p.lon)), "most_visited"),
            _ => (None, "none"),
        };
        let mut ranked: Vec<&Place> = self.places.iter().collect();
        ranked.sort_by(|a, b| match (b.quality(), a.quality()) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => b.samples.cmp(&a.samples),
        });
        let places: Vec<Value> = ranked
            .iter()
            .enumerate()
            .map(|(rank, p)| {
                let (d, dir) = match origin {
                    Some((hl, ho)) => {
                        let d = distance_m(hl, ho, p.lat, p.lon);
                        (Some(d), Some(compass(bearing_deg(hl, ho, p.lat, p.lon))))
                    }
                    None => (None, None),
                };
                let is_home_place = home_kind == "most_visited" && most_visited.is_some_and(|m| m.id == p.id);
                let place_words = match (d, dir) {
                    _ if is_home_place => "where the phone is most often (counted as home)".to_string(),
                    (Some(d), _) if d < 8.0 => "at home".to_string(),
                    (Some(d), Some(dir)) => format!("{} {dir} of home", distance_words(d)),
                    _ => "no home to measure from yet".to_string(),
                };
                json!({
                    "id": p.id,
                    "rank": rank + 1,
                    "label": p.label(),
                    "name": p.name,
                    "where": place_words,
                    "distance_m": d,
                    "compass": dir,
                    "samples": p.samples,
                    "quality_sinr": p.quality(),
                    "router": p.router.view(),
                    "phone": p.phone.view(),
                    "accuracy_m": p.best_acc_m,
                    "first_seen": p.first_seen,
                    "last_seen": p.last_seen,
                    "is_current": self.current == Some(p.id),
                })
            })
            .collect();
        json!({
            "home": home_kind,
            "current_place": self.current,
            "places": places,
        })
    }
}

// ------------------------------------------------------------------ state

pub struct Places {
    pub book: RwLock<PlaceBook>,
    path: PathBuf,
}

impl Places {
    pub fn open(dir: &std::path::Path) -> Self {
        let path = dir.join("places.json");
        let mut book: PlaceBook = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        book.version = 1;
        Self { book: RwLock::new(book), path }
    }

    pub async fn observe(&self, lat: f64, lon: f64, acc_m: Option<f64>, ts: i64, sig: Signal) -> Option<u32> {
        self.book.write().await.observe(lat, lon, acc_m, ts, sig)
    }

    pub async fn save(&self) {
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
            eprintln!("places: could not save {}", self.path.display());
            self.book.write().await.dirty = true;
        }
    }
}

// ------------------------------------------------------------------ API

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/places", get(get_places))
        .route("/api/places/{id}/name", post(rename))
}

async fn get_places(State(s): State<Shared>) -> Json<Value> {
    let home = s.towers.book.read().await.home.as_ref().map(|h| (h.lat, h.lon));
    let fix = s.location.read().await.clone();
    let now = now_ms();
    let mut v = s.places.book.read().await.view(home);
    v["position"] = match fix {
        Some(f) => json!({
            "provider": f.provider, "via": f.via, "accuracy_m": f.accuracy_m,
            "age_s": f.age_ms(now) / 1000, "fresh": f.age_ms(now) <= crate::location::PLACE_MAX_AGE_MS,
        }),
        None => Value::Null,
    };
    v["position_error"] = json!(s.inner.read().await.sources.location.error);
    Json(v)
}

#[derive(Deserialize)]
struct RenameRequest {
    name: Option<String>,
}

async fn rename(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    Json(req): Json<RenameRequest>,
) -> Response {
    if !crate::api::authorized(&s, addr, &headers) {
        return crate::api::denied();
    }
    let name = req.name.map(|n| n.trim().chars().take(40).collect::<String>()).filter(|n| !n.is_empty());
    if !s.places.book.write().await.rename(id, name.clone()) {
        return (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "message": format!("no place {id}") }))).into_response();
    }
    s.places.save().await;
    let message = match name {
        Some(n) => format!("place renamed to \"{n}\""),
        None => "place name cleared".to_string(),
    };
    Json(json!({ "ok": true, "message": message })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made-up positions in open ocean; ~0.00009° of latitude ≈ 10 m.
    const LAT: f64 = 1.0;
    const LON: f64 = 2.0;
    const M: f64 = 1.0 / 111_195.0; // degrees per metre (latitude)

    fn sig(r: f32, p: f32) -> Signal {
        Signal { router_sinr: Some(r), router_rsrp: Some(-90.0), phone_sinr: Some(p), phone_rsrp: Some(-95.0) }
    }

    #[test]
    fn clusters_nearby_fixes_and_splits_far_ones() {
        let mut b = PlaceBook::default();
        let a = b.observe(LAT, LON, Some(10.0), 1, sig(10.0, 5.0)).unwrap();
        // 8 m away, accurate: same place.
        assert_eq!(b.observe(LAT + 8.0 * M, LON, Some(8.0), 2, sig(14.0, 7.0)), Some(a));
        // 30 m away with a 10 m fix: new place.
        let c = b.observe(LAT + 30.0 * M, LON, Some(10.0), 3, sig(20.0, 9.0)).unwrap();
        assert_ne!(a, c);
        // 30 m away from the first with a fuzzy 35 m fix joins the nearest place (the second).
        assert_eq!(b.observe(LAT + 27.0 * M, LON, Some(35.0), 4, sig(18.0, 8.0)), Some(c));
        // A 200 m-accuracy fix far from everything starts nothing.
        assert_eq!(b.observe(LAT + 500.0 * M, LON, Some(200.0), 5, Signal::default()), None);
        assert_eq!(b.current, None);
        assert_eq!(b.places.len(), 2);

        let pa = b.places.iter().find(|p| p.id == a).unwrap();
        assert_eq!(pa.samples, 2);
        assert_eq!(pa.router.avg_sinr(), Some(12.0));
        assert_eq!(pa.router.best_sinr, Some(14.0));
        assert_eq!(pa.phone.avg_sinr(), Some(6.0));
        assert_eq!((pa.first_seen, pa.last_seen), (1, 2));
        // The centre moved toward the more accurate second fix.
        assert!(pa.lat > LAT && pa.lat < LAT + 8.0 * M);
    }

    #[test]
    fn ranks_by_signal_and_describes_in_words() {
        let mut b = PlaceBook::default();
        for i in 0..5 {
            b.observe(LAT, LON, Some(10.0), i, sig(8.0, 3.0)); // home-ish, most visited
        }
        let east = b.observe(LAT, LON + 25.0 * M, Some(10.0), 10, sig(19.0, 9.0)).unwrap();
        let v = b.view(None);
        assert_eq!(v["home"], json!("most_visited"));
        assert_eq!(v["places"][0]["id"], json!(east));
        assert_eq!(v["places"][0]["label"], json!(format!("Place {east}")));
        assert_eq!(v["places"][0]["where"], json!("about 25 m east of home"));
        assert_eq!(v["places"][0]["is_current"], json!(true));
        assert_eq!(v["places"][1]["where"], json!("where the phone is most often (counted as home)"));
        assert!(v["places"][0].get("lat").is_none(), "no coordinates in the API");
        assert_eq!(v["current_place"], json!(east));

        // With a manual home to the west, words are relative to it.
        let v = b.view(Some((LAT, LON - 20.0 * M)));
        assert_eq!(v["home"], json!("manual"));
        assert_eq!(v["places"][1]["where"], json!("about 20 m east of home"));

        assert!(b.rename(east, Some("Window".into())));
        assert_eq!(b.view(None)["places"][0]["label"], json!("Window"));
        assert!(!b.rename(999, None));
        assert_eq!(distance_words(1234.0), "about 1.2 km");
        assert_eq!(distance_words(2.0), "about 5 m");
    }

    #[test]
    fn join_radius_is_accuracy_aware() {
        assert_eq!(join_radius(3.0), JOIN_M);
        assert_eq!(join_radius(25.0), 25.0);
        assert_eq!(join_radius(500.0), JOIN_MAX_M);
    }
}
