use tuiui::catalog::{catalog, category_for, detect_installed};

#[test]
fn catalog_loads_full_awesome_tuis_list() {
    // The bundled catalog is generated from awesome-tuis (hundreds of apps).
    assert!(catalog().len() > 400, "catalog should hold the full list, got {}", catalog().len());
    for c in catalog() {
        assert!(!c.name.is_empty());
        assert!(!c.bin.is_empty());
        assert!(!c.category.is_empty());
    }
}

#[test]
fn category_lookup_by_name_or_bin() {
    // btop is listed as "btop++" with binary "btop" under Dashboards.
    assert_eq!(category_for("btop").as_deref(), Some("Dashboards"));
    assert!(category_for("nonsense-xyz-not-real").is_none());
}

#[test]
fn detect_returns_categorized_entries() {
    for e in detect_installed() {
        assert!(e.category.is_some(), "{} should have a category", e.name);
    }
}

#[test]
fn os_filtering_never_hides_unknown_apps() {
    use tuiui::catalog::{current_os, runs_on_current_os};
    // current OS is a known token
    assert!(["macos","linux","windows"].contains(&current_os()) || !current_os().is_empty());
    // an app with no recipe is shown everywhere (never falsely hidden)
    assert!(runs_on_current_os("definitely-not-a-real-app-xyz"));
}

#[test]
fn ai_tools_require_cwd() {
    assert!(tuiui::catalog::recipe("Claude Code").unwrap().requires_cwd);
    assert!(!tuiui::catalog::recipe("btop").map(|r| r.requires_cwd).unwrap_or(false));
}

/// A catalog entry with no `"cli"` key parses with the field defaulting to
/// `false` (TUI), so the 600+ existing entries don't churn.
#[test]
fn cli_flag_defaults_to_false_when_absent() {
    let json = r#"{"name":"Foo","bin":"foo","category":"Cat","description":"d","homepage":"https://example.com"}"#;
    let app: tuiui::catalog::CatalogApp = serde_json::from_str(json).unwrap();
    assert!(!app.cli);
}

/// A catalog entry with `"cli": true` parses the flag through.
#[test]
fn cli_flag_parses_when_present() {
    let json = r#"{"name":"Foo","bin":"foo","category":"Cat","description":"d","homepage":"https://example.com","cli":true}"#;
    let app: tuiui::catalog::CatalogApp = serde_json::from_str(json).unwrap();
    assert!(app.cli);
}

/// `is_cli` mirrors `category_for`/`requires_cwd_for`: false for apps outside
/// the catalog, and reflects the flag for apps that are in it (none of the
/// bundled entries are flagged yet — the catalog-data sweep ships separately).
#[test]
fn is_cli_lookup_by_name_or_bin() {
    use tuiui::catalog::is_cli;
    assert!(!is_cli("definitely-not-a-real-app-xyz"));
    assert!(!is_cli("btop"));
}

/// `AppEntry.cli` follows the exact `requires_cwd: Option<bool>` pattern:
/// absent in TOML, `None` in memory, so a config entry without the field falls
/// back to the catalog when the launcher backfills it (see `session.rs`).
#[test]
fn app_entry_cli_field_defaults_to_none() {
    let toml = r#"name = "Foo"
command = "foo"
"#;
    let entry: tuiui::config::AppEntry = toml::from_str(toml).unwrap();
    assert_eq!(entry.cli, None);
}

/// `AppEntry.warn` follows the exact `cli: Option<bool>` pattern: absent in
/// TOML, `None` in memory — hand-edited config entries can opt an app into
/// the launch-warning dialog by setting it directly.
#[test]
fn app_entry_warn_field_defaults_to_none_and_round_trips() {
    let toml = r#"name = "Foo"
command = "foo"
"#;
    let entry: tuiui::config::AppEntry = toml::from_str(toml).unwrap();
    assert_eq!(entry.warn, None);

    let toml = r#"name = "Foo"
command = "foo"
warn = "careful?"
"#;
    let entry: tuiui::config::AppEntry = toml::from_str(toml).unwrap();
    assert_eq!(entry.warn.as_deref(), Some("careful?"));
}

/// A catalog entry with no `"variants"` key parses with an empty list, so the
/// 600+ existing entries don't churn.
#[test]
fn variants_default_to_empty_when_absent() {
    let json = r#"{"name":"Foo","bin":"foo","category":"Cat","description":"d","homepage":"https://example.com"}"#;
    let app: tuiui::catalog::CatalogApp = serde_json::from_str(json).unwrap();
    assert!(app.variants.is_empty());
}

/// A declared variant parses its suffix/args/warn through.
#[test]
fn variants_parse_from_json() {
    let json = r#"{"name":"Foo","bin":"foo","category":"Cat","description":"d","homepage":"https://example.com",
        "variants":[{"suffix":"⚠️","args":["--x"],"warn":"careful"}]}"#;
    let app: tuiui::catalog::CatalogApp = serde_json::from_str(json).unwrap();
    assert_eq!(app.variants.len(), 1);
    assert_eq!(app.variants[0].suffix, "\u{26a0}\u{fe0f}");
    assert_eq!(app.variants[0].args, vec!["--x".to_string()]);
    assert_eq!(app.variants[0].warn.as_deref(), Some("careful"));
}

/// A variant with no `args`/`warn` keys parses with the defaults (empty args,
/// no warning) rather than failing.
#[test]
fn variant_args_and_warn_default_when_absent() {
    let json = r#"{"suffix":"x"}"#;
    let v: tuiui::catalog::Variant = serde_json::from_str(json).unwrap();
    assert!(v.args.is_empty());
    assert_eq!(v.warn, None);
}

/// The bundled catalog carries the Claude Code "skip permissions" variant
/// this feature was built for.
#[test]
fn bundled_catalog_has_claude_code_skip_permissions_variant() {
    let claude = catalog().iter().find(|c| c.name == "Claude Code").expect("Claude Code is in the catalog");
    assert_eq!(claude.variants.len(), 1);
    assert_eq!(claude.variants[0].suffix, "\u{26a0}\u{fe0f}");
    assert_eq!(claude.variants[0].args, vec!["--dangerously-skip-permissions".to_string()]);
    assert!(claude.variants[0].warn.is_some());
}
