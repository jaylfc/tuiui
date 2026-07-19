use tuiui::launcher::{Launcher, LauncherMode};
use tuiui::config::AppEntry;
use tuiui::buffer::CellBuffer;

fn entry(n: &str) -> AppEntry { AppEntry { name: n.into(), command: n.into(), args: vec![], category: None, requires_cwd: None, cwd: None, cli: None, warn: None } }

/// The visible text of row `y` in `buf` (spaces where no glyph was written).
fn row_text(buf: &CellBuffer, y: i32) -> String {
    (0..buf.width()).map(|x| buf.get(x, y).map(|c| c.ch).unwrap_or(' ')).collect()
}
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
    // Root: a "Shell" quick-launch first, then the single "Apps" category.
    assert_eq!(l.menu_labels(), vec!["Shell", "Apps"]);
    // Select "Apps" (after the Shell quick-launch) and descend into it.
    l.move_down();
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
    l.move_down(); // select "Apps" so it auto-expands
    let r = l.render(120, 40);
    // The Shell quick-launch (1 leaf) + the 4 auto-expanded "Apps" leaves.
    assert_eq!(r.items.len(), 5);
    assert!(!r.layers.is_empty());
}

#[test]
fn categories_group_with_headers() {
    let cat = |n: &str, c: &str| AppEntry { name: n.into(), command: n.into(), args: vec![], category: Some(c.into()), requires_cwd: None, cwd: None, cli: None, warn: None };
    let mut l = Launcher::new(vec![cat("btop","System"), cat("lazygit","Git"), cat("top","System")]);
    l.toggle_menu();
    // Root: the "Shell" quick-launch, then categories sorted: Git, System.
    assert_eq!(l.menu_labels(), vec!["Shell", "Git", "System"]);
    // Select Git (after Shell) and descend → its single leaf (lazygit).
    l.move_down();
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

/// A `cli`-flagged entry gets a "CLI" tag rendered on its row in Spotlight; an
/// unflagged entry's row does not.
#[test]
fn spotlight_shows_cli_badge_only_on_flagged_entries() {
    let cli = AppEntry { name: "himalaya".into(), command: "himalaya".into(), args: vec![], category: None, requires_cwd: None, cwd: None, cli: Some(true), warn: None };
    let tui = AppEntry { name: "btop".into(), command: "btop".into(), args: vec![], category: None, requires_cwd: None, cwd: None, cli: Some(false), warn: None };
    let mut l = Launcher::new(vec![cli, tui]);
    l.toggle_spotlight();
    let r = l.render(80, 24);
    let buf = &r.layers[0].buf;
    let rows: Vec<String> = (0..buf.height()).map(|y| row_text(buf, y)).collect();
    let himalaya_row = rows.iter().find(|t| t.contains("himalaya")).expect("himalaya row rendered");
    assert!(himalaya_row.contains("CLI"), "flagged row should show the CLI badge: {himalaya_row:?}");
    let btop_row = rows.iter().find(|t| t.contains("btop")).expect("btop row rendered");
    assert!(!btop_row.contains("CLI"), "unflagged row should not show the badge: {btop_row:?}");
}

/// The same badge renders in the cascading Menu (the leaf row inside the
/// auto-expanded "Apps" category).
#[test]
fn menu_shows_cli_badge_on_flagged_leaf() {
    let cli = AppEntry { name: "himalaya".into(), command: "himalaya".into(), args: vec![], category: None, requires_cwd: None, cwd: None, cli: Some(true), warn: None };
    let mut l = Launcher::new(vec![cli]);
    l.toggle_menu();
    l.move_down(); // select the "Apps" category (after the Shell quick-launch)
    let r = l.render(120, 40);
    let found = r.layers.iter().any(|layer| {
        (0..layer.buf.height()).any(|y| row_text(&layer.buf, y).contains("CLI"))
    });
    assert!(found, "the auto-expanded Apps panel should show the CLI badge on himalaya's row");
}
