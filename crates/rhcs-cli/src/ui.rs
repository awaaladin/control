//! Terminal styling and small OS helpers.

use std::sync::atomic::{AtomicBool, Ordering};

static COLOR: AtomicBool = AtomicBool::new(true);

pub fn init() {
    let tty_ok = std::env::var_os("NO_COLOR").is_none() && enable_vt();
    COLOR.store(tty_ok, Ordering::Relaxed);
}

#[cfg(windows)]
fn enable_vt() -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
    };
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(h, &mut mode) == 0 {
            return false; // output is redirected
        }
        SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_vt() -> bool {
    true
}

pub fn color_enabled() -> bool {
    COLOR.load(Ordering::Relaxed)
}

fn paint(code: &str, s: &str) -> String {
    if COLOR.load(Ordering::Relaxed) {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    paint("1", s)
}
pub fn dim(s: &str) -> String {
    paint("2", s)
}
pub fn red(s: &str) -> String {
    paint("31", s)
}
pub fn green(s: &str) -> String {
    paint("32", s)
}
pub fn yellow(s: &str) -> String {
    paint("33", s)
}
pub fn cyan(s: &str) -> String {
    paint("36", s)
}

/// Current Wi-Fi link from `netsh wlan show interfaces` (Windows only).
#[cfg(windows)]
pub fn wifi_link() -> Option<Vec<(String, String)>> {
    let out = std::process::Command::new("netsh").args(["wlan", "show", "interfaces"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let wanted = ["SSID", "BSSID", "Radio type", "Channel", "Signal", "Authentication"];
    let pairs: Vec<(String, String)> = text
        .lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, _)| wanted.contains(&k.as_str()))
        .collect();
    pairs.iter().any(|(k, _)| k == "SSID").then_some(pairs)
}

#[cfg(not(windows))]
pub fn wifi_link() -> Option<Vec<(String, String)>> {
    None
}
