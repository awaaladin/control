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

/// Folder that holds the project's .env (used for logs/), else the current dir.
pub fn project_dir() -> PathBuf {
    find()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
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
        if map.contains_key(&k) || k.starts_with("ROUTER") {
            map.insert(k, v);
        }
    }
    map
}
