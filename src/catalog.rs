//! The bundled app catalog — generated from the awesome-tuis list by
//! `scripts/gen_catalog.py` into `assets/catalog.json`.
//!
//! It powers two things: detecting which catalogued TUIs are already installed
//! on the user's `$PATH`, and (in the Store slice) browsing/installing apps.
//! There is no reliable way to tell whether an arbitrary executable is a TUI, so
//! detection is restricted to this curated set rather than scanning all of `$PATH`.

use crate::config::AppEntry;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::OnceLock;

/// One catalogued terminal application.
#[derive(Clone, Debug, Deserialize)]
pub struct CatalogApp {
    /// Display name (from awesome-tuis).
    pub name: String,
    /// Best-effort executable name used for `$PATH` detection and launching.
    pub bin: String,
    /// Category (awesome-tuis section, e.g. "Dashboards", "Editors").
    pub category: String,
    /// One-line description.
    pub description: String,
    /// Project homepage / repository URL.
    pub homepage: String,
}

const CATALOG_JSON: &str = include_str!("../assets/catalog.json");
const RECIPES_JSON: &str = include_str!("../assets/recipes.json");

/// A curated install recipe for a catalogued app (from `assets/recipes.json`).
#[derive(Clone, Debug, Deserialize)]
pub struct Recipe {
    /// The shell command that installs the app.
    pub install: String,
    /// Install method ("brew", "cargo", "go", …) — informational.
    #[serde(default)]
    pub method: String,
    /// Whether this recipe was verified against the app's docs.
    #[serde(default)]
    pub verified: bool,
    /// Operating systems the app runs on (e.g. `["macos","linux"]`). Empty means
    /// "any/unknown" — shown everywhere.
    #[serde(default)]
    pub os: Vec<String>,
    /// Whether launching this app should first prompt for a working directory
    /// (true for coding agents that operate on a project tree).
    #[serde(default)]
    pub requires_cwd: bool,
    /// A one-paragraph setup tip shown in the Store's detail pane (e.g. "add
    /// models/providers with `hermes model` first").
    #[serde(default)]
    pub tip: String,
}

/// The current operating system as a recipe `os` token ("macos", "linux", …).
pub fn current_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        other => other,
    }
}

/// Whether an app is applicable to the current OS (true unless its recipe lists
/// operating systems that exclude this one).
pub fn runs_on_current_os(name: &str) -> bool {
    match recipe(name) {
        Some(r) if !r.os.is_empty() => r.os.iter().any(|o| o == current_os()),
        _ => true,
    }
}

/// The parsed catalog, loaded once on first use.
pub fn catalog() -> &'static [CatalogApp] {
    static CATALOG: OnceLock<Vec<CatalogApp>> = OnceLock::new();
    CATALOG.get_or_init(|| serde_json::from_str(CATALOG_JSON).unwrap_or_default())
}

/// Curated install recipes keyed by app name, loaded once.
pub fn recipes() -> &'static std::collections::HashMap<String, Recipe> {
    static RECIPES: OnceLock<std::collections::HashMap<String, Recipe>> = OnceLock::new();
    RECIPES.get_or_init(|| serde_json::from_str(RECIPES_JSON).unwrap_or_default())
}

/// The curated install recipe for `name`, if one exists.
pub fn recipe(name: &str) -> Option<&'static Recipe> {
    recipes().get(name)
}

/// Count of verified recipes (progress indicator).
pub fn verified_count() -> usize {
    recipes().values().filter(|r| r.verified).count()
}

/// Look up the category for a known app by its display name or binary.
pub fn category_for(name_or_bin: &str) -> Option<String> {
    catalog()
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name_or_bin) || c.bin.eq_ignore_ascii_case(name_or_bin))
        .map(|c| c.category.clone())
}

/// Whether a known app wants a working-directory prompt on launch, by display
/// name or binary. `None` when the app isn't in the catalog (so callers can keep
/// an explicitly-configured flag instead of overriding it).
pub fn requires_cwd_for(name_or_bin: &str) -> Option<bool> {
    let c = catalog().iter().find(|c| {
        c.name.eq_ignore_ascii_case(name_or_bin) || c.bin.eq_ignore_ascii_case(name_or_bin)
    })?;
    Some(recipe(&c.name).map(|r| r.requires_cwd).unwrap_or(false))
}

/// Cached set of `$PATH` executable names. `None` until first use; replaced by
/// [`refresh_installed`] after an install so newly-added binaries are detected
/// without restarting the daemon.
static BINS: std::sync::RwLock<Option<HashSet<String>>> = std::sync::RwLock::new(None);

/// Snapshot of the current `$PATH` executables (scanning + caching on first use).
fn path_bins() -> HashSet<String> {
    if let Some(set) = BINS.read().unwrap().clone() {
        return set;
    }
    let set = path_executables();
    *BINS.write().unwrap() = Some(set.clone());
    set
}

/// Re-scan `$PATH`, replacing the cache. Call after an install completes so the
/// store and launcher pick up the newly-installed binary.
pub fn refresh_installed() {
    *BINS.write().unwrap() = Some(path_executables());
}

/// Whether an executable `bin` is present on `$PATH`.
pub fn is_installed(bin: &str) -> bool {
    path_bins().contains(&bin.to_lowercase())
}

/// Return catalog apps whose binary is present on the current `$PATH`.
pub fn detect_installed() -> Vec<AppEntry> {
    let bins = path_bins();
    catalog()
        .iter()
        .filter(|c| bins.contains(&c.bin) || bins.contains(&c.name.to_lowercase()))
        .map(|c| AppEntry {
            name: c.name.clone(),
            command: c.bin.clone(),
            args: Vec::new(),
            category: Some(c.category.clone()),
            requires_cwd: Some(recipe(&c.name).map(|r| r.requires_cwd).unwrap_or(false)),
            cwd: None,
        })
        .collect()
}

/// The set of executable names found across `$PATH` (lowercased), scanned once.
fn path_executables() -> HashSet<String> {
    let mut set = HashSet::new();
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if let Ok(name) = entry.file_name().into_string() {
                        set.insert(name.to_lowercase());
                    }
                }
            }
        }
    }
    set
}
