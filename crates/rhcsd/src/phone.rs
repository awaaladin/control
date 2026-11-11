//! Android phone over ADB (USB or wireless debugging), no root needed:
//! LTE signal from the modem, battery, and the clients on its hotspot.

use crate::state::{now_ms, Config, Shared};
use anyhow::{bail, Context, Result};
use rhcs_core::MacAddr;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(5);

/// One adb round-trip for everything we read.
const PROBE: &str = "dumpsys telephony.registry | grep -E 'mSignalStrength=|CellIdentityLte:' | head -n 4; \
echo @@BATTERY; dumpsys battery; echo @@NEIGH; ip neigh";

const INFO: &str = "getprop ro.vivo.product.release.name; getprop ro.product.model; getprop ro.build.version.release";

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct HotspotClient {
    pub ip: String,
    pub mac: String,
    pub iface: String,
    pub state: String,
    pub vendor: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PhoneStatus {
    pub model: Option<String>,
    pub android: Option<String>,
    pub battery_level: Option<u8>,
    pub battery_temp_c: Option<f32>,
    pub charging: Option<bool>,
    pub usb_powered: Option<bool>,
    pub rsrp: Option<f32>,
    pub rsrq: Option<f32>,
    pub sinr: Option<f32>,
    pub rssi: Option<f32>,
    pub pci: Option<u32>,
    pub cell_id: Option<u32>,
    pub tac: Option<u32>,
    pub earfcn: Option<u32>,
    pub band: Option<u16>,
    pub mcc: Option<String>,
    pub mnc: Option<String>,
    pub hotspot_clients: Vec<HotspotClient>,
}

async fn adb(cfg: &Config, script: &str) -> Result<String> {
    let mut cmd = tokio::process::Command::new(&cfg.adb_path);
    if let Some(serial) = &cfg.phone_serial {
        cmd.args(["-s", serial]);
    }
    cmd.args(["shell", script]).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .context("adb timed out")?
        .context("could not run adb")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!("{}", if err.is_empty() { "adb failed".to_string() } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `key=value` tokens after `marker`, up to `stop` (if present).
fn kv_after<'a>(text: &'a str, marker: &str, stop: &str) -> HashMap<&'a str, &'a str> {
    let Some(start) = text.find(marker) else { return HashMap::new() };
    let rest = &text[start + marker.len()..];
    let rest = rest.find(stop).map(|i| &rest[..i]).unwrap_or(rest);
    rest.split_whitespace()
        .filter_map(|tok| tok.split_once('='))
        .map(|(k, v)| (k, v.trim_end_matches([',', '}'])))
        .collect()
}

/// Android reports "unavailable" as Integer.MAX_VALUE.
fn num(v: Option<&&str>) -> Option<f32> {
    let n: i64 = v?.parse().ok()?;
    (n != i32::MAX as i64 && n != i64::MAX).then_some(n as f32)
}

fn int(v: Option<&&str>) -> Option<u32> {
    let n: i64 = v?.parse().ok()?;
    (n >= 0 && n != i32::MAX as i64).then_some(n as u32)
}

pub fn parse(text: &str) -> PhoneStatus {
    let (radio, rest) = text.split_once("@@BATTERY").unwrap_or((text, ""));
    let (battery, neigh) = rest.split_once("@@NEIGH").unwrap_or((rest, ""));

    let sig = kv_after(radio, "CellSignalStrengthLte:", "CellSignalStrengthNr");
    let id = kv_after(radio, "CellIdentityLte:", "}");
    let band = id
        .get("mBands")
        .and_then(|b| b.trim_matches(['[', ']']).split(',').next())
        .and_then(|b| b.trim().parse().ok());
    let text_val = |k: &str| id.get(k).map(|v| v.to_string()).filter(|v| v != "null" && !v.is_empty());

    let mut bat: HashMap<String, String> = HashMap::new();
    for line in battery.lines() {
        if let Some((k, v)) = line.split_once(':') {
            bat.insert(k.trim().to_string(), v.trim().to_string());
        }
    }

    let hotspot_clients = neigh
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let ip = *f.first()?;
            if !ip.contains('.') {
                return None; // IPv6
            }
            let iface = f.iter().position(|x| *x == "dev").and_then(|i| f.get(i + 1))?;
            let mac = f.iter().position(|x| *x == "lladdr").and_then(|i| f.get(i + 1))?;
            let state = f.last().copied().unwrap_or("");
            if matches!(state, "FAILED" | "INCOMPLETE") {
                return None;
            }
            Some(HotspotClient {
                ip: ip.to_string(),
                mac: mac.to_lowercase(),
                iface: iface.to_string(),
                state: state.to_string(),
                vendor: None,
                label: None,
            })
        })
        .collect();

    PhoneStatus {
        battery_level: bat.get("level").and_then(|v| v.parse().ok()),
        battery_temp_c: bat.get("temperature").and_then(|v| v.parse::<f32>().ok()).map(|t| t / 10.0),
        charging: bat.get("status").map(|s| s == "2"),
        usb_powered: bat.get("USB powered").map(|s| s == "true"),
        rsrp: num(sig.get("rsrp")),
        rsrq: num(sig.get("rsrq")),
        sinr: num(sig.get("rssnr")),
        rssi: num(sig.get("rssi")),
        pci: int(id.get("mPci")),
        cell_id: int(id.get("mCi")),
        tac: int(id.get("mTac")),
        earfcn: int(id.get("mEarfcn")),
        band,
        mcc: text_val("mMcc"),
        mnc: text_val("mMnc"),
        hotspot_clients,
        ..Default::default()
    }
}

pub async fn run(s: Shared) {
    let mut online: Option<bool> = None;
    let mut info: Option<(Option<String>, Option<String>)> = None;
    let mut clients: Option<HashMap<String, HotspotClient>> = None;

    loop {
        let result = async {
            if info.is_none() {
                let raw = adb(&s.cfg, INFO).await?;
                let l: Vec<&str> = raw.lines().map(str::trim).collect();
                let market = l.first().copied().unwrap_or("");
                let model = l.get(1).copied().unwrap_or("");
                let name = match (market.is_empty(), model.is_empty()) {
                    (false, false) => Some(format!("{market} ({model})")),
                    (true, false) => Some(model.to_string()),
                    _ => None,
                };
                info = Some((name, l.get(2).map(|v| v.to_string()).filter(|v| !v.is_empty())));
            }
            let mut st = parse(&adb(&s.cfg, PROBE).await?);
            if let Some((m, a)) = &info {
                st.model = m.clone();
                st.android = a.clone();
            }
            anyhow::Ok(st)
        }
        .await;

        match result {
            Ok(mut st) => {
                for c in st.hotspot_clients.iter_mut() {
                    let mac = MacAddr::parse(&c.mac);
                    c.vendor = mac.and_then(|m| s.oui.vendor(&m).map(str::to_string));
                    c.label = s.cfg.label_for(mac.as_ref());
                }
                if online != Some(true) {
                    s.event("phone_online", format!("phone connected: {}", st.model.clone().unwrap_or("android".into())), json!({})).await;
                }
                online = Some(true);

                let now: HashMap<String, HotspotClient> =
                    st.hotspot_clients.iter().map(|c| (c.mac.clone(), c.clone())).collect();
                if let Some(prev) = &clients {
                    for (mac, c) in &now {
                        if !prev.contains_key(mac) {
                            let who = c.label.clone().or(c.vendor.clone()).unwrap_or("unknown device".into());
                            s.event("device_join", format!("hotspot: {who} joined ({} {mac})", c.ip), json!(c)).await;
                        }
                    }
                    for (mac, c) in prev {
                        if !now.contains_key(mac) {
                            let who = c.label.clone().or(c.vendor.clone()).unwrap_or("unknown device".into());
                            s.event("device_leave", format!("hotspot: {who} left ({} {mac})", c.ip), json!(c)).await;
                        }
                    }
                }
                clients = Some(now);

                let ts = now_ms();
                {
                    let mut i = s.inner.write().await;
                    i.sources.phone.online = true;
                    i.sources.phone.error = None;
                    i.sources.phone.last_ok = Some(ts);
                    i.phone = Some(st.clone());
                }
                s.send(json!({ "type": "phone", "ts": ts, "data": st }));
            }
            Err(e) => {
                if online != Some(false) {
                    s.event("phone_offline", format!("phone not reachable over adb: {e}"), json!({})).await;
                }
                online = Some(false);
                info = None;
                let mut i = s.inner.write().await;
                i.sources.phone.online = false;
                i.sources.phone.error = Some(e.to_string());
            }
        }
        s.send_status().await;
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adb_output() {
        // Format of real vivo Y03 / Android 16 output; ids are not real.
        let text = "  mSignalStrength=SignalStrength:{mLte=CellSignalStrengthLte: rssi=-77 rsrp=-85 rsrq=-8 rssnr=21 cqiTableIndex=2147483647 cqi=0 ta=0 level=4 parametersUseForLevel=0,mNr=CellSignalStrengthNr:{ csiRsrp = 2147483647 }}\n\
  mCellIdentity=CellIdentityLte:{ mCi=1234567 mPci=55 mTac=1000 mEarfcn=1650 mBands=[3] mBandwidth=2147483647 mMcc=621 mMnc=30 mAlphaLong=MTN}\n\
@@BATTERY\nCurrent Battery Service state:\n  USB powered: true\n  status: 5\n  level: 100\n  temperature: 350\n\
@@NEIGH\n10.0.0.5 dev ap0 lladdr 04:56:04:00:00:01 REACHABLE\nfe80::1 dev ap0 lladdr 04:56:04:00:00:01 STALE\n10.0.0.9 dev ap0  FAILED\n";
        let p = parse(text);
        assert_eq!(p.rsrp, Some(-85.0));
        assert_eq!(p.sinr, Some(21.0));
        assert_eq!(p.band, Some(3));
        assert_eq!(p.earfcn, Some(1650));
        assert_eq!(p.pci, Some(55));
        assert_eq!(p.mcc.as_deref(), Some("621"));
        assert_eq!(p.battery_level, Some(100));
        assert_eq!(p.battery_temp_c, Some(35.0));
        assert_eq!(p.usb_powered, Some(true));
        assert_eq!(p.hotspot_clients.len(), 1);
        assert_eq!(p.hotspot_clients[0].iface, "ap0");
    }
}
