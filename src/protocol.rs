//! Wire protocol between the tuiui daemon and a thin client over a local Unix
//! socket. Messages are newline-delimited JSON.
//!
//! - **Client → daemon:** [`crate::session::ClientMsg`] (input, resize, shutdown).
//! - **Daemon → client:** [`FrameMsg`] — a frame diff plus the UI-state flags the
//!   client needs to route keyboard input (which overlay is open, etc.).

use crate::compositor::CellChange;
use crate::geometry::{Point, Rect};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Daemon → client frame: changed cells, cursor position, routing flags, and the
/// image layer (placements + any first-time image data).
#[derive(Serialize, Deserialize)]
pub struct FrameMsg {
    pub changes: Vec<CellChange>,
    pub cursor: Option<Point>,
    pub flags: Flags,
    /// Image placements for this frame (where each visible image goes).
    #[serde(default)]
    pub images: Vec<ImagePlacement>,
    /// PNG bytes for images not yet sent to this client (base64), sent once.
    #[serde(default)]
    pub image_data: Vec<ImageBlob>,
}

/// A request to place image `id` at `rect` (screen cells). `visible=false` tells
/// the client to remove the placement (occluded or window closed).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ImagePlacement {
    pub id: u64,
    pub rect: Rect,
    pub cols: u16,
    pub rows: u16,
    pub visible: bool,
}

/// PNG bytes for image `id`, base64-encoded, sent once per attach.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ImageBlob {
    pub id: u64,
    pub png_base64: String,
}

/// UI-state flags the client uses to decide where keyboard input goes. They lag
/// the daemon by one frame, which is imperceptible for typing.
///
/// `#[serde(default)]` makes the struct tolerant of version skew: a client built
/// against a newer `Flags` can still parse frames from an older daemon (missing
/// fields default to `false`) instead of dropping every frame and rendering a
/// blank desktop.
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
#[serde(default)]
pub struct Flags {
    pub launcher_open: bool,
    pub spotlight_open: bool,
    pub store_focused: bool,
    pub settings_focused: bool,
    /// The settings panel is in a text-entry field (Apps section add/edit form),
    /// so the client forwards typed characters instead of treating them as
    /// navigation commands.
    pub settings_editing: bool,
    /// The working-directory picker overlay is open (the client routes navigation
    /// keys to it).
    pub dirpicker_open: bool,
    /// The picker's new-folder name input is active (client forwards typed chars).
    pub dirpicker_creating: bool,
    /// The keyboard-shortcut help overlay is showing (any key dismisses it).
    pub help_open: bool,
    /// The file-manager window is focused; the client routes navigation keys to it.
    pub filemanager_focused: bool,
    /// The file manager has a text overlay open (new-folder / rename); forward chars.
    pub filemanager_editing: bool,
    /// The desktop has a rename/new-folder overlay open; forward typed chars.
    pub desktop_editing: bool,
    /// The daemon asked the client to detach (e.g. the Quit button was clicked).
    pub detach: bool,
}

/// Per-user directory that holds the daemon socket. Created mode `0700` by the
/// daemon so other local users cannot reach the socket inside it.
pub fn socket_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    base.join(format!("tuiui-{user}"))
}

/// Path of the per-user daemon socket.
pub fn socket_path() -> PathBuf {
    socket_dir().join("daemon.sock")
}

/// Path to the apphost socket (apps live behind this; survives frontend restarts).
pub fn apphost_socket_path() -> PathBuf {
    socket_dir().join("apphost.sock")
}
