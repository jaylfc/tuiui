use tuiui::buffer::CellBuffer;
use tuiui::cell::{Cell, Rgba};

#[test]
fn new_buffer_is_default_filled() {
    let b = CellBuffer::new(4, 2);
    assert_eq!(b.width(), 4);
    assert_eq!(b.height(), 2);
    assert_eq!(b.get(0, 0).unwrap().ch, ' ');
}

#[test]
fn set_and_get_roundtrip() {
    let mut b = CellBuffer::new(3, 3);
    let c = Cell { ch: 'X', bg: Rgba::rgb(1, 2, 3), ..Default::default() };
    b.set(1, 2, c);
    assert_eq!(b.get(1, 2).unwrap().ch, 'X');
    assert_eq!(b.get(1, 2).unwrap().bg, Rgba::rgb(1, 2, 3));
}

#[test]
fn out_of_bounds_is_none() {
    let b = CellBuffer::new(2, 2);
    assert!(b.get(2, 0).is_none());
    assert!(b.get(0, 2).is_none());
}

#[test]
fn write_str_sets_consecutive_cells() {
    let mut b = CellBuffer::new(6, 1);
    b.write_str(1, 0, "hi", Rgba::rgb(255,255,255), Rgba::TRANSPARENT);
    assert_eq!(b.get(1, 0).unwrap().ch, 'h');
    assert_eq!(b.get(2, 0).unwrap().ch, 'i');
}
