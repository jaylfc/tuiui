pub mod geometry;
pub mod cell;
pub mod theme;
pub mod buffer;
pub mod compositor;
pub mod terminal;
pub mod ptyhost;
pub mod apphost;
pub mod window;
pub mod wm;
pub mod chrome;
pub mod catalog;
pub mod store;
pub mod settings;
pub mod launcher;
pub mod powermenu;
pub mod confirmclose;
pub mod input;
pub mod session;
pub mod config;
pub mod system;
pub mod poller;
pub mod tray;
pub mod dirpicker;
pub mod help;
pub mod imagestore;
pub mod thumbnail;
pub mod icons;
pub mod kitty;
pub mod kittygfx;
pub mod imageview;
pub mod openwith;
pub mod fileops;
pub mod filemanager;
pub mod desktop;
pub mod mouse;
pub mod gpm;
pub mod badge;
pub mod service;
pub mod toolchain;

#[cfg(feature = "wayland-compositor")]
pub mod wayland;

#[cfg(not(feature = "wayland-compositor"))]
mod wayland_private {
    pub fn run_compositor() -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "tuiui: compositor mode requires the 'wayland-compositor' feature (build with --features wayland-compositor)",
        ))
    }
}

#[cfg(feature = "wayland-compositor")]
pub use wayland::run_compositor;

#[cfg(not(feature = "wayland-compositor"))]
pub use wayland_private::run_compositor;

/// The crate version (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The git commit this binary was built from (stamped by `build.rs`).
pub const GIT_SHA: &str = env!("TUIUI_GIT_SHA");
/// Upstream repository the in-app updater checks/installs from.
pub const REPO_URL: &str = "https://github.com/jaylfc/tuiui";

/// Max size of `~/tuiui-debug.log` before it's reset, so a long-running session
/// can't grow it without bound.
const DBG_LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Append a timestamped line to `~/tuiui-debug.log` when `$TUIUI_DEBUG` is set;
/// a no-op otherwise (zero cost in normal use). Used to localize freezes/hangs.
/// The log is capped at [`DBG_LOG_MAX_BYTES`]: once exceeded it's reset (keeping
/// the most recent activity), so it never grows out of hand.
pub fn dbg_log(msg: &str) {
    if std::env::var_os("TUIUI_DEBUG").is_none() {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    let path = home.join("tuiui-debug.log");
    use std::io::Write;
    // Reset the log if it has grown past the cap (events are low-frequency, so the
    // per-call stat is negligible).
    if std::fs::metadata(&path).map(|m| m.len() > DBG_LOG_MAX_BYTES).unwrap_or(false) {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).truncate(true).open(&path) {
            let _ = writeln!(f, "=== log reset (exceeded {DBG_LOG_MAX_BYTES} bytes) ===");
        }
    }
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{ms} {msg}");
    }
}

/// Truncate `~/tuiui-debug.log` and write a session-start banner when
/// `$TUIUI_DEBUG` is set.  Called once at daemon startup so each run starts
/// with a clean, readable log.  No-op when the env var is unset.
pub fn dbg_init() {
    if std::env::var_os("TUIUI_DEBUG").is_none() {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(home.join("tuiui-debug.log"))
    {
        let _ = writeln!(f, "=== tuiui debug session start (git {}) ===", GIT_SHA);
    }
}
pub mod protocol;
pub mod daemon;
pub mod client;
