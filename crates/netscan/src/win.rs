//! Thin wrappers over the Windows IP Helper API (iphlpapi.dll).
//! IPv4 addresses are passed as u32 in network byte order, which in memory
//! is just the four octets in order, so `from_ne_bytes(octets)`.

use std::net::Ipv4Addr;
use windows_sys::Win32::NetworkManagement::IpHelper::{GetBestRoute, SendARP, MIB_IPFORWARDROW};

fn to_raw(ip: Ipv4Addr) -> u32 {
    u32::from_ne_bytes(ip.octets())
}

fn from_raw(raw: u32) -> Ipv4Addr {
    Ipv4Addr::from(raw.to_ne_bytes())
}

/// Asks the routing table how traffic to `dest` would leave this machine.
/// Returns (next hop, route mask).
pub fn route_to(dest: Ipv4Addr) -> Option<(Ipv4Addr, Ipv4Addr)> {
    unsafe {
        let mut row: MIB_IPFORWARDROW = std::mem::zeroed();
        if GetBestRoute(to_raw(dest), 0, &mut row) != 0 {
            return None;
        }
        Some((from_raw(row.dwForwardNextHop), from_raw(row.dwForwardMask)))
    }
}

/// Sends an ARP request and blocks until the reply arrives or Windows gives
/// up (about 1-3 s). Works without admin rights, unlike raw ICMP ping.
pub fn arp_resolve(ip: Ipv4Addr) -> Option<[u8; 6]> {
    let mut buf = [0u8; 8];
    let mut len: u32 = buf.len() as u32;
    let rc = unsafe { SendARP(to_raw(ip), 0, buf.as_mut_ptr().cast(), &mut len) };
    if rc != 0 || len < 6 {
        return None;
    }
    let mac = [buf[0], buf[1], buf[2], buf[3], buf[4], buf[5]];
    (mac != [0; 6]).then_some(mac)
}
