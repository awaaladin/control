//! The machine's internet links: every local IPv4 address on an adapter that
//! is up and has a default gateway. Read-only: nothing here changes Windows
//! network settings.
//!
//! Windows uses the strong host model, so a socket bound to one link's local
//! address leaves through that link even when another link is the default.
//! That is what lets Game Boost measure each link separately without admin.

use serde::Serialize;
use std::net::Ipv4Addr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Gateway is the configured router (ROUTER_IP).
    Router,
    /// USB tethering from the phone (RNDIS adapter).
    PhoneUsb,
    /// Wi-Fi joined to the phone's hotspot (the phone lists us as a client).
    PhoneHotspot,
    Other,
}

/// One adapter as the OS reports it (input to [`classify`]).
#[derive(Clone, Debug, Default)]
pub struct Adapter {
    /// Windows interface alias, e.g. "Wi-Fi", "Ethernet 2". This is the name
    /// `scripts\lan-only.ps1 -Interface` expects.
    pub alias: String,
    pub description: String,
    /// IANA ifType: 6 = Ethernet (RNDIS too), 71 = Wi-Fi, 24 = loopback.
    pub if_type: u32,
    pub up: bool,
    pub metric: Option<u32>,
    pub addrs: Vec<Ipv4Addr>,
    pub gateways: Vec<Ipv4Addr>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Link {
    /// Stable id for the UI: the local address.
    pub id: String,
    /// Friendly name, e.g. "Router (Wi-Fi)", "Phone (USB)".
    pub name: String,
    pub kind: LinkKind,
    /// "Wi-Fi", "Ethernet", "USB" or "other".
    pub medium: String,
    pub adapter: String,
    pub description: String,
    pub local_ip: Ipv4Addr,
    pub gateway: Option<Ipv4Addr>,
    /// Windows interface metric (lower wins internet traffic).
    pub metric: Option<u32>,
    /// Windows currently sends internet traffic through this link.
    pub is_default: bool,
}

fn is_usable(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast())
}

fn looks_like_usb_tether(a: &Adapter) -> bool {
    let d = a.description.to_ascii_lowercase();
    d.contains("ndis") || d.contains("android") || d.contains("usb tether") || d.contains("internet sharing")
}

/// Turns raw adapters into named internet links. Pure, so it is unit-tested.
/// * `router_ip`: the configured router; a link whose gateway is it is "Router".
/// * `hotspot_ips`: client addresses the phone reports on its hotspot.
/// * `default_ip`: the local address Windows uses for the internet right now.
pub fn classify(adapters: &[Adapter], router_ip: Option<Ipv4Addr>, hotspot_ips: &[Ipv4Addr], default_ip: Option<Ipv4Addr>) -> Vec<Link> {
    let mut out = Vec::new();
    for a in adapters {
        if !a.up || a.if_type == 24 || a.if_type == 131 {
            continue;
        }
        let gateway = a.gateways.iter().copied().find(|g| !g.is_unspecified());
        if gateway.is_none() {
            continue; // no route out: a local-only network
        }
        for &ip in a.addrs.iter().filter(|ip| is_usable(**ip)) {
            let kind = if router_ip.is_some() && gateway == router_ip {
                LinkKind::Router
            } else if looks_like_usb_tether(a) {
                LinkKind::PhoneUsb
            } else if hotspot_ips.contains(&ip) {
                LinkKind::PhoneHotspot
            } else {
                LinkKind::Other
            };
            let medium = match (kind, a.if_type) {
                (LinkKind::PhoneUsb, _) => "USB",
                (_, 71) => "Wi-Fi",
                (_, 6) => "Ethernet",
                _ => "other",
            };
            let name = match kind {
                LinkKind::Router => format!("Router ({medium})"),
                LinkKind::PhoneUsb => "Phone (USB)".to_string(),
                LinkKind::PhoneHotspot => "Phone (hotspot)".to_string(),
                LinkKind::Other => a.alias.clone(),
            };
            out.push(Link {
                id: ip.to_string(),
                name,
                kind,
                medium: medium.to_string(),
                adapter: a.alias.clone(),
                description: a.description.clone(),
                local_ip: ip,
                gateway,
                metric: a.metric,
                is_default: default_ip == Some(ip),
            });
        }
    }
    // Two links with the same name (e.g. two "Wi-Fi" adapters): add the adapter.
    let names: Vec<String> = out.iter().map(|l| l.name.clone()).collect();
    for l in out.iter_mut() {
        if names.iter().filter(|n| **n == l.name).count() > 1 {
            l.name = format!("{} · {}", l.name, l.adapter);
        }
    }
    out
}

/// Adapters as reported by the OS.
#[cfg(windows)]
pub fn adapters() -> Vec<Adapter> {
    use windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN, SOCKET_ADDRESS};

    unsafe fn v4(sa: &SOCKET_ADDRESS) -> Option<Ipv4Addr> {
        if sa.lpSockaddr.is_null() || (*sa.lpSockaddr).sa_family != AF_INET {
            return None;
        }
        let sin = &*(sa.lpSockaddr as *const SOCKADDR_IN);
        Some(Ipv4Addr::from(sin.sin_addr.S_un.S_addr.to_ne_bytes()))
    }
    unsafe fn wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }

    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u64> = Vec::new(); // u64 for alignment
    let mut rc = ERROR_BUFFER_OVERFLOW;
    for _ in 0..3 {
        buf = vec![0u64; (size as usize).div_ceil(8)];
        rc = unsafe {
            GetAdaptersAddresses(AF_INET as u32, flags, std::ptr::null(), buf.as_mut_ptr().cast(), &mut size)
        };
        if rc != ERROR_BUFFER_OVERFLOW {
            break;
        }
    }
    if rc != 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    unsafe {
        while !p.is_null() {
            let a = &*p;
            let mut addrs = Vec::new();
            let mut u = a.FirstUnicastAddress;
            while !u.is_null() {
                if let Some(ip) = v4(&(*u).Address) {
                    addrs.push(ip);
                }
                u = (*u).Next;
            }
            let mut gateways = Vec::new();
            let mut g = a.FirstGatewayAddress;
            while !g.is_null() {
                if let Some(ip) = v4(&(*g).Address) {
                    gateways.push(ip);
                }
                g = (*g).Next;
            }
            out.push(Adapter {
                alias: wide(a.FriendlyName),
                description: wide(a.Description),
                if_type: a.IfType,
                up: a.OperStatus == IfOperStatusUp,
                metric: (a.Ipv4Metric != 0).then_some(a.Ipv4Metric),
                addrs,
                gateways,
            });
            p = a.Next;
        }
    }
    out
}

/// Elsewhere (Linux, Android/Termux): only the default route is known, and
/// binding a source address does not force the interface there.
#[cfg(not(windows))]
pub fn adapters() -> Vec<Adapter> {
    let Ok(ip) = netscan::net::local_ip() else { return Vec::new() };
    vec![Adapter {
        alias: "default route".into(),
        description: String::new(),
        if_type: 6,
        up: true,
        metric: None,
        addrs: vec![ip],
        gateways: netscan::net::default_gateway().into_iter().collect(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(alias: &str, desc: &str, if_type: u32, ip: &str, gw: Option<&str>) -> Adapter {
        Adapter {
            alias: alias.into(),
            description: desc.into(),
            if_type,
            up: true,
            metric: Some(25),
            addrs: vec![ip.parse().unwrap()],
            gateways: gw.map(|g| g.parse().unwrap()).into_iter().collect(),
        }
    }

    #[test]
    fn names_router_phone_and_skips_local_only() {
        let list = vec![
            ad("Wi-Fi", "Intel Wireless-AC", 71, "192.168.0.20", Some("192.168.0.1")),
            ad("Ethernet 2", "Remote NDIS based Internet Sharing Device", 6, "10.0.40.5", Some("10.0.40.1")),
            ad("Ethernet", "Realtek PCIe GbE", 6, "192.168.30.1", None),
            ad("Loopback", "", 24, "127.0.0.1", None),
        ];
        let links = classify(&list, Some("192.168.0.1".parse().unwrap()), &[], Some("10.0.40.5".parse().unwrap()));
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].name, "Router (Wi-Fi)");
        assert_eq!(links[0].kind, LinkKind::Router);
        assert!(!links[0].is_default);
        assert_eq!(links[1].name, "Phone (USB)");
        assert_eq!(links[1].medium, "USB");
        assert!(links[1].is_default);
    }

    #[test]
    fn hotspot_detected_from_phone_client_list() {
        let list = vec![ad("Wi-Fi", "Intel Wireless-AC", 71, "10.0.55.80", Some("10.0.55.1"))];
        let links = classify(&list, Some("192.168.0.1".parse().unwrap()), &["10.0.55.80".parse().unwrap()], None);
        assert_eq!(links[0].name, "Phone (hotspot)");
        assert_eq!(links[0].kind, LinkKind::PhoneHotspot);
    }

    #[test]
    fn duplicate_names_get_adapter_suffix() {
        let list = vec![
            ad("Wi-Fi", "a", 71, "10.1.0.2", Some("10.1.0.1")),
            ad("Wi-Fi", "b", 71, "10.2.0.2", Some("10.2.0.1")),
        ];
        let mut a = list.clone();
        a[1].alias = "Wi-Fi 2".into();
        let links = classify(&a, None, &[], None);
        assert_eq!(links[0].name, "Wi-Fi");
        assert_eq!(links[1].name, "Wi-Fi 2");
        let mut b = list;
        b[0].alias = "Ethernet".into();
        b[0].description = "Remote NDIS".into();
        b[1].description = "Remote NDIS".into();
        let links = classify(&b, None, &[], None);
        assert_eq!(links[0].name, "Phone (USB) · Ethernet");
        assert_eq!(links[1].name, "Phone (USB) · Wi-Fi");
    }
}
