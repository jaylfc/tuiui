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
    /// Whether this is a CLI tool (prints output and exits, or needs
    /// subcommands) rather than a persistent full-screen TUI. Absent in the
    /// catalog JSON means `false` (TUI), so the existing entries don't churn.
    #[serde(default)]
    pub cli: bool,
    /// Alternate launch variants of this app (e.g. a "skip permissions" flavor
    /// of an AI coding agent), each surfaced as its own extra launcher entry
    /// when the app is installed. Empty for most apps.
    #[serde(default)]
    pub variants: Vec<Variant>,
}

/// An alternate way to launch a catalogued app: same binary, different
/// arguments, shown as a separate launcher entry named `"<app name> <suffix>"`.
#[derive(Clone, Debug, Deserialize)]
pub struct Variant {
    /// Appended to the app's name to label the variant entry (e.g. "⚠️").
    pub suffix: String,
    /// Extra arguments passed to the binary for this variant.
    #[serde(default)]
    pub args: Vec<String>,
    /// When set, launching this variant asks for confirmation first (see
    /// `AppEntry::warn` / `launchwarn.rs`) — e.g. a flag that skips safety
    /// prompts in the wrapped tool.
    #[serde(default)]
    pub warn: Option<String>,
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

/// Whether a known app is a CLI tool (not a persistent TUI), by display name or
/// binary. Apps not in the catalog default to `false` (unknown apps launch as-is).
pub fn is_cli(name_or_bin: &str) -> bool {
    catalog()
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name_or_bin) || c.bin.eq_ignore_ascii_case(name_or_bin))
        .map(|c| c.cli)
        .unwrap_or(false)
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

/// Return catalog apps whose binary is present on the current `$PATH`, plus
/// one extra entry per declared [`Variant`] of each installed app (e.g. the
/// Claude Code "⚠️" skip-permissions flavor) — data-driven, no per-app
/// special-casing here.
pub fn detect_installed() -> Vec<AppEntry> {
    let bins = path_bins();
    let mut entries = Vec::new();
    for c in catalog().iter().filter(|c| bins.contains(&c.bin) || bins.contains(&c.name.to_lowercase())) {
        let requires_cwd = Some(recipe(&c.name).map(|r| r.requires_cwd).unwrap_or(false));
        entries.push(AppEntry {
            name: c.name.clone(),
            command: c.bin.clone(),
            args: Vec::new(),
            category: Some(c.category.clone()),
            requires_cwd,
            cwd: None,
            cli: Some(c.cli),
            warn: None,
        });
        for v in &c.variants {
            entries.push(AppEntry {
                name: format!("{} {}", c.name, v.suffix),
                command: c.bin.clone(),
                args: v.args.clone(),
                category: Some(c.category.clone()),
                requires_cwd,
                cwd: None,
                // Variants launch exactly as declared, not through the CLI
                // help-then-shell wrapper.
                cli: None,
                warn: v.warn.clone(),
            });
        }
    }
    entries
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `detect_installed` emits a base entry plus one extra entry per declared
    /// variant, carrying the variant's args/warn through and leaving `cli`
    /// unset (variants launch exactly as declared). Fakes the `$PATH` scan via
    /// the private `BINS` cache — real `is_installed`/`detect_installed`
    /// callers elsewhere have no inline tests, so this can't race them.
    #[test]
    fn detect_installed_emits_variant_entry_for_claude_code() {
        let mut fake = HashSet::new();
        fake.insert("claude".to_string());
        *BINS.write().unwrap() = Some(fake);

        let entries = detect_installed();

        let base = entries.iter().find(|e| e.name == "Claude Code").expect("base entry present");
        assert_eq!(base.command, "claude");
        assert!(base.args.is_empty());
        assert_eq!(base.warn, None, "the plain entry carries no warning");

        let variant = entries.iter().find(|e| e.name == "Claude Code ⚠️").expect("variant entry present");
        assert_eq!(variant.command, "claude");
        assert_eq!(variant.args, vec!["--dangerously-skip-permissions".to_string()]);
        assert_eq!(variant.cli, None, "variants launch as given, not through the CLI wrapper");
        assert_eq!(variant.category, base.category);
        assert_eq!(variant.requires_cwd, base.requires_cwd);
        assert!(variant.warn.as_deref().unwrap_or("").contains("--dangerously-skip-permissions"));

        // Leave the cache empty so a later real scan repopulates it lazily.
        *BINS.write().unwrap() = None;
    }
}
