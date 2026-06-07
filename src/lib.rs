pub mod geometry;
pub mod cell;
pub mod theme;
pub mod buffer;
pub mod compositor;
pub mod terminal;
pub mod ptyhost;
pub mod window;
pub mod wm;
pub mod chrome;
pub mod catalog;
pub mod store;
pub mod settings;
pub mod launcher;
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

/// The crate version (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The git commit this binary was built from (stamped by `build.rs`).
pub const GIT_SHA: &str = env!("TUIUI_GIT_SHA");
/// Upstream repository the in-app updater checks/installs from.
pub const REPO_URL: &str = "https://github.com/jaylfc/tuiui";

/// Append a timestamped line to `~/tuiui-debug.log` when `$TUIUI_DEBUG` is set;
/// a no-op otherwise (zero cost in normal use). Used to localize freezes/hangs.
pub fn dbg_log(msg: &str) {
    if std::env::var_os("TUIUI_DEBUG").is_none() {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("tuiui-debug.log"))
    {
        let _ = writeln!(f, "{ms} {msg}");
    }
}
pub mod protocol;
pub mod daemon;
pub mod client;
