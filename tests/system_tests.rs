use tuiui::system::{bars_glyph, mem_pct, volume_glyph, VolumeInfo};

#[test]
fn signal_bars_fill_left_to_right() {
    assert_eq!(bars_glyph(0), "····");
    assert_eq!(bars_glyph(1), "▮···");
    assert_eq!(bars_glyph(2), "▮▮··");
    assert_eq!(bars_glyph(4), "▮▮▮▮");
    assert_eq!(bars_glyph(9), "▮▮▮▮"); // clamps
}

#[test]
fn volume_glyph_reflects_mute_and_level() {
    assert_eq!(volume_glyph(&VolumeInfo { level: 0, muted: false }), "🔇");
    assert_eq!(volume_glyph(&VolumeInfo { level: 50, muted: false }), "🔉");
    assert_eq!(volume_glyph(&VolumeInfo { level: 90, muted: false }), "🔊");
    assert_eq!(volume_glyph(&VolumeInfo { level: 50, muted: true }), "🔇");
}

#[test]
fn mem_pct_rounds_and_guards_zero() {
    assert_eq!(mem_pct(6, 10), 60);
    assert_eq!(mem_pct(0, 0), 0);
}
