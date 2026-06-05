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
fn file_manager_emits_thumbnail_placement_for_image() {
    let dir = std::env::temp_dir().join(format!("tuiui-fmthumb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
    image::DynamicImage::ImageRgba8(img).save(dir.join("p.png")).unwrap();

    let mut core = SessionCore::new(100, 30, Config::default());
    core.apply(ClientMsg::OpenFileManager);
    // navigate the FM into our temp dir by faking it: open at temp via a second open
    core.open_filemanager_at(dir.clone()); // test helper below
    let frame = core.build_frame();
    assert!(frame.images.iter().any(|p| p.cols >= 1), "expected a thumbnail placement");
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn desktop_click_selects_and_double_click_opens_files() {
    let dir = std::env::temp_dir().join(format!("tuiui-deskwire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir(dir.join("proj")).unwrap();

    // Point the desktop at our temp dir via a test helper.
    let mut core = SessionCore::new(100, 30, Config { desktop_pins: vec![], ..Config::default() });
    core.set_desktop_dir_for_test(dir.clone()); // reloads desktop at `dir`
    // "proj" is at cell (0,0): tile glyph at (7,1); click then double-click.
    let p = tuiui::geometry::Point::new(2, 1);
    core.apply(ClientMsg::MouseDown(p));
    assert_eq!(core.desktop_selection_len_for_test(), 1);
    let before = core.window_count();
    core.apply(ClientMsg::MouseDouble(p));
    assert_eq!(core.window_count(), before + 1); // a Files window opened on the folder
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn desktop_new_folder_via_menu_creates_dir() {
    let dir = std::env::temp_dir().join(format!("tuiui-deskmk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut core = SessionCore::new(100, 30, Config { desktop_pins: vec![], ..Config::default() });
    core.set_desktop_dir_for_test(dir.clone());
    core.apply(ClientMsg::MouseRightDown(tuiui::geometry::Point::new(60, 20))); // empty desktop menu
    // Drive new-folder directly via the editing messages (menu click tested in unit tests):
    core.begin_desktop_new_folder_for_test();
    for c in "Stuff".chars() {
        core.apply(ClientMsg::DesktopChar(c));
    }
    core.apply(ClientMsg::DesktopCommit);
    assert!(dir.join("Stuff").is_dir());
    core.shutdown();
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

#[test]
fn desktop_image_icon_emits_thumbnail_placement() {
    let dir = std::env::temp_dir().join(format!("tuiui-deskthumb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([9, 9, 9, 255]));
    image::DynamicImage::ImageRgba8(img).save(dir.join("p.png")).unwrap();

    let mut core = SessionCore::new(100, 30, Config { desktop_pins: vec![], ..Config::default() });
    core.set_desktop_dir_for_test(dir.clone());
    let frame = core.build_frame();
    assert!(frame.images.iter().any(|pl| pl.cols >= 1), "expected a desktop thumbnail placement");
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cascade_keyboard_launches_app_from_submenu() {
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::ToggleMenu);
    assert!(core.launcher_open_for_test());
    let before = core.window_count();
    core.apply(ClientMsg::LauncherRight); // descend into the first category
    core.apply(ClientMsg::LauncherEnter); // launch the first app in it
    assert!(!core.launcher_open_for_test()); // menu closed after launch
    assert_eq!(core.window_count(), before + 1);
    core.shutdown();
}
