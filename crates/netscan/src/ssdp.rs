//! UPnP / SSDP discovery.
//!
//! Routers, smart TVs, Android TV boxes, media players and many AV receivers
//! answer a multicast "M-SEARCH" with a URL to an XML file describing
//! themselves: manufacturer, model name, model number. That is often the
//! fastest way to identify exact hardware without touching a login page.

use crate::http;
use anyhow::Result;
use rhcs_core::UpnpInfo;
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::task::JoinSet;
use tokio::time::Instant;

const SSDP_ADDR: (Ipv4Addr, u16) = (Ipv4Addr::new(239, 255, 255, 250), 1900);

#[derive(Debug, Clone)]
pub struct SsdpHit {
    pub ip: Ipv4Addr,
    pub info: UpnpInfo,
}

pub async fn discover(bind_ip: Ipv4Addr, wait: Duration) -> Result<Vec<SsdpHit>> {
    let sock = UdpSocket::bind((bind_ip, 0)).await?;
    sock.set_multicast_ttl_v4(2)?;
    for st in ["ssdp:all", "upnp:rootdevice"] {
        let msg = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {st}\r\nUSER-AGENT: rhcs/0.1 UPnP/1.1\r\n\r\n"
        );
        sock.send_to(msg.as_bytes(), SSDP_ADDR).await?;
        tokio::time::sleep(Duration::from_millis(120)).await;
    }

    // location -> (ip, SERVER header)
    let mut found: BTreeMap<String, (Ipv4Addr, Option<String>)> = BTreeMap::new();
    let deadline = Instant::now() + wait;
    let mut buf = vec![0u8; 4096];
    let mut errors = 0;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, sock.recv_from(&mut buf)).await {
            Err(_) => break,
            Ok(Err(_)) => {
                errors += 1;
                if errors > 20 {
                    break;
                }
            }
            Ok(Ok((n, SocketAddr::V4(src)))) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let mut location = None;
                let mut server = None;
                for line in text.lines().skip(1) {
                    if let Some((k, v)) = line.split_once(':') {
                        match k.trim().to_ascii_lowercase().as_str() {
                            "location" => location = Some(v.trim().to_string()),
                            "server" => server = Some(v.trim().to_string()),
                            _ => {}
                        }
                    }
                }
                if let Some(loc) = location {
                    found.entry(loc).or_insert((*src.ip(), server));
                }
            }
            Ok(Ok(_)) => {}
        }
    }

    let mut set = JoinSet::new();
    for (location, (ip, server)) in found {
        set.spawn(async move {
            let mut info = UpnpInfo { location: location.clone(), server, ..Default::default() };
            if let Some((host, port, path)) = http::parse_url(&location) {
                if let Ok(resp) = http::get(&host, port, &path, Duration::from_secs(3)).await {
                    fill_from_xml(&mut info, &resp.body);
                }
            }
            SsdpHit { ip, info }
        });
    }
    let mut hits = Vec::new();
    while let Some(Ok(hit)) = set.join_next().await {
        hits.push(hit);
    }
    hits.sort_by_key(|h| (u32::from(h.ip), h.info.location.clone()));
    Ok(hits)
}

/// The root <device> comes first in a description document, so the first
/// occurrence of each tag describes the physical box.
pub fn fill_from_xml(info: &mut UpnpInfo, xml: &str) {
    info.friendly_name = xml_tag(xml, "friendlyName");
    info.manufacturer = xml_tag(xml, "manufacturer");
    info.model_name = xml_tag(xml, "modelName");
    info.model_number = xml_tag(xml, "modelNumber");
    info.model_description = xml_tag(xml, "modelDescription");
    info.device_type = xml_tag(xml, "deviceType");
}

fn xml_tag(doc: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = doc.find(&open)? + open.len();
    let end = start + doc[start..].find(&close)?;
    let value = http::collapse_ws(&unescape(&doc[start..end]));
    (!value.is_empty()).then_some(value)
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_root_device() {
        let xml = "<root><device><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>\
            <friendlyName>Home &amp; Co</friendlyName><manufacturer>ACME</manufacturer><modelName>R1</modelName>\
            <deviceList><device><modelName>inner</modelName></device></deviceList></device></root>";
        let mut info = UpnpInfo::default();
        fill_from_xml(&mut info, xml);
        assert_eq!(info.friendly_name.as_deref(), Some("Home & Co"));
        assert_eq!(info.identity().as_deref(), Some("ACME R1"));
    }
}
