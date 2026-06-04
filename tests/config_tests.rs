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

#[test]
fn save_and_load_roundtrip() {
    let dir = std::env::temp_dir().join(format!("tuiui-cfg-{}", std::process::id()));
    std::env::set_var("XDG_CONFIG_HOME", &dir);
    let c = Config {
        snapping_enabled: false,
        window_shadows: false,
        snap_threshold: 7,
        ..Config::default()
    };
    c.save().unwrap();
    let loaded = Config::load();
    assert!(!loaded.snapping_enabled);
    assert!(!loaded.window_shadows);
    assert_eq!(loaded.snap_threshold, 7);
    std::env::remove_var("XDG_CONFIG_HOME");
    let _ = std::fs::remove_dir_all(&dir);
}
