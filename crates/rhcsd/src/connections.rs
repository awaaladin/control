//! Every internet connection by name, with the same data for each: radio
//! (from the router or the phone behind it), live throughput where known,
//! and the latest Game Boost ping / jitter / loss. `GET /api/connections`.
//!
//! Names come from links.rs ("Jeffnou (router)", "vivo Y (phone hotspot)",
//! "Phone (USB)"). On the phone hub (Termux) the phone's own radio is listed
//! as "This phone (mobile data)".

use crate::boost::LinkResult;
use crate::links::{Link, LinkKind};
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
                LinkKind::Other => ("other", None, Some("rhcs doesn't know which device is behind this connection".to_string())),
            };
            let throughput = match (l.kind, i.router) {
                (LinkKind::Router, Some(t)) => {
                    Some(Throughput { rx_bps: t.rx_bytes_per_s, tx_bps: t.tx_bytes_per_s, source: "router WAN".into() })
                }
                _ => None,
            };
            let ping = i.boost.iter().find(|r| r.link.id == l.id).filter(|r| r.stats.sent > 0).map(|r| Ping {
                ping_ms: r.stats.ping_ms,
                jitter_ms: r.stats.jitter_ms,
                loss_pct: r.stats.loss_pct,
                score: r.score,
                grade: r.grade.clone(),
                at: i.boost_at,
            });
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
                radio_note: note,
                throughput,
                ping,
            }
        })
        .collect();
    if i.on_phone {
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
            });
        }
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
    let list = build(&Inputs {
        links: &links,
        router,
        phone,
        phone_model: phone.and_then(|p| p.model.clone()),
        boost: &boost.links,
        boost_at: boost.finished.or(boost.started),
        on_phone: crate::phone::in_termux(),
    });
    let best_id = best(&list).map(|c| c.id.clone());
    Json(json!({
        "connections": list,
        "best": best_id,
        "admin": s.admin,
        "switch_supported": crate::switch::supported(),
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
}
