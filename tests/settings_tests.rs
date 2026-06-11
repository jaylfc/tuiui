use tuiui::settings::Settings;
use tuiui::config::Config;

#[test]
fn toggle_changes_config() {
    let mut s = Settings::new(Config::default());
    assert!(s.config().snapping_enabled);
    s.toggle(); // Windows / snapping
    assert!(!s.config().snapping_enabled);
    s.next_section(); // Appearance
    s.toggle(); // window shadows
    assert!(!s.config().window_shadows);
}

#[test]
fn threshold_adjusts_within_bounds() {
    let mut s = Settings::new(Config::default());
    s.move_down(); // snap_threshold row
    let before = s.config().snap_threshold;
    s.right();
    assert_eq!(s.config().snap_threshold, before + 1);
    s.left();
    s.left();
    assert_eq!(s.config().snap_threshold, before - 1);
}

#[test]
fn updates_section_requests_check_and_install() {
    use tuiui::settings::SettingsAction;
    let mut s = Settings::new(Config::default());
    s.next_section(); // Appearance
    s.next_section(); // Updates
    s.toggle();       // row 0 -> Check
    assert_eq!(s.take_action(), Some(SettingsAction::CheckUpdates));
    s.move_down();    // row 1
    s.toggle();       // -> Install
    assert_eq!(s.take_action(), Some(SettingsAction::InstallUpdate));
    // a plain settings toggle elsewhere requests nothing
    let mut w = Settings::new(Config::default());
    w.toggle();
    assert_eq!(w.take_action(), None);
}

#[test]
fn updates_branch_switcher_cycles_channels() {
    use tuiui::settings::Settings;
    use tuiui::config::Config;
    let mut s = Settings::new(Config::default());
    s.show_updates_section();
    assert_eq!(s.config().update_branch, "main");
    // Row 2 is the Channel cycler; select it and toggle.
    s.move_down();
    s.move_down();
    s.right();
    assert_eq!(s.config().update_branch, "dev", "right cycles main -> dev");
    s.right();
    assert_eq!(s.config().update_branch, "main", "wraps back to main");
}

#[test]
fn restart_app_server_row_only_when_flagged() {
    use tuiui::settings::{Settings, SettingsAction};
    use tuiui::config::Config;
    let mut s = Settings::new(Config::default());
    s.show_updates_section();
    // Not flagged: selecting past the last row stops at Channel (row 2).
    s.move_down(); s.move_down(); s.move_down();
    s.toggle();
    assert_ne!(s.take_action(), Some(SettingsAction::RestartApphost));

    let mut s = Settings::new(Config::default());
    s.set_apphost_outdated(true);
    s.show_updates_section();
    s.move_down(); s.move_down(); s.move_down(); // → row 3: Restart app server
    s.toggle();
    assert_eq!(s.take_action(), Some(SettingsAction::RestartApphost));
}
