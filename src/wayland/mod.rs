//! Wayland compositor backend using smithay.
//!
//! This module provides a Wayland compositor that renders tuiui app windows
//! using KMS/DRM (with graceful fallback) and implements the necessary
//! Wayland protocols (xdg-shell, layer-shell, wl_seat, etc.).

mod compositor;
mod backend;
mod protocols;

pub use compositor::{WaylandCompositor, OutputId, SeatId, LayerType, Anchor, CompositorState, SeatState, OutputInfo};
pub use backend::{DrmBackend, DrmBuffer, DrmFormat, DrmLease, DrmDeviceHandle};
pub use protocols::{XdgShellSurface as XdgSurface, LayerShellSurface as LayerSurfaceProtocol, Seat, Pointer, Keyboard, Touch, CursorIcon, ProtocolManager};

use std::io::{self, Result};

/// Run the Wayland compositor. This is the main entry point called from `main.rs`.
pub fn run_compositor() -> Result<()> {
    WaylandCompositor::new()?.run()
}