use tuiui::input::{route_mouse, MouseKind, Hit, Action};
use tuiui::geometry::{Rect, Point};
use tuiui::window::{Window, WindowId, WindowState};

fn win(id: u64, rect: Rect, z: i32) -> Window {
    Window { id: WindowId(id), title: "t".into(), rect, z, state: WindowState::Floating, restore_rect: rect }
}

#[test]
fn click_on_titlebar_starts_move() {
    let w = win(1, Rect::new(0,1,20,8), 1);
    let act = route_mouse(MouseKind::Down, Point::new(3,1), &[w.clone()], None);
    assert_eq!(act, Action::BeginMove(WindowId(1)));
}

#[test]
fn click_on_close_glyph_closes() {
    let w = win(1, Rect::new(0,1,20,8), 1);
    // close glyph at rect.w-2 => x=18, titlebar row y=1
    let act = route_mouse(MouseKind::Down, Point::new(18,1), &[w], None);
    assert_eq!(act, Action::Close(WindowId(1)));
}

#[test]
fn click_on_bottom_right_corner_starts_resize() {
    let w = win(1, Rect::new(0,1,20,8), 1); // bottom row y=8, right col x=19
    let act = route_mouse(MouseKind::Down, Point::new(19,8), &[w], None);
    assert_eq!(act, Action::BeginResize(WindowId(1)));
}

#[test]
fn click_in_content_focuses_and_forwards() {
    let w = win(1, Rect::new(0,1,20,8), 1);
    let act = route_mouse(MouseKind::Down, Point::new(5,4), &[w], None);
    // content area: raises + forwards local coords (5-1, 4-2) = (4,2)
    assert_eq!(act, Action::FocusAndForward { id: WindowId(1), local: Point::new(4,2) });
}

#[test]
fn drag_while_moving_emits_move_to() {
    let w = win(1, Rect::new(0,1,20,8), 1);
    let act = route_mouse(MouseKind::Drag, Point::new(10,5), &[w], Some(Hit::Moving { id: WindowId(1), grab_dx: 3, grab_dy: 0 }));
    assert_eq!(act, Action::MoveTo { id: WindowId(1), x: 7, y: 5 });
}

#[test]
fn click_empty_desktop_is_noop() {
    let act = route_mouse(MouseKind::Down, Point::new(70,20), &[], None);
    assert_eq!(act, Action::None);
}
