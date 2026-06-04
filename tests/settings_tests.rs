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
