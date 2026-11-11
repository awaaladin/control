//! TCP service probing and banner grabbing.

use crate::http;
use rhcs_core::OpenPort;
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::task::JoinSet;

/// Ports that say something interesting about a home device.
pub const COMMON: &[(u16, &str)] = &[
    (21, "ftp"),
    (22, "ssh"),
    (23, "telnet"),
    (53, "dns"),
    (80, "http"),
    (139, "netbios"),
    (443, "https"),
    (445, "smb"),
    (554, "rtsp (camera/media)"),
    (1883, "mqtt (IoT)"),
    (5000, "upnp/http"),
    (5555, "adb (Android TV/box)"),
    (7000, "airplay"),
    (8008, "chromecast http"),
    (8009, "chromecast"),
    (8080, "http-alt"),
    (8443, "https-alt"),
    (8888, "http-alt"),
    (9100, "printer"),
    (49152, "upnp"),
    (62078, "iphone-sync"),
];

/// Extra ports that only make sense on a router/gateway.
pub const ROUTER_EXTRA: &[(u16, &str)] = &[
    (4567, "tr-069 alt"),
    (7547, "tr-069 (ISP remote mgmt)"),
    (8291, "winbox (MikroTik)"),
    (10000, "webmin"),
    (20005, "netusb"),
    (37215, "huawei hg mgmt"),
    (52869, "upnp-alt"),
];

const HTTP_PORTS: &[u16] = &[80, 4567, 5000, 7547, 8008, 8080, 8888, 10000, 37215, 49152, 52869];
const LINE_PORTS: &[u16] = &[21, 22, 23];

pub fn router_ports() -> Vec<(u16, &'static str)> {
    COMMON.iter().chain(ROUTER_EXTRA).copied().collect()
}

pub async fn probe(ip: Ipv4Addr, ports: &[(u16, &'static str)], timeout: Duration) -> Vec<OpenPort> {
    let mut set = JoinSet::new();
    for &(port, label) in ports {
        set.spawn(async move {
            let stream = tokio::time::timeout(timeout, TcpStream::connect((ip, port))).await.ok()?.ok()?;
            let banner = banner(ip, port, stream).await;
            Some(OpenPort { port, label, banner })
        });
    }
    let mut open = Vec::new();
    while let Some(res) = set.join_next().await {
        if let Ok(Some(p)) = res {
            open.push(p);
        }
    }
    open.sort_by_key(|p| p.port);
    open
}

async fn banner(ip: Ipv4Addr, port: u16, mut stream: TcpStream) -> Option<String> {
    if LINE_PORTS.contains(&port) {
        // SSH/FTP/Telnet servers speak first.
        let mut buf = [0u8; 256];
        let n = tokio::time::timeout(Duration::from_millis(1500), stream.read(&mut buf))
            .await
            .ok()?
            .ok()?;
        let text: String = String::from_utf8_lossy(&buf[..n])
            .chars()
            .map(|c| if c.is_ascii_graphic() || c == ' ' || c == '\n' { c } else { ' ' })
            .collect();
        let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
        return Some(http::truncate(line, 80));
    }
    if HTTP_PORTS.contains(&port) {
        drop(stream);
        let resp = http::get(&ip.to_string(), port, "/", Duration::from_secs(3)).await.ok()?;
        return Some(resp.summary());
    }
    None
}
