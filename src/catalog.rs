//! A bundled catalog of well-known terminal apps, used to auto-detect which TUIs
//! are already installed on the user's `$PATH` and to assign launcher categories.
//!
//! There is no reliable way to tell whether an arbitrary executable is a
//! full-screen TUI, so rather than scan every binary on `$PATH`, we scan for a
//! *curated* set of known TUI binaries (a seed of the future Store catalog).

use crate::config::AppEntry;

/// One known terminal application.
pub struct CatalogApp {
    /// Display name shown in the launcher.
    pub name: &'static str,
    /// Executable name looked up on `$PATH`.
    pub bin: &'static str,
    /// Launcher category.
    pub category: &'static str,
}

macro_rules! app {
    ($name:expr, $bin:expr, $cat:expr) => {
        CatalogApp { name: $name, bin: $bin, category: $cat }
    };
}

/// The curated known-TUI catalog (seeded from awesome-tuis).
pub const CATALOG: &[CatalogApp] = &[
    // System / monitoring
    app!("btop", "btop", "System"),
    app!("htop", "htop", "System"),
    app!("top", "top", "System"),
    app!("glances", "glances", "System"),
    app!("bottom", "btm", "System"),
    app!("ncdu", "ncdu", "System"),
    app!("dust", "dust", "System"),
    app!("gdu", "gdu", "System"),
    // Git
    app!("lazygit", "lazygit", "Git"),
    app!("gitui", "gitui", "Git"),
    app!("tig", "tig", "Git"),
    // Files
    app!("yazi", "yazi", "Files"),
    app!("ranger", "ranger", "Files"),
    app!("nnn", "nnn", "Files"),
    app!("lf", "lf", "Files"),
    app!("broot", "broot", "Files"),
    app!("superfile", "spf", "Files"),
    // Editors
    app!("helix", "hx", "Editors"),
    app!("neovim", "nvim", "Editors"),
    app!("vim", "vim", "Editors"),
    app!("emacs", "emacs", "Editors"),
    app!("micro", "micro", "Editors"),
    // DevOps
    app!("k9s", "k9s", "DevOps"),
    app!("lazydocker", "lazydocker", "DevOps"),
    app!("lazysql", "lazysql", "DevOps"),
    // Network
    app!("bandwhich", "bandwhich", "Network"),
    app!("gping", "gping", "Network"),
    app!("trippy", "trip", "Network"),
    // Media
    app!("spotify-player", "spotify_player", "Media"),
    app!("ncspot", "ncspot", "Media"),
    app!("cmus", "cmus", "Media"),
    // Misc
    app!("lazyjournal", "lazyjournal", "System"),
    app!("taskwarrior-tui", "taskwarrior-tui", "Productivity"),
];

/// Look up the category for a known app by its display name or binary.
pub fn category_for(name_or_bin: &str) -> Option<String> {
    CATALOG
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name_or_bin) || c.bin.eq_ignore_ascii_case(name_or_bin))
        .map(|c| c.category.to_string())
}

/// Return catalog apps whose binary is present on the current `$PATH`.
pub fn detect_installed() -> Vec<AppEntry> {
    CATALOG
        .iter()
        .filter(|c| on_path(c.bin))
        .map(|c| AppEntry {
            name: c.name.to_string(),
            command: c.bin.to_string(),
            args: Vec::new(),
            category: Some(c.category.to_string()),
        })
        .collect()
}

/// Whether an executable named `bin` exists on `$PATH`.
fn on_path(bin: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file())
}
