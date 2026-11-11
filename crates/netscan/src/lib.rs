//! Network reconnaissance for your own LAN.
//!
//! Everything here is read-only: ARP lookups, TCP connects, UPnP discovery,
//! reverse-DNS queries and plain HTTP GETs. Nothing logs in, changes
//! configuration, or tries to get around authentication.

pub mod discover;
pub mod dns;
pub mod http;
pub mod net;
pub mod oui;
pub mod ports;
pub mod router;
pub mod scan;
pub mod ssdp;

#[cfg(windows)]
mod win;
