//! Client for ZTE MF-series LTE routers (verified on the MTN MF293N,
//! firmware BD_MTNNGMF293NV1.0.0B06) using the router's own web JSON API:
//!
//! * `GET  /goform/goform_get_cmd_process?isTest=false&multi_data=1&cmd=a,b,c`: read fields
//! * `POST /goform/goform_set_cmd_process`: actions (we only use `LOGIN`)
//!
//! Login: `password = UPPER(SHA256(UPPER(SHA256(pw)) + LD))`, where `LD` is a
//! one-time challenge from the router; the session is the `zsidn` cookie.
//! Every request needs a `Referer` header pointing at the router's UI.
//!
//! This module is read-only apart from logging in.

mod sha256;

pub use sha256::sha256_hex_upper;

use anyhow::{bail, Context, Result};
use netscan::http::{self, HttpResponse, Request};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(6);

/// Refuse to attempt a login when fewer attempts than this remain.
const MIN_ATTEMPTS_LEFT: u32 = 3;

pub const TELEMETRY_FIELDS: &[&str] = &[
    "loginfo",
    "modem_main_state",
    "network_type",
    "network_provider",
    "signalbar",
    "lte_rsrp",
    "lte_rsrq",
    "lte_snr",
    "lte_rssi",
    "lte_pci",
    "cell_id",
    "wan_active_band",
    "wan_active_channel",
    "rmcc",
    "rmnc",
    "lac_code",
    "realtime_rx_thrpt",
    "realtime_tx_thrpt",
    "realtime_rx_bytes",
    "realtime_tx_bytes",
    "realtime_time",
    "ppp_status",
    "monthly_rx_bytes",
    "monthly_tx_bytes",
];

pub struct ZteClient {
    host: String,
    port: u16,
    password: Option<String>,
    cookie: Option<String>,
    login_blocked: Option<String>,
}

impl ZteClient {
    pub fn new(host: impl Into<String>, password: Option<String>) -> Self {
        Self { host: host.into(), port: 80, password, cookie: None, login_blocked: None }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// Why automatic login was stopped, if it was.
    pub fn login_problem(&self) -> Option<&str> {
        self.login_blocked.as_deref()
    }

    async fn send(&mut self, method: &str, path: &str, body: Option<&str>) -> Result<HttpResponse> {
        let referer = format!("http://{}/index.html", self.host);
        let mut headers: Vec<(&str, &str)> = vec![("Referer", referer.as_str())];
        if let Some(c) = &self.cookie {
            headers.push(("Cookie", c.as_str()));
        }
        let req = Request { method, path, headers: &headers, body };
        let resp = http::request(&self.host, self.port, &req, TIMEOUT).await?;
        if let Some(set) = resp.header("set-cookie") {
            if let Some(pair) = set.split(';').next().filter(|p| p.contains('=')) {
                self.cookie = Some(pair.trim().to_string());
            }
        }
        Ok(resp)
    }

    /// Reads raw fields. Unknown fields come back as empty strings.
    pub async fn get_fields(&mut self, fields: &[&str]) -> Result<HashMap<String, String>> {
        let path = format!("/goform/goform_get_cmd_process?isTest=false&multi_data=1&cmd={}", fields.join(","));
        let resp = self.send("GET", &path, None).await?;
        parse_flat_json(&resp.body)
    }

    pub async fn login(&mut self) -> Result<()> {
        let pw = self
            .password
            .clone()
            .context("no router password: set ROUTER_ADMIN_PASSWORD in .env")?;
        let st = self.get_fields(&["psw_fail_num_str", "LD"]).await?;
        let left: u32 = st.get("psw_fail_num_str").and_then(|s| s.parse().ok()).unwrap_or(5);
        if left < MIN_ATTEMPTS_LEFT {
            bail!("only {left} login attempt(s) left before the router locks; not trying. Log in once in a browser to reset it");
        }
        let ld = st.get("LD").filter(|s| !s.is_empty()).context("router did not return a login challenge (LD)")?;
        let hash = sha256_hex_upper(&format!("{}{}", sha256_hex_upper(&pw), ld));
        let body = format!("isTest=false&goformId=LOGIN&password={hash}");
        let resp = self.send("POST", "/goform/goform_set_cmd_process", Some(&body)).await?;
        let r = parse_flat_json(&resp.body)?;
        match r.get("result").map(String::as_str) {
            Some("0") => Ok(()),
            other => bail!("login rejected by router (result={other:?}); check ROUTER_ADMIN_PASSWORD"),
        }
    }

    /// Reads telemetry, logging in first (once) if the session is not active.
    /// After a failed login it stops retrying so it can never trip the lockout.
    pub async fn telemetry(&mut self) -> Result<Telemetry> {
        let mut f = self.get_fields(TELEMETRY_FIELDS).await?;
        let logged_in = f.get("loginfo").map(String::as_str) == Some("ok");
        if !logged_in && self.password.is_some() && self.login_blocked.is_none() {
            match self.login().await {
                Ok(()) => f = self.get_fields(TELEMETRY_FIELDS).await?,
                Err(e) => self.login_blocked = Some(e.to_string()),
            }
        }
        Ok(Telemetry::from_fields(&f))
    }
}

fn parse_flat_json(body: &str) -> Result<HashMap<String, String>> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).context("router returned non-JSON")?;
    let obj = v.as_object().context("router JSON is not an object")?;
    Ok(obj
        .iter()
        .map(|(k, v)| {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (k.clone(), s)
        })
        .collect())
}

/// One snapshot of the router's radio and data state.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Telemetry {
    pub logged_in: bool,
    pub modem_state: String,
    pub network_type: String,
    pub provider: String,
    pub ppp_status: String,
    pub signal_bars: Option<u8>,
    /// Reference Signal Received Power, dBm (signal strength)
    pub rsrp: Option<f32>,
    /// Reference Signal Received Quality, dB
    pub rsrq: Option<f32>,
    /// Signal to interference+noise ratio, dB
    pub sinr: Option<f32>,
    pub rssi: Option<f32>,
    pub pci: Option<u32>,
    pub cell_id: Option<u32>,
    pub band: Option<u16>,
    pub earfcn: Option<u32>,
    pub mcc: Option<String>,
    pub mnc: Option<String>,
    pub tac: Option<String>,
    /// Live throughput as reported by the router (bytes per second, INFERRED unit).
    pub rx_bytes_per_s: Option<u64>,
    pub tx_bytes_per_s: Option<u64>,
    pub session_rx_bytes: Option<u64>,
    pub session_tx_bytes: Option<u64>,
    pub session_secs: Option<u64>,
    pub month_rx_bytes: Option<u64>,
    pub month_tx_bytes: Option<u64>,
}

fn field<T: std::str::FromStr>(f: &HashMap<String, String>, k: &str) -> Option<T> {
    f.get(k).map(|s| s.trim()).filter(|s| !s.is_empty()).and_then(|s| s.parse().ok())
}

fn text(f: &HashMap<String, String>, k: &str) -> String {
    f.get(k).cloned().unwrap_or_default()
}

impl Telemetry {
    pub fn from_fields(f: &HashMap<String, String>) -> Self {
        let opt_text = |k: &str| Some(text(f, k)).filter(|s| !s.is_empty());
        Self {
            logged_in: text(f, "loginfo") == "ok",
            modem_state: text(f, "modem_main_state"),
            network_type: text(f, "network_type"),
            provider: text(f, "network_provider"),
            ppp_status: text(f, "ppp_status"),
            signal_bars: field(f, "signalbar"),
            rsrp: field(f, "lte_rsrp"),
            rsrq: field(f, "lte_rsrq"),
            sinr: field(f, "lte_snr"),
            rssi: field(f, "lte_rssi"),
            pci: field(f, "lte_pci"),
            cell_id: f.get("cell_id").and_then(|s| u32::from_str_radix(s.trim(), 16).ok()),
            band: f
                .get("wan_active_band")
                .and_then(|s| s.split_whitespace().last())
                .and_then(|n| n.parse().ok()),
            earfcn: field(f, "wan_active_channel"),
            mcc: opt_text("rmcc"),
            mnc: opt_text("rmnc"),
            tac: opt_text("lac_code"),
            rx_bytes_per_s: field(f, "realtime_rx_thrpt"),
            tx_bytes_per_s: field(f, "realtime_tx_thrpt"),
            session_rx_bytes: field(f, "realtime_rx_bytes"),
            session_tx_bytes: field(f, "realtime_tx_bytes"),
            session_secs: field(f, "realtime_time"),
            month_rx_bytes: field(f, "monthly_rx_bytes"),
            month_tx_bytes: field(f, "monthly_tx_bytes"),
        }
    }

    /// LTE cell id = eNodeB id (upper 20 bits) + sector (lower 8 bits).
    pub fn enb(&self) -> Option<u32> {
        self.cell_id.map(|c| c >> 8)
    }

    pub fn sector(&self) -> Option<u32> {
        self.cell_id.map(|c| c & 0xff)
    }

    pub fn connected(&self) -> bool {
        self.ppp_status.contains("connected") && !self.ppp_status.contains("dis")
    }
}

/// Downlink frequency of common LTE bands (3GPP TS 36.101).
pub fn band_mhz(band: u16) -> Option<&'static str> {
    Some(match band {
        1 => "2100",
        3 => "1800",
        5 => "850",
        7 => "2600",
        8 => "900",
        20 => "800",
        28 => "700",
        38 => "2600 TDD",
        40 => "2300 TDD",
        41 => "2500 TDD",
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Grade {
    Excellent,
    Good,
    Fair,
    Poor,
}

impl Grade {
    pub fn label(self) -> &'static str {
        match self {
            Grade::Excellent => "excellent",
            Grade::Good => "good",
            Grade::Fair => "fair",
            Grade::Poor => "poor",
        }
    }
}

fn grade(v: f32, excellent: f32, good: f32, fair: f32) -> Grade {
    if v >= excellent {
        Grade::Excellent
    } else if v >= good {
        Grade::Good
    } else if v >= fair {
        Grade::Fair
    } else {
        Grade::Poor
    }
}

/// Common field-engineering thresholds for LTE.
pub fn grade_rsrp(v: f32) -> Grade {
    grade(v, -80.0, -90.0, -100.0)
}
pub fn grade_rsrq(v: f32) -> Grade {
    grade(v, -10.0, -15.0, -20.0)
}
pub fn grade_sinr(v: f32) -> Grade {
    grade(v, 20.0, 13.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KEY=VALUE pairs from the workspace's private .env, if present.
    fn private_env() -> HashMap<String, String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect()
    }

    #[test]
    fn parses_real_mf293n_snapshot() {
        // Real MF293N reading from 2026-11-11. The tower ids reveal location, so
        // they live in the project's private .env (ROUTER_CELL_ID / ROUTER_ENB /
        // ROUTER_SECTOR / ROUTER_TAC). Without .env the tower checks are skipped.
        let env = private_env();
        let cell = env.get("ROUTER_CELL_ID").cloned().unwrap_or_default();
        let tac = env.get("ROUTER_TAC").cloned().unwrap_or_default();
        let body = format!(
            r#"{{"loginfo":"ok","signalbar":"5","lte_rsrp":"-83","lte_rsrq":"-7","lte_snr":"26.0",
            "lte_rssi":"-56","lte_pci":"55","cell_id":"{cell}","wan_active_band":"LTE BAND 7",
            "wan_active_channel":"2852","rmcc":"621","rmnc":"30","lac_code":"{tac}",
            "realtime_rx_thrpt":"0","realtime_tx_thrpt":"0","ppp_status":"ppp_connected"}}"#
        );
        let t = Telemetry::from_fields(&parse_flat_json(&body).unwrap());
        assert!(t.logged_in && t.connected());
        assert_eq!(t.rsrp, Some(-83.0));
        assert_eq!(t.sinr, Some(26.0));
        assert_eq!(t.band, Some(7));
        assert_eq!(t.earfcn, Some(2852));
        assert_eq!(band_mhz(7), Some("2600"));
        if let (Some(enb), Some(sector)) = (env.get("ROUTER_ENB"), env.get("ROUTER_SECTOR")) {
            assert_eq!(t.enb().map(|e| e.to_string()).as_deref(), Some(enb.as_str()));
            assert_eq!(t.sector().map(|s| s.to_string()).as_deref(), Some(sector.as_str()));
            assert_eq!(t.tac.as_deref(), Some(tac.as_str()));
        } else {
            assert_eq!(t.cell_id, None);
        }
        assert_eq!(grade_rsrp(-83.0), Grade::Good);
        assert_eq!(grade_sinr(26.0), Grade::Excellent);
    }

    #[test]
    fn login_hash_matches_router_scheme() {
        let ld = "670671CD97404156226E507973F2AB8330D3022CA96E0C93BDBDB320C41ADCAF";
        let inner = sha256_hex_upper("admin");
        assert_eq!(inner, "8C6976E5B5410415BDE908BD4DEE15DFB167A9C873FC4BB8A81F6F2AB448A918");
        assert_eq!(sha256_hex_upper(&format!("{inner}{ld}")).len(), 64);
    }
}
