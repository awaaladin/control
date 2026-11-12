//! Full LAN scan of one or more networks the machine is attached to.
//!
//! A laptop can sit on several networks at once (Wi-Fi to a router, USB
//! tethering to a phone, Ethernet...). `targets` works out each network we
//! have an address on, and `scan` sweeps one of them.

use crate::net::{self, Subnet};
use crate::oui::OuiDb;
use crate::{discover, dns, ports, ssdp};
use rhcs_core::Device;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanTarget {
    pub subnet: Subnet,
    pub local_ip: Ipv4Addr,
    pub gateway: Option<Ipv4Addr>,
}

/// Networks to scan: the default-route network, every network that one of
/// `gateways` sits on (if we have an address there), and any extra CIDRs.
/// Large subnets are clamped to the /24 around our address.
pub fn targets(gateways: &[Ipv4Addr], extra_cidrs: &[String]) -> Vec<ScanTarget> {
    let mut out: Vec<ScanTarget> = Vec::new();
    let add = |t: ScanTarget, out: &mut Vec<ScanTarget>| {
        if !out.iter().any(|o| o.subnet == t.subnet) {
            out.push(t);
        }
    };

    if let Ok(info) = net::detect() {
        let subnet = if info.subnet.prefix < 22 { Subnet::new(info.local_ip, 24) } else { info.subnet };
        add(ScanTarget { subnet, local_ip: info.local_ip, gateway: info.gateway }, &mut out);
    }
    for &gw in gateways {
        if let Ok(local) = net::local_ip_for(gw) {
            let subnet = Subnet::new(local, 24);
            // Only if we are actually on that network (not just routed to it).
            if subnet.contains(gw) {
                add(ScanTarget { subnet, local_ip: local, gateway: Some(gw) }, &mut out);
            }
        }
    }
    for cidr in extra_cidrs {
        if let Ok(subnet) = Subnet::parse(cidr) {
            let probe = subnet.hosts().first().copied().unwrap_or(subnet.network);
            if let Ok(local) = net::local_ip_for(probe) {
                if subnet.contains(local) {
                    add(ScanTarget { subnet, local_ip: local, gateway: None }, &mut out);
                }
            }
        }
    }
    out
}

/// ARP sweep + UPnP + reverse DNS (+ optional port probe) of one network.
pub async fn scan(t: &ScanTarget, oui: &OuiDb, probe_ports: bool) -> Vec<Device> {
    let hosts = t.subnet.hosts();
    let (seen, hits) = tokio::join!(
        discover::sweep(&hosts, 64),
        ssdp::discover(t.local_ip, Duration::from_secs(3))
    );

    let mut devices: BTreeMap<u32, Device> = BTreeMap::new();
    for s in seen {
        let d = devices.entry(s.ip.into()).or_insert_with(|| Device::new(s.ip));
        d.mac = s.mac;
        d.rtt_ms = Some(s.rtt_ms);
    }
    devices.entry(t.local_ip.into()).or_insert_with(|| Device::new(t.local_ip)).is_self = true;
    if let Some(gw) = t.gateway.filter(|g| t.subnet.contains(*g)) {
        devices.entry(gw.into()).or_insert_with(|| Device::new(gw)).is_gateway = true;
    }
    for h in hits.unwrap_or_default() {
        if t.subnet.contains(h.ip) {
            let d = devices.entry(h.ip.into()).or_insert_with(|| Device::new(h.ip));
            if !d.upnp.iter().any(|u| u.identity() == h.info.identity() && u.friendly_name == h.info.friendly_name) {
                d.upnp.push(h.info);
            }
        }
    }

    let sem = Arc::new(Semaphore::new(16));
    let mut set = JoinSet::new();
    for (_, mut d) in std::mem::take(&mut devices) {
        let sem = sem.clone();
        let gw = t.gateway;
        set.spawn(async move {
            let _p = sem.acquire_owned().await;
            let probe = async {
                if !probe_ports || d.is_self {
                    Vec::new()
                } else if d.is_gateway {
                    ports::probe(d.ip, &ports::router_ports(), Duration::from_millis(900)).await
                } else {
                    ports::probe(d.ip, ports::COMMON, Duration::from_millis(900)).await
                }
            };
            let (name, open) = tokio::join!(dns::reverse_lookup(d.ip, gw), probe);
            d.hostname = name;
            d.ports = open;
            d
        });
    }
    let mut out = Vec::new();
    while let Some(Ok(mut d)) = set.join_next().await {
        d.vendor = d.mac.and_then(|m| oui.vendor(&m).map(str::to_string));
        out.push(d);
    }
    out.sort_by_key(|d| u32::from(d.ip));
    out
}
