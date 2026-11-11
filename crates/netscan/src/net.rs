//! Where am I on the network: local IP, default gateway, subnet.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subnet {
    pub network: Ipv4Addr,
    pub prefix: u8,
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    }
}

impl Subnet {
    pub fn new(ip: Ipv4Addr, prefix: u8) -> Self {
        let prefix = prefix.min(32);
        Self { network: Ipv4Addr::from(u32::from(ip) & mask(prefix)), prefix }
    }

    pub fn parse(s: &str) -> Result<Self> {
        let (ip, p) = s.split_once('/').context("expected CIDR like 192.168.1.0/24")?;
        let ip: Ipv4Addr = ip.trim().parse().context("bad IPv4 address")?;
        let p: u8 = p.trim().parse().context("bad prefix length")?;
        if p > 32 {
            bail!("prefix must be 0-32");
        }
        Ok(Self::new(ip, p))
    }

    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & mask(self.prefix) == u32::from(self.network)
    }

    pub fn len(&self) -> u64 {
        1u64 << (32 - self.prefix as u32)
    }

    /// Usable host addresses (network and broadcast excluded for /30 and wider).
    pub fn hosts(&self) -> Vec<Ipv4Addr> {
        let base = u32::from(self.network) as u64;
        let size = self.len();
        let range = if self.prefix >= 31 { 0..size } else { 1..size - 1 };
        range.map(|i| Ipv4Addr::from((base + i) as u32)).collect()
    }
}

impl fmt::Display for Subnet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

impl Serialize for Subnet {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NetInfo {
    pub local_ip: Ipv4Addr,
    pub gateway: Option<Ipv4Addr>,
    pub subnet: Subnet,
}

/// The local IP the OS would use to reach the internet. Connecting a UDP
/// socket sends no packets; it only makes the OS pick a route.
pub fn local_ip() -> Result<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect("1.1.1.1:53").context("no IPv4 route; are you connected to a network?")?;
    match sock.local_addr()?.ip() {
        IpAddr::V4(v4) => Ok(v4),
        IpAddr::V6(_) => bail!("only IPv4 is supported for now"),
    }
}

pub fn detect() -> Result<NetInfo> {
    let local_ip = local_ip()?;
    let gateway = default_gateway();
    let prefix = gateway
        .and_then(subnet_prefix_for)
        .filter(|p| *p < 32)
        .unwrap_or(24);
    Ok(NetInfo { local_ip, gateway, subnet: Subnet::new(local_ip, prefix) })
}

#[cfg(windows)]
pub fn default_gateway() -> Option<Ipv4Addr> {
    let (hop, _) = crate::win::route_to(Ipv4Addr::new(1, 1, 1, 1))?;
    (!hop.is_unspecified()).then_some(hop)
}

/// The on-link route that covers the gateway carries the subnet mask.
#[cfg(windows)]
fn subnet_prefix_for(ip: Ipv4Addr) -> Option<u8> {
    let (_, m) = crate::win::route_to(ip)?;
    Some(u32::from(m).count_ones() as u8)
}

#[cfg(target_os = "linux")]
fn proc_routes() -> Vec<(u32, u32, u32)> {
    // /proc/net/route stores addresses as little-endian hex.
    let ip = |h: &str| u32::from_str_radix(h, 16).ok().map(|v| u32::from_be_bytes(v.to_le_bytes()));
    std::fs::read_to_string("/proc/net/route")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((ip(f.get(1)?)?, ip(f.get(2)?)?, ip(f.get(7)?)?))
        })
        .collect()
}

#[cfg(target_os = "linux")]
pub fn default_gateway() -> Option<Ipv4Addr> {
    proc_routes().into_iter().find(|r| r.0 == 0 && r.1 != 0).map(|r| Ipv4Addr::from(r.1))
}

#[cfg(target_os = "linux")]
fn subnet_prefix_for(ip: Ipv4Addr) -> Option<u8> {
    let ip = u32::from(ip);
    proc_routes()
        .into_iter()
        .filter(|&(dest, gw, m)| gw == 0 && m != 0 && ip & m == dest)
        .map(|(_, _, m)| m.count_ones() as u8)
        .max()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn default_gateway() -> Option<Ipv4Addr> {
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
fn subnet_prefix_for(_ip: Ipv4Addr) -> Option<u8> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_math() {
        let s = Subnet::parse("10.234.55.83/24").unwrap();
        assert_eq!(s.to_string(), "10.234.55.0/24");
        assert_eq!(s.hosts().len(), 254);
        assert!(s.contains("10.234.55.62".parse().unwrap()));
        assert!(!s.contains("10.234.56.1".parse().unwrap()));
        assert_eq!(Subnet::parse("192.168.1.7/32").unwrap().hosts().len(), 1);
    }
}
