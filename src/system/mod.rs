//! Host system state (clock, CPU/mem, battery, WiFi, Bluetooth, volume) and the
//! traits that read and control it. Portable metrics come from the `sysinfo`
//! crate; WiFi/volume/Bluetooth go through a per-OS backend.

/// A full snapshot of host state, refreshed by the poller and read each frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemState {
    pub clock: ClockInfo,
    pub cpu_pct: f32,
    pub mem: MemInfo,
    pub battery: Option<BatteryInfo>,
    pub wifi: Option<WifiInfo>,
    pub bluetooth: BluetoothInfo,
    pub volume: VolumeInfo,
    pub known_networks: Vec<String>,
    pub caps: BackendCaps,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClockInfo {
    pub time: String,
    pub date: String,
    pub uptime_secs: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MemInfo {
    pub used: u64,
    pub total: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BatteryInfo {
    pub pct: u8,
    pub charging: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiInfo {
    pub ssid: String,
    pub signal: u8,
    pub enabled: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtDevice {
    pub name: String,
    pub addr: String,
    pub connected: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothInfo {
    pub enabled: bool,
    pub devices: Vec<BtDevice>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeInfo {
    pub level: u8,
    pub muted: bool,
}
/// Which controls the active backend can actually perform (a missing helper tool
/// flips a bit off, so its popover renders read-only).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BackendCaps {
    pub volume: bool,
    pub wifi: bool,
    pub bluetooth: bool,
}

/// Four-segment signal bar, filled left-to-right for `signal` in 0..=4 (clamped).
pub fn bars_glyph(signal: u8) -> String {
    let n = signal.min(4) as usize;
    (0..4).map(|i| if i < n { '▮' } else { '·' }).collect()
}

/// Speaker glyph reflecting mute and level.
pub fn volume_glyph(v: &VolumeInfo) -> &'static str {
    if v.muted || v.level == 0 {
        "🔇"
    } else if v.level < 66 {
        "🔉"
    } else {
        "🔊"
    }
}

/// Integer percentage of `used`/`total`, guarding division by zero.
pub fn mem_pct(used: u64, total: u64) -> u8 {
    if total == 0 {
        0
    } else {
        ((used as u128 * 100) / total as u128) as u8
    }
}
