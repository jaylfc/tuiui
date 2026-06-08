use tuiui::chrome::DockKind;
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
    // dock_regions returns (pill_index, Rect); apps "a" and "b" each get their own pill
    let items = core.dock_items_for_test();
    let regions = core.dock_regions();
    let (idx, r) = regions[0];
    // Extract the WindowId from the first pill (must be a Single since names differ)
    let first_id = match &items[idx].kind {
        DockKind::Single(id) => *id,
        DockKind::Group(_, ids) => ids[0],
    };
    core.apply(ClientMsg::MouseDown(Point::new(r.x, r.y)));
    assert_eq!(core.focused(), Some(first_id));
    core.shutdown();
}

#[test]
fn clicking_power_button_opens_menu_without_quitting() {
    let mut core = SessionCore::new(80, 24, Config::default());
    assert!(!core.quit_requested());
    assert!(!core.power_menu_open());
    // The power button (host name + ▾) is right-aligned, so the far-right cell of
    // the menubar is always inside it regardless of the host-name length.
    core.apply(ClientMsg::MouseDown(Point::new(79, 0)));
    assert!(core.power_menu_open(), "power button should open the menu");
    assert!(!core.quit_requested(), "opening the menu must not quit immediately");
    // A click elsewhere dismisses the menu.
    core.apply(ClientMsg::MouseDown(Point::new(1, 12)));
    assert!(!core.power_menu_open(), "click outside should dismiss the menu");
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

    // Disable the desktop so the only image placement is the ImageView's (desktop
    // icons now emit their own image placements).
    let mut core = SessionCore::new(80, 24, Config { apps: vec![], desktop_enabled: false, ..Config::default() });
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
    // Thumbnails load on a background thread now; pump until it arrives.
    let frame = pump_until_image(&mut core);
    assert!(frame.images.iter().any(|p| p.cols >= 1), "expected a thumbnail placement");
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Drive the background thumbnail loader until `build_frame` emits an image
/// placement (or a timeout), returning that frame.
fn pump_until_image(core: &mut SessionCore) -> tuiui::session::Frame {
    for _ in 0..200 {
        core.pump_thumbnails();
        let frame = core.build_frame();
        if frame.images.iter().any(|p| p.cols >= 1) {
            return frame;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    core.build_frame()
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
    // "proj" is the only icon (idx 0); derive a point inside its tile (the layout
    // is right-aligned, so don't assume top-left).
    let tr = core.desktop_icon_tile_for_test(0);
    let p = tuiui::geometry::Point::new(tr.x + 1, tr.y + 1);
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
    // Thumbnails load on a background thread now; pump until it arrives.
    let frame = pump_until_image(&mut core);
    assert!(frame.images.iter().any(|pl| pl.cols >= 1), "expected a desktop thumbnail placement");
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn app_graphics_placement_is_emitted_in_frame() {
    // Drive an AppInstance's GraphicsState directly, then assert build_frame emits it.
    let mut core = SessionCore::new(100, 30, Config::default());
    core.apply(ClientMsg::Launch { name: "sh".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    // Inject a placement+image into the launched app's graphics state via a test helper.
    let png = {
        let i = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
        let mut b = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(i).write_to(&mut b, image::ImageFormat::Png).unwrap();
        b.into_inner()
    };
    core.inject_app_graphics_for_test(&png);
    let frame = core.build_frame();
    assert!(frame.images.iter().any(|p| p.cols >= 1), "expected an app graphics placement");
    core.shutdown();
}

#[test]
fn restore_rebuilds_app_window_from_meta() {
    // Launch an app, push its meta, then simulate a fresh frontend over the SAME
    // in-process host by constructing a new SessionCore around a host that already
    // owns the app. We approximate this by reusing the same core: drop its window
    // mapping, then restore.
    let mut core = SessionCore::new(100, 30, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.sync_app_meta();
    let before = core.window_count();
    assert_eq!(before, 1);
    // Forget the window (as a fresh frontend would) but keep the app in the host.
    core.forget_windows_for_test();
    assert_eq!(core.window_count(), 0);
    let restored = core.restore_windows_from_host();
    assert_eq!(restored, 1);
    assert_eq!(core.window_count(), 1);
    core.shutdown();
}

#[test]
fn sync_app_meta_records_window_state() {
    let mut core = SessionCore::new(100, 30, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.sync_app_meta();
    // The app's meta is now populated (non-empty) for restore.
    assert!(core.app_meta_count_for_test() > 0);
    core.shutdown();
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

#[test]
fn mode_toggle_switches_view() {
    use tuiui::chrome::menubar_mode_region;
    let mut core = SessionCore::new(120, 40, Config::default());
    assert!(!core.simple_mode());
    let r = menubar_mode_region();
    core.apply(ClientMsg::MouseDown(Point::new(r.x, 0)));
    assert!(core.simple_mode(), "clicking the toggle enters simple mode");
    core.apply(ClientMsg::MouseDown(Point::new(r.x, 0)));
    assert!(!core.simple_mode(), "clicking again returns to desktop");
    core.shutdown();
}

#[test]
fn simple_mode_renders_focused_app_fullscreen_without_desktop() {
    use tuiui::chrome::menubar_mode_region;
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.apply(ClientMsg::MouseDown(Point::new(menubar_mode_region().x, 0)));
    assert!(core.simple_mode());
    let frame = core.build_frame();
    // The focused app's content layer originates at the work area top (row 1),
    // i.e. there is a content layer at y=1 spanning the width.
    assert!(frame.layers.iter().any(|l| l.origin.y == 1 && l.buf.width() == 120),
        "focused app should fill the work-area width at row 1");
    core.shutdown();
}

#[test]
fn app_mouse_area_none_without_mouse_mode() {
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    // A bare shell hasn't enabled mouse reporting → no app area, passthrough off.
    assert!(core.app_mouse_area().is_none());
    core.shutdown();
}

#[test]
fn app_mouse_area_suppressed_while_launcher_open() {
    // An open overlay (the Go launcher) must take the mouse — even over a focused
    // app — so clicking an app in the menu is never swallowed by passthrough.
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.apply(ClientMsg::ToggleMenu); // open the Go launcher
    assert!(core.launcher_open());
    assert!(core.app_mouse_area().is_none(), "no app passthrough area while the launcher is open");
    core.shutdown();
}

#[test]
fn two_same_app_windows_group_in_dock() {
    let mut core = SessionCore::new(120, 40, Config::default());
    let launch = |c: &mut SessionCore| c.apply(ClientMsg::Launch { name: "Claude".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    launch(&mut core);
    assert_eq!(core.dock_pill_count_for_test(), 1); // one window → one pill
    launch(&mut core);
    assert_eq!(core.dock_pill_count_for_test(), 1); // two Claude → still ONE grouped pill
    assert_eq!(core.window_count(), 2);
    core.shutdown();
}

#[test]
fn rename_changes_label_not_grouping() {
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "Claude".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.apply(ClientMsg::RenameFocused);
    for c in "appname".chars() { core.apply(ClientMsg::RenameChar(c)); }
    core.apply(ClientMsg::RenameCommit);
    assert_eq!(core.focused_label_for_test(), "appname");
    // a second Claude still groups with the renamed one (same app_key)
    core.apply(ClientMsg::Launch { name: "Claude".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    assert_eq!(core.dock_pill_count_for_test(), 1);
    core.shutdown();
}

#[test]
fn clicking_titlebar_does_not_move_tiled_window() {
    // Regression: a plain click on a tiled window's titlebar (e.g. to rename it)
    // must NOT untile/move the window — only a real drag should.
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "a".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.apply(ClientMsg::SendToCell(1)); // tile into a grid cell
    let before = core.focused_window_rect_for_test().unwrap();
    // Click (down then up at the SAME titlebar point — no drag motion).
    let p = Point::new(before.x + 2, before.y);
    core.apply(ClientMsg::MouseDown(p));
    core.apply(ClientMsg::MouseUp(p));
    let after = core.focused_window_rect_for_test().unwrap();
    assert_eq!(before, after, "a plain titlebar click must not move/untile a tiled window");
    core.shutdown();
}

#[test]
fn double_click_titlebar_full_sequence_keeps_tiled_window_put() {
    // Faithful repro of a real double-click on a tiled window's titlebar with
    // all-motion mouse: Down, Drag(same p), Up, Double, Up.
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "a".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.apply(ClientMsg::SendToCell(1));
    let before = core.focused_window_rect_for_test().unwrap();
    let p = Point::new(before.x + 2, before.y);
    core.apply(ClientMsg::MouseDown(p));
    core.apply(ClientMsg::MouseDrag(p));   // all-motion Moved at same cell
    core.apply(ClientMsg::MouseUp(p));
    core.apply(ClientMsg::MouseDouble(p)); // second click → rename
    core.apply(ClientMsg::MouseUp(p));
    let after = core.focused_window_rect_for_test().unwrap();
    assert_eq!(before, after, "double-click titlebar must not move the tiled window (before={before:?} after={after:?})");
    core.shutdown();
}

#[test]
fn spurious_teleport_drag_does_not_fling_window() {
    // Repro of the real bug: while dragging, a single stray mouse report jumped
    // >half the screen (a click at the top yielded a drag to the bottom),
    // flinging the window off-screen. Such teleports must be ignored.
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "a".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.apply(ClientMsg::SendToCell(1));
    let before = core.focused_window_rect_for_test().unwrap();
    let p = Point::new(before.x + 2, before.y);
    core.apply(ClientMsg::MouseDown(p));
    // Stray report teleporting to row 30 (jump > h/2 = 20) — impossible for a real drag.
    core.apply(ClientMsg::MouseDrag(Point::new(p.x, 30)));
    core.apply(ClientMsg::MouseUp(Point::new(p.x, 30)));
    let after = core.focused_window_rect_for_test().unwrap();
    assert_eq!(before, after, "a spurious teleport drag must not move the window (before={before:?} after={after:?})");
    core.shutdown();
}

#[test]
fn dock_plus_button_opens_a_shell_window() {
    use tuiui::chrome::dock_new_shell_region;
    let mut core = SessionCore::new(120, 40, Config::default());
    let before = core.window_count();
    let r = dock_new_shell_region(40); // bottom-left of a height-40 screen
    core.apply(ClientMsg::MouseDown(Point::new(r.x, r.y)));
    assert_eq!(core.window_count(), before + 1, "clicking + should open a new shell window");
    core.shutdown();
}

#[test]
fn closing_launcher_by_clicking_brand_does_not_launch_shell() {
    // Open the launcher (click the brand), then click the brand again to close it.
    // Closing must NOT activate the auto-selected first row (now "Shell").
    use tuiui::chrome::menubar_brand_region;
    let mut core = SessionCore::new(120, 40, Config::default());
    let before = core.window_count();
    let p = Point::new(menubar_brand_region().x + 1, 0);
    core.apply(ClientMsg::MouseDown(p)); // open
    assert!(core.launcher_open());
    core.apply(ClientMsg::MouseDown(p)); // click brand again → close
    assert!(!core.launcher_open(), "clicking the brand again should close the launcher");
    assert_eq!(core.window_count(), before, "closing the launcher must not open a shell");
    core.shutdown();
}

#[test]
fn closing_app_window_requires_confirmation() {
    // Clicking the titlebar ✕ on an app window opens a modal confirm dialog and
    // does NOT kill the app until the user confirms (it kills the process).
    let mut core = SessionCore::new(
        80,
        24,
        Config { apps: vec![], desktop_enabled: false, ..Config::default() },
    );
    core.apply(ClientMsg::Launch {
        name: "shell".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 5".into()],
    });
    assert_eq!(core.window_count(), 1);
    let r = core.focused_window_rect_for_test().unwrap();
    let close = Point::new(r.x + r.w - 3, r.y); // close glyph column

    // First click: dialog opens, window stays.
    core.apply(ClientMsg::MouseDown(close));
    assert!(core.confirm_close_open(), "closing an app window opens the confirm dialog");
    assert_eq!(core.window_count(), 1, "window must not close until confirmed");

    // Cancel keeps the window.
    core.apply(ClientMsg::ConfirmCloseNo);
    assert!(!core.confirm_close_open());
    assert_eq!(core.window_count(), 1);

    // Click again, then confirm: the window closes.
    core.apply(ClientMsg::MouseDown(close));
    assert!(core.confirm_close_open());
    core.apply(ClientMsg::ConfirmCloseYes);
    assert!(!core.confirm_close_open());
    assert_eq!(core.window_count(), 0, "confirming closes the app window");
    core.shutdown();
}

#[test]
fn closing_filemanager_does_not_confirm() {
    // Built-in panels (File Manager / Store / Settings) have no process to lose,
    // so the titlebar ✕ closes them immediately with no confirm dialog.
    let mut core = SessionCore::new(
        100,
        30,
        Config { apps: vec![], desktop_enabled: false, ..Config::default() },
    );
    core.apply(ClientMsg::OpenFileManager);
    assert_eq!(core.window_count(), 1);
    let r = core.focused_window_rect_for_test().unwrap();
    core.apply(ClientMsg::MouseDown(Point::new(r.x + r.w - 3, r.y)));
    assert!(!core.confirm_close_open(), "built-in panels close without a prompt");
    assert_eq!(core.window_count(), 0, "file manager closes immediately");
    core.shutdown();
}
