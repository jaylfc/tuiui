use tuiui::cell::{Rgba, Cell, CellAttrs};

#[test]
fn opaque_over_keeps_src() {
    let dst = Rgba::rgb(0, 0, 0);
    let src = Rgba::rgb(255, 0, 0);
    assert_eq!(src.over(dst), Rgba::rgb(255, 0, 0));
}

#[test]
fn half_alpha_blends_midway() {
    let dst = Rgba::rgb(0, 0, 0);
    let src = Rgba::new(255, 255, 255, 128);
    let out = src.over(dst);
    // 255 * 128/255 + 0 ≈ 128
    assert_eq!(out, Rgba::rgb(128, 128, 128));
}

#[test]
fn transparent_over_keeps_dst() {
    let dst = Rgba::rgb(10, 20, 30);
    let src = Rgba::new(255, 255, 255, 0);
    assert_eq!(src.over(dst), dst);
}

#[test]
fn default_cell_is_blank_space() {
    let c = Cell::default();
    assert_eq!(c.ch, ' ');
    assert_eq!(c.attrs, CellAttrs::default());
}
