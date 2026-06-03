use tuiui::ptyhost::AppInstance;
use std::time::Duration;

#[test]
fn spawns_and_captures_output() {
    // child prints "READY" then sleeps; we read the parsed grid
    let mut app = AppInstance::spawn("sh", &["-c".into(), "printf READY; sleep 1".into()], 20, 5).unwrap();
    // give the reader thread a moment
    std::thread::sleep(Duration::from_millis(300));
    let grid = app.snapshot();
    let row0: String = (0..20).map(|x| grid.get(x, 0).map(|c| c.ch).unwrap_or(' ')).collect();
    assert!(row0.starts_with("READY"), "got: {:?}", row0);
    app.kill();
}

#[test]
fn resize_changes_grid_dims() {
    let mut app = AppInstance::spawn("sh", &["-c".into(), "sleep 1".into()], 20, 5).unwrap();
    app.resize(30, 8);
    let grid = app.snapshot();
    assert_eq!(grid.width(), 30);
    assert_eq!(grid.height(), 8);
    app.kill();
}
