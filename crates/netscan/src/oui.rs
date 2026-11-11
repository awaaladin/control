//! MAC prefix → manufacturer, from the official IEEE registry.
//!
//! Download once with `scripts/get-oui.ps1` (about 4 MB, public data).
//! Without the file, vendors are simply not shown; nothing is guessed.

use rhcs_core::MacAddr;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct OuiDb {
    map: HashMap<u32, String>,
    pub source: Option<PathBuf>,
}

impl OuiDb {
    pub fn load() -> Self {
        for path in candidates() {
            if let Ok(bytes) = std::fs::read(&path) {
                let map = parse(&String::from_utf8_lossy(&bytes));
                if !map.is_empty() {
                    return Self { map, source: Some(path) };
                }
            }
        }
        Self { map: HashMap::new(), source: None }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// None for randomized/private MACs: they carry no vendor information.
    pub fn vendor(&self, mac: &MacAddr) -> Option<&str> {
        if mac.is_locally_administered() {
            return None;
        }
        let [a, b, c] = mac.oui();
        self.map.get(&u32::from_be_bytes([0, a, b, c])).map(String::as_str)
    }
}

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("RHCS_OUI") {
        out.push(PathBuf::from(p));
    }
    out.push(PathBuf::from("data/oui.csv"));
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(5) {
            out.push(dir.join("data").join("oui.csv"));
        }
    }
    out
}

/// IEEE format: `Registry,Assignment,Organization Name,Organization Address`.
fn parse(text: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    for line in text.lines().skip(1) {
        let mut f = line.splitn(3, ',');
        let (Some("MA-L"), Some(hex), Some(rest)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let Ok(key) = u32::from_str_radix(hex.trim(), 16) else { continue };
        let org = match rest.strip_prefix('"') {
            Some(q) => q.split('"').next().unwrap_or(""),
            None => rest.split(',').next().unwrap_or(""),
        };
        let org = org.trim();
        if !org.is_empty() {
            map.insert(key, org.to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ieee_rows() {
        let csv = "Registry,Assignment,Organization Name,Organization Address\n\
            MA-L,001A2B,\"Example, Inc.\",Somewhere\n\
            MA-L,ABCDEF,Plain Co,Addr\n\
            MA-M,123456,Skip Me,Addr\n";
        let db = OuiDb { map: parse(csv), source: None };
        assert_eq!(db.vendor(&MacAddr([0x00, 0x1a, 0x2b, 1, 2, 3])), Some("Example, Inc."));
        assert_eq!(db.len(), 2);
        // locally administered → no vendor even if prefix collides
        assert_eq!(db.vendor(&MacAddr([0xab, 0xcd, 0xef, 0, 0, 0])), None);
    }
}
