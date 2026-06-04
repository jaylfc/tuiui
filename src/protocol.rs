//! Wire protocol between the tuiui daemon and a thin client over a local Unix
//! socket. Messages are newline-delimited JSON.
//!
//! - **Client → daemon:** [`crate::session::ClientMsg`] (input, resize, shutdown).
//! - **Daemon → client:** [`FrameMsg`] — a frame diff plus the UI-state flags the
//!   client needs to route keyboard input (which overlay is open, etc.).

use crate::compositor::CellChange;
use crate::geometry::Point;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Daemon → client frame: changed cells, cursor position, and routing flags.
#[derive(Serialize, Deserialize)]
pub struct FrameMsg {
    pub changes: Vec<CellChange>,
    pub cursor: Option<Point>,
    pub flags: Flags,
}

/// UI-state flags the client uses to decide where keyboard input goes. They lag
/// the daemon by one frame, which is imperceptible for typing.
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct Flags {
    pub launcher_open: bool,
    pub spotlight_open: bool,
    pub store_focused: bool,
    pub settings_focused: bool,
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
