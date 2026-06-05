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

#[test]
fn config_defaults_grid_2x2() {
    let c = Config::default();
    assert_eq!(c.grid_rows, 2);
    assert_eq!(c.grid_cols, 2);
    assert_eq!(c.tile_gap, 0);
    assert!(!c.auto_tile);
}

#[test]
fn default_apps_has_builtin_image_handler() {
    let c = Config::default();
    assert_eq!(c.default_apps.get("image").map(String::as_str), Some("@image"));
    assert_eq!(c.default_apps.get("directory").map(String::as_str), Some("@navigate"));
}

#[test]
fn desktop_defaults_have_files_and_store_pins() {
    let c = Config::default();
    assert!(c.desktop_enabled);
    let cmds: Vec<&str> = c.desktop_pins.iter().map(|p| p.command.as_str()).collect();
    assert!(cmds.contains(&"@files"));
    assert!(cmds.contains(&"@store"));
    assert!(c.desktop_positions.is_empty());
}
