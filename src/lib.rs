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
pub mod kitty;
pub mod imageview;
pub mod openwith;
pub mod fileops;

/// The crate version (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The git commit this binary was built from (stamped by `build.rs`).
pub const GIT_SHA: &str = env!("TUIUI_GIT_SHA");
/// Upstream repository the in-app updater checks/installs from.
pub const REPO_URL: &str = "https://github.com/jaylfc/tuiui";
pub mod protocol;
pub mod daemon;
pub mod client;
