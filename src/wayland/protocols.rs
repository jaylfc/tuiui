//! Wayland protocol implementations: xdg-shell, layer-shell, wl_seat, wl_keyboard, wl_pointer.
//!
//! This module implements the necessary Wayland protocols for the compositor.

use crate::geometry::Point;
use crate::buffer::CellBuffer;

/// xdg-shell surface implementation.
pub struct XdgShellSurface {
    pub surface_id: u64,
    pub toplevel_id: u64,
    pub buffer: Option<CellBuffer>,
    pub title: String,
    pub app_id: String,
    pub maximized: bool,
    pub minimized: bool,
    pub fullscreen: bool,
    pub resizable: bool,
    pub movable: bool,
}

impl XdgShellSurface {
    pub fn new(surface_id: u64, app_id: &str) -> Self {
        Self {
            surface_id,
            toplevel_id: surface_id + 1,
            buffer: None,
            title: String::new(),
            app_id: app_id.to_string(),
            maximized: false,
            minimized: false,
            fullscreen: false,
            resizable: true,
            movable: true,
        }
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
    }

    pub fn set_buffer(&mut self, buf: CellBuffer) {
        self.buffer = Some(buf);
    }

    pub fn size(&self) -> (i32, i32) {
        self.buffer.as_ref().map(|b| (b.width(), b.height())).unwrap_or((80, 24))
    }
}

/// layer-shell surface implementation.
pub struct LayerShellSurface {
    pub surface_id: u64,
    pub layer: super::compositor::LayerType,
    pub anchor: super::compositor::Anchor,
    pub exclusive_zone: i32,
    pub margin: (i32, i32, i32, i32),
    pub buffer: Option<CellBuffer>,
}

impl LayerShellSurface {
    pub fn new(surface_id: u64, layer: super::compositor::LayerType) -> Self {
        Self {
            surface_id,
            layer,
            anchor: super::compositor::Anchor { top: false, bottom: false, left: false, right: false },
            exclusive_zone: 0,
            margin: (0, 0, 0, 0),
            buffer: None,
        }
    }
}

/// wl_seat implementation for multi-seat input.
pub struct Seat {
    pub seat_id: u64,
    pub name: String,
    pub has_pointer: bool,
    pub has_keyboard: bool,
    pub has_touch: bool,
}

impl Seat {
    pub fn new(seat_id: u64, name: &str) -> Self {
        Self {
            seat_id,
            name: name.to_string(),
            has_pointer: false,
            has_keyboard: false,
            has_touch: false,
        }
    }

    pub fn create_pointer(&mut self) -> Pointer {
        self.has_pointer = true;
        Pointer::new(self.seat_id)
    }

    pub fn create_keyboard(&mut self) -> Keyboard {
        self.has_keyboard = true;
        Keyboard::new(self.seat_id)
    }

    pub fn create_touch(&mut self) -> Touch {
        self.has_touch = true;
        Touch::new(self.seat_id)
    }
}

/// wl_pointer implementation.
pub struct Pointer {
    pub seat_id: u64,
    pub focus_surface: Option<u64>,
    pub cursor_icon: CursorIcon,
    pub position: Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorIcon {
    Default,
    Pointer,
    Text,
    Move,
    ResizeH,
    ResizeV,
    ResizeF,
}

impl Pointer {
    pub fn new(seat_id: u64) -> Self {
        Self {
            seat_id,
            focus_surface: None,
            cursor_icon: CursorIcon::Default,
            position: Point::new(0, 0),
        }
    }

    pub fn enter(&mut self, surface_id: u64, position: Point) {
        self.focus_surface = Some(surface_id);
        self.position = position;
    }

    pub fn leave(&mut self) {
        self.focus_surface = None;
    }
}

/// wl_keyboard implementation.
pub struct Keyboard {
    pub seat_id: u64,
    pub focus_surface: Option<u64>,
}

impl Keyboard {
    pub fn new(seat_id: u64) -> Self {
        Self {
            seat_id,
            focus_surface: None,
        }
    }

    pub fn enter(&mut self, surface_id: u64) {
        self.focus_surface = Some(surface_id);
    }

    pub fn leave(&mut self) {
        self.focus_surface = None;
    }
}

/// wl_touch implementation.
pub struct Touch {
    pub seat_id: u64,
    pub focus_surface: Option<u64>,
}

impl Touch {
    pub fn new(seat_id: u64) -> Self {
        Self {
            seat_id,
            focus_surface: None,
        }
    }
}

/// Protocol manager - tracks all protocol state.
pub struct ProtocolManager {
    xdg_surfaces: Vec<XdgShellSurface>,
    layer_surfaces: Vec<LayerShellSurface>,
    seats: Vec<Seat>,
}

impl Default for ProtocolManager {
    fn default() -> Self {
        Self {
            xdg_surfaces: Vec::new(),
            layer_surfaces: Vec::new(),
            seats: Vec::new(),
        }
    }
}

impl ProtocolManager {
    pub fn new_toplevel(&mut self, app_id: &str) -> u64 {
        let id = self.xdg_surfaces.len() as u64;
        self.xdg_surfaces.push(XdgShellSurface::new(id, app_id));
        id
    }

    pub fn new_layer(&mut self, layer: super::compositor::LayerType) -> u64 {
        let id = (self.layer_surfaces.len() as u64).wrapping_add(0x10000);
        self.layer_surfaces.push(LayerShellSurface::new(id, layer));
        id
    }

    pub fn new_seat(&mut self, name: &str) -> u64 {
        let id = self.seats.len() as u64;
        self.seats.push(Seat::new(id, name));
        id
    }

    pub fn toplevels(&self) -> &[XdgShellSurface] {
        &self.xdg_surfaces
    }

    pub fn layers(&self) -> &[LayerShellSurface] {
        &self.layer_surfaces
    }

    pub fn seats(&self) -> &[Seat] {
        &self.seats
    }
}