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
pub mod activity;
pub mod launcher;
pub mod powermenu;
pub mod confirmclose;
pub mod launchwarn;
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
pub mod systems;
pub mod calendar;
pub mod logsview;
pub mod assistant;

/// The crate version (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The git commit this binary was built from (stamped by `build.rs`).
pub const GIT_SHA: &str = env!("TUIUI_GIT_SHA");
/// Upstream repository the in-app updater checks/installs from.
pub const REPO_URL: &str = "https://github.com/jaylfc/tuiui";

/// Max size of `~/tuiui-debug.log` before it's reset, so a long-running session
/// can't grow it without bound.
const DBG_LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Append a timestamped line to `~/tuiui-debug.log`. Always on — events are
/// low-frequency and the in-app Logs viewer (launcher → tuiui → Logs) reads
/// this file, so there must be something to show without re-running with an
/// env var. The log is capped at [`DBG_LOG_MAX_BYTES`]: once exceeded it's
/// reset (keeping the most recent activity), so it never grows out of hand.
pub fn dbg_log(msg: &str) {
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

/// Append a session-start banner to `~/tuiui-debug.log` (running version, git
/// sha, and binary path). Called once at daemon startup.
///
/// Deliberately APPENDS rather than truncates: when an in-app update reloads
/// the daemon, the previous session's trace (the `update:` steps, the reload)
/// must survive alongside the new banner so the *whole* update is visible in
/// one log — truncating here is what made update failures invisible (the new
/// daemon wiped the evidence). `dbg_log`'s size cap keeps the file bounded.
pub fn dbg_init() {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".into());
    dbg_log(&format!("=== tuiui session start (v{VERSION}, git {GIT_SHA}, exe {exe}) ==="));
}
pub mod protocol;
pub mod daemon;
pub mod client;
