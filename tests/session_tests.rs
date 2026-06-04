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
