//! Every internet connection by name, with the same data for each: radio
//! (from the router or the phone behind it), live throughput where known,
//! and the latest Game Boost ping / jitter / loss. `GET /api/connections`.
//!
//! Names come from links.rs ("Jeffnou (router)", "vivo Y (phone hotspot)",
//! "Phone (USB)"). On the phone hub (Termux) the phone's own radio is listed
//! as "This phone (mobile data)".
//!
//! Every connection also carries `metrics`: readings with their own range, thresholds and
//! direction, so the page can draw a dial for ANY kind of connection (4G radio, Wi-Fi signal,
//! link speed). Bluetooth has none: Windows gives no signal strength for it, and none is invented.

use crate::bluetooth::BtDevice;
use crate::boost::LinkResult;
use crate::links::{Link, LinkKind, WlanIf};
use crate::phone::PhoneStatus;
use crate::state::Shared;
use crate::towers::cell_key;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};
use zte::Telemetry;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Radio {
    /// "router" or "phone": which device's modem this is.
    pub source: String,
    pub rsrp: Option<f32>,
    pub rsrq: Option<f32>,
    pub sinr: Option<f32>,
    pub rssi: Option<f32>,
    pub band: Option<u16>,
    pub earfcn: Option<u32>,
    pub pci: Option<u32>,
    pub cell_id: Option<u32>,
    pub enb: Option<u32>,
    pub sector: Option<u32>,
    /// Key of the serving cell on the tower map (`/api/towers`).
    pub tower: Option<String>,
    pub network_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Throughput {
    pub rx_bps: Option<u64>,
    pub tx_bps: Option<u64>,
    /// Where the numbers come from, e.g. "router WAN".
    pub source: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Ping {
    pub ping_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: Option<f64>,
    pub score: Option<f64>,
    pub grade: String,
    /// When the Boost measurement finished (or started, while measuring).
    pub at: Option<i64>,
}

/// One reading of one connection, self-describing so the web page needs no per-kind code.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Metric {
    /// rsrp | sinr | rsrq | wifi_signal | wifi_quality | link_speed
    pub key: String,
    pub label: String,
    pub unit: String,
    /// None = no reading right now (shown grey, never made up).
    pub value: Option<f64>,
    /// Dial range.
    pub min: f64,
    pub max: f64,
    /// Grade edges [excellent, good, fair]; below fair is poor. With `higher_is_better`
    /// false the values descend the other way (value <= excellent is excellent).
    pub thresholds: [f64; 3],
    pub higher_is_better: bool,
    pub note: Option<String>,
}

fn metric(key: &str, label: &str, unit: &str, value: Option<f64>, range: (f64, f64), thresholds: [f64; 3]) -> Metric {
    Metric {
        key: key.into(),
        label: label.into(),
        unit: unit.into(),
        value: value.filter(|v| v.is_finite()),
        min: range.0,
        max: range.1,
        thresholds,
        higher_is_better: true,
        note: None,
    }
}

/// Windows' Wi-Fi "Signal" quality (0-100 %) as dBm. Windows defines quality as linear between
/// -100 dBm (0 %) and -50 dBm (100 %), so dBm = quality / 2 - 100.
pub fn dbm_from_quality(pct: f64) -> f64 {
    pct.clamp(0.0, 100.0) / 2.0 - 100.0
}

/// Wi-Fi readings of this PC's wireless adapter.
/// - `wifi_signal` dBm, dial -100..-30, grades -50 / -60 / -70 (excellent / good / fair).
/// - `wifi_quality` %, dial 0..100, grades 100 / 80 / 60: exactly the same edges as the dBm
///   dial (quality 100 % = -50 dBm, 80 % = -60 dBm, 60 % = -70 dBm), so the two never disagree.
pub fn wifi_metrics(w: Option<&WlanIf>) -> Vec<Metric> {
    let q = w.and_then(|w| w.signal_pct).map(f64::from);
    vec![
        metric("wifi_signal", "Wi-Fi signal", "dBm", q.map(dbm_from_quality), (-100.0, -30.0), [-50.0, -60.0, -70.0]),
        metric("wifi_quality", "Wi-Fi quality", "%", q, (0.0, 100.0), [100.0, 80.0, 60.0]),
    ]
}

/// The negotiated speed of the adapter.
/// - Wi-Fi: dial 0..600 Mbit/s, grades 200 / 100 / 40. A Wi-Fi rate is the best the radio can
///   do right now (it falls when the signal weakens), so it is graded lower than a cable.
/// - Cable, USB tethering and others: dial 0..1000 Mbit/s, grades 1000 / 100 / 10.
pub fn speed_metric(mbps: Option<f64>, wifi: bool) -> Metric {
    let (range, thr) = if wifi { ((0.0, 600.0), [200.0, 100.0, 40.0]) } else { ((0.0, 1000.0), [1000.0, 100.0, 10.0]) };
    metric("link_speed", "Link speed", "Mbit/s", mbps.filter(|v| *v > 0.0), range, thr)
}

/// The 4G radio readings (RSRP / SINR / RSRQ); `owner` prefixes the label ("Phone ") when the
/// radio belongs to another device than the one drawn (a phone behind a hotspot or USB tether).
pub fn cell_metrics(r: Option<&Radio>, owner: &str, note: Option<&str>) -> Vec<Metric> {
    let mut v = vec![
        metric("rsrp", &format!("{owner}RSRP"), "dBm", r.and_then(|r| r.rsrp).map(f64::from), (-120.0, -70.0), [-80.0, -90.0, -100.0]),
        metric("sinr", &format!("{owner}SINR"), "dB", r.and_then(|r| r.sinr).map(f64::from), (-5.0, 30.0), [20.0, 13.0, 0.0]),
        metric("rsrq", &format!("{owner}RSRQ"), "dB", r.and_then(|r| r.rsrq).map(f64::from), (-20.0, -3.0), [-10.0, -15.0, -20.0]),
    ];
    for m in v.iter_mut() {
        m.note = note.map(str::to_string);
    }
    v
}

/// Plain one-line description of the PC-side link: SSID, standard, channel, band, rates.
pub fn wifi_detail(w: &WlanIf) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = &w.ssid {
        parts.push(s.clone());
    }
    if let Some(r) = &w.radio_type {
        parts.push(r.clone());
    }
    match (w.channel, &w.band) {
        (Some(c), Some(b)) => parts.push(format!("channel {c} ({b})")),
        (Some(c), None) => parts.push(format!("channel {c}")),
        (None, Some(b)) => parts.push(b.clone()),
        _ => {}
    }
    let rate = |v: f64| if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.1}") };
    match (w.rx_mbps, w.tx_mbps) {
        (Some(r), Some(t)) => parts.push(format!("receive {} / transmit {} Mbit/s", rate(r), rate(t))),
        (Some(r), None) => parts.push(format!("receive {} Mbit/s", rate(r))),
        (None, Some(t)) => parts.push(format!("transmit {} Mbit/s", rate(t))),
        _ => {}
    }
    parts.join(" · ")
}

fn wired_detail(l: &Link, mbps: Option<f64>) -> String {
    let what = if l.kind == LinkKind::PhoneUsb { "USB tethering".to_string() } else { l.medium.clone() };
    let mut parts = vec![what];
    if !l.description.is_empty() {
        parts.push(l.description.clone());
    }
    if let Some(m) = mbps {
        parts.push(format!("{} Mbit/s", if m.fract() == 0.0 { format!("{m:.0}") } else { format!("{m:.1}") }));
    }
    parts.join(" · ")
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Connection {
    /// Stable id: the link's local address, or "phone-self".
    pub id: String,
    pub name: String,
    /// router | phone | other
    pub kind: String,
    pub link_kind: Option<LinkKind>,
    /// Windows interface alias (what switching uses); None for the phone itself.
    pub alias: Option<String>,
    pub ssid: Option<String>,
    pub medium: Option<String>,
    /// Carries the default route (new connections go here). None = unknown.
    pub in_use: Option<bool>,
    pub radio: Option<Radio>,
    /// Why there is no radio data, in plain words.
    pub radio_note: Option<String>,
    pub throughput: Option<Throughput>,
    pub ping: Option<Ping>,
    /// Signal readings for any kind of connection (see [`Metric`]); empty when there are none.
    pub metrics: Vec<Metric>,
    /// One plain line: SSID, standard, channel, band, rates / adapter and speed / device state.
    pub detail: Option<String>,
    /// Why `metrics` is empty or grey, in plain words.
    pub note: Option<String>,
}

pub fn router_radio(t: &Telemetry) -> Radio {
    let tac = t.tac.as_deref().and_then(|s| u32::from_str_radix(s.trim(), 16).ok()).filter(|t| *t > 0 && *t != 0xFFFF);
    Radio {
        source: "router".into(),
        rsrp: t.rsrp,
        rsrq: t.rsrq,
        sinr: t.sinr,
        rssi: t.rssi,
        band: t.band,
        earfcn: t.earfcn,
        pci: t.pci,
        cell_id: t.cell_id,
        enb: t.cell_id.map(|c| c >> 8),
        sector: t.cell_id.map(|c| c & 0xff),
        tower: t.cell_id.filter(|c| *c > 0).map(|c| cell_key(t.mcc.as_deref(), t.mnc.as_deref(), tac, c)),
        network_type: Some(t.network_type.clone()).filter(|s| !s.is_empty()),
    }
}

pub fn phone_radio(p: &PhoneStatus) -> Radio {
    Radio {
        source: "phone".into(),
        rsrp: p.rsrp,
        rsrq: p.rsrq,
        sinr: p.sinr,
        rssi: p.rssi,
        band: p.band,
        earfcn: p.earfcn,
        pci: p.pci,
        cell_id: p.cell_id,
        enb: p.cell_id.map(|c| c >> 8),
        sector: p.cell_id.map(|c| c & 0xff),
        tower: p.cell_id.filter(|c| *c > 0).map(|c| cell_key(p.mcc.as_deref(), p.mnc.as_deref(), p.tac.filter(|t| *t > 0), c)),
        network_type: Some("LTE".into()).filter(|_| p.rsrp.is_some()),
    }
}

/// What the daemon knows right now, as plain inputs (so [`build`] is pure).
#[derive(Default)]
pub struct Inputs<'a> {
    pub links: &'a [Link],
    /// Router telemetry, only when the router is online.
    pub router: Option<&'a Telemetry>,
    /// Phone status, only when the phone is online.
    pub phone: Option<&'a PhoneStatus>,
    pub phone_model: Option<String>,
    /// Latest Game Boost results and when they were taken.
    pub boost: &'a [LinkResult],
    pub boost_at: Option<i64>,
    /// rhcsd runs on the phone itself (Termux).
    pub on_phone: bool,
    /// Wireless interfaces (netsh), for the Wi-Fi readings.
    pub wlan: &'a [WlanIf],
    /// Negotiated adapter speed in Mbit/s by interface index.
    pub speeds: &'a [(u32, f64)],
    /// Connected Bluetooth devices (names only; there is no signal reading for them).
    pub bluetooth: &'a [BtDevice],
}

/// "Pocket Phone (Bluetooth)" -> stable id "bluetooth:pocket-phone".
fn bt_id(name: &str) -> String {
    let slug: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    format!("bluetooth:{}", slug.trim_matches('-'))
}

/// Builds the connection list. Pure, so it is unit-tested.
pub fn build(i: &Inputs) -> Vec<Connection> {
    let mut out: Vec<Connection> = i
        .links
        .iter()
        .map(|l| {
            let (kind, radio, note) = match l.kind {
                LinkKind::Router => (
                    "router",
                    i.router.map(router_radio),
                    i.router.is_none().then(|| "the router isn't answering rhcs right now".to_string()),
                ),
                LinkKind::PhoneUsb | LinkKind::PhoneHotspot => (
                    "phone",
                    i.phone.map(phone_radio),
                    i.phone
                        .is_none()
                        .then(|| "the phone isn't connected over ADB (USB debugging or ADB over Wi-Fi), so its radio can't be read".to_string()),
                ),
                // rhcsd runs on this phone: the connection IS this phone, so its own radio readings belong to it.
                LinkKind::Other if i.on_phone => (
                    "phone",
                    i.phone.map(phone_radio),
                    i.phone.is_none().then(|| "this phone's signal has not been read yet".to_string()),
                ),
                LinkKind::Other => ("other", None, Some("rhcs doesn't know which device is behind this connection".to_string())),
            };
            let throughput = match (l.kind, i.router) {
                (LinkKind::Router, Some(t)) => {
                    Some(Throughput { rx_bps: t.rx_bytes_per_s, tx_bps: t.tx_bytes_per_s, source: "router WAN".into() })
                }
                _ => None,
            };
            let is_wifi = l.medium == "Wi-Fi";
            let w = i.wlan.iter().find(|w| w.name == l.adapter && w.signal_pct.is_some());
            let adapter_mbps = i.speeds.iter().find(|(idx, _)| *idx == l.if_index).map(|(_, m)| *m);
            // Wi-Fi: the adapter's negotiated rate, else what netsh reports (slower direction).
            let mbps = adapter_mbps.or_else(|| w.and_then(|w| match (w.rx_mbps, w.tx_mbps) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }));
            let mut pc = if is_wifi { wifi_metrics(w) } else { Vec::new() };
            pc.push(speed_metric(mbps, is_wifi));
            let detail = if is_wifi { w.map(wifi_detail).filter(|d| !d.is_empty()) } else { Some(wired_detail(l, mbps)) };
            let phone_note = "The phone's mobile signal, not this PC's link";
            let metrics = match l.kind {
                LinkKind::Router => {
                    let mut m = cell_metrics(radio.as_ref(), "", None);
                    m.extend(pc);
                    m
                }
                LinkKind::PhoneUsb | LinkKind::PhoneHotspot => {
                    let mut m = pc;
                    m.extend(cell_metrics(radio.as_ref(), "Phone ", Some(phone_note)));
                    m
                }
                LinkKind::Other if i.on_phone => cell_metrics(radio.as_ref(), "", None),
                LinkKind::Other => pc,
            };
            let ping = i.boost.iter().find(|r| r.link.id == l.id).filter(|r| r.stats.sent > 0).map(|r| Ping {
                ping_ms: r.stats.ping_ms,
                jitter_ms: r.stats.jitter_ms,
                loss_pct: r.stats.loss_pct,
                score: r.score,
                grade: r.grade.clone(),
                at: i.boost_at,
            });
            let note = if radio.is_none() { note.clone() } else { None };
            Connection {
                id: l.id.clone(),
                name: l.name.clone(),
                kind: kind.into(),
                link_kind: Some(l.kind),
                alias: Some(l.adapter.clone()),
                ssid: l.ssid.clone(),
                medium: Some(l.medium.clone()),
                in_use: Some(l.is_default),
                radio,
                radio_note: note.clone(),
                throughput,
                ping,
                metrics,
                detail,
                note,
            }
        })
        .collect();
    if i.on_phone && !out.iter().any(|c| c.kind == "phone") {
        if let Some(p) = i.phone {
            out.push(Connection {
                id: "phone-self".into(),
                name: match &i.phone_model {
                    Some(m) => format!("This phone (mobile data) · {m}"),
                    None => "This phone (mobile data)".into(),
                },
                kind: "phone".into(),
                link_kind: None,
                alias: None,
                ssid: None,
                medium: Some("mobile".into()),
                in_use: None,
                radio: Some(phone_radio(p)),
                radio_note: None,
                throughput: None,
                ping: None,
                metrics: cell_metrics(Some(&phone_radio(p)), "", None),
                detail: Some("This phone's own mobile data".into()),
                note: None,
            });
        }
    }
    for d in i.bluetooth.iter().filter(|d| d.connected) {
        out.push(Connection {
            id: bt_id(&d.name),
            name: format!("{} (Bluetooth)", d.name),
            kind: "bluetooth".into(),
            link_kind: None,
            alias: None,
            ssid: None,
            medium: Some("Bluetooth".into()),
            in_use: None,
            radio: None,
            radio_note: None,
            throughput: None,
            ping: None,
            metrics: Vec::new(),
            detail: Some("Connected".into()),
            note: Some(crate::bluetooth::NO_RSSI_NOTE.into()),
        });
    }
    out
}

/// The connection with the best ping score, else the best SINR.
pub fn best(list: &[Connection]) -> Option<&Connection> {
    let by_score = list
        .iter()
        .filter_map(|c| c.ping.as_ref().and_then(|p| p.score).map(|s| (s, c)))
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, c)) = by_score {
        return Some(c);
    }
    list.iter()
        .filter_map(|c| c.radio.as_ref().and_then(|r| r.sinr).map(|s| (s, c)))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c)
}

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/connections", get(get_connections))
}

async fn get_connections(State(s): State<Shared>) -> Json<Value> {
    let links = crate::links::discover(&s).await;
    let boost = s.boost.state.read().await.clone();
    let i = s.inner.read().await;
    let router = i.telemetry.as_ref().filter(|_| i.sources.router.online);
    let phone = i.phone.as_ref().filter(|_| i.sources.phone.online);
    let wlan = crate::links::wlan_interfaces().await;
    let speeds = tokio::task::spawn_blocking(crate::links::adapter_speeds).await.unwrap_or_default();
    let bluetooth = crate::bluetooth::connected();
    let list = build(&Inputs {
        links: &links,
        router,
        phone,
        phone_model: phone.and_then(|p| p.model.clone()),
        boost: &boost.links,
        boost_at: boost.finished.or(boost.started),
        on_phone: crate::phone::in_termux() || s.cfg.app_mode,
        wlan: &wlan,
        speeds: &speeds,
        bluetooth: &bluetooth,
    });
    let best_id = best(&list).map(|c| c.id.clone());
    Json(json!({
        "connections": list,
        "best": best_id,
        "admin": s.admin,
        "switch_supported": crate::switch::supported(),
        "bluetooth_supported": crate::bluetooth::supported(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boost::LinkStats;

    fn link(name: &str, kind: LinkKind, ip: &str, default: bool) -> Link {
        Link {
            id: ip.into(),
            name: name.into(),
            kind,
            medium: "Wi-Fi".into(),
            adapter: "WiFi".into(),
            description: String::new(),
            local_ip: ip.parse().unwrap(),
            gateway: None,
            metric: None,
            is_default: default,
            if_index: 4,
            ssid: Some(name.split(" (").next().unwrap().into()),
        }
    }

    #[test]
    fn the_phones_own_link_carries_its_radio_readings() {
        let a = crate::links::Adapter {
            alias: "This phone's connection".into(),
            if_type: 6,
            up: true,
            addrs: vec!["10.1.2.3".parse().unwrap()],
            route_unknown: true,
            ..Default::default()
        };
        let links = crate::links::classify(&[a], &crate::links::Hints::default());
        let p = PhoneStatus { rsrp: Some(-84.0), rsrq: Some(-12.0), ..Default::default() };
        let list = build(&Inputs { links: &links, phone: Some(&p), phone_model: Some("Y".into()), on_phone: true, ..Default::default() });
        assert_eq!(list.len(), 1, "no duplicate phone-self entry");
        assert_eq!(list[0].kind, "phone");
        assert!(list[0].radio.is_some());
    }

    #[test]
    fn connections_carry_radio_ping_and_names() {
        let links = vec![
            link("HomeNet (router)", LinkKind::Router, "192.168.0.20", false),
            link("Pocket Phone (phone hotspot)", LinkKind::PhoneHotspot, "10.0.55.80", true),
            link("Cafe (Wi-Fi)", LinkKind::Other, "172.16.4.9", false),
        ];
        let t = Telemetry {
            rsrp: Some(-95.0),
            sinr: Some(8.0),
            cell_id: Some(0x1234),
            tac: Some("1F".into()),
            mcc: Some("001".into()),
            mnc: Some("01".into()),
            rx_bytes_per_s: Some(1000),
            ..Default::default()
        };
        let p = PhoneStatus { rsrp: Some(-85.0), sinr: Some(21.0), cell_id: Some(0x1235), tac: Some(31), ..Default::default() };
        let boost = vec![LinkResult {
            link: links[1].clone(),
            stats: LinkStats { sent: 10, received: 10, loss_pct: Some(0.0), ping_ms: Some(41.0), jitter_ms: Some(3.0), ..Default::default() },
            targets: vec![],
            radio: None,
            score: Some(47.0),
            grade: "excellent".into(),
            verdict: String::new(),
            recent: vec![],
            error: None,
        }];
        let list = build(&Inputs { links: &links, router: Some(&t), phone: Some(&p), boost: &boost, boost_at: Some(9), ..Default::default() });
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "HomeNet (router)");
        assert_eq!(list[0].kind, "router");
        let r = list[0].radio.as_ref().unwrap();
        assert_eq!((r.source.as_str(), r.sinr, r.enb, r.sector), ("router", Some(8.0), Some(0x12), Some(0x34)));
        assert_eq!(r.tower.as_deref(), Some("001-01-31-4660"));
        assert_eq!(list[0].throughput.as_ref().unwrap().rx_bps, Some(1000));
        assert!(list[0].ping.is_none());

        assert_eq!(list[1].kind, "phone");
        assert_eq!(list[1].in_use, Some(true));
        assert_eq!(list[1].radio.as_ref().unwrap().sinr, Some(21.0));
        assert_eq!(list[1].radio.as_ref().unwrap().tower.as_deref(), Some("x-x-31-4661"));
        assert_eq!(list[1].ping.as_ref().unwrap().ping_ms, Some(41.0));
        assert_eq!(list[1].ssid.as_deref(), Some("Pocket Phone"));

        assert_eq!(list[2].kind, "other");
        assert!(list[2].radio.is_none() && list[2].radio_note.is_some());
        // Best = lowest ping score.
        assert_eq!(best(&list).unwrap().id, "10.0.55.80");

        // Router and phone gone: plain-words notes, no made-up values.
        let list = build(&Inputs { links: &links, ..Default::default() });
        assert!(list[0].radio.is_none());
        assert!(list[0].radio_note.as_deref().unwrap().contains("router"));
        assert!(list[1].radio_note.as_deref().unwrap().contains("ADB"));
        assert!(best(&list).is_none());

        // Phone hub: the phone's own radio is a connection too.
        let list = build(&Inputs { links: &[], phone: Some(&p), phone_model: Some("Y03".into()), on_phone: true, ..Default::default() });
        assert_eq!(list[0].name, "This phone (mobile data) · Y03");
        assert_eq!(best(&list).unwrap().id, "phone-self");
    }

    fn get<'a>(c: &'a Connection, key: &str) -> &'a Metric {
        c.metrics.iter().find(|m| m.key == key).unwrap_or_else(|| panic!("no metric {key}"))
    }

    #[test]
    fn dbm_conversion_matches_windows_quality() {
        assert_eq!(dbm_from_quality(100.0), -50.0);
        assert_eq!(dbm_from_quality(0.0), -100.0);
        assert_eq!(dbm_from_quality(80.0), -60.0);
        assert_eq!(dbm_from_quality(60.0), -70.0);
        assert_eq!(dbm_from_quality(250.0), -50.0, "clamped");
    }

    #[test]
    fn wifi_link_gets_signal_quality_and_speed() {
        let mut l = link("Cafe (Wi-Fi)", LinkKind::Other, "172.16.4.9", true);
        l.if_index = 9;
        let w = WlanIf {
            name: "WiFi".into(),
            ssid: Some("Cafe".into()),
            signal_pct: Some(90),
            radio_type: Some("802.11ac".into()),
            channel: Some(36),
            band: Some("5 GHz".into()),
            rx_mbps: Some(433.3),
            tx_mbps: Some(300.0),
            ..Default::default()
        };
        let list = build(&Inputs { links: &[l], wlan: &[w], speeds: &[(9, 300.0)], ..Default::default() });
        let c = &list[0];
        let keys: Vec<&str> = c.metrics.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["wifi_signal", "wifi_quality", "link_speed"]);
        assert_eq!(get(c, "wifi_signal").value, Some(-55.0));
        assert_eq!(get(c, "wifi_signal").thresholds, [-50.0, -60.0, -70.0]);
        assert_eq!((get(c, "wifi_signal").min, get(c, "wifi_signal").max), (-100.0, -30.0));
        assert_eq!(get(c, "wifi_quality").value, Some(90.0));
        assert_eq!(get(c, "link_speed").value, Some(300.0));
        assert_eq!(get(c, "link_speed").thresholds, [200.0, 100.0, 40.0]);
        assert!(c.metrics.iter().all(|m| m.higher_is_better));
        assert_eq!(c.detail.as_deref(), Some("Cafe · 802.11ac · channel 36 (5 GHz) · receive 433.3 / transmit 300 Mbit/s"));

        // netsh gave nothing (other language, service off): grey dials, no invented number.
        let list = build(&Inputs { links: &[link("Cafe (Wi-Fi)", LinkKind::Other, "172.16.4.9", true)], ..Default::default() });
        assert_eq!(get(&list[0], "wifi_signal").value, None);
        assert_eq!(get(&list[0], "link_speed").value, None);
    }

    #[test]
    fn wired_and_usb_links_get_link_speed() {
        let mut eth = link("Ethernet", LinkKind::Other, "192.168.30.5", true);
        eth.medium = "Ethernet".into();
        eth.description = "Example GbE".into();
        eth.if_index = 3;
        let mut usb = link("Phone (USB)", LinkKind::PhoneUsb, "10.0.40.5", false);
        usb.medium = "USB".into();
        usb.description = "Remote NDIS based Internet Sharing Device".into();
        usb.if_index = 5;
        let p = PhoneStatus { rsrp: Some(-92.0), sinr: Some(11.0), rsrq: Some(-12.0), ..Default::default() };
        let list = build(&Inputs { links: &[eth, usb], phone: Some(&p), speeds: &[(3, 1000.0), (5, 425.0)], ..Default::default() });
        let e = &list[0];
        assert_eq!(e.metrics.len(), 1);
        assert_eq!(get(e, "link_speed").value, Some(1000.0));
        assert_eq!(get(e, "link_speed").thresholds, [1000.0, 100.0, 10.0]);
        assert_eq!(e.detail.as_deref(), Some("Ethernet · Example GbE · 1000 Mbit/s"));
        // USB tether: the PC's cable speed, then the phone's own mobile signal, labelled as such.
        let u = &list[1];
        let keys: Vec<&str> = u.metrics.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["link_speed", "rsrp", "sinr", "rsrq"]);
        assert_eq!(get(u, "rsrp").label, "Phone RSRP");
        assert_eq!(get(u, "rsrp").value, Some(-92.0));
        assert!(get(u, "rsrp").note.as_deref().unwrap().contains("phone's mobile signal"));
        assert!(u.detail.as_deref().unwrap().starts_with("USB tethering"));
        // Phone not readable: the mobile dials stay, empty, with the reason.
        let list = build(&Inputs { links: &[link("Phone (USB)", LinkKind::PhoneUsb, "10.0.40.5", false)], ..Default::default() });
        assert_eq!(get(&list[0], "rsrp").value, None);
        assert!(list[0].note.as_deref().unwrap().contains("ADB"));
    }

    #[test]
    fn router_keeps_its_radio_metrics_first() {
        let mut l = link("HomeNet (router)", LinkKind::Router, "192.168.0.20", true);
        l.if_index = 4;
        let t = Telemetry { rsrp: Some(-95.0), sinr: Some(8.0), rsrq: Some(-11.0), ..Default::default() };
        let w = WlanIf { name: "WiFi".into(), signal_pct: Some(70), ..Default::default() };
        let list = build(&Inputs { links: &[l], router: Some(&t), wlan: &[w], ..Default::default() });
        let keys: Vec<&str> = list[0].metrics.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["rsrp", "sinr", "rsrq", "wifi_signal", "wifi_quality", "link_speed"]);
        assert_eq!(get(&list[0], "sinr").value, Some(8.0));
        assert_eq!(get(&list[0], "rsrp").label, "RSRP");
        assert_eq!(get(&list[0], "rsrp").thresholds, [-80.0, -90.0, -100.0]);
        assert_eq!(get(&list[0], "wifi_signal").value, Some(-65.0));
    }

    #[test]
    fn bluetooth_devices_are_connections_without_invented_signal() {
        let bt = [
            BtDevice { name: "Example Headset".into(), connected: true },
            BtDevice { name: "Idle Mouse".into(), connected: false },
        ];
        let list = build(&Inputs { bluetooth: &bt, ..Default::default() });
        assert_eq!(list.len(), 1, "only connected devices");
        let c = &list[0];
        assert_eq!((c.kind.as_str(), c.medium.as_deref()), ("bluetooth", Some("Bluetooth")));
        assert_eq!(c.id, "bluetooth:example-headset");
        assert_eq!(c.name, "Example Headset (Bluetooth)");
        assert!(c.metrics.is_empty());
        assert_eq!(c.detail.as_deref(), Some("Connected"));
        assert!(c.note.as_deref().unwrap().contains("does not report a signal strength"));
        assert!(c.radio.is_none() && c.in_use.is_none());
    }

    #[test]
    fn json_keeps_old_fields_and_adds_metrics() {
        let list = build(&Inputs { links: &[link("Cafe (Wi-Fi)", LinkKind::Other, "172.16.4.9", true)], ..Default::default() });
        let v = serde_json::to_value(&list[0]).unwrap();
        for k in ["id", "name", "kind", "link_kind", "alias", "ssid", "medium", "in_use", "radio", "radio_note", "throughput", "ping", "metrics", "detail", "note"] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        let m = &v["metrics"][0];
        for k in ["key", "label", "unit", "value", "min", "max", "thresholds", "higher_is_better", "note"] {
            assert!(m.get(k).is_some(), "metric missing {k}");
        }
    }
}
