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
    // No categories set, so all sort under "Apps" alphabetically:
    // btop, helix, lazygit, yazi.
    let mut l = launcher();
    l.toggle_menu();
    assert_eq!(l.selected_entry().unwrap().name, "btop");
    l.move_down();
    l.move_down();
    assert_eq!(l.selected_entry().unwrap().name, "lazygit");
    l.move_up();
    assert_eq!(l.selected_entry().unwrap().name, "helix");
}

#[test]
fn menu_render_exposes_clickable_items() {
    let mut l = launcher();
    l.toggle_menu();
    let r = l.render(120, 40);
    assert_eq!(r.items.len(), 4);
    assert!(!r.layers.is_empty());
    // menubar row 0, dropdown border row 1, "APPS" header row 2, first item row 3
    assert_eq!(r.items[0].1.y, 3);
}

#[test]
fn categories_group_with_headers() {
    let cat = |n: &str, c: &str| AppEntry { name: n.into(), command: n.into(), args: vec![], category: Some(c.into()), requires_cwd: None, cwd: None };
    let mut l = Launcher::new(vec![cat("btop","System"), cat("lazygit","Git"), cat("top","System")]);
    l.toggle_menu();
    let r = l.render(120, 40);
    // 3 clickable items (headers are not items)
    assert_eq!(r.items.len(), 3);
    // sorted by category then name: Git(lazygit), System(btop, top)
    let names: Vec<_> = r.items.iter().map(|(e,_)| e.name.clone()).collect();
    assert_eq!(names, vec!["lazygit", "btop", "top"]);
}
