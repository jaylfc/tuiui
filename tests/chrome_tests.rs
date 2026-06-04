use tuiui::chrome::{render_menubar, render_dock, dock_hit_regions, DockItem};
use tuiui::geometry::Point;
use tuiui::window::WindowId;

#[test]
fn menubar_layer_spans_top_row_and_shows_brand() {
    // 40 cols: realistic width where brand + app name + quit button all fit.
    let layer = render_menubar(40, "btop", &[]);
    assert_eq!(layer.origin, Point::new(0,0));
    assert_eq!(layer.buf.height(), 1);
    let row: String = (0..40).map(|x| layer.buf.get(x,0).unwrap().ch).collect();
    assert!(row.contains("Tuiui"));
    assert!(row.contains("btop"));
}

#[test]
fn dock_layer_is_bottom_row() {
    let items = vec![DockItem { id: WindowId(1), label: "btop".into(), focused: true }];
    let layer = render_dock(40, 24, &items);
    assert_eq!(layer.origin, Point::new(0, 23));
}

#[test]
fn dock_hit_regions_map_clicks_to_windows() {
    let items = vec![
        DockItem { id: WindowId(1), label: "btop".into(), focused: true },
        DockItem { id: WindowId(2), label: "lazygit".into(), focused: false },
    ];
    let regions = dock_hit_regions(40, 24, &items);
    // a click inside the first region resolves to WindowId(1)
    let first = regions.iter().find(|(_, r)| r.contains(Point::new(r.x, 23))).map(|(id,_)| *id);
    assert_eq!(first, Some(WindowId(1)));
    // regions are on the bottom row
    assert!(regions.iter().all(|(_, r)| r.y == 23));
}

#[test]
fn menubar_has_quit_button_on_right() {
    use tuiui::chrome::menubar_quit_region;
    let width = 40;
    let layer = render_menubar(width, "btop", &[]);
    let row: String = (0..width).map(|x| layer.buf.get(x, 0).unwrap().ch).collect();
    assert!(row.contains("Quit"), "menubar should show a Quit button, got: {row:?}");
    // region is on the top row, flush to the right edge
    let r = menubar_quit_region(width);
    assert_eq!(r.y, 0);
    assert_eq!(r.right(), width - 1);
    // the region actually covers the 'Q' of "Quit"
    let qcol = row.find('Q').unwrap() as i32;
    assert!(r.contains(Point::new(qcol, 0)));
}
