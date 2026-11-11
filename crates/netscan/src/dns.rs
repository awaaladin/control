//! Hostname discovery with hand-built DNS packets.
//!
//! 1. Ask the router's DNS server (port 53) for the PTR record: home routers
//!    running dnsmasq answer with the name each device gave in its DHCP request.
//! 2. Ask the device itself over mDNS (port 5353): phones, Macs, Linux boxes,
//!    printers and Chromecasts often answer with their `.local` name.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

pub async fn reverse_lookup(ip: Ipv4Addr, gateway: Option<Ipv4Addr>) -> Option<String> {
    if let Some(gw) = gateway {
        if let Some(name) = ptr_query(ip, SocketAddr::from((gw, 53)), true).await {
            return Some(name);
        }
    }
    ptr_query(ip, SocketAddr::from((ip, 5353)), false).await
}

async fn ptr_query(ip: Ipv4Addr, server: SocketAddr, recursion: bool) -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.subsec_nanos();
    let id = (nanos as u16) ^ u16::from(ip.octets()[3]);
    sock.send_to(&build_ptr_query(id, ip, recursion), server).await.ok()?;
    let mut buf = [0u8; 1500];
    let (n, _) = tokio::time::timeout(Duration::from_millis(800), sock.recv_from(&mut buf))
        .await
        .ok()?
        .ok()?;
    parse_ptr_answer(&buf[..n], id)
}

pub fn build_ptr_query(id: u16, ip: Ipv4Addr, recursion: bool) -> Vec<u8> {
    let mut p = Vec::with_capacity(64);
    p.extend_from_slice(&id.to_be_bytes());
    p.extend_from_slice(&(if recursion { 0x0100u16 } else { 0 }).to_be_bytes());
    p.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // 1 question
    let o = ip.octets();
    for label in [o[3].to_string(), o[2].to_string(), o[1].to_string(), o[0].to_string()] {
        p.push(label.len() as u8);
        p.extend_from_slice(label.as_bytes());
    }
    for label in ["in-addr", "arpa"] {
        p.push(label.len() as u8);
        p.extend_from_slice(label.as_bytes());
    }
    p.push(0);
    p.extend_from_slice(&[0, 12, 0, 1]); // QTYPE=PTR, QCLASS=IN
    p
}

fn u16_at(buf: &[u8], pos: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*buf.get(pos)?, *buf.get(pos + 1)?]))
}

/// Reads a possibly-compressed DNS name. Returns the name and the offset just
/// past it in the original (non-jumped) byte stream.
fn read_name(buf: &[u8], mut pos: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    let mut hops = 0;
    loop {
        let len = *buf.get(pos)? as usize;
        if len & 0xC0 == 0xC0 {
            let ptr = ((len & 0x3F) << 8) | *buf.get(pos + 1)? as usize;
            end.get_or_insert(pos + 2);
            pos = ptr;
            hops += 1;
            if hops > 16 {
                return None;
            }
            continue;
        }
        if len == 0 {
            end.get_or_insert(pos + 1);
            break;
        }
        labels.push(String::from_utf8_lossy(buf.get(pos + 1..pos + 1 + len)?).into_owned());
        pos += 1 + len;
    }
    Some((labels.join("."), end?))
}

pub fn parse_ptr_answer(buf: &[u8], id: u16) -> Option<String> {
    let rid = u16_at(buf, 0)?;
    if rid != id && rid != 0 {
        return None;
    }
    if buf.get(3)? & 0x0F != 0 {
        return None; // NXDOMAIN or other error
    }
    let qd = u16_at(buf, 4)?;
    let an = u16_at(buf, 6)?;
    let mut pos = 12;
    for _ in 0..qd {
        pos = read_name(buf, pos)?.1 + 4;
    }
    for _ in 0..an {
        pos = read_name(buf, pos)?.1;
        let rtype = u16_at(buf, pos)?;
        let rdlen = u16_at(buf, pos + 8)? as usize;
        let rdata = pos + 10;
        if rtype == 12 {
            let (name, _) = read_name(buf, rdata)?;
            let name = name.trim_end_matches('.').trim_end_matches(".local").to_string();
            if !name.is_empty() && !name.ends_with("in-addr.arpa") {
                return Some(name);
            }
        }
        pos = rdata + rdlen;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ptr() {
        let ip = Ipv4Addr::new(192, 168, 1, 20);
        let q = build_ptr_query(0xBEEF, ip, true);
        // Build a response: header + echoed question + answer using a pointer to the question name.
        let mut r = q.clone();
        r[2] = 0x81;
        r[3] = 0x80;
        r[7] = 1; // ANCOUNT = 1
        r.extend_from_slice(&[0xC0, 12, 0, 12, 0, 1, 0, 0, 0, 60]);
        let target = b"\x06laptop\x03lan\x00";
        r.extend_from_slice(&(target.len() as u16).to_be_bytes());
        r.extend_from_slice(target);
        assert_eq!(parse_ptr_answer(&r, 0xBEEF).as_deref(), Some("laptop.lan"));
        assert_eq!(parse_ptr_answer(&r, 0x1234), None);
    }
}
