//! The in-app help overlay: a centered keyboard-shortcut cheatsheet, toggled
//! with `leader ?`. Content is a pure data model so it is unit-testable; the
//! renderer lays it out into a bordered overlay.

use crate::buffer::CellBuffer;
use crate::cell::Cell;
use crate::compositor::Layer;
use crate::geometry::Point;

/// A titled group of `(keys, description)` shortcut rows.
pub struct HelpSection {
    pub title: &'static str,
    pub rows: &'static [(&'static str, &'static str)],
}

/// All shortcut groups shown in the overlay.
pub fn help_sections() -> &'static [HelpSection] {
    &[
        HelpSection {
            title: "Windows",
            rows: &[
                ("m / n", "maximize / minimize"),
                ("[ / ]", "snap left / right half"),
            ],
        },
        HelpSection {
            title: "Tiling",
            rows: &[
                ("t", "tile all into the grid"),
                ("T", "toggle auto-tile mode"),
                ("1–9", "send window to cell N"),
            ],
        },
        HelpSection {
            title: "Apps",
            rows: &[
                ("Space", "spotlight search"),
                ("a", "app menu"),
                ("s / ,", "store / settings"),
            ],
        },
        HelpSection {
            title: "Session",
            rows: &[
                ("q / Q", "detach / shut down"),
                ("?", "this help"),
            ],
        },
    ]
}

/// Render the help overlay centered on a `w × h` screen.
pub fn render_help(w: i32, h: i32) -> Vec<Layer> {
    let t = crate::theme::current();
    let sections = help_sections();

    // Build the body lines: a header note, then each section.
    let mut lines: Vec<(String, bool)> = Vec::new(); // (text, is_heading)
    lines.push(("Leader = Ctrl+Space  (press, release, then a key)".into(), false));
    lines.push((String::new(), false));
    for s in sections {
        lines.push((s.title.to_uppercase(), true));
        for (keys, desc) in s.rows {
            lines.push((format!("  {keys:<8} {desc}"), false));
        }
    }
    lines.push((String::new(), false));
    lines.push(("In overlays:  ↑↓ move · →← expand · Enter open · Esc cancel".into(), false));
    lines.push((String::new(), false));
    lines.push(("Press any key to close".into(), false));

    let inner_w = lines.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(40) as i32;
    let box_w = (inner_w + 4).clamp(30, w - 2);
    let box_h = (lines.len() as i32 + 3).min(h - 1);
    let origin = Point::new((w - box_w) / 2, ((h - box_h) / 2).max(0));

    let mut buf = CellBuffer::new(box_w, box_h);
    buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
    // Title bar + rounded border.
    for x in 0..box_w {
        buf.set(x, 0, Cell { ch: ' ', fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
    }
    buf.write_str(2, 0, " tuiui — keyboard shortcuts ", t.title_fg, t.title_focus);
    let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
    for y in 1..box_h {
        buf.set(0, y, b('│'));
        buf.set(box_w - 1, y, b('│'));
    }
    for x in 0..box_w {
        buf.set(x, box_h - 1, b('─'));
    }
    buf.set(0, box_h - 1, b('╰'));
    buf.set(box_w - 1, box_h - 1, b('╯'));

    for (i, (text, heading)) in lines.iter().enumerate() {
        let y = 1 + i as i32;
        if y >= box_h - 1 {
            break;
        }
        let fg = if *heading { t.accent } else { t.text };
        buf.write_str(2, y, text, fg, t.window_bg);
    }

    vec![Layer { z: 5500, origin, buf, opacity: 1.0, scissor: None }]
}
