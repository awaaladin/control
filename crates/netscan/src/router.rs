//! Gateway reconnaissance: what is this router and what can we do with it?
//! Read-only. Produces findings tagged with how sure we are.

use crate::{discover, dns, net::NetInfo, oui::OuiDb, ports, ssdp};
use anyhow::{Context, Result};
use rhcs_core::{Device, Evidence, Finding};
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct RouterReport {
    pub net: NetInfo,
    pub gateway: Device,
    pub probed_ports: usize,
    pub findings: Vec<Finding>,
}

pub async fn recon(net: &NetInfo, oui: &OuiDb) -> Result<RouterReport> {
    let gw = net.gateway.context("no default gateway; are you connected to a network?")?;
    let port_list = ports::router_ports();
    let (mac, open, upnp, hostname) = tokio::join!(
        discover::resolve_mac(gw),
        ports::probe(gw, &port_list, Duration::from_millis(1200)),
        ssdp::discover(net.local_ip, Duration::from_secs(3)),
        dns::reverse_lookup(gw, Some(gw)),
    );

    let mut d = Device::new(gw);
    d.is_gateway = true;
    d.mac = mac;
    d.vendor = mac.and_then(|m| oui.vendor(&m).map(str::to_string));
    d.hostname = hostname;
    d.ports = open;
    d.upnp = upnp
        .unwrap_or_default()
        .into_iter()
        .filter(|h| h.ip == gw)
        .map(|h| h.info)
        .collect();

    let findings = assess(&d, port_list.len(), !oui.is_empty());
    Ok(RouterReport { net: net.clone(), gateway: d, probed_ports: port_list.len(), findings })
}

fn has(text: &Option<String>, needles: &[&str]) -> bool {
    text.as_deref()
        .map(|t| {
            let t = t.to_ascii_lowercase();
            needles.iter().any(|n| t.contains(n))
        })
        .unwrap_or(false)
}

pub fn assess(d: &Device, probed: usize, have_oui: bool) -> Vec<Finding> {
    use Evidence::*;
    let mut f = Vec::new();

    match d.mac {
        Some(m) if m.is_locally_administered() => f.push(Finding::new(
            Observed,
            format!("Gateway MAC {m} is locally administered (randomized). Phone hotspots and virtual interfaces do this; a home router's LAN port normally carries a vendor-assigned MAC."),
        )),
        Some(m) => match &d.vendor {
            Some(v) => f.push(Finding::new(Observed, format!("Gateway MAC {m} belongs to: {v} (IEEE registry)."))),
            None if have_oui => f.push(Finding::new(Observed, format!("Gateway MAC {m}: prefix not in the IEEE registry."))),
            None => f.push(Finding::new(Observed, format!("Gateway MAC {m}. Run scripts/get-oui.ps1 to resolve the manufacturer."))),
        },
        None => f.push(Finding::new(Observed, "Gateway did not answer ARP; cannot read its MAC.")),
    }

    if let Some(name) = &d.hostname {
        f.push(Finding::new(Observed, format!("Gateway reverse-DNS name: {name}")));
    }

    for u in &d.upnp {
        if let Some(id) = u.identity() {
            f.push(Finding::new(
                Observed,
                format!("UPnP self-identification: {id}{}. Search \"{id} OpenWrt\" for custom firmware support.",
                    u.friendly_name.as_ref().map(|n| format!(" (\"{n}\")")).unwrap_or_default()),
            ));
            break;
        }
    }
    if d.upnp.iter().any(|u| has(&u.device_type, &["internetgatewaydevice"])) {
        f.push(Finding::new(
            Observed,
            "UPnP Internet Gateway Device service is enabled: any LAN device can open ports on this router. Convenient, but worth turning off once we have our own tooling.",
        ));
    }

    let port = |p: u16| d.ports.iter().find(|o| o.port == p);

    if let Some(ssh) = port(22) {
        f.push(Finding::new(
            Observed,
            format!("SSH is open{}. With the admin password this gives a shell: the path to running our own Rust binaries on the router. Next step: log in and run `uname -m; cat /etc/os-release; free; df -h`.",
                ssh.banner.as_ref().map(|b| format!(" ({b})")).unwrap_or_default()),
        ));
        if has(&ssh.banner, &["dropbear"]) {
            f.push(Finding::new(Documented, "Dropbear is the default SSH server on OpenWrt and many embedded Linux firmwares, so this is a strong hint that the router runs Linux."));
        }
    }
    if port(23).is_some() {
        f.push(Finding::new(Observed, "Telnet is open. It is unencrypted, so use it only on your own LAN and prefer SSH. It often means a BusyBox shell is available."));
    }

    let web: Vec<_> = d.ports.iter().filter(|p| p.banner.as_deref().is_some_and(|b| b.starts_with("HTTP"))).collect();
    for w in &web {
        f.push(Finding::new(Observed, format!("Web admin on port {}: {}", w.port, w.banner.as_deref().unwrap_or(""))));
    }
    if web.iter().any(|w| has(&w.banner, &["luci", "openwrt", "uhttpd"])) {
        f.push(Finding::new(Inferred, "Web server fingerprint matches OpenWrt (uhttpd/LuCI). Open firmware is likely already installed, which is the best case for running Rust on the router."));
    }
    if port(443).is_some() && web.is_empty() {
        f.push(Finding::new(Observed, "HTTPS admin on port 443 (TLS is not inspected by this tool). Open https://<gateway> in a browser to see the login page."));
    }
    if port(7547).is_some() || port(4567).is_some() {
        f.push(Finding::new(Observed, "TR-069 (ISP remote management) port is open. ISP-managed routers may be firmware-locked and can have settings pushed over yours."));
    }
    if port(53).is_some() {
        f.push(Finding::new(Observed, "Router runs a DNS service (TCP 53). This is where a Rust DNS monitor or ad-block layer would hook in."));
    }

    if d.ports.is_empty() {
        f.push(Finding::new(
            Observed,
            format!("No TCP service answered on any of the {probed} probed ports. The gateway either blocks admin from this client or uses an unusual port."),
        ));
        if d.mac.is_some_and(|m| m.is_locally_administered()) {
            f.push(Finding::new(
                Inferred,
                "This gateway is very likely a phone hotspot, not your home router. Connect to the router (Ethernet or its own Wi-Fi) and run `rhcs router` again.",
            ));
        }
    }
    f
}
