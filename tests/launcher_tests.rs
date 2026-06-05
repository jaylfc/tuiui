use tuiui::launcher::{Launcher, LauncherMode};
use tuiui::config::AppEntry;

fn entry(n: &str) -> AppEntry { AppEntry { name: n.into(), command: n.into(), args: vec![], category: None, requires_cwd: None, cwd: None } }
fn launcher() -> Launcher {
    Launcher::new(vec![entry("btop"), entry("lazygit"), entry("yazi"), entry("helix")])
}

#[test]
fn starts_closed() {
    let l = launcher();
    assert!(!l.is_open());
    assert_eq!(l.mode(), None);
}

#[test]
fn toggle_menu_and_spotlight() {
    let mut l = launcher();
    l.toggle_menu();
    assert_eq!(l.mode(), Some(LauncherMode::Menu));
    l.toggle_menu();
    assert!(!l.is_open());
    l.toggle_spotlight();
    assert_eq!(l.mode(), Some(LauncherMode::Spotlight));
}

#[test]
fn spotlight_query_filters() {
    let mut l = launcher();
    l.toggle_spotlight();
    assert_eq!(l.filtered().len(), 4);
    l.type_char('z'); // matches "lazygit" and "yazi"
    let names: Vec<_> = l.filtered().into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["lazygit", "yazi"]);
    l.backspace();
    assert_eq!(l.filtered().len(), 4);
}

#[test]
fn navigation_and_selection() {
    // No categories set, so all group under a single "Apps" submenu, sorted
    // alphabetically inside it: btop, helix, lazygit, yazi.
    let mut l = launcher();
    l.toggle_menu();
    // Root holds the single "Apps" category.
    assert_eq!(l.menu_labels(), vec!["Apps"]);
    // Descend into it and navigate by focused leaf.
    l.expand();
    assert_eq!(l.focused_label(), Some("btop".to_string()));
    l.move_down();
    l.move_down();
    assert_eq!(l.focused_label(), Some("lazygit".to_string()));
    l.move_up();
    assert_eq!(l.focused_label(), Some("helix".to_string()));
}

#[test]
fn menu_render_exposes_clickable_items() {
    let mut l = launcher();
    l.toggle_menu();
    let r = l.render(120, 40);
    // The single "Apps" category auto-expands, so its 4 leaves are clickable.
    assert_eq!(r.items.len(), 4);
    assert!(!r.layers.is_empty());
}

#[test]
fn categories_group_with_headers() {
    let cat = |n: &str, c: &str| AppEntry { name: n.into(), command: n.into(), args: vec![], category: Some(c.into()), requires_cwd: None, cwd: None };
    let mut l = Launcher::new(vec![cat("btop","System"), cat("lazygit","Git"), cat("top","System")]);
    l.toggle_menu();
    // Root groups by category, sorted: Git, System.
    assert_eq!(l.menu_labels(), vec!["Git", "System"]);
    // Descend into Git → its single leaf (lazygit).
    l.expand();
    assert_eq!(l.focused_label(), Some("lazygit".to_string()));
    // Back to root, move to System and descend → leaves sort by name: btop, top.
    l.collapse();
    l.move_down();
    l.expand();
    assert_eq!(l.focused_label(), Some("btop".to_string()));
    l.move_down();
    assert_eq!(l.focused_label(), Some("top".to_string()));
}
