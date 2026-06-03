use tuiui::config::Config;

#[test]
fn defaults_are_sane() {
    let c = Config::default();
    assert!(c.snapping_enabled);
    assert_eq!(c.snap_threshold, 3);
    assert!(!c.apps.is_empty());
}

#[test]
fn parses_toml_overrides() {
    let toml = r#"
snapping_enabled = false
snap_threshold = 5
[[apps]]
name = "shell"
command = "bash"
"#;
    let c = Config::from_toml_str(toml).unwrap();
    assert!(!c.snapping_enabled);
    assert_eq!(c.snap_threshold, 5);
    assert_eq!(c.apps.len(), 1);
    assert_eq!(c.apps[0].command, "bash");
}
