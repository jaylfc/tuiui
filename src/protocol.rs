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
    /// The screen was re-baselined (attach or resize): before applying this
    /// frame the client must erase all cells, delete every image placement
    /// (and its transmitted data), and forget its cached image state. A resize
    /// invalidates everything incremental — the emulator may have reflowed
    /// cells and kept placements the diff/delete stream no longer knows about.
    #[serde(default)]
    pub clear: bool,
    /// Set when the user picked a system in the power menu: the client should
    /// exit and hand this to `main`, which runs `ssh -t` in the real terminal
    /// (and the optional first-time setup). `None` on every normal frame.
    #[serde(default)]
    pub switch_to: Option<crate::systems::SwitchSpec>,
    /// Text the client should place on the HOST terminal's clipboard via
    /// OSC 52 (Logs-viewer copy; apps' own OSC 52 stores). One frame only.
    #[serde(default)]
    pub clipboard: Option<String>,
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
    /// The file manager's context (right-click) menu is open; the client
    /// routes Up/Down/Enter/Esc to the menu instead of file navigation.
    pub filemanager_context: bool,
    /// The desktop has a rename/new-folder overlay open; forward typed chars.
    pub desktop_editing: bool,
    /// The daemon asked the client to detach (e.g. the Quit button was clicked).
    pub detach: bool,
    /// The daemon is reloading the frontend; the client should reconnect (not
    /// fully detach). Apps stay alive in the apphost.
    pub reload: bool,
    /// The focused app's content rect, set only when that app wants mouse. The
    /// client routes events inside it as `ClientMsg::MouseInput` (passthrough);
    /// `None` keeps all mouse on the normal chrome/WM path.
    pub app_area: Option<Rect>,
    /// A window rename is in progress; the client should forward typed
    /// characters to the rename buffer rather than the focused app.
    pub renaming: bool,
    /// The confirm-close dialog is open; the client routes Enter/Esc and y/n to
    /// it (confirm / cancel) instead of the focused app.
    pub confirm_close: bool,
    /// The launch-warning dialog is open (launching an app entry flagged
    /// `warn`); the client routes Enter/Esc and y/n to it (launch / cancel).
    pub launch_warn: bool,
    /// The power menu's "Add Remote" form is open; the client forwards typed
    /// characters and field navigation to it.
    pub power_editing: bool,
    /// The logs-viewer window is focused; the client routes scroll/copy keys to it.
    pub logs_focused: bool,
    /// The activity monitor is the focused window; the client routes
    /// navigation / kill keys to it.
    pub activity_focused: bool,
    /// The activity monitor is showing a kill-confirm overlay; the client
    /// forwards Enter / y to confirm and Esc / n to cancel.
    pub activity_confirming: bool,
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

/// Path of the daemon's out-of-band control socket. `tuiui kill` / `tuiui reload`
/// send here so they work even while a client is attached — the daemon serves a
/// single client on `socket_path()` serially, so a control message on the main
/// socket would queue behind the attached client and never be read.
pub fn daemon_ctl_path() -> PathBuf {
    socket_dir().join("daemon-ctl.sock")
}

/// Path to the apphost socket (apps live behind this; survives frontend restarts).
pub fn apphost_socket_path() -> PathBuf {
    socket_dir().join("apphost.sock")
}
