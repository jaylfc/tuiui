# Menubar Tray + System Managers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a macOS-style menubar status tray (clock, WiFi, Bluetooth, volume, battery, CPU/memory) with click-through popovers that control the host's volume, switch to a known WiFi network, and connect a paired Bluetooth device.

**Architecture:** Daemon-side widgets mirroring the launcher/store/settings. A pure `system` module defines a `SystemState` snapshot plus `SystemMonitor`/`SystemControl` traits; a background `SystemPoller` refreshes the snapshot on throttled cadences with hard timeouts; `tray.rs` renders indicator segments + popovers and hit-tests clicks; `session.rs` wires clicks to backend control calls with optimistic snapshot updates. No display protocol change; one new `Flags` bit for popover keyboard routing is optional and deferred.

**Tech Stack:** Rust 2021, new `sysinfo` crate (portable CPU/mem/battery/net), `std::process::Command` for OS control (timeout-guarded), existing compositor/chrome/session.

**Reference spec:** `docs/superpowers/specs/2026-06-04-menubar-tray-design.md`

---

## File Structure

- **Create `src/system/mod.rs`** — `SystemState` + sub-structs; `SystemMonitor`/`SystemControl` traits; `ControlIntent`; pure formatting helpers (`bars_glyph`, `volume_glyph`, `mem_pct`); `sysinfo`-backed portable metrics; runtime backend selection.
- **Create `src/system/macos.rs`** — macOS backend (read + control), with pure argv-builder functions.
- **Create `src/system/linux.rs`** — Linux backend (read + control), with pure argv-builder functions.
- **Create `src/poller.rs`** — `SystemPoller`: background thread, throttled cadences, shared `Arc<RwLock<SystemState>>`, hard command timeouts.
- **Create `src/tray.rs`** — `Tray` widget: segment layout + hit regions, popover render + hit-testing → `ControlIntent`.
- **Modify `src/chrome.rs`** — `render_menubar` draws tray segments; reads them from a passed slice.
- **Modify `src/session.rs`** — own `Tray` + shared snapshot; hit-test tray on `MouseDown`; dispatch `ControlIntent`s; optimistic updates; include tray layers in `build_frame`.
- **Modify `src/daemon.rs`** — start the `SystemPoller`; pass the shared snapshot into `SessionCore`.
- **Modify `src/lib.rs`** — `pub mod system; pub mod poller; pub mod tray;`.
- **Modify `Cargo.toml`** — add `sysinfo`.
- **Modify `install.sh`** — optional OS-aware dependency step (blueutil).
- **Test files:** `tests/system_tests.rs`, `tests/tray_tests.rs` (pure logic; no process execution).

Tasks are ordered so each leaves the tree compiling and tested.

---

### Task 1: Add the `sysinfo` dependency and empty modules

**Files:**
- Modify: `Cargo.toml`
- Create: `src/system/mod.rs`, `src/poller.rs`, `src/tray.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add the dependency**

In `Cargo.toml` under `[dependencies]`, add:

```toml
sysinfo = "0.33"
```

- [ ] **Step 2: Create empty modules**

`src/system/mod.rs`:

```rust
//! Host system state (clock, CPU/mem, battery, WiFi, Bluetooth, volume) and the
//! traits that read and control it. Portable metrics come from the `sysinfo`
//! crate; WiFi/volume/Bluetooth go through a per-OS backend.
```

`src/poller.rs`:

```rust
//! Background poller that refreshes a shared `SystemState` on throttled cadences.
```

`src/tray.rs`:

```rust
//! The menubar status tray: indicator segments + click-through popovers.
```

- [ ] **Step 3: Register modules**

In `src/lib.rs`, add alongside the existing `pub mod` lines:

```rust
pub mod system;
pub mod poller;
pub mod tray;
```

- [ ] **Step 4: Build**

Run: `cargo build --offline` (if the crate is not cached, run `cargo build` once with network).
Expected: compiles (modules are empty but valid).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/system/mod.rs src/poller.rs src/tray.rs src/lib.rs
git commit -m "tray: scaffold system/poller/tray modules + sysinfo dep"
```

---

### Task 2: `SystemState` types and pure formatting helpers

**Files:**
- Modify: `src/system/mod.rs`
- Test: `tests/system_tests.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/system_tests.rs`:

```rust
use tuiui::system::{bars_glyph, volume_glyph, mem_pct, VolumeInfo};

#[test]
fn signal_bars_fill_left_to_right() {
    assert_eq!(bars_glyph(0), "····");
    assert_eq!(bars_glyph(1), "▮···");
    assert_eq!(bars_glyph(2), "▮▮··");
    assert_eq!(bars_glyph(4), "▮▮▮▮");
    assert_eq!(bars_glyph(9), "▮▮▮▮"); // clamps
}

#[test]
fn volume_glyph_reflects_mute_and_level() {
    assert_eq!(volume_glyph(&VolumeInfo { level: 0, muted: false }), "🔇");
    assert_eq!(volume_glyph(&VolumeInfo { level: 50, muted: false }), "🔉");
    assert_eq!(volume_glyph(&VolumeInfo { level: 90, muted: false }), "🔊");
    assert_eq!(volume_glyph(&VolumeInfo { level: 50, muted: true }), "🔇");
}

#[test]
fn mem_pct_rounds() {
    assert_eq!(mem_pct(6, 10), 60);
    assert_eq!(mem_pct(0, 0), 0); // guard against divide-by-zero
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --offline --test system_tests`
Expected: FAIL — `bars_glyph`, `volume_glyph`, `mem_pct`, `VolumeInfo` not found.

- [ ] **Step 3: Implement the types and helpers**

In `src/system/mod.rs`, add:

```rust
use std::path::Path;

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
pub struct ClockInfo { pub time: String, pub date: String, pub uptime_secs: u64 }
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MemInfo { pub used: u64, pub total: u64 }
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BatteryInfo { pub pct: u8, pub charging: bool }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiInfo { pub ssid: String, pub signal: u8, pub enabled: bool }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtDevice { pub name: String, pub addr: String, pub connected: bool }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothInfo { pub enabled: bool, pub devices: Vec<BtDevice> }
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeInfo { pub level: u8, pub muted: bool }
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BackendCaps { pub volume: bool, pub wifi: bool, pub bluetooth: bool }

/// Four-segment signal bar, filled left-to-right for `signal` in 0..=4 (clamped).
pub fn bars_glyph(signal: u8) -> String {
    let n = signal.min(4) as usize;
    let mut s = String::new();
    for i in 0..4 { s.push(if i < n { '▮' } else { '·' }); }
    s
}

/// Speaker glyph reflecting mute and level.
pub fn volume_glyph(v: &VolumeInfo) -> &'static str {
    if v.muted || v.level == 0 { "🔇" } else if v.level < 66 { "🔉" } else { "🔊" }
}

/// Integer percentage of `used`/`total`, guarding division by zero.
pub fn mem_pct(used: u64, total: u64) -> u8 {
    if total == 0 { 0 } else { ((used as u128 * 100) / total as u128) as u8 }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --offline --test system_tests`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/system/mod.rs tests/system_tests.rs
git commit -m "tray: SystemState types + pure formatting helpers"
```

---

### Task 3: Control intents and backend traits

**Files:**
- Modify: `src/system/mod.rs`

- [ ] **Step 1: Add the traits and intent enum (no test — type-only scaffolding exercised by later tasks)**

Append to `src/system/mod.rs`:

```rust
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

/// Reads the OS-specific parts of the snapshot. Portable metrics (CPU/mem/
/// battery/net) are filled in by the poller via `sysinfo`.
pub trait SystemMonitor: Send + Sync {
    fn read(&self) -> SystemReadout;
    fn caps(&self) -> BackendCaps;
}

/// The OS-specific slice of `SystemState` produced by a `SystemMonitor`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemReadout {
    pub wifi: Option<WifiInfo>,
    pub bluetooth: BluetoothInfo,
    pub volume: VolumeInfo,
    pub known_networks: Vec<String>,
}

/// Applies a `ControlIntent` to the host (best-effort, timeout-guarded).
pub trait SystemControl: Send + Sync {
    fn apply(&self, intent: &ControlIntent);
}

/// Pick the backend for the current OS. Returns a stub on unsupported targets.
pub fn backend() -> Box<dyn Backend> {
    #[cfg(target_os = "macos")]
    { return Box::new(crate::system::macos::MacBackend::new()); }
    #[cfg(target_os = "linux")]
    { return Box::new(crate::system::linux::LinuxBackend::new()); }
    #[allow(unreachable_code)]
    { Box::new(StubBackend) }
}

/// A backend is both a monitor and a controller.
pub trait Backend: SystemMonitor + SystemControl {}
impl<T: SystemMonitor + SystemControl> Backend for T {}

/// Fallback backend on unsupported targets: no caps, no-op control.
pub struct StubBackend;
impl SystemMonitor for StubBackend {
    fn read(&self) -> SystemReadout { SystemReadout::default() }
    fn caps(&self) -> BackendCaps { BackendCaps::default() }
}
impl SystemControl for StubBackend {
    fn apply(&self, _intent: &ControlIntent) {}
}

/// Helper used by backends: run a command with a hard timeout, returning stdout
/// on success. Never blocks longer than `secs`.
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
                if !status.success() { return None; }
                let out = child.wait_with_output().ok()?;
                return String::from_utf8(out.stdout).ok();
            }
            Ok(None) => {
                if Instant::now() >= deadline { let _ = child.kill(); return None; }
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
```

Note: `wait_with_output` after `try_wait` returning `Some` is safe because stdout is piped and the child has exited.

- [ ] **Step 2: Build**

Run: `cargo build --offline`
Expected: FAIL — `crate::system::macos` / `linux` modules don't exist yet. That is expected; the next task creates the active-OS module. To keep the tree green between tasks, temporarily stub the missing module:

On macOS dev hosts, proceed to Task 4 (creates `macos.rs`). On Linux dev hosts, do Task 5 first. Only the current-OS module must exist to compile. (Both are created before Task 7 wires them in.)

- [ ] **Step 3: Commit (after the current-OS backend task compiles)**

Defer the commit to the end of Task 4 (macOS) or Task 5 (Linux), whichever matches the dev host, so the tree compiles.

---

### Task 4: macOS backend (argv builders tested; exec untested)

**Files:**
- Create: `src/system/macos.rs`
- Test: `tests/system_tests.rs` (append)

- [ ] **Step 1: Write the failing test**

Append to `tests/system_tests.rs`:

```rust
#[cfg(target_os = "macos")]
mod macos_argv {
    use tuiui::system::macos::*;

    #[test]
    fn set_volume_builds_osascript() {
        assert_eq!(
            set_volume_argv(40),
            vec!["-e".to_string(), "set volume output volume 40".to_string()]
        );
    }

    #[test]
    fn volume_clamps_to_100() {
        assert_eq!(set_volume_argv(250)[1], "set volume output volume 100");
    }

    #[test]
    fn wifi_power_argv() {
        assert_eq!(wifi_power_argv("en0", true), vec!["-setairportpower", "en0", "on"]);
        assert_eq!(wifi_power_argv("en0", false), vec!["-setairportpower", "en0", "off"]);
    }

    #[test]
    fn wifi_connect_known_argv() {
        assert_eq!(
            wifi_connect_argv("en0", "HomeNet"),
            vec!["-setairportnetwork", "en0", "HomeNet"]
        );
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run (on a macOS host): `cargo test --offline --test system_tests`
Expected: FAIL — `tuiui::system::macos` functions not found.

- [ ] **Step 3: Implement `src/system/macos.rs`**

```rust
//! macOS system backend: volume via `osascript`, WiFi via `networksetup`,
//! Bluetooth via `blueutil` when present (read-only via `system_profiler`
//! otherwise). All reads/writes go through `run_capped` with a hard timeout.

use super::{
    run_capped, BackendCaps, BluetoothInfo, BtDevice, ControlIntent, SystemControl,
    SystemMonitor, SystemReadout, VolumeInfo, WifiInfo,
};

const IF: &str = "en0"; // primary Wi-Fi interface on Apple Silicon Macs

/// argv for `osascript` to set the output volume (clamped 0..=100).
pub fn set_volume_argv(level: u8) -> Vec<String> {
    vec!["-e".into(), format!("set volume output volume {}", level.min(100))]
}
/// argv for `osascript` to set the mute state.
pub fn set_mute_argv(muted: bool) -> Vec<String> {
    vec!["-e".into(), format!("set volume output muted {}", if muted { "true" } else { "false" })]
}
/// argv for `networksetup -setairportpower`.
pub fn wifi_power_argv(iface: &str, on: bool) -> Vec<String> {
    vec!["-setairportpower".into(), iface.into(), if on { "on".into() } else { "off".into() }]
}
/// argv for `networksetup -setairportnetwork` (known network; no password).
pub fn wifi_connect_argv(iface: &str, ssid: &str) -> Vec<String> {
    vec!["-setairportnetwork".into(), iface.into(), ssid.into()]
}

pub struct MacBackend { has_blueutil: bool }

impl MacBackend {
    pub fn new() -> Self {
        let has_blueutil = run_capped("which", &["blueutil"], 1).is_some();
        Self { has_blueutil }
    }
}

impl SystemMonitor for MacBackend {
    fn read(&self) -> SystemReadout {
        let volume = read_volume();
        let wifi = read_wifi();
        let known_networks = read_known_networks();
        let bluetooth = if self.has_blueutil { read_bluetooth_blueutil() } else { BluetoothInfo::default() };
        SystemReadout { wifi, bluetooth, volume, known_networks }
    }
    fn caps(&self) -> BackendCaps {
        BackendCaps { volume: true, wifi: true, bluetooth: self.has_blueutil }
    }
}

impl SystemControl for MacBackend {
    fn apply(&self, intent: &ControlIntent) {
        let s = |a: &[String]| { run_capped("osascript", &a.iter().map(String::as_str).collect::<Vec<_>>(), 2); };
        let n = |a: &[String]| { run_capped("networksetup", &a.iter().map(String::as_str).collect::<Vec<_>>(), 3); };
        match intent {
            ControlIntent::VolumeSet(l) => s(&set_volume_argv(*l)),
            ControlIntent::VolumeUp => { let v = read_volume(); s(&set_volume_argv(v.level.saturating_add(5))); }
            ControlIntent::VolumeDown => { let v = read_volume(); s(&set_volume_argv(v.level.saturating_sub(5))); }
            ControlIntent::ToggleMute => { let v = read_volume(); s(&set_mute_argv(!v.muted)); }
            ControlIntent::WifiSetEnabled(on) => n(&wifi_power_argv(IF, *on)),
            ControlIntent::WifiConnectKnown(ssid) => n(&wifi_connect_argv(IF, ssid)),
            ControlIntent::BtSetEnabled(on) if self.has_blueutil => { run_capped("blueutil", &["--power", if *on {"1"} else {"0"}], 2); }
            ControlIntent::BtConnect { addr, connect } if self.has_blueutil => {
                run_capped("blueutil", &[if *connect {"--connect"} else {"--disconnect"}, addr], 5);
            }
            _ => {}
        }
    }
}

fn read_volume() -> VolumeInfo {
    let level = run_capped("osascript", &["-e", "output volume of (get volume settings)"], 2)
        .and_then(|s| s.trim().parse::<u8>().ok()).unwrap_or(0);
    let muted = run_capped("osascript", &["-e", "output muted of (get volume settings)"], 2)
        .map(|s| s.trim() == "true").unwrap_or(false);
    VolumeInfo { level, muted }
}

fn read_wifi() -> Option<WifiInfo> {
    let out = run_capped("networksetup", &["-getairportnetwork", IF], 3)?;
    // "Current Wi-Fi Network: SSID" or "You are not associated..."
    let ssid = out.split_once(": ").map(|(_, s)| s.trim().to_string())?;
    if ssid.is_empty() || out.contains("not associated") { return Some(WifiInfo { ssid: String::new(), signal: 0, enabled: true }); }
    Some(WifiInfo { ssid, signal: 3, enabled: true }) // signal refined later; 3 bars default when associated
}

fn read_known_networks() -> Vec<String> {
    run_capped("networksetup", &["-listpreferredwirelessnetworks", IF], 3)
        .map(|s| s.lines().skip(1).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

fn read_bluetooth_blueutil() -> BluetoothInfo {
    let enabled = run_capped("blueutil", &["--power"], 2).map(|s| s.trim() == "1").unwrap_or(false);
    let devices = run_capped("blueutil", &["--paired", "--format", "json"], 3)
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_array().cloned())
        .map(|arr| arr.iter().filter_map(|d| {
            Some(BtDevice {
                name: d.get("name")?.as_str()?.to_string(),
                addr: d.get("address")?.as_str()?.to_string(),
                connected: d.get("connected").and_then(|c| c.as_bool()).unwrap_or(false),
            })
        }).collect())
        .unwrap_or_default();
    BluetoothInfo { enabled, devices }
}
```

- [ ] **Step 4: Run to verify it passes**

Run (macOS host): `cargo test --offline --test system_tests`
Expected: PASS — argv builder tests pass; the exec functions are not invoked by tests.

- [ ] **Step 5: Commit**

```bash
git add src/system/mod.rs src/system/macos.rs tests/system_tests.rs
git commit -m "tray: macOS backend (volume/wifi/bluetooth) + argv tests"
```

---

### Task 5: Linux backend (argv builders tested; exec untested)

**Files:**
- Create: `src/system/linux.rs`
- Test: `tests/system_tests.rs` (append)

- [ ] **Step 1: Write the failing test**

Append:

```rust
#[cfg(target_os = "linux")]
mod linux_argv {
    use tuiui::system::linux::*;

    #[test]
    fn set_volume_builds_wpctl() {
        assert_eq!(set_volume_argv(40),
            vec!["set-volume", "@DEFAULT_AUDIO_SINK@", "0.40"]);
        assert_eq!(set_volume_argv(100)[2], "1.00");
        assert_eq!(set_volume_argv(250)[2], "1.00"); // clamps
    }
    #[test]
    fn wifi_radio_argv() {
        assert_eq!(wifi_radio_argv(true), vec!["radio", "wifi", "on"]);
        assert_eq!(wifi_radio_argv(false), vec!["radio", "wifi", "off"]);
    }
    #[test]
    fn wifi_connect_argv_known() {
        assert_eq!(wifi_connect_argv("HomeNet"), vec!["dev", "wifi", "connect", "HomeNet"]);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run (Linux host): `cargo test --offline --test system_tests`
Expected: FAIL — `tuiui::system::linux` not found.

- [ ] **Step 3: Implement `src/system/linux.rs`**

```rust
//! Linux backend: volume via `wpctl` (PipeWire), WiFi via `nmcli`, Bluetooth via
//! `bluetoothctl`/`rfkill`. All commands go through `run_capped`.

use super::{
    run_capped, BackendCaps, BluetoothInfo, BtDevice, ControlIntent, SystemControl,
    SystemMonitor, SystemReadout, VolumeInfo, WifiInfo,
};

/// argv for `wpctl set-volume` (level 0..=100 → 0.00..=1.00).
pub fn set_volume_argv(level: u8) -> Vec<String> {
    let frac = level.min(100) as f32 / 100.0;
    vec!["set-volume".into(), "@DEFAULT_AUDIO_SINK@".into(), format!("{:.2}", frac)]
}
/// argv for `nmcli radio wifi on|off`.
pub fn wifi_radio_argv(on: bool) -> Vec<String> {
    vec!["radio".into(), "wifi".into(), if on { "on".into() } else { "off".into() }]
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

impl SystemMonitor for LinuxBackend {
    fn read(&self) -> SystemReadout {
        let volume = if self.has_wpctl { read_volume() } else { VolumeInfo::default() };
        let wifi = if self.has_nmcli { read_wifi() } else { None };
        let known_networks = if self.has_nmcli { read_known() } else { Vec::new() };
        let bluetooth = if self.has_bt { read_bt() } else { BluetoothInfo::default() };
        SystemReadout { wifi, bluetooth, volume, known_networks }
    }
    fn caps(&self) -> BackendCaps {
        BackendCaps { volume: self.has_wpctl, wifi: self.has_nmcli, bluetooth: self.has_bt }
    }
}

impl SystemControl for LinuxBackend {
    fn apply(&self, intent: &ControlIntent) {
        let w = |a: &[String]| { run_capped("wpctl", &a.iter().map(String::as_str).collect::<Vec<_>>(), 2); };
        let n = |a: &[String]| { run_capped("nmcli", &a.iter().map(String::as_str).collect::<Vec<_>>(), 5); };
        match intent {
            ControlIntent::VolumeSet(l) => w(&set_volume_argv(*l)),
            ControlIntent::VolumeUp => { let v = read_volume(); w(&set_volume_argv(v.level.saturating_add(5))); }
            ControlIntent::VolumeDown => { let v = read_volume(); w(&set_volume_argv(v.level.saturating_sub(5))); }
            ControlIntent::ToggleMute => { run_capped("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"], 2); }
            ControlIntent::WifiSetEnabled(on) => n(&wifi_radio_argv(*on)),
            ControlIntent::WifiConnectKnown(ssid) => n(&wifi_connect_argv(ssid)),
            ControlIntent::BtSetEnabled(on) => { run_capped("bluetoothctl", &["power", if *on {"on"} else {"off"}], 3); }
            ControlIntent::BtConnect { addr, connect } => {
                run_capped("bluetoothctl", &[if *connect {"connect"} else {"disconnect"}, addr], 5);
            }
            _ => {}
        }
    }
}

fn read_volume() -> VolumeInfo {
    let out = run_capped("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"], 2).unwrap_or_default();
    // "Volume: 0.60" or "Volume: 0.60 [MUTED]"
    let muted = out.contains("MUTED");
    let level = out.split_whitespace().nth(1)
        .and_then(|f| f.parse::<f32>().ok())
        .map(|f| (f * 100.0).round() as u8).unwrap_or(0);
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
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

fn read_bt() -> BluetoothInfo {
    let enabled = run_capped("bluetoothctl", &["show"], 2).map(|s| s.contains("Powered: yes")).unwrap_or(false);
    let devices = run_capped("bluetoothctl", &["devices"], 3)
        .map(|s| s.lines().filter_map(|l| {
            let mut p = l.splitn(3, ' ');
            if p.next() != Some("Device") { return None; }
            let addr = p.next()?.to_string();
            let name = p.next().unwrap_or("").to_string();
            Some(BtDevice { name, addr, connected: false })
        }).collect())
        .unwrap_or_default();
    BluetoothInfo { enabled, devices }
}
```

- [ ] **Step 4: Run to verify it passes**

Run (Linux host): `cargo test --offline --test system_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/system/mod.rs src/system/linux.rs tests/system_tests.rs
git commit -m "tray: Linux backend (wpctl/nmcli/bluetoothctl) + argv tests"
```

---

### Task 6: `SystemPoller` — shared snapshot with throttled refresh

**Files:**
- Modify: `src/poller.rs`

- [ ] **Step 1: Implement the poller (integration-shaped; no unit test — it owns a thread and clock)**

`src/poller.rs`:

```rust
//! Background poller that refreshes a shared `SystemState` on throttled cadences.

use crate::system::{backend, mem_pct, BatteryInfo, ClockInfo, SystemState};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Owns a background thread that periodically refreshes a shared snapshot.
pub struct SystemPoller {
    state: Arc<RwLock<SystemState>>,
}

impl SystemPoller {
    /// Spawn the poller thread and return a handle whose `state()` is updated in
    /// place. The thread runs for the lifetime of the process (daemon).
    pub fn start() -> Self {
        let state = Arc::new(RwLock::new(SystemState::default()));
        let worker = state.clone();
        std::thread::spawn(move || run(worker));
        SystemPoller { state }
    }

    /// The shared snapshot handle (cloned into the session).
    pub fn state(&self) -> Arc<RwLock<SystemState>> { self.state.clone() }
}

fn run(state: Arc<RwLock<SystemState>>) {
    let backend = backend();
    let mut sys = sysinfo::System::new_all();
    let mut last_slow = Instant::now() - Duration::from_secs(10);
    loop {
        // Clock + CPU/mem every ~1s.
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let cpu_pct = sys.global_cpu_usage();
        let (used, total) = (sys.used_memory(), sys.total_memory());
        let clock = now_clock();
        let battery = read_battery();

        // WiFi/volume/Bluetooth (shell-outs) every ~3s.
        let slow = last_slow.elapsed() >= Duration::from_secs(3);
        let (readout, caps) = if slow {
            last_slow = Instant::now();
            (Some(backend.read()), Some(backend.caps()))
        } else { (None, None) };

        if let Ok(mut s) = state.write() {
            s.clock = clock;
            s.cpu_pct = cpu_pct;
            s.mem.used = used;
            s.mem.total = total;
            let _ = mem_pct; // formatting done at render time
            s.battery = battery;
            if let Some(r) = readout {
                s.wifi = r.wifi;
                s.bluetooth = r.bluetooth;
                s.volume = r.volume;
                s.known_networks = r.known_networks;
            }
            if let Some(c) = caps { s.caps = c; }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn now_clock() -> ClockInfo {
    // Local time via `date`, uptime via sysinfo. `date` is cheap and avoids a
    // chrono dependency.
    let time = crate::system::run_capped("date", &["+%H:%M"], 1).unwrap_or_default().trim().to_string();
    let date = crate::system::run_capped("date", &["+%a %d %b"], 1).unwrap_or_default().trim().to_string();
    let uptime_secs = sysinfo::System::uptime();
    ClockInfo { time, date, uptime_secs }
}

fn read_battery() -> Option<BatteryInfo> {
    // sysinfo does not expose battery on all platforms; use the components/known
    // paths. Return None when unavailable (hides the segment).
    None // refined per-OS in a follow-up; absent battery is valid (the mini).
}
```

Note: battery is intentionally `None` initially (the mini has none); a later
enhancement can populate it on laptop hosts without blocking this feature.

- [ ] **Step 2: Build**

Run: `cargo build --offline`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src/poller.rs
git commit -m "tray: SystemPoller — throttled shared SystemState refresh"
```

---

### Task 7: Tray segments + hit regions (pure layout)

**Files:**
- Modify: `src/tray.rs`
- Test: `tests/tray_tests.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/tray_tests.rs`:

```rust
use tuiui::system::{SystemState, VolumeInfo, WifiInfo, MemInfo};
use tuiui::tray::{tray_segments, SegmentKind};

fn sample() -> SystemState {
    SystemState {
        clock: tuiui::system::ClockInfo { time: "09:41".into(), date: "Wed 04 Jun".into(), uptime_secs: 0 },
        cpu_pct: 32.0,
        mem: MemInfo { used: 6, total: 10 },
        wifi: Some(WifiInfo { ssid: "wlan".into(), signal: 3, enabled: true }),
        volume: VolumeInfo { level: 60, muted: false },
        ..Default::default()
    }
}

#[test]
fn segments_are_right_aligned_and_ordered() {
    let segs = tray_segments(&sample(), 100);
    // Clock is the right-most segment.
    let clock = segs.iter().find(|s| s.kind == SegmentKind::Clock).unwrap();
    let max_x = segs.iter().map(|s| s.rect.x + s.rect.w).max().unwrap();
    assert_eq!(clock.rect.x + clock.rect.w, max_x);
    // Every segment fits on row 0.
    assert!(segs.iter().all(|s| s.rect.y == 0));
}

#[test]
fn narrow_width_drops_cpu_then_battery_but_keeps_clock() {
    let wide = tray_segments(&sample(), 100);
    let narrow = tray_segments(&sample(), 24);
    assert!(narrow.iter().any(|s| s.kind == SegmentKind::Clock));
    assert!(!narrow.iter().any(|s| s.kind == SegmentKind::Cpu));
    assert!(wide.iter().any(|s| s.kind == SegmentKind::Cpu));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --offline --test tray_tests`
Expected: FAIL — `tray_segments` / `SegmentKind` not found.

- [ ] **Step 3: Implement segment layout in `src/tray.rs`**

```rust
use crate::geometry::Rect;
use crate::system::{bars_glyph, volume_glyph, mem_pct, SystemState};

/// Which indicator a tray segment represents (used for hit-testing + drop order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind { Cpu, Mem, Battery, Volume, Bluetooth, Wifi, Clock }

/// A laid-out tray segment: its kind, display text, and screen rect (row 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Segment { pub kind: SegmentKind, pub text: String, pub rect: Rect }

/// Reserve this many columns on the right for the Quit button (matches chrome).
const QUIT_RESERVE: i32 = 9;

/// Build the right-aligned, ordered list of tray segments for a `width`-wide
/// menubar. Lowest-priority segments (CPU, then Memory, then Battery) drop out
/// first when space is tight; the clock is always kept.
pub fn tray_segments(state: &SystemState, width: i32) -> Vec<Segment> {
    // Display order, left→right. Priority for dropping is the reverse of "keep":
    // keep Clock > Wifi > Bluetooth > Volume > Battery > Mem > Cpu.
    let mut texts: Vec<(SegmentKind, String)> = Vec::new();
    texts.push((SegmentKind::Cpu, format!("⊙{}%", state.cpu_pct.round() as u32)));
    texts.push((SegmentKind::Mem, format!("▤{}%", mem_pct(state.mem.used, state.mem.total))));
    if let Some(b) = &state.battery {
        texts.push((SegmentKind::Battery, format!("{}{}%", if b.charging { "⚡" } else { "🔋" }, b.pct)));
    }
    texts.push((SegmentKind::Volume, format!("{}{}", volume_glyph(&state.volume), state.volume.level)));
    if state.caps.bluetooth || state.bluetooth.enabled {
        texts.push((SegmentKind::Bluetooth, "⏻bt".to_string()));
    }
    if let Some(w) = &state.wifi {
        let name = if w.ssid.is_empty() { "wifi".to_string() } else { w.ssid.clone() };
        texts.push((SegmentKind::Wifi, format!("{} {}", bars_glyph(w.signal), name)));
    }
    texts.push((SegmentKind::Clock, state.clock.time.clone()));

    // Drop order when out of space: Cpu, Mem, Battery (Clock always kept).
    let drop_priority = [SegmentKind::Cpu, SegmentKind::Mem, SegmentKind::Battery];
    let gap = 2;
    let avail = width - QUIT_RESERVE - 1;
    let total = |v: &Vec<(SegmentKind, String)>| -> i32 {
        v.iter().map(|(_, t)| t.chars().count() as i32 + gap).sum()
    };
    for k in drop_priority {
        if total(&texts) <= avail { break; }
        texts.retain(|(sk, _)| *sk != k);
    }

    // Right-align: lay out from the right edge leftward, then return left→right.
    let mut x = width - QUIT_RESERVE - 1;
    let mut out: Vec<Segment> = Vec::new();
    for (kind, text) in texts.iter().rev() {
        let w = text.chars().count() as i32;
        x -= w;
        out.push(Segment { kind: *kind, text: text.clone(), rect: Rect::new(x, 0, w, 1) });
        x -= gap;
    }
    out.reverse();
    out
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --offline --test tray_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tray.rs tests/tray_tests.rs
git commit -m "tray: right-aligned segment layout + drop-out + hit rects"
```

---

### Task 8: Render tray into the menubar

**Files:**
- Modify: `src/chrome.rs:33-66` (the `render_menubar` fn and brand/quit regions)
- Modify: `src/session.rs` (pass segments)

- [ ] **Step 1: Change `render_menubar` to accept tray segments**

In `src/chrome.rs`, change the signature and draw segments. Replace the body of `render_menubar` so that, after drawing the brand and before the quit button, it paints each segment's text at its rect using `theme::current()` colors:

```rust
pub fn render_menubar(width: i32, focused_app: &str, segments: &[crate::tray::Segment]) -> Layer {
    let t = crate::theme::current();
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.menubar_bg, attrs: Default::default() });
    buf.write_str(1, 0, "\u{2726} Tuiui", t.accent, t.menubar_bg);
    // Focused app name (truncate so it never overlaps the left-most segment).
    let app_limit = segments.iter().map(|s| s.rect.x).min().unwrap_or(width) - 10;
    if app_limit > 0 {
        let name: String = focused_app.chars().take(app_limit as usize).collect();
        buf.write_str(10, 0, &name, t.dim, t.menubar_bg);
    }
    for s in segments {
        buf.write_str(s.rect.x, 0, &s.text, t.text, t.menubar_bg);
    }
    // Quit button (unchanged), right-aligned.
    let qx = (width - QUIT_LABEL.chars().count() as i32).max(0);
    buf.write_str(qx, 0, QUIT_LABEL, t.close_fg, QUIT_BG);
    Layer { z: 1000, origin: Point::new(0, 0), buf, opacity: 1.0, scissor: None }
}
```

(Adjust to match the exact existing brand/quit drawing already in the file; keep `QUIT_BG`/`QUIT_LABEL`.)

- [ ] **Step 2: Update the call site in `src/session.rs`**

In `build_frame`, build segments from the snapshot and pass them in. Add a field `tray_state: Arc<RwLock<SystemState>>` to `SessionCore` (Task 9 wires construction); for now read it:

```rust
let segs = {
    let st = self.tray_state.read().unwrap();
    crate::tray::tray_segments(&st, self.w)
};
layers.push(render_menubar(self.w, &app_name, &segs));
```

- [ ] **Step 3: Build (expect a constructor gap)**

Run: `cargo build --offline`
Expected: FAIL — `SessionCore` has no `tray_state` field yet. Proceed to Task 9 which adds it; these two tasks land together. (If executing strictly one-task-at-a-time, fold Task 9's field addition into this step.)

- [ ] **Step 4: Commit (with Task 9)**

Defer commit to Task 9 so the tree compiles.

---

### Task 9: Wire the poller + snapshot into the session and daemon

**Files:**
- Modify: `src/session.rs` (struct field, constructor, `build_frame` already updated)
- Modify: `src/daemon.rs`

- [ ] **Step 1: Add the field and constructor parameter**

In `SessionCore`, add:

```rust
tray_state: std::sync::Arc<std::sync::RwLock<crate::system::SystemState>>,
```

Change `SessionCore::new` to take it:

```rust
pub fn new(w: i32, h: i32, cfg: Config, tray_state: std::sync::Arc<std::sync::RwLock<crate::system::SystemState>>) -> Self {
    // ... existing fields ...
    // add: tray_state,
}
```

- [ ] **Step 2: Start the poller in the daemon and pass the handle**

In `src/daemon.rs::run`, after loading config:

```rust
let poller = crate::poller::SystemPoller::start();
let mut core = SessionCore::new(w, h, cfg.clone(), poller.state());
```

Keep `poller` alive for the function's duration (bind it; do not drop).

- [ ] **Step 3: Build**

Run: `cargo build --offline`
Expected: compiles.

- [ ] **Step 4: Run the full suite**

Run: `cargo test --offline`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/session.rs src/daemon.rs src/chrome.rs
git commit -m "tray: render indicators in the menubar; start poller in daemon"
```

---

### Task 10: Popovers — render + click → ControlIntent

**Files:**
- Modify: `src/tray.rs`
- Test: `tests/tray_tests.rs` (append)

- [ ] **Step 1: Write the failing test**

Append to `tests/tray_tests.rs`:

```rust
use tuiui::geometry::Point;
use tuiui::tray::{Tray, PopoverHit};
use tuiui::system::ControlIntent;

#[test]
fn clicking_a_segment_opens_its_popover() {
    let mut tray = Tray::new();
    let segs = tray_segments(&sample(), 100);
    let vol = segs.iter().find(|s| s.kind == SegmentKind::Volume).unwrap();
    tray.on_menubar_click(Point::new(vol.rect.x, 0), &segs);
    assert_eq!(tray.open(), Some(SegmentKind::Volume));
}

#[test]
fn volume_popover_plus_minus_yield_intents() {
    let mut tray = Tray::new();
    tray.force_open(SegmentKind::Volume);
    let r = tray.render(100, 30, &sample());
    // The popover exposes clickable hot-zones; find the "+" zone.
    let plus = r.hits.iter().find(|h| matches!(h.intent, ControlIntent::VolumeUp)).unwrap();
    let intent = tray.on_popover_click(plus.rect.center(), &r);
    assert_eq!(intent, Some(ControlIntent::VolumeUp));
}
```

(Add a `center()` helper to `Rect` if not present: returns the integer midpoint.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --offline --test tray_tests`
Expected: FAIL — `Tray`, `PopoverHit`, methods not found.

- [ ] **Step 3: Implement the `Tray` widget + popovers**

In `src/tray.rs`, add the `Tray` state machine: tracks the open `SegmentKind`, renders the matching popover (reusing the launcher's `fill_box` look — copy the small helper or factor it into a shared `widgets` module), and exposes `PopoverHit { rect, intent }` zones. Provide `new()`, `open()`, `force_open(kind)`, `on_menubar_click(p, segments)`, `render(w,h,state) -> Rendered { layers, hits }`, and `on_popover_click(p, rendered) -> Option<ControlIntent>`. Volume popover hot-zones: `VolumeDown` (◂), `VolumeUp` (▸), `ToggleMute` (speaker); WiFi rows → `WifiConnectKnown(ssid)` + a toggle → `WifiSetEnabled`; Bluetooth rows → `BtConnect`; clock/cpu/mem/battery → read-only (no hits).

(Full code mirrors `launcher.rs`'s render/hit-test structure; the assistant executing this task implements it following that established pattern, keeping every popover ≤ the screen and anchored under its segment.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --offline --test tray_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tray.rs tests/tray_tests.rs
git commit -m "tray: popovers (volume/wifi/bluetooth) + click→ControlIntent"
```

---

### Task 11: Session wiring — open popovers, dispatch intents, optimistic update

**Files:**
- Modify: `src/session.rs`

- [ ] **Step 1: Hit-test the tray on MouseDown before window routing**

In `handle_mouse`, before the menubar brand/quit checks, when `MouseDown`:
1. If a popover is open, route the click through `tray.on_popover_click`; if it yields a `ControlIntent`, call `self.apply_intent(intent)` and return; otherwise close the popover.
2. Else, if the click is on row 0, call `tray.on_menubar_click(p, &segs)` (segments computed from the snapshot) and return when it opened/closed a popover.

- [ ] **Step 2: Implement `apply_intent` with optimistic update**

```rust
fn apply_intent(&mut self, intent: crate::system::ControlIntent) {
    use crate::system::ControlIntent as I;
    // Optimistic cache update so the UI responds before the next poll.
    if let Ok(mut s) = self.tray_state.write() {
        match &intent {
            I::VolumeUp => s.volume.level = s.volume.level.saturating_add(5).min(100),
            I::VolumeDown => s.volume.level = s.volume.level.saturating_sub(5),
            I::VolumeSet(l) => s.volume.level = (*l).min(100),
            I::ToggleMute => s.volume.muted = !s.volume.muted,
            I::WifiSetEnabled(on) => { if let Some(w) = s.wifi.as_mut() { w.enabled = *on; } }
            I::BtSetEnabled(on) => s.bluetooth.enabled = *on,
            _ => {}
        }
    }
    self.backend.apply(&intent);
}
```

Add a `backend: Box<dyn crate::system::Backend>` field to `SessionCore`, built in `new` via `crate::system::backend()`.

- [ ] **Step 3: Render the open popover in `build_frame`**

After pushing the dock, push the tray popover layers (above chrome), like the launcher:

```rust
let pop = { let st = self.tray_state.read().unwrap(); self.tray.render(self.w, self.h, &st) };
layers.extend(pop.layers);
```

- [ ] **Step 4: Build + test**

Run: `cargo build --offline && cargo test --offline`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/session.rs
git commit -m "tray: session wiring — open popovers, dispatch intents, optimistic UI"
```

---

### Task 12: OS-aware optional dependency installer (blueutil)

**Files:**
- Modify: `install.sh`

- [ ] **Step 1: Add an opt-in dependency step**

Append to `install.sh`, after the binary is installed, a step that — only when `TUIUI_INSTALL_DEPS=1` (or an interactive TTY) and a package manager is present — installs `blueutil` on macOS:

```sh
install_optional_deps() {
  [ "${TUIUI_SKIP_DEPS:-0}" = "1" ] && return 0
  # In a piped non-interactive install, require explicit opt-in.
  if [ ! -t 0 ] && [ "${TUIUI_INSTALL_DEPS:-0}" != "1" ]; then return 0; fi
  case "$(uname -s)" in
    Darwin)
      if command -v brew >/dev/null 2>&1 && ! command -v blueutil >/dev/null 2>&1; then
        echo "tuiui: installing optional dependency blueutil (Bluetooth control)…"
        brew install blueutil || echo "tuiui: blueutil install skipped (you can run 'brew install blueutil' later)"
      fi ;;
    Linux) : ;; # bluetoothctl/rfkill ship with the distro
  esac
}
install_optional_deps
```

- [ ] **Step 2: Lint the script**

Run: `sh -n install.sh`
Expected: no syntax errors.

- [ ] **Step 3: Commit**

```bash
git add install.sh
git commit -m "tray: installer optionally installs blueutil (opt-in, OS-aware)"
```

---

### Task 13: Final verification

- [ ] **Step 1: Full build, clippy, tests**

Run: `cargo build --offline && cargo clippy --offline --all-targets && cargo test --offline`
Expected: builds, zero clippy warnings, all tests pass.

- [ ] **Step 2: Manual smoke (on the mini)**

`tuiui kill ; tuiui` → confirm the menubar shows clock + CPU/mem + volume + WiFi; clicking volume opens a popover; ◂/▸ change the level; WiFi popover lists known networks. Battery is absent (correct on the mini). Bluetooth popover shows a hint if `blueutil` is missing.

- [ ] **Step 3: Commit any fixups**

```bash
git commit -am "tray: smoke-test fixups"
```

---

## Notes for the implementer

- Run shell-outs only via `run_capped` (Task 3) — never spawn an un-timed command on a path that the render loop can reach.
- The poller thread reads `sysinfo` and shells out; the session/render never blocks on the backend except `apply_intent`, which calls fast commands (≤ 5s timeout) and updates the cache optimistically first.
- Keep every popover within screen bounds and anchored under its segment, clamped at the right edge, mirroring `launcher.rs`.
- Battery stays `None` for now (valid: the mini has none); populating it on laptop hosts is a follow-up, not part of this plan.
