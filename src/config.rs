use serde::Deserialize;

/// A single application entry in the config file.
#[derive(Clone, Debug, Deserialize)]
pub struct AppEntry {
    /// Human-readable name shown in the dock.
    pub name: String,
    /// Executable to launch (resolved via `$PATH`).
    pub command: String,
    /// Optional extra arguments passed to the executable.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Top-level configuration for tuiui.
///
/// All fields have sensible defaults; missing keys in the TOML file
/// fall back to [`Default::default`].
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Whether dragging a window to a screen edge snaps it.
    pub snapping_enabled: bool,
    /// Distance in cells from the screen edge that triggers snapping.
    pub snap_threshold: i32,
    /// Ordered list of apps shown in the dock.
    pub apps: Vec<AppEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            snapping_enabled: true,
            snap_threshold: 3,
            apps: vec![
                AppEntry { name: "shell".into(), command: default_shell(), args: vec![] },
            ],
        }
    }
}

fn default_shell() -> String { std::env::var("SHELL").unwrap_or_else(|_| "bash".into()) }

impl Config {
    /// Parse a `Config` from a TOML string.
    pub fn from_toml_str(s: &str) -> Result<Config, toml::de::Error> { toml::from_str(s) }

    /// Load from `~/.config/tuiui/config.toml`, falling back to defaults on any error.
    pub fn load() -> Config {
        let path = dirs::config_dir().map(|d| d.join("tuiui").join("config.toml"));
        if let Some(p) = path {
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(cfg) = Config::from_toml_str(&text) { return cfg; }
            }
        }
        Config::default()
    }
}
