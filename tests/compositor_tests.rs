use tuiui::compositor::{Compositor, Layer, CellChange};
use tuiui::buffer::CellBuffer;
use tuiui::cell::{Cell, Rgba};
use tuiui::geometry::Point;

fn glyph(ch: char, bg: Rgba) -> Cell { Cell { ch, fg: Rgba::rgb(255,255,255), bg, attrs: Default::default() } }

#[test]
fn higher_z_layer_wins_glyph() {
    let mut comp = Compositor::new(4, 1);
    let mut low = CellBuffer::new(4, 1); low.set(0,0, glyph('A', Rgba::rgb(10,10,10)));
    let mut high = CellBuffer::new(4, 1); high.set(0,0, glyph('B', Rgba::rgb(20,20,20)));
    let frame = comp.composite(&[
        Layer { z: 0, origin: Point::new(0,0), buf: low, opacity: 1.0, scissor: None },
        Layer { z: 5, origin: Point::new(0,0), buf: high, opacity: 1.0, scissor: None },
    ], None);
    assert_eq!(frame.get(0,0).unwrap().ch, 'B');
}

#[test]
fn transparent_bg_shows_lower_layer_through() {
    let mut comp = Compositor::new(1, 1);
    let mut low = CellBuffer::new(1,1); low.set(0,0, glyph('A', Rgba::rgb(0,0,0)));
    // shadow: space cell, semi-transparent black bg
    let mut shadow = CellBuffer::new(1,1); shadow.set(0,0, Cell { ch:' ', fg: Rgba::TRANSPARENT, bg: Rgba::new(0,0,0,128), attrs: Default::default() });
    let frame = comp.composite(&[
        Layer { z:0, origin: Point::new(0,0), buf: low, opacity:1.0, scissor: None },
        Layer { z:1, origin: Point::new(0,0), buf: shadow, opacity:1.0, scissor: None },
    ], None);
    // glyph 'A' preserved (shadow has no glyph), bg darkened toward black
    assert_eq!(frame.get(0,0).unwrap().ch, 'A');
}

#[test]
fn cursor_overlays_inverse() {
    let mut comp = Compositor::new(2,1);
    let base = CellBuffer::new(2,1);
    let frame = comp.composite(&[Layer{z:0,origin:Point::new(0,0),buf:base,opacity:1.0,scissor:None}], Some(Point::new(1,0)));
    assert!(frame.get(1,0).unwrap().attrs.inverse);
    assert!(!frame.get(0,0).unwrap().attrs.inverse);
}

#[test]
fn diff_reports_only_changed_cells() {
    let mut comp = Compositor::new(2,1);
    let base = || CellBuffer::new(2,1);
    let l0 = Layer{z:0,origin:Point::new(0,0),buf:base(),opacity:1.0,scissor:None};
    let _ = comp.composite(&[l0], None); // first frame: everything "changed"
    comp.commit();
    let mut b2 = CellBuffer::new(2,1); b2.set(1,0, glyph('Z', Rgba::rgb(0,0,0)));
    let _ = comp.composite(&[Layer{z:0,origin:Point::new(0,0),buf:b2,opacity:1.0,scissor:None}], None);
    let changes: Vec<CellChange> = comp.diff();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].x, 1);
    assert_eq!(changes[0].cell.ch, 'Z');
}

#[test]
fn opaque_blank_cell_covers_lower_glyph() {
    // A window's empty (opaque) cell must hide a window beneath it — not let it show through.
    let mut comp = Compositor::new(1, 1);
    let mut low = CellBuffer::new(1,1);
    low.set(0,0, glyph('X', Rgba::rgb(0,0,0)));
    let mut top = CellBuffer::new(1,1);
    top.set(0,0, Cell { ch:' ', fg: Rgba::rgb(200,200,200), bg: Rgba::rgb(13,15,22), attrs: Default::default() });
    let frame = comp.composite(&[
        Layer { z:0, origin: Point::new(0,0), buf: low, opacity:1.0, scissor: None },
        Layer { z:1, origin: Point::new(0,0), buf: top, opacity:1.0, scissor: None },
    ], None);
    assert_eq!(frame.get(0,0).unwrap().ch, ' ');           // covered, not 'X'
    assert_eq!(frame.get(0,0).unwrap().bg, Rgba::rgb(13,15,22));
}
