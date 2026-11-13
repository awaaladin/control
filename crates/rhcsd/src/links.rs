//! The machine's internet links: every local IPv4 address on an adapter that
//! is up and has a default gateway, named for people ("Jeffnou (router)",
//! "vivo Y (phone hotspot)", "Phone (USB)"). Read-only: switching the
//! default link lives in `switch.rs`.
//!
//! Windows uses the strong host model, so a socket bound to one link's local
//! address leaves through that link even when another link is the default.
//! That is what lets Game Boost measure each link separately without admin.

use serde::Serialize;
use std::net::Ipv4Addr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Gateway is the configured router (ROUTER_IP), or its Wi-Fi name matches.
    Router,
    /// USB tethering from the phone (RNDIS adapter).
    PhoneUsb,
    /// Wi-Fi joined to the phone's hotspot (the phone lists us as a client,
    /// or the Wi-Fi name is the phone's hotspot name).
    PhoneHotspot,
    Other,
}

/// One adapter as the OS reports it (input to [`classify`]).
#[derive(Clone, Debug, Default)]
pub struct Adapter {
    /// OS interface index (IPv4); used to change the metric when switching.
    pub if_index: u32,
    /// Wi-Fi network name, when the adapter is joined to one.
    pub ssid: Option<String>,
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
    /// Friendly name, e.g. "Jeffnou (router)", "Phone (USB)".
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
    pub if_index: u32,
    /// Wi-Fi network name (SSID), if this link is Wi-Fi.
    pub ssid: Option<String>,
}

/// What rhcs knows that helps name links.
#[derive(Clone, Debug, Default)]
pub struct Hints {
    /// The configured router (ROUTER_IP); a link whose gateway is it is "router".
    pub router_ip: Option<Ipv4Addr>,
    /// Wi-Fi names of the routers (ROUTER_WIFI_SSID, ROUTER2_WIFI_SSID).
    pub router_ssids: Vec<String>,
    /// Client addresses the phone reports on its hotspot.
    pub hotspot_ips: Vec<Ipv4Addr>,
    /// Wi-Fi names known to be the phone's hotspot (PHONE_HOTSPOT_SSID, or learned).
    pub hotspot_ssids: Vec<String>,
    /// The local address Windows uses for the internet right now.
    pub default_ip: Option<Ipv4Addr>,
}

fn is_usable(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast())
}

fn looks_like_usb_tether(a: &Adapter) -> bool {
    let d = a.description.to_ascii_lowercase();
    d.contains("ndis") || d.contains("android") || d.contains("usb tether") || d.contains("internet sharing")
}

/// Turns raw adapters into named internet links. Pure, so it is unit-tested.
pub fn classify(adapters: &[Adapter], h: &Hints) -> Vec<Link> {
    let listed = |list: &[String], ssid: Option<&str>| ssid.is_some_and(|s| list.iter().any(|x| x.trim() == s));
    let mut out = Vec::new();
    for a in adapters {
        if !a.up || a.if_type == 24 || a.if_type == 131 {
            continue;
        }
        let gateway = a.gateways.iter().copied().find(|g| !g.is_unspecified());
        if gateway.is_none() {
            continue; // no route out: a local-only network
        }
        let ssid = a.ssid.as_deref().map(str::trim).filter(|s| !s.is_empty());
        for &ip in a.addrs.iter().filter(|ip| is_usable(**ip)) {
            let kind = if (h.router_ip.is_some() && gateway == h.router_ip) || listed(&h.router_ssids, ssid) {
                LinkKind::Router
            } else if looks_like_usb_tether(a) {
                LinkKind::PhoneUsb
            } else if h.hotspot_ips.contains(&ip) || listed(&h.hotspot_ssids, ssid) {
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
            let name = match (kind, ssid) {
                (LinkKind::Router, Some(n)) => format!("{n} (router)"),
                (LinkKind::Router, None) => format!("Router ({medium})"),
                (LinkKind::PhoneUsb, _) => "Phone (USB)".to_string(),
                (LinkKind::PhoneHotspot, Some(n)) => format!("{n} (phone hotspot)"),
                (LinkKind::PhoneHotspot, None) => "Phone (hotspot)".to_string(),
                (LinkKind::Other, Some(n)) => format!("{n} (Wi-Fi)"),
                (LinkKind::Other, None) => a.alias.clone(),
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
                is_default: h.default_ip == Some(ip),
                if_index: a.if_index,
                ssid: ssid.map(str::to_string),
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
                if_index: a.Anonymous1.Anonymous.IfIndex,
                ssid: None,
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
        if_index: 0,
        ssid: None,
        alias: "default route".into(),
        description: String::new(),
        if_type: 6,
        up: true,
        metric: None,
        addrs: vec![ip],
        gateways: netscan::net::default_gateway().into_iter().collect(),
    }]
}

// ------------------------------------------------------------------ Wi-Fi names

/// One wireless interface from `netsh wlan show interfaces`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WlanIf {
    /// Interface alias, e.g. "WiFi" (matches [`Adapter::alias`]).
    pub name: String,
    pub ssid: Option<String>,
    pub state: Option<String>,
    /// Windows' signal quality, 0-100 %.
    pub signal_pct: Option<u8>,
}

/// Parses `netsh wlan show interfaces` (English Windows). Keys are matched
/// exactly, so "BSSID" never counts as the SSID.
pub fn parse_netsh_wlan(text: &str) -> Vec<WlanIf> {
    let mut out: Vec<WlanIf> = Vec::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if k == "Name" {
            out.push(WlanIf { name: v.to_string(), ..Default::default() });
            continue;
        }
        let Some(w) = out.last_mut() else { continue };
        match k {
            "SSID" => w.ssid = Some(v.to_string()).filter(|s| !s.is_empty()),
            "State" => w.state = Some(v.to_string()),
            "Signal" => w.signal_pct = v.trim_end_matches('%').trim().parse().ok(),
            _ => {}
        }
    }
    for w in out.iter_mut() {
        if w.state.as_deref().is_some_and(|s| !s.eq_ignore_ascii_case("connected")) {
            w.ssid = None;
        }
    }
    out
}

/// Wireless interfaces right now (Windows only; empty elsewhere or on error).
pub async fn wlan_interfaces() -> Vec<WlanIf> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["wlan", "show", "interfaces"]).kill_on_drop(true);
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        match tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output()).await {
            Ok(Ok(out)) if out.status.success() => parse_netsh_wlan(&String::from_utf8_lossy(&out.stdout)),
            _ => Vec::new(),
        }
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Every internet link with its name, from what the daemon knows: router
/// address and Wi-Fi names from .env, the phone's hotspot clients, SSIDs.
pub async fn discover(s: &crate::state::Shared) -> Vec<Link> {
    let (hotspot_ips, phone_online) = {
        let i = s.inner.read().await;
        let ips: Vec<Ipv4Addr> = i
            .phone
            .as_ref()
            .map(|p| p.hotspot_clients.iter().filter_map(|c| c.ip.parse().ok()).collect())
            .unwrap_or_default();
        (ips, i.sources.phone.online)
    };
    let mut adapters = tokio::task::spawn_blocking(adapters).await.unwrap_or_default();
    let wlan = wlan_interfaces().await;
    for a in adapters.iter_mut() {
        if let Some(w) = wlan.iter().find(|w| w.name == a.alias) {
            a.ssid = w.ssid.clone();
        }
    }
    let mut hotspot_ssids = s.cfg.phone_hotspot_ssids.clone();
    hotspot_ssids.extend(s.learned_hotspot_ssids.read().await.iter().cloned());
    let hints = Hints {
        router_ip: s.cfg.router_ip.parse().ok(),
        router_ssids: s.cfg.router_ssids.clone(),
        hotspot_ips: hotspot_ips.clone(),
        hotspot_ssids,
        default_ip: netscan::net::local_ip().ok(),
    };
    let links = classify(&adapters, &hints);
    // Learn the phone's hotspot name while the phone lists this machine as a
    // client, so the link keeps its name when the phone is not reachable.
    if phone_online {
        let mut learned = s.learned_hotspot_ssids.write().await;
        for l in &links {
            if let (true, Some(ssid)) = (hotspot_ips.contains(&l.local_ip), &l.ssid) {
                if !learned.contains(ssid) {
                    learned.push(ssid.clone());
                }
            }
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(alias: &str, desc: &str, if_type: u32, ip: &str, gw: Option<&str>) -> Adapter {
        Adapter {
            if_index: 7,
            ssid: None,
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
        let h = Hints { router_ip: Some("192.168.0.1".parse().unwrap()), default_ip: Some("10.0.40.5".parse().unwrap()), ..Default::default() };
        let links = classify(&list, &h);
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
        let h = Hints {
            router_ip: Some("192.168.0.1".parse().unwrap()),
            hotspot_ips: vec!["10.0.55.80".parse().unwrap()],
            ..Default::default()
        };
        let links = classify(&list, &h);
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
        let links = classify(&a, &Hints::default());
        assert_eq!(links[0].name, "Wi-Fi");
        assert_eq!(links[1].name, "Wi-Fi 2");
        let mut b = list;
        b[0].alias = "Ethernet".into();
        b[0].description = "Remote NDIS".into();
        b[1].description = "Remote NDIS".into();
        let links = classify(&b, &Hints::default());
        assert_eq!(links[0].name, "Phone (USB) · Ethernet");
        assert_eq!(links[1].name, "Phone (USB) · Wi-Fi");
    }

    #[test]
    fn connections_named_by_wifi_name() {
        // Made-up network names; the shapes match what netsh reports.
        let mut wifi = ad("WiFi", "Intel Wireless-AC", 71, "192.168.0.20", Some("192.168.0.1"));
        wifi.ssid = Some("HomeNet".into());
        let mut hs = ad("WiFi 2", "USB Wi-Fi", 71, "10.0.55.80", Some("10.0.55.1"));
        hs.ssid = Some("Pocket Phone".into());
        let mut cafe = ad("WiFi 3", "USB Wi-Fi", 71, "172.16.4.9", Some("172.16.4.1"));
        cafe.ssid = Some("Cafe".into());
        let usb = ad("Ethernet 2", "Remote NDIS based Internet Sharing Device", 6, "10.0.40.5", Some("10.0.40.1"));
        let h = Hints {
            router_ip: Some("192.168.0.1".parse().unwrap()),
            hotspot_ssids: vec!["Pocket Phone".into()],
            ..Default::default()
        };
        let names: Vec<String> = classify(&[wifi.clone(), hs, cafe, usb], &h).into_iter().map(|l| l.name).collect();
        assert_eq!(names, vec!["HomeNet (router)", "Pocket Phone (phone hotspot)", "Cafe (Wi-Fi)", "Phone (USB)"]);

        // The router is recognised by its Wi-Fi name even behind another gateway address.
        let mut other_gw = wifi;
        other_gw.gateways = vec!["192.168.8.1".parse().unwrap()];
        let h = Hints { router_ssids: vec!["HomeNet".into()], ..Default::default() };
        let l = classify(&[other_gw], &h);
        assert_eq!((l[0].kind, l[0].name.as_str()), (LinkKind::Router, "HomeNet (router)"));
        assert_eq!(l[0].ssid.as_deref(), Some("HomeNet"));
    }

    #[test]
    fn parses_netsh_wlan_interfaces() {
        let text = "\r\nThere are 2 interfaces on the system: \r\n\r\n    Name                   : WiFi\r\n    Description            : Intel(R) Dual Band Wireless-AC 7260\r\n    GUID                   : 00000000-0000-0000-0000-000000000000\r\n    Physical address       : 02:00:00:00:00:01\r\n    State                  : connected\r\n    SSID                   : Pocket Phone\r\n    BSSID                  : 02:00:00:00:00:02\r\n    Network type           : Infrastructure\r\n    Signal                 : 99% \r\n    Profile                : Pocket Phone \r\n\r\n    Name                   : WiFi 2\r\n    Description            : USB adapter\r\n    State                  : disconnected\r\n\r\n    Hosted network status  : Not available\r\n";
        let w = parse_netsh_wlan(text);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].name, "WiFi");
        assert_eq!(w[0].ssid.as_deref(), Some("Pocket Phone"));
        assert_eq!(w[0].signal_pct, Some(99));
        assert_eq!(w[1].name, "WiFi 2");
        assert_eq!(w[1].ssid, None);
        assert!(parse_netsh_wlan("The Wireless AutoConfig Service (wlansvc) is not running.").is_empty());
    }
}
