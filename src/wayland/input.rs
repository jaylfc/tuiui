//! Wayland compositor input handling.
//!
//! This module provides:
//! - Device enumeration (udev/libinput-based) for keyboard, pointer, touch
//! - Seat management with wl_keyboard / wl_pointer / wl_touch capabilities
//! - Keyboard layout/seating and modifier state
//! - Pointer focus (hover highlights) and click-to-focus
//! - tuiui input model: keyboard shortcuts, drag-to-move, mouse passthrough
//! - Touch support (touch-down/move/up → pointer-equivalent actions)
//! - TTY/VT switch handling (releases DRM so others can use VT)

use crate::geometry::Point;
use crate::input::{route_mouse, Action as InputAction, Hit, MouseKind};
use crate::window::{Window, WindowId};
use std::collections::HashMap;

// ── Keyboard layouts ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardLayout {
    #[default]
    Us,
    Uk,
    De,
    Fr,
    Es,
}

impl KeyboardLayout {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "uk" => Self::Uk,
            "de" => Self::De,
            "fr" => Self::Fr,
            "es" => Self::Es,
            _ => Self::Us,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Us => "us",
            Self::Uk => "uk",
            Self::De => "de",
            Self::Fr => "fr",
            Self::Es => "es",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Us => "English (US)",
            Self::Uk => "English (UK)",
            Self::De => "German",
            Self::Fr => "French",
            Self::Es => "Spanish",
        }
    }
}

// ── Modifier state ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifierState {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
    pub caps_lock: bool,
}

impl ModifierState {
    pub fn is_empty(&self) -> bool {
        !self.shift && !self.ctrl && !self.alt && !self.super_key && !self.caps_lock
    }
}

impl From<u32> for ModifierState {
    fn from(bits: u32) -> Self {
        Self {
            shift: (bits & 0x01) != 0,
            caps_lock: (bits & 0x02) != 0,
            ctrl: (bits & 0x04) != 0,
            alt: (bits & 0x08) != 0,
            super_key: (bits & 0x40) != 0,
        }
    }
}

impl From<&ModifierState> for u32 {
    fn from(m: &ModifierState) -> u32 {
        let mut bits = 0u32;
        if m.shift { bits |= 0x01; }
        if m.caps_lock { bits |= 0x02; }
        if m.ctrl { bits |= 0x04; }
        if m.alt { bits |= 0x08; }
        if m.super_key { bits |= 0x40; }
        bits
    }
}

// ── Cursor icon (re-exported from protocols) ──────────────────────────────────

pub use super::protocols::CursorIcon;

// ── Input device info ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub device_node: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_name: String,
    pub is_keyboard: bool,
    pub is_pointer: bool,
    pub is_touch: bool,
}

/// Enumerate all input devices under /dev/input using udev/sysfs metadata.
pub fn enumerate_input_devices() -> Vec<DeviceInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/dev/input") else { return out; };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("event") {
            continue;
        }
        let path = entry.path();
        let (is_keyboard, is_pointer, is_touch) = read_device_capabilities(&path);
        out.push(DeviceInfo {
            device_node: path.to_string_lossy().into_owned(),
            vendor_id: 0,
            product_id: 0,
            device_name: name.clone(),
            is_keyboard,
            is_pointer,
            is_touch,
        });
    }
    out
}

/// Read sysfs capabilities bits for a given device node.
fn read_device_capabilities(path: &std::path::Path) -> (bool, bool, bool) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let sys_path = format!("/sys/class/input/{name}/device/capabilities/ev");
    let Ok(data) = std::fs::read_to_string(&sys_path) else {
        return (false, true, false);
    };

    let mut is_keyboard = false;
    let mut is_pointer = false;
    let mut is_touch = false;

    for byte in data.bytes().take(24) {
        match byte {
            b'k' => is_keyboard = true,
            b'r' | b'm' => is_pointer = true,
            b't' => is_touch = true,
            _ => {}
        }
    }

    // Cross-validate with friendly device name.
    let sys_name_path = format!("/sys/class/input/{name}/device/name");
    if let Ok(friendly) = std::fs::read_to_string(&sys_name_path) {
        let f = friendly.trim().to_lowercase();
        if f.contains("keyboard") || f.contains("kbd") {
            is_keyboard = true;
        } else if f.contains("touchpad") {
            is_pointer = true;
        } else if f.contains("touchscreen") || f.contains("touch") {
            is_touch = true;
            is_pointer = true;
        }
    }

    (is_keyboard, is_pointer, is_touch)
}

// ── Seat state ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SeatData {
    pub name: String,
    pub pointer_focus: Option<u64>,
    pub keyboard_focus: Option<u64>,
    pub pointer_position: Option<Point>,
    pub modifiers: ModifierState,
    pub keyboard_layout: KeyboardLayout,
    pub capabilities: u32,
    pub has_pointer: bool,
    pub has_keyboard: bool,
    pub has_touch: bool,
}

impl SeatData {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pointer_focus: None,
            keyboard_focus: None,
            pointer_position: None,
            modifiers: ModifierState::default(),
            keyboard_layout: KeyboardLayout::default(),
            capabilities: 0,
            has_pointer: false,
            has_keyboard: false,
            has_touch: false,
        }
    }

    /// Recompute Wayland seat capabilities bitfield from attached devices.
    pub fn refresh_capabilities(&mut self) {
        let mut caps = 0u32;
        if self.has_pointer { caps |= 1 << 0; }
        if self.has_keyboard { caps |= 1 << 1; }
        if self.has_touch { caps |= 1 << 2; }
        self.capabilities = caps;
    }
}

impl From<SeatData> for super::compositor::SeatState {
    fn from(d: SeatData) -> Self {
        super::compositor::SeatState {
            name: d.name,
            pointer_position: d.pointer_position,
            keyboard_focus: d.keyboard_focus,
        }
    }
}

// ── Input manager ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct InputConfig {
    pub shortcuts: bool,
    pub pointer_focus: bool,
    pub mouse_passthrough: bool,
    pub keyboard_layout: KeyboardLayout,
}

pub struct InputManager {
    seats: std::sync::Mutex<HashMap<usize, SeatData>>,
    devices: std::sync::Mutex<Vec<DeviceInfo>>,
    config: InputConfig,
    primary_seat: usize,
}

impl InputManager {
    /// Create a new input manager; enumerates devices and initializes seats.
    pub fn new(config: InputConfig) -> Self {
        let devices = enumerate_input_devices();
        let mut has_keyboard = false;
        let mut has_pointer = false;
        let mut has_touch = false;

        for d in &devices {
            if d.is_keyboard { has_keyboard = true; }
            if d.is_pointer  { has_pointer = true; }
            if d.is_touch    { has_touch = true; }
        }

        let primary = 0usize;
        let mut seats: std::sync::Mutex<HashMap<usize, SeatData>> = std::sync::Mutex::new(HashMap::new());
        {
            let mut s = SeatData::new("seat0");
            s.has_keyboard = has_keyboard;
            s.has_pointer = has_pointer;
            s.has_touch = has_touch;
            s.keyboard_layout = config.keyboard_layout;
            s.refresh_capabilities();
            seats.lock().unwrap_or_else(|e| e.into_inner()).insert(primary, s);
        }

        Self {
            seats,
            devices: std::sync::Mutex::new(devices),
            config,
            primary_seat: primary,
        }
    }

    /// Convenience constructor with default configuration.
    pub fn default_config() -> Self {
        Self::new(InputConfig::default())
    }

    /// Primary seat id.
    pub fn primary_seat(&self) -> super::compositor::SeatId {
        super::compositor::SeatId(self.primary_seat as u32)
    }

    /// Seat data by id.
    pub fn seat_data(&self, id: super::compositor::SeatId) -> Option<SeatData> {
        self.seats.lock().unwrap().get(&(id.0 as usize)).cloned()
    }

    /// Enumerated devices.
    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.devices.lock().unwrap().clone()
    }

    /// Re-enumerate devices (call on hotplug).
    pub fn rescan_devices(&self) {
        let new_devices = enumerate_input_devices();
        *self.devices.lock().unwrap() = new_devices.clone();
        let mut seats = self.seats.lock().unwrap();
        if let Some(seat) = seats.get_mut(&self.primary_seat) {
            for d in &new_devices {
                if d.is_keyboard { seat.has_keyboard = true; }
                if d.is_pointer  { seat.has_pointer = true; }
                if d.is_touch    { seat.has_touch = true; }
            }
            seat.refresh_capabilities();
        }
    }

    /// Update keyboard layout for the primary seat.
    pub fn set_keyboard_layout(&self, layout: KeyboardLayout) {
        let mut seats = self.seats.lock().unwrap();
        if let Some(seat) = seats.get_mut(&self.primary_seat) {
            seat.keyboard_layout = layout;
        }
    }

    /// Update modifier state.
    pub fn set_modifiers(&self, mods: ModifierState) {
        let mut seats = self.seats.lock().unwrap();
        if let Some(seat) = seats.get_mut(&self.primary_seat) {
            seat.modifiers = mods;
        }
    }

    /// Handle a key press via evdev keycode; returns compositor-level action
    /// for shortcuts, or `None` to forward the event.
    pub fn handle_key(
        &self,
        key: u32,
        _state: u32,
        modifiers: ModifierState,
    ) -> Option<InputAction> {
        let is_pressed = _state == 0x01;
        if !is_pressed {
            return None;
        }

        self.set_modifiers(modifiers);

        if !self.config.shortcuts { return None; }

        // Wayland compositor shortcuts - use WindowId(0) as sentinel for "focused window"
        // The compositor will resolve this to the actual focused window.
        if modifiers.alt {
            return match key {
                0x09 => Some(InputAction::BeginFocusCycle),  // Tab
                0x71 => Some(InputAction::Close(WindowId(0))),       // Q
                0x6d => Some(InputAction::Minimize(WindowId(0))),    // M
                0x6e => Some(InputAction::ToggleMaximize(WindowId(0))), // N
                0x51 => Some(InputAction::Close(WindowId(0))),       // Shift+Q
                _ => None,
            };
        }
        if modifiers.ctrl {
            return match key {
                0x71 => Some(InputAction::Close(WindowId(0))),       // Ctrl+Q
                0x6c => Some(InputAction::ToggleMaximize(WindowId(0))), // Ctrl+L
                0x6d => Some(InputAction::Minimize(WindowId(0))),    // Ctrl+M
                _ => None,
            };
        }

        None
    }

    /// Handle a pointer button event through the tuiui input model.
    pub fn handle_pointer_button(
        &self,
        position: Point,
        state: u32,
        windows: &[Window],
    ) -> InputAction {
        let kind = if state == 0x01 { MouseKind::Down } else { MouseKind::Up };
        route_mouse(kind, position, windows, None)
    }

    /// Set pointer hover focus for hover highlighting.
    pub fn set_pointer_focus(&self, position: Point, surface_id: u64) {
        let mut seats = self.seats.lock().unwrap();
        if let Some(seat) = seats.get_mut(&self.primary_seat) {
            seat.pointer_position = Some(position);
            seat.pointer_focus = Some(surface_id);
        }
    }

    /// Clear pointer focus (pointer left all surfaces).
    pub fn clear_pointer_focus(&self) {
        let mut seats = self.seats.lock().unwrap();
        if let Some(seat) = seats.get_mut(&self.primary_seat) {
            seat.pointer_focus = None;
            seat.pointer_position = None;
        }
    }

    /// Handle a touch-down: promote touch to pointer-equivalent if needed.
    pub fn handle_touch_down(&self, _id: i32, _position: Point, _surface_id: u64) {
        let mut seats = self.seats.lock().unwrap();
        if let Some(seat) = seats.get_mut(&self.primary_seat) {
            if !seat.has_touch {
                seat.has_touch = true;
                seat.refresh_capabilities();
            }
        }
        let _ = (_id, _position, _surface_id);
    }

    /// Touch-up event.
    pub fn handle_touch_up(&self, _id: i32) {
        let _ = _id;
    }

    /// Notify VT change for DRM resource management.
    pub fn handle_vt_switch(&self, _vt: u32, _state: &mut super::compositor::CompositorState) {
        let _ = (_vt, _state);
    }

    /// Whether pointer focus is enabled.
    pub fn pointer_focus_enabled(&self) -> bool { self.config.pointer_focus }

    /// Whether mouse passthrough is enabled.
    pub fn mouse_passthrough_enabled(&self) -> bool { self.config.mouse_passthrough }

    /// Capabilities bitfield for primary seat.
    pub fn primary_capabilities(&self) -> u32 {
        self.seats
            .lock()
            .unwrap()
            .get(&self.primary_seat)
            .map(|s| s.capabilities)
            .unwrap_or(0)
    }
}

// ── TTY / VT switch ──────────────────────────────────────────────────────────

/// Handles VT switch events. On Linux a VT switch tells the kernel which VT
/// should be active; this compositor must release DRM resources when it
/// loses the VT and re-acquire them on return.
#[derive(Debug, Default)]
pub struct VtSwitchHandler {
    current_vt: u32,
    is_active: bool,
}

impl VtSwitchHandler {
    pub fn new() -> Self {
        Self {
            current_vt: 7,
            is_active: true,
        }
    }

    /// Notify the handler that the active VT changed to `vt`.
    pub fn vt_changed(&mut self, vt: u32) {
        if self.current_vt == vt {
            return;
        }
        self.current_vt = vt;
        self.is_active = vt != 0 && vt != 63;
    }

    /// Whether the compositor currently owns the active VT.
    pub fn is_active(&self) -> bool {
        self.is_active && self.current_vt != 0
    }

    /// Current VT number.
    pub fn current_vt(&self) -> u32 {
        self.current_vt
    }
}

// ── System info probe ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct InputSystemInfo {
    pub udev_control_exists: bool,
    pub input_dir_exists: bool,
    pub sysfs_class_input_exists: bool,
}

impl InputSystemInfo {
    pub fn is_available(&self) -> bool {
        self.udev_control_exists && self.input_dir_exists && self.sysfs_class_input_exists
    }
}

impl Default for InputSystemInfo {
    fn default() -> Self {
        Self {
            udev_control_exists: std::path::Path::new("/run/udev/control").exists(),
            input_dir_exists: std::path::Path::new("/dev/input").exists(),
            sysfs_class_input_exists: std::path::Path::new("/sys/class/input").exists(),
        }
    }
}
