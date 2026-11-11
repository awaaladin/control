//! Periodic (and on-demand) LAN scans of every network the machine is on:
//! e.g. the phone hotspot over USB/Wi-Fi and the router's LAN at the same time.

use crate::state::{now_ms, DeviceView, Shared};
use netscan::scan;
use serde_json::json;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

/// A device must be missing from this many scans in a row before it "left".
const MISSES_TO_LEAVE: u32 = 2;

fn key(d: &DeviceView) -> String {
    d.device.mac.map(|m| m.to_string()).unwrap_or_else(|| format!("{}@{}", d.device.ip, d.network))
}

pub async fn run(s: Shared) {
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut known: HashMap<String, (DeviceView, u32)> = HashMap::new();
    let mut first = true;

    loop {
        let gateways: Vec<Ipv4Addr> = s.cfg.router_ip.parse().into_iter().collect();
        let targets = scan::targets(&gateways, &s.cfg.scan_cidrs);
        let networks: Vec<String> = targets.iter().map(|t| t.subnet.to_string()).collect();
        {
            let mut i = s.inner.write().await;
            i.sources.scan.running = true;
            i.sources.scan.networks = networks.clone();
        }
        s.send_status().await;

        let now = now_ms();
        let mut seen: HashMap<String, DeviceView> = HashMap::new();
        for t in &targets {
            for d in scan::scan(t, &s.oui, true).await {
                let label = s.cfg.label_for(d.mac.as_ref());
                let view = DeviceView { network: t.subnet.to_string(), label, first_seen: now, last_seen: now, device: d };
                seen.insert(key(&view), view);
            }
        }

        // Merge with what we knew: keep first_seen, count misses, emit join/leave.
        for (k, mut v) in seen.clone() {
            match known.get_mut(&k) {
                Some((old, misses)) => {
                    v.first_seen = old.first_seen;
                    *old = v;
                    *misses = 0;
                }
                None => {
                    if !first {
                        let who = v.label.clone().or(v.device.vendor.clone()).unwrap_or_else(|| "unknown device".into());
                        s.event("device_join", format!("{who} joined {} ({})", v.network, v.device.ip), json!(v)).await;
                    }
                    known.insert(k, (v, 0));
                }
            }
        }
        let mut gone = Vec::new();
        for (k, (v, misses)) in known.iter_mut() {
            if !seen.contains_key(k) {
                *misses += 1;
                if *misses >= MISSES_TO_LEAVE {
                    gone.push((k.clone(), v.clone()));
                }
            }
        }
        for (k, v) in gone {
            known.remove(&k);
            let who = v.label.clone().or(v.device.vendor.clone()).unwrap_or_else(|| "unknown device".into());
            s.event("device_leave", format!("{who} left {} ({})", v.network, v.device.ip), json!(v)).await;
        }
        first = false;

        let mut list: Vec<DeviceView> = known.values().map(|(v, _)| v.clone()).collect();
        list.sort_by(|a, b| a.network.cmp(&b.network).then(u32::from(a.device.ip).cmp(&u32::from(b.device.ip))));
        {
            let mut i = s.inner.write().await;
            i.devices = list.clone();
            i.sources.scan.running = false;
            i.sources.scan.last_run = Some(now_ms());
        }
        s.send(json!({ "type": "devices", "ts": now_ms(), "data": list }));
        s.send_status().await;
        println!(
            "{} [scan] {} device(s) on {}",
            chrono::Local::now().format("%H:%M:%S"),
            known.len(),
            if networks.is_empty() { "no networks".into() } else { networks.join(", ") }
        );

        tokio::select! {
            _ = s.scan_now.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(s.cfg.scan_interval_s.max(30))) => {}
        }
    }
}
