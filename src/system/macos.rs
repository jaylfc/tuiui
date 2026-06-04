//! macOS system backend: volume via `osascript`, WiFi via `networksetup`,
//! Bluetooth via `blueutil` when present (read-only via `system_profiler`
//! otherwise). All reads/writes go through [`run_capped`] with a hard timeout.

use super::{
    run_capped, BackendCaps, BluetoothInfo, BtDevice, ControlIntent, SystemControl, SystemMonitor,
    SystemReadout, VolumeInfo, WifiInfo,
};

const IF: &str = "en0"; // primary Wi-Fi interface on Apple Silicon Macs

/// argv for `osascript` to set the output volume (clamped 0..=100).
pub fn set_volume_argv(level: u8) -> Vec<String> {
    vec![
        "-e".into(),
        format!("set volume output volume {}", level.min(100)),
    ]
}
/// argv for `osascript` to set the mute state.
pub fn set_mute_argv(muted: bool) -> Vec<String> {
    vec![
        "-e".into(),
        format!("set volume output muted {}", if muted { "true" } else { "false" }),
    ]
}
/// argv for `networksetup -setairportpower`.
pub fn wifi_power_argv(iface: &str, on: bool) -> Vec<String> {
    vec![
        "-setairportpower".into(),
        iface.into(),
        if on { "on".into() } else { "off".into() },
    ]
}
/// argv for `networksetup -setairportnetwork` (known network; no password).
pub fn wifi_connect_argv(iface: &str, ssid: &str) -> Vec<String> {
    vec!["-setairportnetwork".into(), iface.into(), ssid.into()]
}

pub struct MacBackend {
    has_blueutil: bool,
}

impl MacBackend {
    pub fn new() -> Self {
        Self {
            has_blueutil: run_capped("which", &["blueutil"], 1).is_some(),
        }
    }
}

impl Default for MacBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemMonitor for MacBackend {
    fn read(&self) -> SystemReadout {
        SystemReadout {
            volume: read_volume(),
            wifi: read_wifi(),
            known_networks: read_known_networks(),
            bluetooth: if self.has_blueutil {
                read_bluetooth_blueutil()
            } else {
                BluetoothInfo::default()
            },
        }
    }
    fn caps(&self) -> BackendCaps {
        BackendCaps {
            volume: true,
            wifi: true,
            bluetooth: self.has_blueutil,
        }
    }
}

impl SystemControl for MacBackend {
    fn apply(&self, intent: &ControlIntent) {
        let osa = |a: &[String]| {
            run_capped("osascript", &a.iter().map(String::as_str).collect::<Vec<_>>(), 2);
        };
        let net = |a: &[String]| {
            run_capped("networksetup", &a.iter().map(String::as_str).collect::<Vec<_>>(), 3);
        };
        match intent {
            ControlIntent::VolumeSet(l) => osa(&set_volume_argv(*l)),
            ControlIntent::VolumeUp => {
                let v = read_volume();
                osa(&set_volume_argv(v.level.saturating_add(5)));
            }
            ControlIntent::VolumeDown => {
                let v = read_volume();
                osa(&set_volume_argv(v.level.saturating_sub(5)));
            }
            ControlIntent::ToggleMute => {
                let v = read_volume();
                osa(&set_mute_argv(!v.muted));
            }
            ControlIntent::WifiSetEnabled(on) => net(&wifi_power_argv(IF, *on)),
            ControlIntent::WifiConnectKnown(ssid) => net(&wifi_connect_argv(IF, ssid)),
            ControlIntent::BtSetEnabled(on) if self.has_blueutil => {
                run_capped("blueutil", &["--power", if *on { "1" } else { "0" }], 2);
            }
            ControlIntent::BtConnect { addr, connect } if self.has_blueutil => {
                run_capped(
                    "blueutil",
                    &[if *connect { "--connect" } else { "--disconnect" }, addr],
                    5,
                );
            }
            _ => {}
        }
    }
}

fn read_volume() -> VolumeInfo {
    let level = run_capped("osascript", &["-e", "output volume of (get volume settings)"], 2)
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(0);
    let muted = run_capped("osascript", &["-e", "output muted of (get volume settings)"], 2)
        .map(|s| s.trim() == "true")
        .unwrap_or(false);
    VolumeInfo { level, muted }
}

fn read_wifi() -> Option<WifiInfo> {
    let out = run_capped("networksetup", &["-getairportnetwork", IF], 3)?;
    if out.contains("not associated") {
        return Some(WifiInfo { ssid: String::new(), signal: 0, enabled: true });
    }
    let ssid = out.split_once(": ").map(|(_, s)| s.trim().to_string())?;
    Some(WifiInfo { ssid, signal: 3, enabled: true })
}

fn read_known_networks() -> Vec<String> {
    run_capped("networksetup", &["-listpreferredwirelessnetworks", IF], 3)
        .map(|s| {
            s.lines()
                .skip(1)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn read_bluetooth_blueutil() -> BluetoothInfo {
    let enabled = run_capped("blueutil", &["--power"], 2)
        .map(|s| s.trim() == "1")
        .unwrap_or(false);
    let devices = run_capped("blueutil", &["--paired", "--format", "json"], 3)
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|d| {
                    Some(BtDevice {
                        name: d.get("name")?.as_str()?.to_string(),
                        addr: d.get("address")?.as_str()?.to_string(),
                        connected: d.get("connected").and_then(|c| c.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    BluetoothInfo { enabled, devices }
}
