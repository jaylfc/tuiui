use tuiui::geometry::{Point, Rect, SnapZone, snap_zone, snapped_rect};

#[test]
fn contains_point() {
    let r = Rect::new(2, 3, 10, 5);
    assert!(r.contains(Point::new(2, 3)));
    assert!(r.contains(Point::new(11, 7)));
    assert!(!r.contains(Point::new(12, 7)));
    assert!(!r.contains(Point::new(1, 3)));
}

#[test]
fn right_bottom_edges() {
    let r = Rect::new(0, 0, 4, 3);
    assert_eq!(r.right(), 3);
    assert_eq!(r.bottom(), 2);
}

#[test]
fn snap_zone_detects_left_right_within_threshold() {
    let screen = Rect::new(0, 0, 80, 24);
    assert_eq!(snap_zone(Point::new(2, 10), screen, 3), Some(SnapZone::Left));
    assert_eq!(snap_zone(Point::new(78, 10), screen, 3), Some(SnapZone::Right));
    assert_eq!(snap_zone(Point::new(40, 10), screen, 3), None);
}

#[test]
fn snapped_rect_left_is_left_half_below_menubar_above_dock() {
    // work area excludes 1-row menubar (top) and 1-row dock (bottom)
    let work = Rect::new(0, 1, 80, 22);
    let left = snapped_rect(SnapZone::Left, work);
    assert_eq!(left, Rect::new(0, 1, 40, 22));
    let right = snapped_rect(SnapZone::Right, work);
    assert_eq!(right, Rect::new(40, 1, 40, 22));
}
