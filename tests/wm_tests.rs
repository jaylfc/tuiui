use tuiui::wm::WindowManager;
use tuiui::window::WindowState;
use tuiui::geometry::{Rect, Point, SnapZone};

fn wm() -> WindowManager { WindowManager::new(Rect::new(0, 1, 80, 22)) } // work area

#[test]
fn add_window_focuses_it_and_assigns_top_z() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2, 2, 20, 8));
    let b = m.add_window("b".into(), Rect::new(5, 5, 20, 8));
    assert_eq!(m.focused(), Some(b));
    assert!(m.get(b).unwrap().z > m.get(a).unwrap().z);
}

#[test]
fn raise_brings_to_front_and_focuses() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2, 2, 20, 8));
    let _b = m.add_window("b".into(), Rect::new(5, 5, 20, 8));
    m.raise(a);
    assert_eq!(m.focused(), Some(a));
    assert_eq!(m.topmost_at(Point::new(6, 6)), Some(a)); // a now covers overlap
}

#[test]
fn topmost_at_returns_highest_z_window_under_point() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(0, 1, 20, 8));
    assert_eq!(m.topmost_at(Point::new(1, 2)), Some(a));
    assert_eq!(m.topmost_at(Point::new(79, 20)), None);
}

#[test]
fn move_by_translates_rect() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2, 2, 20, 8));
    m.move_by(a, 3, 1);
    assert_eq!(m.get(a).unwrap().rect, Rect::new(5, 3, 20, 8));
}

#[test]
fn move_to_sets_position_and_floats() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2, 2, 20, 8));
    m.snap(a, SnapZone::Left); // put it in a snapped state first
    m.move_to(a, 5, 9);
    let w = m.get(a).unwrap();
    assert_eq!(w.rect.x, 5);
    assert_eq!(w.rect.y, 9);
    assert_eq!(w.state, WindowState::Floating);
}

#[test]
fn snap_left_sets_state_and_left_half_and_saves_restore() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(10, 5, 20, 8));
    m.snap(a, SnapZone::Left);
    let w = m.get(a).unwrap();
    assert_eq!(w.state, WindowState::SnappedLeft);
    assert_eq!(w.rect, Rect::new(0, 1, 40, 22));
    assert_eq!(w.restore_rect, Rect::new(10, 5, 20, 8));
}

#[test]
fn resize_to_enforces_minimum() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2, 2, 20, 8));
    m.resize_to(a, 1, 1); // below min
    let w = m.get(a).unwrap();
    assert!(w.rect.w >= 8 && w.rect.h >= 3);
}

#[test]
fn close_removes_and_refocuses_next_top() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2, 2, 20, 8));
    let b = m.add_window("b".into(), Rect::new(5, 5, 20, 8));
    m.close(b);
    assert!(m.get(b).is_none());
    assert_eq!(m.focused(), Some(a));
}

// ── Task 9: render_window ─────────────────────────────────────────────────────

use tuiui::wm::render_window;
use tuiui::buffer::CellBuffer;

#[test]
fn render_window_draws_title_and_content() {
    let mut m = wm();
    // 24 wide: enough room for the "btop" title plus the right-side controls.
    let id = m.add_window("btop".into(), Rect::new(0, 1, 24, 5));
    let mut content = CellBuffer::new(10, 3);
    content.write_str(0, 0, "hello", tuiui::cell::Rgba::rgb(255, 255, 255), tuiui::cell::Rgba::TRANSPARENT);
    let layers = render_window(m.get(id).unwrap(), &content, true, true);
    // shadow layer + window layer
    assert!(!layers.is_empty());
    let win_layer = layers.last().unwrap();
    let titlerow: String = (0..12).map(|x| win_layer.buf.get(x, 0).unwrap().ch).collect();
    assert!(titlerow.contains("btop"));
    // content 'h' should appear at inner (1,1)
    assert_eq!(win_layer.buf.get(1, 1).unwrap().ch, 'h');
}

#[test]
fn maximize_toggle_fills_work_area_and_restores() {
    let mut m = wm(); // work area Rect::new(0,1,80,22)
    let a = m.add_window("a".into(), Rect::new(10,5,20,8));
    m.maximize_toggle(a);
    assert_eq!(m.get(a).unwrap().rect, Rect::new(0,1,80,22));
    assert_eq!(m.get(a).unwrap().state, WindowState::Maximized);
    m.maximize_toggle(a);
    assert_eq!(m.get(a).unwrap().rect, Rect::new(10,5,20,8));
    assert_eq!(m.get(a).unwrap().state, WindowState::Floating);
}

#[test]
fn minimize_hides_and_moves_focus_then_unminimize_restores() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2,2,20,8));
    let b = m.add_window("b".into(), Rect::new(5,5,20,8));
    m.minimize(b);
    assert!(m.get(b).unwrap().minimized);
    assert_eq!(m.focused(), Some(a)); // focus moved off the minimized window
    m.unminimize(b);
    assert!(!m.get(b).unwrap().minimized);
    assert_eq!(m.focused(), Some(b)); // restored + raised
}

use tuiui::geometry::Grid;

#[test]
fn send_to_cell_places_window_in_grid() {
    let work = Rect::new(0, 1, 12, 6);
    let mut m = WindowManager::new(work);
    let a = m.add_window("a".into(), Rect::new(0, 1, 3, 3));
    m.send_to_cell(a, Grid { rows: 2, cols: 3 }, 1, 2, 0);
    let w = m.get(a).unwrap();
    assert_eq!(w.rect, Rect::new(8, 4, 4, 3));
    assert_eq!(w.state, WindowState::Tiled { row: 1, col: 2 });
}

#[test]
fn tile_all_assigns_cells_in_z_order() {
    let work = Rect::new(0, 1, 12, 6);
    let mut m = WindowManager::new(work);
    let a = m.add_window("a".into(), Rect::new(0, 1, 3, 3));
    let b = m.add_window("b".into(), Rect::new(0, 1, 3, 3));
    m.tile_all(Grid { rows: 1, cols: 2 }, 0);
    assert_eq!(m.get(a).unwrap().rect, Rect::new(0, 1, 6, 6));
    assert_eq!(m.get(b).unwrap().rect, Rect::new(6, 1, 6, 6));
}

#[test]
fn swap_cells_exchanges_two_windows() {
    let work = Rect::new(0, 1, 12, 6);
    let mut m = WindowManager::new(work);
    let a = m.add_window("a".into(), Rect::new(0, 1, 3, 3));
    let b = m.add_window("b".into(), Rect::new(0, 1, 3, 3));
    m.tile_all(Grid { rows: 1, cols: 2 }, 0);
    let (ra, rb) = (m.get(a).unwrap().rect, m.get(b).unwrap().rect);
    m.swap_cells(a, b);
    assert_eq!(m.get(a).unwrap().rect, rb);
    assert_eq!(m.get(b).unwrap().rect, ra);
}

#[test]
fn clamp_pulls_offscreen_and_minimized_windows_into_work() {
    let mut m = WindowManager::new(Rect::new(0, 1, 300, 38));
    let stranded = m.add_window("far".into(), Rect::new(200, 5, 80, 20));
    let minimized = m.add_window("min".into(), Rect::new(250, 8, 40, 12));
    let fine = m.add_window("ok".into(), Rect::new(2, 2, 30, 10));
    m.minimize(minimized);
    m.set_work_area(Rect::new(0, 1, 120, 28));
    let changed = m.clamp_all_into_work();
    assert!(changed.contains(&stranded) && changed.contains(&minimized));
    assert!(!changed.contains(&fine), "an in-bounds window is untouched");
    for id in [stranded, minimized, fine] {
        let r = m.get(id).unwrap().rect;
        assert!(r.x >= 0 && r.y >= 1 && r.x + r.w <= 120 && r.y + r.h <= 29, "{id:?} inside: {r:?}");
    }
    // A window larger than the new screen is shrunk to fit.
    let huge = m.add_window("huge".into(), Rect::new(0, 1, 500, 100));
    m.clamp_all_into_work();
    let r = m.get(huge).unwrap().rect;
    assert!(r.w <= 120 && r.h <= 28, "oversized window shrunk: {r:?}");
}
