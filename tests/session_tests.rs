use tuiui::session::{SessionCore, ClientMsg};
use tuiui::config::Config;
use tuiui::geometry::Point;

#[test]
fn launching_app_creates_window_and_dock_entry() {
    let mut core = SessionCore::new(80, 24, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 2".into()] });
    let frame = core.build_frame();
    // a window layer + menubar + dock present
    assert!(frame.layers.len() >= 3);
    assert_eq!(core.window_count(), 1);
    core.shutdown();
}

#[test]
fn click_dock_focuses_window() {
    let mut core = SessionCore::new(80, 24, Config::default());
    core.apply(ClientMsg::Launch { name: "a".into(), command: "sh".into(), args: vec!["-c".into(),"sleep 2".into()] });
    core.apply(ClientMsg::Launch { name: "b".into(), command: "sh".into(), args: vec!["-c".into(),"sleep 2".into()] });
    let regions = core.dock_regions();
    let (first_id, r) = regions[0];
    core.apply(ClientMsg::MouseDown(Point::new(r.x, r.y)));
    assert_eq!(core.focused(), Some(first_id));
    core.shutdown();
}

#[test]
fn clicking_menubar_quit_button_requests_quit() {
    use tuiui::chrome::menubar_quit_region;
    let mut core = SessionCore::new(80, 24, Config::default());
    assert!(!core.quit_requested());
    let r = menubar_quit_region(80);
    core.apply(ClientMsg::MouseDown(Point::new(r.x, 0)));
    assert!(core.quit_requested());
    core.shutdown();
}

#[test]
fn spotlight_launches_selected_app() {
    let mut core = SessionCore::new(80, 24, Config::default());
    assert!(!core.spotlight_open());
    core.apply(ClientMsg::ToggleSpotlight);
    assert!(core.spotlight_open());
    let before = core.window_count();
    core.apply(ClientMsg::LauncherEnter); // launch highlighted (the default shell)
    assert!(!core.launcher_open());
    assert_eq!(core.window_count(), before + 1);
    core.shutdown();
}

#[test]
fn open_store_creates_focused_store_window() {
    let mut core = SessionCore::new(120, 40, Config::default());
    assert!(!core.focused_is_store());
    core.apply(ClientMsg::OpenStore);
    assert!(core.focused_is_store());
    let n = core.window_count();
    // opening again focuses the same window, doesn't make a second
    core.apply(ClientMsg::OpenStore);
    assert_eq!(core.window_count(), n);
    core.apply(ClientMsg::StoreClose);
    assert!(!core.focused_is_store());
    core.shutdown();
}

#[test]
fn open_settings_creates_focused_settings_window() {
    let mut core = SessionCore::new(120, 40, Config::default());
    assert!(!core.focused_is_settings());
    core.apply(ClientMsg::OpenSettings);
    assert!(core.focused_is_settings());
    core.apply(ClientMsg::SettingsClose);
    assert!(!core.focused_is_settings());
    core.shutdown();
}

#[test]
fn image_window_emits_a_visible_placement() {
    // Write a tiny real PNG to a temp file.
    let dir = std::env::temp_dir().join(format!("tuiui-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.png");
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
    image::DynamicImage::ImageRgba8(img).save(&path).unwrap();

    let mut core = SessionCore::new(80, 24, Config { apps: vec![], ..Config::default() });
    core.apply(ClientMsg::OpenImage(path.to_string_lossy().to_string()));
    let frame = core.build_frame();
    assert_eq!(frame.images.len(), 1, "one image placement");
    assert!(frame.images[0].visible, "unobstructed → visible");
    assert!(frame.images[0].cols >= 1 && frame.images[0].rows >= 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_file_manager_creates_focused_window_and_is_single_instance() {
    let mut core = SessionCore::new(120, 40, Config::default());
    assert!(!core.focused_is_filemanager());
    core.apply(ClientMsg::OpenFileManager);
    assert!(core.focused_is_filemanager());
    let n = core.window_count();
    core.apply(ClientMsg::OpenFileManager);
    assert_eq!(core.window_count(), n); // re-focus, not a second window
    core.apply(ClientMsg::FileManagerClose);
    assert!(!core.focused_is_filemanager());
    core.shutdown();
}
