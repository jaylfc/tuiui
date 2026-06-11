//! Wayland compositor backend using smithay.
//!
//! This module provides a Wayland compositor that renders tuiui app windows
//! using KMS/DRM (with graceful fallback) and implements the necessary
//! Wayland protocols (xdg-shell, layer-shell, wl_seat, etc.).

mod compositor;
mod backend;
mod protocols;
mod input;

pub use compositor::{WaylandCompositor, OutputId, SeatId, LayerType, Anchor, CompositorState, SeatState, OutputInfo};
pub use backend::{DrmBackend, DrmBuffer, DrmFormat, DrmLease, DrmDeviceHandle};
pub use protocols::{XdgShellSurface as XdgSurface, LayerShellSurface as LayerSurfaceProtocol, Seat, Pointer, Keyboard, Touch, CursorIcon, ProtocolManager};
pub use input::{InputManager, InputConfig, DeviceInfo, KeyboardLayout, ModifierState, SeatData, VtSwitchHandler, InputSystemInfo, enumerate_input_devices};

use std::io::{self, Result};

/// Set compositor environment variables for Wayland session integration.
fn set_compositor_env() {
    if std::env::var("WAYLAND_DISPLAY").is_err() {
        std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
    }
    if std::env::var("XDG_CURRENT_DESKTOP").is_err() {
        std::env::set_var("XDG_CURRENT_DESKTOP", "tuiui");
    }
    if std::env::var("XDG_SESSION_TYPE").is_err() {
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
    }
}

/// Run the Wayland compositor. This is the main entry point called from `main.rs`.
pub fn run_compositor() -> Result<()> {
    set_compositor_env();
    WaylandCompositor::new()?.run()
}