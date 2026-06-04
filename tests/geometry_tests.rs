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

use tuiui::geometry::Grid;

#[test]
fn grid_cell_rects_tile_the_work_area() {
    let work = Rect::new(0, 1, 12, 6);
    let g = Grid { rows: 2, cols: 3 };
    assert_eq!(g.cells(), 6);
    assert_eq!(g.cell_rect(work, 0, 0, 0), Rect::new(0, 1, 4, 3));
    assert_eq!(g.cell_rect(work, 1, 2, 0), Rect::new(8, 4, 4, 3));
}

#[test]
fn grid_cell_at_maps_pointer_to_cell() {
    let work = Rect::new(0, 1, 12, 6);
    let g = Grid { rows: 2, cols: 3 };
    assert_eq!(g.cell_at(work, Point::new(1, 2)), (0, 0));
    assert_eq!(g.cell_at(work, Point::new(9, 5)), (1, 2));
    assert_eq!(g.cell_at(work, Point::new(999, 999)), (1, 2));
}

#[test]
fn grid_index_round_trip() {
    let g = Grid { rows: 2, cols: 3 };
    assert_eq!(g.index_of(1, 2), 5);
    assert_eq!(g.row_col(5), (1, 2));
    assert_eq!(g.row_col(0), (0, 0));
}

#[test]
fn grid_gap_shrinks_cells() {
    let work = Rect::new(0, 0, 12, 4);
    let g = Grid { rows: 1, cols: 2 };
    let a = g.cell_rect(work, 0, 0, 1);
    let b = g.cell_rect(work, 0, 1, 1);
    assert!(a.x + a.w <= b.x);
}
