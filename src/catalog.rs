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

/// The parsed catalog, loaded once on first use.
pub fn catalog() -> &'static [CatalogApp] {
    static CATALOG: OnceLock<Vec<CatalogApp>> = OnceLock::new();
    CATALOG.get_or_init(|| serde_json::from_str(CATALOG_JSON).unwrap_or_default())
}

/// Look up the category for a known app by its display name or binary.
pub fn category_for(name_or_bin: &str) -> Option<String> {
    catalog()
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name_or_bin) || c.bin.eq_ignore_ascii_case(name_or_bin))
        .map(|c| c.category.clone())
}

/// Return catalog apps whose binary is present on the current `$PATH`.
pub fn detect_installed() -> Vec<AppEntry> {
    let bins = path_executables();
    catalog()
        .iter()
        .filter(|c| bins.contains(&c.bin) || bins.contains(&c.name.to_lowercase()))
        .map(|c| AppEntry {
            name: c.name.clone(),
            command: c.bin.clone(),
            args: Vec::new(),
            category: Some(c.category.clone()),
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
