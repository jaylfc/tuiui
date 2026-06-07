use tuiui::chrome::{render_menubar, render_dock, dock_hit_regions, DockItem, menubar_mode_region};
use tuiui::geometry::Point;
use tuiui::window::WindowId;

#[test]
fn menubar_layer_spans_top_row_and_shows_brand() {
    // 40 cols: realistic width where the Go button + app name + power button all fit.
    let layer = render_menubar(40, "btop", &[], false);
    assert_eq!(layer.origin, Point::new(0,0));
    assert_eq!(layer.buf.height(), 1);
    let row: String = (0..40).map(|x| layer.buf.get(x,0).unwrap().ch).collect();
    assert!(row.contains("Go"));
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
fn menubar_shows_mode_toggle_glyph() {
    let desktop: String = (0..40).map(|x| render_menubar(40, "x", &[], false).buf.get(x, 0).unwrap().ch).collect();
    assert!(desktop.contains('\u{229E}'), "desktop mode shows ⊞, got {desktop:?}");
    let simple: String = (0..40).map(|x| render_menubar(40, "x", &[], true).buf.get(x, 0).unwrap().ch).collect();
    assert!(simple.contains('\u{25A6}'), "simple mode shows ▦, got {simple:?}");
    // region sits just right of Go
    let r = menubar_mode_region();
    assert_eq!(r.y, 0);
    assert!(r.x >= 4);
}

#[test]
fn menubar_has_power_button_on_right() {
    use tuiui::chrome::menubar_power_region;
    let width = 40;
    let layer = render_menubar(width, "btop", &[], false);
    let row: String = (0..width).map(|x| layer.buf.get(x, 0).unwrap().ch).collect();
    assert!(row.contains("tuiui"), "menubar should show the tuiui power button, got: {row:?}");
    // region is on the top row, flush to the right edge
    let r = menubar_power_region(width);
    assert_eq!(r.y, 0);
    assert_eq!(r.right(), width - 1);
    // the region actually covers the power-button label
    let tcol = row.rfind('t').unwrap() as i32;
    assert!(r.contains(Point::new(tcol, 0)));
}
