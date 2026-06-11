use tuiui::chrome::{render_menubar, render_dock, dock_hit_regions, DockItem, DockKind, menubar_new_shell_region, dock_mode_region};
use tuiui::cell::Rgba;
use tuiui::geometry::Point;
use tuiui::window::WindowId;

fn badge_color() -> Rgba { Rgba::rgb(70, 130, 230) }

#[test]
fn menubar_layer_spans_top_row_and_shows_brand() {
    // 40 cols: realistic width where the Go button + app name + power button all fit.
    let layer = render_menubar(40, "btop", &[], " devbox \u{25be} ");
    assert_eq!(layer.origin, Point::new(0,0));
    assert_eq!(layer.buf.height(), 1);
    let row: String = (0..40).map(|x| layer.buf.get(x,0).unwrap().ch).collect();
    assert!(row.contains("tuiui"));  // left brand button (opens launcher)
    assert!(row.contains("btop"));
    assert!(row.contains("devbox"));  // right power button shows the host name
}

#[test]
fn dock_layer_is_bottom_row() {
    let items = vec![DockItem {
        kind: DockKind::Single(WindowId(1)),
        label: "btop".into(),
        count: 1,
        badge_letter: 'B',
        badge_color: badge_color(),
        focused: true,
        attention: false,
    }];
    let layer = render_dock(40, 24, &items, false);
    assert_eq!(layer.origin, Point::new(0, 23));
}

#[test]
fn dock_hit_regions_map_clicks_to_pills() {
    let items = vec![
        DockItem {
            kind: DockKind::Single(WindowId(1)),
            label: "btop".into(),
            count: 1,
            badge_letter: 'B',
            badge_color: badge_color(),
            focused: true,
        attention: false,
        },
        DockItem {
            kind: DockKind::Single(WindowId(2)),
            label: "lazygit".into(),
            count: 1,
            badge_letter: 'L',
            badge_color: badge_color(),
            focused: false,
        attention: false,
        },
    ];
    let regions = dock_hit_regions(40, 24, &items);
    // first region is pill 0, second is pill 1
    assert_eq!(regions[0].0, 0);
    assert_eq!(regions[1].0, 1);
    // a click inside the first region hits pill 0
    let first_r = regions[0].1;
    assert!(first_r.contains(Point::new(first_r.x, 23)));
    // regions are on the bottom row
    assert!(regions.iter().all(|(_, r)| r.y == 23));
}

#[test]
fn dock_single_pill_renders_badge_letter() {
    let items = vec![DockItem {
        kind: DockKind::Single(WindowId(1)),
        label: "btop".into(),
        count: 1,
        badge_letter: 'B',
        badge_color: badge_color(),
        focused: false,
        attention: false,
    }];
    let layer = render_dock(40, 24, &items, false);
    // The bottom row should contain 'B' (the badge letter)
    let row: String = (0..40).map(|x| layer.buf.get(x, 0).unwrap().ch).collect();
    assert!(row.contains('B'), "badge letter 'B' should appear in dock row: {row:?}");
}

#[test]
fn dock_group_pill_renders_count_glyph() {
    let items = vec![DockItem {
        kind: DockKind::Group("Claude".into(), vec![WindowId(1), WindowId(2)]),
        label: "Claude".into(),
        count: 2,
        badge_letter: 'C',
        badge_color: badge_color(),
        focused: false,
        attention: false,
    }];
    let layer = render_dock(60, 24, &items, false);
    let row: String = (0..60).map(|x| layer.buf.get(x, 0).unwrap().ch).collect();
    // Group of 2 → superscript ² should appear
    assert!(row.contains('\u{00B2}'), "group pill should show ² for count=2: {row:?}");
}

#[test]
fn menubar_has_new_shell_and_dock_has_mode_toggle() {
    // The "+" new-shell button lives next to the brand (swapped with the mode
    // toggle, which moved to the dock's bottom-left).
    let bar: String = (0..40).map(|x| render_menubar(40, "x", &[], " h \u{25be} ").buf.get(x, 0).unwrap().ch).collect();
    assert!(bar.contains('+'), "menubar shows the new-shell +, got {bar:?}");
    let r = menubar_new_shell_region();
    assert_eq!(r.y, 0);
    assert!(r.x >= 7);

    let desktop: String = (0..40).map(|x| render_dock(40, 24, &[], false).buf.get(x, 0).unwrap().ch).collect();
    assert!(desktop.contains('\u{229E}'), "desktop mode shows ⊞ on the dock, got {desktop:?}");
    let simple: String = (0..40).map(|x| render_dock(40, 24, &[], true).buf.get(x, 0).unwrap().ch).collect();
    assert!(simple.contains('\u{25A6}'), "simple mode shows ▦ on the dock, got {simple:?}");
    let d = dock_mode_region(24);
    assert_eq!((d.x, d.y), (0, 23), "toggle sits at the dock's bottom-left");
}

#[test]
fn menubar_has_power_button_on_right() {
    use tuiui::chrome::menubar_power_region;
    let width = 40;
    let power = " devbox \u{25be} ";
    let layer = render_menubar(width, "btop", &[], power);
    let row: String = (0..width).map(|x| layer.buf.get(x, 0).unwrap().ch).collect();
    assert!(row.contains("devbox"), "menubar power button should show the host name, got: {row:?}");
    // region is on the top row, flush to the right edge
    let r = menubar_power_region(width, power);
    assert_eq!(r.y, 0);
    assert_eq!(r.right(), width - 1);
    // the region actually covers the power-button label (the 'x' in "devbox").
    // Use a CHAR column, not `rfind`'s byte offset — the bar holds multi-byte
    // glyphs (the ⊞ mode toggle and ✦ assistant button) left of the label.
    let chars: Vec<char> = row.chars().collect();
    let xcol = chars.iter().rposition(|c| *c == 'x').unwrap() as i32;
    assert!(r.contains(Point::new(xcol, 0)));
}
