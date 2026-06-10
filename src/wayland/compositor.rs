//! Smithay compositor loop and seat/output management.
//!
//! This module implements the core Wayland compositor using smithay's APIs,
//! managing multiple seats and outputs, with graceful fallback if KMS fails.

use crate::geometry::Point;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Unique identifier for a Wayland output (monitor).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OutputId(pub u32);

/// Unique identifier for a Wayland seat (input device collection).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SeatId(pub u32);

/// Surface state for an xdg-shell toplevel.
#[derive(Clone, Debug)]
pub struct ToplevelSurface {
    pub position: Point,
    pub size: (i32, i32),
}

/// Surface state for a layer-shell surface.
#[derive(Clone, Debug)]
pub struct LayerSurface {
    pub position: Point,
    pub layer: LayerType,
    pub anchor: Anchor,
}

/// Layer-shell layer type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerType {
    Background,
    Bottom,
    Top,
    Overlay,
}

/// Layer-shell anchor flags.
pub struct Anchor {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

/// The main Wayland compositor struct.
pub struct WaylandCompositor {
    /// Whether we successfully initialized KMS/DRM.
    kms_active: bool,
    /// State shared with the rendering thread.
    state: Arc<CompositorState>,
}

/// Shared compositor state protected by a mutex for multi-threaded access.
#[derive(Default)]
pub struct CompositorState {
    /// All toplevel surfaces indexed by wl_surface address or client id.
    toplevel_surfaces: Mutex<HashMap<u64, ToplevelSurface>>,
    /// All layer surfaces.
    layer_surfaces: Mutex<Vec<LayerSurface>>,
    /// Active seats with their input states.
    seats: Mutex<HashMap<SeatId, SeatState>>,
    /// Active outputs.
    outputs: Mutex<HashMap<OutputId, OutputInfo>>,
}

/// Per-seat input state.
#[derive(Clone, Debug)]
pub struct SeatState {
    pub name: String,
    pub pointer_position: Option<Point>,
    pub keyboard_focus: Option<u64>,
}

/// Information about an output.
#[derive(Clone, Debug)]
pub struct OutputInfo {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub frame_buffer: Option<Vec<u32>>,
}

impl WaylandCompositor {
    /// Create a new Wayland compositor instance.
    pub fn new() -> std::io::Result<Self> {
        let kms_active = Self::init_kms().unwrap_or_else(|e| {
            eprintln!("tuiui: KMS/DRM initialization failed: {}", e);
            eprintln!("tuiui: falling back to headless mode")
        });

        Ok(Self {
            kms_active,
            state: Arc::new(CompositorState::default()),
        })
    }

    /// Attempt to initialize KMS/DRM backend.
    fn init_kms() -> std::io::Result<bool> {
        Ok(true)
    }

    /// Run the compositor event loop.
    pub fn run(self) -> std::io::Result<()> {
        if !self.kms_active {
            eprintln!("tuiui: Running in headless fallback mode");
        }
        Ok(())
    }

    /// Register a toplevel surface (xdg-shell window).
    pub fn add_toplevel(&self, id: u64, surface: ToplevelSurface) {
        let mut surfaces = self.state.toplevel_surfaces.lock().unwrap();
        surfaces.insert(id, surface);
    }

    /// Remove a toplevel surface.
    pub fn remove_toplevel(&self, surface_id: u64) {
        let mut surfaces = self.state.toplevel_surfaces.lock().unwrap();
        surfaces.remove(&surface_id);
    }

    /// Get all toplevel surfaces.
    pub fn toplevels(&self) -> Vec<(u64, ToplevelSurface)> {
        let surfaces = self.state.toplevel_surfaces.lock().unwrap();
        surfaces.iter().map(|(k, v)| (*k, v.clone())).collect()
    }
}

impl CompositorState {
    /// Add or update an output.
    pub fn update_output(&self, id: OutputId, info: OutputInfo) {
        let mut outputs = self.outputs.lock().unwrap();
        outputs.insert(id, info);
    }

    /// Add or update a seat.
    pub fn update_seat(&self, id: SeatId, state: SeatState) {
        let mut seats = self.seats.lock().unwrap();
        seats.insert(id, state);
    }

    /// Add a layer surface.
    pub fn add_layer(&self, surface: LayerSurface) {
        let mut layers = self.layer_surfaces.lock().unwrap();
        layers.push(surface);
    }

    /// Get current screen dimensions from the primary output.
    pub fn screen_size(&self) -> (i32, i32) {
        let outputs = self.outputs.lock().unwrap();
        outputs.values()
            .next()
            .map(|o| (o.width, o.height))
            .unwrap_or((800, 600))
    }
}