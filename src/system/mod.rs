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

/// A control action requested by a popover click, dispatched by the session to
/// the active backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlIntent {
    VolumeUp,
    VolumeDown,
    VolumeSet(u8),
    ToggleMute,
    WifiSetEnabled(bool),
    WifiConnectKnown(String),
    BtSetEnabled(bool),
    BtConnect { addr: String, connect: bool },
}

/// The OS-specific slice of [`SystemState`] produced by a [`SystemMonitor`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemReadout {
    pub wifi: Option<WifiInfo>,
    pub bluetooth: BluetoothInfo,
    pub volume: VolumeInfo,
    pub known_networks: Vec<String>,
}

/// Reads the OS-specific parts of the snapshot. Portable metrics (CPU/mem/
/// battery) are filled in by the poller via `sysinfo`.
pub trait SystemMonitor: Send + Sync {
    fn read(&self) -> SystemReadout;
    fn caps(&self) -> BackendCaps;
}

/// Applies a [`ControlIntent`] to the host (best-effort, timeout-guarded).
pub trait SystemControl: Send + Sync {
    fn apply(&self, intent: &ControlIntent);
}

/// A backend is both a monitor and a controller.
pub trait Backend: SystemMonitor + SystemControl {}
impl<T: SystemMonitor + SystemControl> Backend for T {}

/// Fallback backend on unsupported targets: no caps, no-op control.
pub struct StubBackend;
impl SystemMonitor for StubBackend {
    fn read(&self) -> SystemReadout {
        SystemReadout::default()
    }
    fn caps(&self) -> BackendCaps {
        BackendCaps::default()
    }
}
impl SystemControl for StubBackend {
    fn apply(&self, _intent: &ControlIntent) {}
}

/// Pick the backend for the current OS. Returns a stub on unsupported targets.
pub fn backend() -> Box<dyn Backend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacBackend::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxBackend::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Box::new(StubBackend)
    }
}

/// Run a command with a hard timeout, returning trimmed stdout on success.
/// Never blocks longer than `secs`; kills the child on timeout. This is the
/// **only** sanctioned way to shell out — it keeps a hung tool from freezing
/// any caller (the poller or a control dispatch).
pub fn run_capped(program: &str, args: &[&str], secs: u64) -> Option<String> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().ok()?;
                if !status.success() {
                    return None;
                }
                return String::from_utf8(out.stdout).ok();
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "linux")]
pub mod linux;
