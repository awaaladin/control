//! Host discovery: who is on the LAN right now.
//!
//! Windows: an ARP request per address (SendARP). ARP works below IP, so it
//! finds devices that ignore ping and have every port closed.
//! Linux (e.g. running on an OpenWrt router): TCP touch on a few ports (a
//! "connection refused" still proves the host is alive), then MACs are read
//! from the kernel's ARP cache.

use rhcs_core::MacAddr;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[derive(Debug, Clone)]
pub struct Seen {
    pub ip: Ipv4Addr,
    pub mac: Option<MacAddr>,
    pub rtt_ms: u64,
}

pub async fn sweep(hosts: &[Ipv4Addr], concurrency: usize) -> Vec<Seen> {
    let sem = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut set = JoinSet::new();
    for &ip in hosts {
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.ok()?;
            probe_one(ip).await
        });
    }
    let mut out = Vec::new();
    while let Some(res) = set.join_next().await {
        if let Ok(Some(seen)) = res {
            out.push(seen);
        }
    }
    #[cfg(target_os = "linux")]
    for s in out.iter_mut() {
        s.mac = s.mac.or_else(|| proc_arp(s.ip));
    }
    out.sort_by_key(|s| u32::from(s.ip));
    out
}

/// Resolves one address to a MAC, if it answers.
pub async fn resolve_mac(ip: Ipv4Addr) -> Option<MacAddr> {
    probe_one(ip).await.and_then(|s| s.mac)
}

#[cfg(windows)]
async fn probe_one(ip: Ipv4Addr) -> Option<Seen> {
    let start = Instant::now();
    let mac = tokio::task::spawn_blocking(move || crate::win::arp_resolve(ip)).await.ok()??;
    Some(Seen { ip, mac: Some(MacAddr(mac)), rtt_ms: start.elapsed().as_millis() as u64 })
}

#[cfg(not(windows))]
async fn probe_one(ip: Ipv4Addr) -> Option<Seen> {
    use std::time::Duration;
    const PORTS: [u16; 6] = [80, 443, 22, 53, 445, 62078];
    let start = Instant::now();
    let mut set = JoinSet::new();
    for port in PORTS {
        set.spawn(async move {
            let conn = tokio::net::TcpStream::connect((ip, port));
            match tokio::time::timeout(Duration::from_millis(600), conn).await {
                Ok(Ok(_)) => true,
                Ok(Err(e)) => e.kind() == std::io::ErrorKind::ConnectionRefused,
                Err(_) => false,
            }
        });
    }
    while let Some(alive) = set.join_next().await {
        if alive.unwrap_or(false) {
            let mac = proc_arp_now(ip);
            return Some(Seen { ip, mac, rtt_ms: start.elapsed().as_millis() as u64 });
        }
    }
    None
}

#[cfg(all(not(windows), target_os = "linux"))]
fn proc_arp_now(ip: Ipv4Addr) -> Option<MacAddr> {
    proc_arp(ip)
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn proc_arp_now(_ip: Ipv4Addr) -> Option<MacAddr> {
    None
}

#[cfg(target_os = "linux")]
fn proc_arp(ip: Ipv4Addr) -> Option<MacAddr> {
    let table = std::fs::read_to_string("/proc/net/arp").ok()?;
    table.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.first()?.parse::<Ipv4Addr>().ok()? != ip {
            return None;
        }
        MacAddr::parse(f.get(3)?).filter(|m| m.0 != [0; 6])
    })
}
