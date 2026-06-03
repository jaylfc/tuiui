use tuiui::terminal::{Caps, frame_to_ansi};
use tuiui::compositor::CellChange;
use tuiui::cell::{Cell, Rgba};

#[test]
fn truecolor_change_emits_sgr_and_glyph() {
    let caps = Caps { truecolor: true, pixel_mouse: false };
    let changes = vec![CellChange { x: 2, y: 1, cell: Cell { ch: 'A', fg: Rgba::rgb(255,0,0), bg: Rgba::rgb(0,0,0), attrs: Default::default() } }];
    let out = frame_to_ansi(&changes, &caps);
    // cursor move to row 2 col 3 (1-based), set truecolor fg 255;0;0, print A
    assert!(out.contains("\x1b[2;3H"));
    assert!(out.contains("38;2;255;0;0"));
    assert!(out.contains('A'));
}

#[test]
fn no_changes_emits_nothing() {
    let caps = Caps { truecolor: true, pixel_mouse: false };
    assert_eq!(frame_to_ansi(&[], &caps), "");
}
