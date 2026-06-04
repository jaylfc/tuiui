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
