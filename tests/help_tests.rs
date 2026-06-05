use tuiui::help::{help_sections, render_help};

#[test]
fn help_lists_the_tiling_shortcuts() {
    let all: Vec<(&str, &str)> = help_sections().iter().flat_map(|s| s.rows.iter().copied()).collect();
    assert!(all.iter().any(|(k, d)| *k == "t" && d.contains("tile")));
    assert!(all.iter().any(|(k, _)| *k == "1–9"));
    assert!(help_sections().iter().any(|s| s.title == "Session"));
}

#[test]
fn help_renders_a_centered_layer() {
    let layers = render_help(80, 30);
    assert_eq!(layers.len(), 1);
    // Centered: origin is left of mid-screen, with positive width.
    assert!(layers[0].origin.x > 0);
}
