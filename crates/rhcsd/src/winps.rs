//! Runs a short PowerShell script on Windows and returns its output. The script goes in on
//! standard input (`-Command -`), so no quoting of the script text can go wrong. Elsewhere it
//! is a harmless stub that always says "not available". Used by the Bluetooth listing and by
//! the Game Boost device actions (devboost.rs).

use std::time::Duration;

/// Runs `script` and returns its stdout (UTF-8) when PowerShell exits with 0.
#[cfg(windows)]
pub fn run_blocking(script: &str, timeout: Duration) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", "-"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start PowerShell: {e}"))?;
    let full = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; $ErrorActionPreference='Stop'; {script}\n");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(full.as_bytes());
    }
    // Read both pipes on threads so a full pipe can never block the child.
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let mut err = child.stderr.take().ok_or("no stderr")?;
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("PowerShell took too long".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let stdout = String::from_utf8_lossy(&t_out.join().unwrap_or_default()).trim().to_string();
    let stderr = String::from_utf8_lossy(&t_err.join().unwrap_or_default()).trim().to_string();
    if status.success() {
        Ok(stdout)
    } else {
        Err(if stderr.is_empty() { format!("PowerShell exited with {status}") } else { first_line(&stderr) })
    }
}

#[cfg(not(windows))]
pub fn run_blocking(_script: &str, _timeout: Duration) -> Result<String, String> {
    Err("PowerShell is only available on Windows".into())
}

/// PowerShell error text is long; the first line says what went wrong.
pub fn first_line(s: &str) -> String {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(300).collect()
}

/// Single-quotes a value for use inside a PowerShell script ('' escapes a quote).
pub fn quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_first_line() {
        assert_eq!(quote("a'b"), "'a''b'");
        assert_eq!(first_line("\n  \nboom here\nsecond"), "boom here");
    }
}
