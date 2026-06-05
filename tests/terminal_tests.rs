use tuiui::terminal::{Caps, frame_to_ansi};
use tuiui::compositor::CellChange;
use tuiui::cell::{Cell, Rgba};

#[test]
fn truecolor_change_emits_sgr_and_glyph() {
    let caps = Caps { truecolor: true, pixel_mouse: false, kitty_graphics: false };
    let changes = vec![CellChange { x: 2, y: 1, cell: Cell { ch: 'A', fg: Rgba::rgb(255,0,0), bg: Rgba::rgb(0,0,0), attrs: Default::default() } }];
    let out = frame_to_ansi(&changes, &caps);
    // cursor move to row 2 col 3 (1-based), set truecolor fg 255;0;0, print A
    assert!(out.contains("\x1b[2;3H"));
    assert!(out.contains("38;2;255;0;0"));
    assert!(out.contains('A'));
}

#[test]
fn no_changes_emits_nothing() {
    let caps = Caps { truecolor: true, pixel_mouse: false, kitty_graphics: false };
    assert_eq!(frame_to_ansi(&[], &caps), "");
}

#[test]
fn dark_grey_uses_grayscale_ramp_not_black_in_256() {
    let caps = Caps { truecolor: false, pixel_mouse: false, kitty_graphics: false };
    let changes = vec![CellChange { x: 0, y: 0, cell: Cell { ch: ' ', fg: Rgba::rgb(200,200,200), bg: Rgba::rgb(44,46,50), attrs: Default::default() } }];
    let out = frame_to_ansi(&changes, &caps);
    assert!(out.contains("48;5;235"), "dark grey should map to grayscale ramp 235, got: {out}");
    assert!(!out.contains("48;5;16"), "dark grey must not collapse to black (16)");
}
