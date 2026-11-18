//! Connected Bluetooth devices, listed as connections of kind `bluetooth`.
//!
//! Windows reports no signal strength (RSSI) for a connected Bluetooth device, so none is
//! shown: only the name and the connected state. The listing takes several seconds
//! (PowerShell + the PnP database), so it is cached and refreshed in the background; a
//! request never waits for it.
//!
//! The list comes from the PnP entries of paired devices (`BTHENUM\DEV_*` and `BTHLE\DEV_*`;
//! the other Bluetooth entries are services and the radio itself) and the PnP property
//! "Bluetooth device connected" ({83DA6326-97A6-4088-9453-A1923F573B29} 15). Device
//! addresses (which appear in the PnP ids) are never read out of the script.

use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Why there is no number: shown on every Bluetooth connection.
pub const NO_RSSI_NOTE: &str = "Windows does not report a signal strength for a connected Bluetooth device, so rhcs can only show that it is connected, not how strong the signal is.";

const REFRESH_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BtDevice {
    pub name: String,
    #[serde(default)]
    pub connected: bool,
}

/// Parses the script's JSON (one object or an array) and keeps connected devices once each
/// (a device paired twice, for example a mouse with two addresses, can list under one name).
pub fn parse_devices(json: &str) -> Vec<BtDevice> {
    let json = json.trim();
    if json.is_empty() {
        return Vec::new();
    }
    let list: Vec<BtDevice> = serde_json::from_str::<Vec<BtDevice>>(json)
        .or_else(|_| serde_json::from_str::<BtDevice>(json).map(|d| vec![d]))
        .unwrap_or_default();
    let mut out: Vec<BtDevice> = Vec::new();
    for d in list {
        let name = d.name.trim().to_string();
        if d.connected && !name.is_empty() && !out.iter().any(|o| o.name == name) {
            out.push(BtDevice { name, connected: true });
        }
    }
    out
}

/// The script run on Windows. Output: JSON [{name, connected}].
pub const SCRIPT: &str = r#"
$d = @(Get-CimInstance Win32_PnPEntity | Where-Object { $_.PNPClass -eq 'Bluetooth' -and $_.DeviceID -match '^BTH(ENUM|LE)\\DEV_' })
if ($d.Count -eq 0) { '[]'; exit 0 }
$p = @(Get-PnpDeviceProperty -InstanceId ($d | ForEach-Object { $_.DeviceID }) -KeyName '{83DA6326-97A6-4088-9453-A1923F573B29} 15' -ErrorAction SilentlyContinue)
$rows = @($d | ForEach-Object { $id = $_.DeviceID; $v = $p | Where-Object { $_.InstanceId -eq $id } | Select-Object -First 1; [pscustomobject]@{ name = [string]$_.Name; connected = [bool]($v -and $v.Data) } })
ConvertTo-Json -InputObject $rows -Compress
"#;

struct Cache {
    at: Option<Instant>,
    devices: Vec<BtDevice>,
    refreshing: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { at: None, devices: Vec::new(), refreshing: false });

pub fn supported() -> bool {
    cfg!(windows)
}

/// Connected devices as of the last refresh (empty before the first one finishes). Starts a
/// background refresh when the data is older than 30 s.
pub fn connected() -> Vec<BtDevice> {
    if !supported() {
        return Vec::new();
    }
    let (devices, start) = {
        let Ok(mut c) = CACHE.lock() else { return Vec::new() };
        let stale = c.at.is_none_or(|t| t.elapsed() > REFRESH_AFTER);
        let start = stale && !c.refreshing;
        if start {
            c.refreshing = true;
        }
        (c.devices.clone(), start)
    };
    if start {
        // A plain thread: this is called from async code and the script blocks for seconds.
        std::thread::spawn(|| {
            let found = crate::winps::run_blocking(SCRIPT, Duration::from_secs(40)).map(|j| parse_devices(&j));
            if let Ok(mut c) = CACHE.lock() {
                if let Ok(list) = found {
                    c.devices = list;
                }
                c.at = Some(Instant::now());
                c.refreshing = false;
            }
        });
    }
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_array_object_and_empty() {
        let j = r#"[{"name":"Example Headset","connected":true},{"name":"Example Mouse","connected":false},{"name":"Example Headset","connected":true},{"name":"  ","connected":true}]"#;
        let d = parse_devices(j);
        assert_eq!(d, vec![BtDevice { name: "Example Headset".into(), connected: true }]);
        // PowerShell prints a single object, not an array, when there is one.
        assert_eq!(parse_devices(r#"{"name":"Example Pad","connected":true}"#).len(), 1);
        assert!(parse_devices("[]").is_empty());
        assert!(parse_devices("").is_empty());
        assert!(parse_devices("not json").is_empty());
        assert!(parse_devices(r#"[{"name":"Idle","connected":false}]"#).is_empty());
    }

    #[test]
    fn script_never_prints_addresses() {
        // Only name + connected leave the script.
        assert!(SCRIPT.contains("name =") && SCRIPT.contains("connected ="));
        assert!(!SCRIPT.contains("DeviceID = ") && !SCRIPT.contains("address"));
    }
}
