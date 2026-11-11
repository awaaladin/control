//! Shared types for the Rust Home Hardware Control System (RHCS).
//!
//! Every device module (router, decoder, amplifier) speaks in terms of these
//! types so the API and UI layers never need to know how a fact was obtained.

use serde::{Serialize, Serializer};
use std::fmt;
use std::net::Ipv4Addr;

/// A 48-bit hardware (MAC) address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    /// Bit 1 of the first octet is set when the address was *not* assigned by
    /// a manufacturer. Phones use these "private"/randomized MACs per network,
    /// so no vendor can be inferred from them.
    pub fn is_locally_administered(&self) -> bool {
        self.0[0] & 0b10 != 0
    }

    pub fn is_multicast(&self) -> bool {
        self.0[0] & 1 != 0
    }

    /// The manufacturer prefix (Organizationally Unique Identifier).
    pub fn oui(&self) -> [u8; 3] {
        [self.0[0], self.0[1], self.0[2]]
    }

    /// Parses `aa:bb:cc:dd:ee:ff` or `aa-bb-cc-dd-ee-ff`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut out = [0u8; 6];
        let mut parts = s.trim().split(|c| c == ':' || c == '-');
        for byte in out.iter_mut() {
            *byte = u8::from_str_radix(parts.next()?, 16).ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        Some(Self(out))
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }
}

impl Serialize for MacAddr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

/// A TCP port that accepted a connection, plus whatever it said about itself.
#[derive(Debug, Clone, Serialize)]
pub struct OpenPort {
    pub port: u16,
    pub label: &'static str,
    pub banner: Option<String>,
}

/// What a device announced about itself over UPnP/SSDP.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct UpnpInfo {
    pub location: String,
    pub server: Option<String>,
    pub friendly_name: Option<String>,
    pub manufacturer: Option<String>,
    pub model_name: Option<String>,
    pub model_number: Option<String>,
    pub model_description: Option<String>,
    pub device_type: Option<String>,
}

impl UpnpInfo {
    /// "Manufacturer Model Number", skipping missing parts.
    pub fn identity(&self) -> Option<String> {
        let parts: Vec<&str> = [&self.manufacturer, &self.model_name, &self.model_number]
            .into_iter()
            .filter_map(|p| p.as_deref())
            .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

/// One host seen on the network.
#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub ip: Ipv4Addr,
    pub mac: Option<MacAddr>,
    pub vendor: Option<String>,
    pub hostname: Option<String>,
    pub rtt_ms: Option<u64>,
    pub is_gateway: bool,
    pub is_self: bool,
    pub ports: Vec<OpenPort>,
    pub upnp: Vec<UpnpInfo>,
}

impl Device {
    pub fn new(ip: Ipv4Addr) -> Self {
        Self {
            ip,
            mac: None,
            vendor: None,
            hostname: None,
            rtt_ms: None,
            is_gateway: false,
            is_self: false,
            ports: Vec::new(),
            upnp: Vec::new(),
        }
    }

    pub fn has_port(&self, port: u16) -> bool {
        self.ports.iter().any(|p| p.port == port)
    }
}

/// How sure we are about a claim. The project rule: never present a theory
/// as a tested fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// We exercised it end to end on the real hardware.
    Tested,
    /// We measured it directly on the wire just now.
    Observed,
    /// The manufacturer or project documentation says so.
    Documented,
    /// A reasoned conclusion from observations, not directly measured.
    Inferred,
    /// Possible in principle; not yet checked.
    Theoretical,
    /// Checked and not available.
    Unsupported,
}

impl Evidence {
    pub fn tag(self) -> &'static str {
        match self {
            Evidence::Tested => "TESTED",
            Evidence::Observed => "OBSERVED",
            Evidence::Documented => "DOCUMENTED",
            Evidence::Inferred => "INFERRED",
            Evidence::Theoretical => "THEORY",
            Evidence::Unsupported => "UNSUPPORTED",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub evidence: Evidence,
    pub text: String,
}

impl Finding {
    pub fn new(evidence: Evidence, text: impl Into<String>) -> Self {
        Self { evidence, text: text.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_parse_and_flags() {
        let m = MacAddr::parse("3A-7D-67-D2-9D-3B").unwrap();
        assert_eq!(m.to_string(), "3a:7d:67:d2:9d:3b");
        assert!(m.is_locally_administered());
        assert!(!MacAddr::parse("00:1a:2b:3c:4d:5e").unwrap().is_locally_administered());
        assert!(MacAddr::parse("00:11:22").is_none());
        assert!(MacAddr::parse("00:11:22:33:44:55:66").is_none());
    }
}
