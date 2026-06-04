use tuiui::catalog::{category_for, detect_installed};

#[test]
fn category_lookup_by_name_or_bin() {
    assert_eq!(category_for("btop").as_deref(), Some("System"));
    assert_eq!(category_for("hx").as_deref(), Some("Editors")); // by binary
    assert_eq!(category_for("nonsense-xyz"), None);
}

#[test]
fn detect_returns_categorized_entries() {
    // We can't assume specific apps, but every detected entry must carry a category.
    for e in detect_installed() {
        assert!(e.category.is_some(), "{} should have a category", e.name);
    }
}
