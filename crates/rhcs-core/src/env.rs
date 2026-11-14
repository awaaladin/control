//! Minimal `.env` loader (KEY=VALUE lines, # comments). Real environment
//! variables win over the file. Looks in the current directory, then next
//! to/above the executable, so `target\release\rhcs.exe` finds the project's .env.

use std::collections::HashMap;
use std::path::PathBuf;

fn find() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(".env")];
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(5) {
            candidates.push(dir.join(".env"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Folder that holds the project's .env (used for logs/ and web/), else the current dir.
pub fn project_dir() -> PathBuf {
    find()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Folder for the files rhcsd keeps (towers.json, places.json, switch.json):
/// `RHCSD_DATA_DIR` if set (the Android app points it at its private storage),
/// else `data/` in the project folder.
pub fn data_dir() -> PathBuf {
    data_dir_from(&load())
}

pub fn data_dir_from(env: &HashMap<String, String>) -> PathBuf {
    match env.get("RHCSD_DATA_DIR").map(|v| v.trim()).filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => project_dir().join("data"),
    }
}

/// True for `1`, `on`, `true`, `yes` (any case).
pub fn truthy(v: Option<&str>) -> bool {
    matches!(v.map(|v| v.trim().to_ascii_lowercase()).as_deref(), Some("1" | "on" | "true" | "yes"))
}

pub fn load() -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(text) = find().and_then(|p| std::fs::read_to_string(p).ok()) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
            }
        }
    }
    for (k, v) in std::env::vars() {
        if map.contains_key(&k) || is_rhcs_key(&k) {
            map.insert(k, v);
        }
    }
    map
}

/// Settings rhcs reads, so a real environment variable can set them even
/// when `.env` doesn't mention them.
fn is_rhcs_key(k: &str) -> bool {
    const PREFIXES: &[&str] = &["ROUTER", "RHCSD", "PHONE_", "TOWER_", "HOME_", "OPENCELLID_", "BOOST_", "SCAN_", "ADB_"];
    PREFIXES.iter().any(|p| k.starts_with(p))
}

#[cfg(test)]
mod tests {
    #[test]
    fn rhcs_keys_from_environment() {
        assert!(super::is_rhcs_key("TOWER_LOOKUP"));
        assert!(super::is_rhcs_key("PHONE_HOTSPOT_SSID"));
        assert!(!super::is_rhcs_key("PATH"));
        // the Android app sets these through the environment only
        for k in ["RHCSD_APP", "RHCSD_WEB_DIR", "RHCSD_DATA_DIR", "RHCSD_TOKEN", "RHCSD_BIND", "ROUTER_IP", "TOWER_LOOKUP"] {
            assert!(super::is_rhcs_key(k), "{k}");
        }
    }

    #[test]
    fn data_dir_override_and_truthy() {
        let mut m = std::collections::HashMap::new();
        m.insert("RHCSD_DATA_DIR".to_string(), " /data/user/0/app/files/data ".to_string());
        assert_eq!(super::data_dir_from(&m), std::path::PathBuf::from("/data/user/0/app/files/data"));
        m.insert("RHCSD_DATA_DIR".to_string(), "  ".to_string());
        assert!(super::data_dir_from(&m).ends_with("data"));
        assert!(super::data_dir_from(&std::collections::HashMap::new()).ends_with("data"));
        assert!(super::truthy(Some("1")) && super::truthy(Some(" On ")) && super::truthy(Some("TRUE")));
        assert!(!super::truthy(Some("0")) && !super::truthy(Some("")) && !super::truthy(None));
    }
}
