//! Linux backend: volume via `wpctl` (PipeWire), WiFi via `nmcli`, Bluetooth via
//! `bluetoothctl`/`rfkill`. All commands go through [`run_capped`].

use super::{
    run_capped, BackendCaps, BluetoothInfo, BtDevice, ControlIntent, SystemControl, SystemMonitor,
    SystemReadout, VolumeInfo, WifiInfo,
};

/// argv for `wpctl set-volume` (level 0..=100 → 0.00..=1.00).
pub fn set_volume_argv(level: u8) -> Vec<String> {
    let frac = level.min(100) as f32 / 100.0;
    vec![
        "set-volume".into(),
        "@DEFAULT_AUDIO_SINK@".into(),
        format!("{:.2}", frac),
    ]
}
/// argv for `nmcli radio wifi on|off`.
pub fn wifi_radio_argv(on: bool) -> Vec<String> {
    vec![
        "radio".into(),
        "wifi".into(),
        if on { "on".into() } else { "off".into() },
    ]
}
/// argv for `nmcli dev wifi connect <ssid>` (known network).
pub fn wifi_connect_argv(ssid: &str) -> Vec<String> {
    vec!["dev".into(), "wifi".into(), "connect".into(), ssid.into()]
}

pub struct LinuxBackend {
    has_wpctl: bool,
    has_nmcli: bool,
    has_bt: bool,
}

impl LinuxBackend {
    pub fn new() -> Self {
        Self {
            has_wpctl: run_capped("which", &["wpctl"], 1).is_some(),
            has_nmcli: run_capped("which", &["nmcli"], 1).is_some(),
            has_bt: run_capped("which", &["bluetoothctl"], 1).is_some(),
        }
    }
}

impl Default for LinuxBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemMonitor for LinuxBackend {
    fn read(&self) -> SystemReadout {
        SystemReadout {
            volume: if self.has_wpctl { read_volume() } else { VolumeInfo::default() },
            wifi: if self.has_nmcli { read_wifi() } else { None },
            known_networks: if self.has_nmcli { read_known() } else { Vec::new() },
            bluetooth: if self.has_bt { read_bt() } else { BluetoothInfo::default() },
        }
    }
    fn caps(&self) -> BackendCaps {
        BackendCaps {
            volume: self.has_wpctl,
            wifi: self.has_nmcli,
            bluetooth: self.has_bt,
        }
    }
}

impl SystemControl for LinuxBackend {
    fn apply(&self, intent: &ControlIntent) {
        let wp = |a: &[String]| {
            run_capped("wpctl", &a.iter().map(String::as_str).collect::<Vec<_>>(), 2);
        };
        let nm = |a: &[String]| {
            run_capped("nmcli", &a.iter().map(String::as_str).collect::<Vec<_>>(), 5);
        };
        match intent {
            ControlIntent::VolumeSet(l) => wp(&set_volume_argv(*l)),
            ControlIntent::VolumeUp => {
                let v = read_volume();
                wp(&set_volume_argv(v.level.saturating_add(5)));
            }
            ControlIntent::VolumeDown => {
                let v = read_volume();
                wp(&set_volume_argv(v.level.saturating_sub(5)));
            }
            ControlIntent::ToggleMute => {
                run_capped("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"], 2);
            }
            ControlIntent::WifiSetEnabled(on) => nm(&wifi_radio_argv(*on)),
            ControlIntent::WifiConnectKnown(ssid) => nm(&wifi_connect_argv(ssid)),
            ControlIntent::BtSetEnabled(on) => {
                run_capped("bluetoothctl", &["power", if *on { "on" } else { "off" }], 3);
            }
            ControlIntent::BtConnect { addr, connect } => {
                run_capped(
                    "bluetoothctl",
                    &[if *connect { "connect" } else { "disconnect" }, addr],
                    5,
                );
            }
            // UI-only intents (calendar nav, notifications) are handled by the
            // session's tray state, never the OS backend.
            _ => {}
        }
    }
}

fn read_volume() -> VolumeInfo {
    let out = run_capped("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"], 2).unwrap_or_default();
    let muted = out.contains("MUTED");
    let level = out
        .split_whitespace()
        .nth(1)
        .and_then(|f| f.parse::<f32>().ok())
        .map(|f| (f * 100.0).round() as u8)
        .unwrap_or(0);
    VolumeInfo { level: level.min(100), muted }
}

fn read_wifi() -> Option<WifiInfo> {
    let out = run_capped("nmcli", &["-t", "-f", "ACTIVE,SSID,SIGNAL", "dev", "wifi"], 3)?;
    for line in out.lines() {
        let mut f = line.split(':');
        if f.next() == Some("yes") {
            let ssid = f.next().unwrap_or("").to_string();
            let sig: u8 = f.next().unwrap_or("0").parse().unwrap_or(0);
            return Some(WifiInfo { ssid, signal: (sig / 25).min(4), enabled: true });
        }
    }
    Some(WifiInfo { ssid: String::new(), signal: 0, enabled: true })
}

fn read_known() -> Vec<String> {
    run_capped("nmcli", &["-t", "-f", "NAME", "connection", "show"], 3)
        .map(|s| {
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn read_bt() -> BluetoothInfo {
    let enabled = run_capped("bluetoothctl", &["show"], 2)
        .map(|s| s.contains("Powered: yes"))
        .unwrap_or(false);
    let devices = run_capped("bluetoothctl", &["devices"], 3)
        .map(|s| {
            s.lines()
                .filter_map(|l| {
                    let mut p = l.splitn(3, ' ');
                    if p.next() != Some("Device") {
                        return None;
                    }
                    let addr = p.next()?.to_string();
                    let name = p.next().unwrap_or("").to_string();
                    Some(BtDevice { name, addr, connected: false })
                })
                .collect()
        })
        .unwrap_or_default();
    BluetoothInfo { enabled, devices }
}
