use serde::{Deserialize, Serialize};

/// A single application entry in the config file.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AppEntry {
    /// Human-readable name shown in the dock.
    pub name: String,
    /// Executable to launch (resolved via `$PATH`).
    pub command: String,
    /// Optional extra arguments passed to the executable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Optional launcher category (e.g. "System", "Git"). Apps without one are
    /// grouped under "Apps".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// Top-level configuration for tuiui.
///
/// All fields have sensible defaults; missing keys in the TOML file
/// fall back to [`Default::default`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Whether dragging a window to a screen edge snaps it.
    pub snapping_enabled: bool,
    /// Distance in cells from the screen edge that triggers snapping.
    pub snap_threshold: i32,
    /// Whether windows draw drop shadows.
    pub window_shadows: bool,
    /// Active color theme name (one of the preset names in `theme::PRESETS`).
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Ordered list of apps auto-started at launch (and shown in the dock).
    pub apps: Vec<AppEntry>,
    /// Apps offered in the launcher menu / spotlight. Falls back to `apps`
    /// (via [`Config::launcher_apps`]) when left empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launcher: Vec<AppEntry>,
}

impl Config {
    /// The apps the launcher should offer: the explicit `launcher` list, or the
    /// autostart `apps` when no launcher list is configured.
    pub fn launcher_apps(&self) -> Vec<AppEntry> {
        if self.launcher.is_empty() {
            self.apps.clone()
        } else {
            self.launcher.clone()
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            snapping_enabled: true,
            snap_threshold: 3,
            window_shadows: true,
            theme: "midnight".into(),
            apps: vec![
                AppEntry { name: "shell".into(), command: default_shell(), args: vec![], category: Some("Shell".into()) },
            ],
            launcher: vec![],
        }
    }
}

fn default_shell() -> String { std::env::var("SHELL").unwrap_or_else(|_| "bash".into()) }

fn default_theme() -> String { "midnight".into() }

/// Resolve the config file path from `$XDG_CONFIG_HOME` (if set) else
/// `<home>/.config`, on every platform.
///
/// Note: we deliberately do **not** use `dirs::config_dir()` — on macOS that
/// returns `~/Library/Application Support`, but tuiui standardises on the
/// XDG-style `~/.config/tuiui/config.toml` across all platforms.
fn config_file_path(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let base = xdg_config_home
        .map(std::path::PathBuf::from)
        .or_else(|| home.map(|h| h.join(".config")))?;
    Some(base.join("tuiui").join("config.toml"))
}

impl Config {
    /// Parse a `Config` from a TOML string.
    pub fn from_toml_str(s: &str) -> Result<Config, toml::de::Error> { toml::from_str(s) }

    /// Load from `$XDG_CONFIG_HOME/tuiui/config.toml` (or `~/.config/tuiui/config.toml`),
    /// falling back to defaults on any error.
    pub fn load() -> Config {
        let path = config_file_path(std::env::var_os("XDG_CONFIG_HOME"), dirs::home_dir());
        if let Some(p) = path {
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(cfg) = Config::from_toml_str(&text) { return cfg; }
            }
        }
        Config::default()
    }

    /// Write the config back to `$XDG_CONFIG_HOME/tuiui/config.toml` (or
    /// `~/.config/tuiui/config.toml`), creating the directory if needed.
    ///
    /// Note: this serialises the live config, so any hand-written comments in the
    /// file are not preserved.
    pub fn save(&self) -> std::io::Result<()> {
        let path = config_file_path(std::env::var_os("XDG_CONFIG_HOME"), dirs::home_dir())
            .ok_or_else(|| std::io::Error::other("no config directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, toml)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn config_path_prefers_xdg_config_home() {
        let p = config_file_path(Some("/x/cfg".into()), Some(PathBuf::from("/home/u")));
        assert_eq!(p.unwrap(), PathBuf::from("/x/cfg/tuiui/config.toml"));
    }

    #[test]
    fn config_path_falls_back_to_dotconfig_on_all_platforms() {
        let p = config_file_path(None, Some(PathBuf::from("/home/u")));
        assert_eq!(p.unwrap(), PathBuf::from("/home/u/.config/tuiui/config.toml"));
    }
}
