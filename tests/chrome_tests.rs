use tuiui::chrome::{render_menubar, render_dock, dock_hit_regions, DockItem};
use tuiui::geometry::Point;
use tuiui::window::WindowId;

#[test]
fn menubar_layer_spans_top_row_and_shows_brand() {
    let layer = render_menubar(20, "btop");
    assert_eq!(layer.origin, Point::new(0,0));
    assert_eq!(layer.buf.height(), 1);
    let row: String = (0..20).map(|x| layer.buf.get(x,0).unwrap().ch).collect();
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
