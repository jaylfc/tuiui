# Menubar Tray + Interactive System Managers — Design

**Status:** Approved design (2026-06-04)

**Goal:** Add a macOS-style menubar status tray to tuiui — clock, WiFi, Bluetooth,
volume, battery, and CPU/memory indicators — with click-through popovers that
control the **host's** volume, switch to a known WiFi network, and connect a
paired Bluetooth device.

**Architecture (chosen):** Daemon-side tray widgets (mirroring the existing
launcher/store/settings) backed by a throttled background poller and an
OS-pluggable system backend. Portable metrics come from the `sysinfo` crate;
WiFi/volume/Bluetooth go through per-OS command backends behind a trait.

**Tech stack:** Rust 2021, `sysinfo` crate (new dependency), `std::process::Command`
for OS control commands (timeout-guarded), existing compositor/chrome/session
plumbing.

---

## Scope

**In scope (v1):**
- Right-aligned menubar indicators: **clock + date, WiFi, Bluetooth, volume,
  battery (auto-hidden when absent), CPU/memory load.**
- Click-through popovers anchored under each indicator.
- Control limited to the reliable, cross-platform tier:
  - Volume up / down / mute (host output).
  - WiFi on/off and switch to an **already-known** network.
  - Bluetooth on/off and connect an **already-paired** device.
- macOS and Linux backends behind one trait.

**Out of scope (deferred, YAGNI):**
- Joining **new** WiFi networks / password entry.
- Bluetooth **pairing** of new devices.
- DBus / native helpers (NetworkManager, BlueZ, CoreWLAN) — the trait is shaped
  so these can replace the command backends later without touching the tray.
- Audio transport over SSH (separate track).
- Keyboard-driven tray navigation beyond a single open/close shortcut (mouse is
  the primary surface; keyboard hooks are present but minimal).

## Why daemon-side

tuiui is daemon-on-host + thin-client. The tray therefore reflects the **host's**
state (e.g. the headless Mac mini), which is exactly the useful thing: manage the
mini's WiFi/volume/Bluetooth from the menubar over SSH. Rendering the tray
daemon-side means it ships to the client as ordinary compositor cells — **no new
display protocol** — and clicks arrive through the existing `MouseDown(p)` path,
just like the launcher.

## Module layout

- **`src/system/mod.rs`** — `SystemState` snapshot type; `SystemMonitor` (read)
  and `SystemControl` (write) traits; runtime backend selection
  (`#[cfg(target_os)]`); portable metric collection via `sysinfo`.
- **`src/system/macos.rs`** — macOS backend (`osascript`, `networksetup`,
  `blueutil` when present).
- **`src/system/linux.rs`** — Linux backend (`wpctl`/`pactl`, `nmcli`,
  `bluetoothctl`/`rfkill`).
- **`src/poller.rs`** — `SystemPoller`: owns a background thread that refreshes a
  shared `Arc<RwLock<SystemState>>` on throttled cadences with a hard timeout on
  every shell-out.
- **`src/tray.rs`** — the tray widget (mirrors `launcher.rs`): owns the open
  popover state, renders the indicator segments + popover layers, exposes hit
  regions, and converts popover clicks into `ControlIntent`s.
- **`src/chrome.rs`** — `render_menubar` draws the tray segments between the
  focused-app title and the Quit button; new `tray_hit_regions()`.
- **`src/session.rs`** — owns the tray widget and the shared snapshot handle;
  hit-tests the tray on `MouseDown` before window routing; dispatches popover
  `ControlIntent`s to the backend and optimistically updates the cached snapshot.

## Data flow

1. The poller thread (daemon) periodically reads metrics (`sysinfo` + per-OS
   commands, each timeout-guarded) and writes a fresh `SystemState` into the
   shared `RwLock`.
2. Each render tick, `SessionCore::build_frame` reads the latest snapshot, asks
   `tray` to render its segments (passed into `render_menubar`) and any open
   popover layer (z above chrome, like the launcher).
3. The client receives cells as usual. A `MouseDown(p)` is hit-tested against the
   tray segments and any open popover **before** window routing.
4. A popover control click resolves to a `ControlIntent`; the session calls the
   backend `SystemControl` method and **optimistically** updates the cached
   snapshot so the UI responds immediately. The next poll reconciles reality.

## `SystemState` snapshot

```rust
pub struct SystemState {
    pub clock: ClockInfo,                  // host local time + date + uptime secs
    pub cpu_pct: f32,                      // 0.0..=100.0 aggregate
    pub mem: MemInfo,                      // used / total bytes
    pub battery: Option<BatteryInfo>,      // None when the host has no battery
    pub wifi: Option<WifiInfo>,            // None when there is no WiFi interface
    pub bluetooth: BluetoothInfo,
    pub volume: VolumeInfo,
    pub known_networks: Vec<String>,       // for the WiFi popover
    pub backend_caps: BackendCaps,         // which controls are available
}

pub struct ClockInfo { pub time: String, pub date: String, pub uptime_secs: u64 }
pub struct MemInfo { pub used: u64, pub total: u64 }
pub struct BatteryInfo { pub pct: u8, pub charging: bool }
pub struct WifiInfo { pub ssid: String, pub signal: u8 /* 0..=4 bars */, pub enabled: bool }
pub struct BtDevice { pub name: String, pub addr: String, pub connected: bool }
pub struct BluetoothInfo { pub enabled: bool, pub devices: Vec<BtDevice> }
pub struct VolumeInfo { pub level: u8 /* 0..=100 */, pub muted: bool }
pub struct BackendCaps { pub volume: bool, pub wifi: bool, pub bluetooth: bool }
```

`backend_caps` records whether each control is actually available (e.g. `blueutil`
present on macOS). Indicators with no backing capability render read-only and
their popover shows a one-line hint instead of disabled controls.

## Menubar layout

```
✦ Tuiui  btop            ⊙32% ▤61%  🔋82%  ◂▮▮▮▯ 🔊  ⏻ bt  ▮▮▮◦ wlan  09:41   ✕ Quit
```

- Segments are fixed-order and right-aligned, drawn just left of the Quit button.
- Each segment is one hit region returned by `tray_hit_regions()`.
- The focused-app title (left) is truncated first when horizontal space is tight;
  if the menubar is still too narrow, lowest-priority segments (CPU/mem, then
  battery) drop out, clock is kept last.
- Colors come from `theme::current()`. Glyphs are chosen to render under the
  truecolor terminals tuiui already targets; a plain-ASCII fallback set is used
  when the segment would otherwise overflow.

## Popovers

Reuse the launcher dropdown visuals (`fill_box`, rows, rounded border). A popover
is anchored under its segment, right-edge-clamped to stay on-screen, at a z above
all chrome. Exactly one popover is open at a time.

- **Volume:** `◂ ▮▮▮▮▯▯ ▸` bar plus a mute toggle. Clicking `◂`/`▸` steps the
  level by 5; clicking the bar sets it proportionally; clicking the speaker
  toggles mute.
- **WiFi:** current SSID + signal, a Wi-Fi on/off toggle, then the list of known
  networks (click one to switch). Current network marked.
- **Bluetooth:** on/off toggle, then the list of paired devices with a
  connect/disconnect action per row.
- **Clock:** date + host uptime (read-only).
- **CPU/mem and Battery:** small read-only detail panels.

## Backends

```rust
pub trait SystemMonitor {
    /// Read the OS-specific parts of the snapshot (WiFi/volume/Bluetooth/known
    /// networks). Portable metrics are filled in by the caller via `sysinfo`.
    fn read(&self) -> SystemReadout;
    fn caps(&self) -> BackendCaps;
}

pub trait SystemControl {
    fn set_volume(&self, level: u8);
    fn toggle_mute(&self);
    fn wifi_set_enabled(&self, on: bool);
    fn wifi_connect_known(&self, ssid: &str);
    fn bt_set_enabled(&self, on: bool);
    fn bt_connect(&self, addr: &str, connect: bool);
}
```

- **macOS:** volume via `osascript -e 'set volume output volume N'` /
  `'output muted'`; WiFi status/switch/power via `networksetup`
  (`-getairportnetwork`, `-setairportnetwork`, `-setairportpower`) and
  `-listpreferredwirelessnetworks` for known networks; Bluetooth via `blueutil`
  when installed, otherwise read-only via `system_profiler SPBluetoothDataType`.
- **Linux:** volume via `wpctl`/`pactl`; WiFi via `nmcli` (`radio wifi`,
  `dev wifi connect`, status); Bluetooth via `bluetoothctl` / `rfkill`.
- Every command is built by a small pure helper that returns the argv (unit
  tested) and executed by the poller/session with a hard timeout. A missing tool
  flips the relevant `BackendCaps` bit off — never a panic.

## Optional dependencies & installer

Some controls need a small external tool that may not be present — currently only
**`blueutil`** (macOS Bluetooth control). The design keeps tuiui fully functional
without it (the BT popover degrades to read-only via `system_profiler`), but we
make the happy path easy to reach two ways:

1. **Installer (`install.sh`):** after placing the `tuiui` binary, an optional,
   **OS-aware** dependency step installs known helpers when a package manager is
   present:
   - macOS + Homebrew → `brew install blueutil`.
   - Linux → nothing required (BlueZ `bluetoothctl`/`rfkill` ship with the distro);
     if a future dep is needed, use the detected manager (`apt`/`dnf`/`pacman`).
   The step is **transparent and skippable**: it prints exactly what it will run,
   skips silently when no package manager is found, and honours
   `TUIUI_SKIP_DEPS=1` (and is auto-skipped in non-interactive `curl | sh` unless
   `TUIUI_INSTALL_DEPS=1` is set, so piping the installer never surprises a user
   with package installs). A `tuiui --install-deps` subcommand runs the same step
   on demand.
2. **Runtime offer:** when the user opens a control whose backing tool is missing
   (e.g. the Bluetooth popover with no `blueutil`), the popover shows a one-line
   hint with the exact install command, and — if a package manager is available —
   an "Install blueutil" action that spawns the install in a visible shell window
   (the same pattern the store and the in-app updater already use).

This keeps installs explicit and OS-correct, never silent, and never required.

## Error handling & safety

- All shell-outs run off the render thread (in the poller or as fire-and-forget
  control calls) and carry a hard timeout, so a hung `networksetup` can never
  freeze the desktop.
- Read failures keep the last-known value and mark the snapshot stale; the tray
  keeps rendering.
- Missing backend tools degrade to read-only with a hint; absent hardware
  (no battery / no WiFi) hides the segment.
- Control calls are best-effort: optimistic cache update now, reconciled on the
  next poll.

## Testing

Pure, deterministic units only — no process execution in CI:

- Segment layout + `tray_hit_regions()` math: given a `SystemState` and a width,
  assert each segment's rect and the drop-out order under narrow widths.
- Popover hit-testing: clicking a volume arrow / bar position / a WiFi row maps to
  the correct `ControlIntent`.
- Volume-slider arithmetic (step, proportional set, clamp 0..=100).
- Signal-strength → bars mapping and battery/percentage formatting.
- Per-OS **argv construction**: each `SystemControl` action builds the expected
  command arguments (assert the argv; do not execute).
- `BackendCaps` gating: a backend reporting `bluetooth: false` renders the BT
  popover in read-only form.

Actual command execution is covered only by manual/integration testing on a real
host, never in CI.

## Build sequence (informs the implementation plan)

1. `system` module: `SystemState` types + `sysinfo`-backed portable metrics +
   trait definitions + a no-op/stub backend.
2. `SystemPoller` with throttled cadences and timeouts; shared snapshot.
3. Tray rendering in `chrome.rs` + `tray.rs` (indicators only), wired into
   `build_frame`; hit regions.
4. Popovers (render + hit-testing) producing `ControlIntent`s.
5. macOS backend, then Linux backend (read + control), behind `BackendCaps`.
6. Session wiring: open/close popovers on click, dispatch intents, optimistic
   updates.
7. Tests at each layer per the Testing section.
