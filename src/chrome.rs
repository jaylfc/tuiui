use crate::buffer::CellBuffer;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};
use crate::window::WindowId;

// ── Theme constants ────────────────────────────────────────────────────────────

/// Label drawn for the top-left launcher button (opens the app launcher).
const GO_LABEL: &str = " tuiui ";

/// Column where the new-shell "+" button is drawn (just right of the brand).
const NEW_SHELL_X: i32 = 8; // GO_LABEL is 7 cells wide, + a 1-cell gap

/// Column where the assistant (✦) button is drawn (right of the mode toggle).
const ASSIST_X: i32 = 12;

/// Label for the assistant button (opens the AI chat panel).
const ASSIST_LABEL: &str = " \u{2726} "; // ✦

/// Column where the focused-app name starts (after brand + mode + assistant).
const APP_X: i32 = 16;

/// Menubar view-mode toggle glyphs (shows the CURRENT mode; click to switch).
const MODE_DESKTOP: &str = " \u{229E} "; // ⊞  windowed desktop
const MODE_SIMPLE: &str = " \u{25A6} ";  // ▦  full-screen single app

// ── Public types ───────────────────────────────────────────────────────────────

/// What a dock pill activates.
#[derive(Clone, Debug)]
pub enum DockKind {
    /// A single window — clicking focuses it.
    Single(WindowId),
    /// A group of windows of the same app — clicking expands a chooser.
    Group(String, Vec<WindowId>), // (app_key, window ids)
}

/// One pill in the dock.
#[derive(Clone, Debug)]
pub struct DockItem {
    pub kind: DockKind,
    /// Text shown after the badge (single → its label; group → app key).
    pub label: String,
    /// Number of windows (>1 ⇒ a group; shown as a count).
    pub count: usize,
    /// Badge letter + color.
    pub badge_letter: char,
    pub badge_color: crate::cell::Rgba,
    /// Whether any window in this pill is focused.
    pub focused: bool,
    /// Whether any window in this pill has an unseen bell notification.
    pub attention: bool,
}

// ── Public render functions ────────────────────────────────────────────────────

/// Build a compositor [`Layer`] for the top menubar row.
///
/// The layer is 1 row tall, `width` columns wide, positioned at `(0, 0)`.
/// It displays the brand name on the left and `focused_app` at a fixed offset.
pub fn render_menubar(width: i32, focused_app: &str, segments: &[crate::tray::Segment], power_label: &str) -> Layer {
    let t = crate::theme::current();
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.menubar_bg, attrs: Default::default() });
    buf.write_str(0, 0, GO_LABEL, t.accent, t.active_bg);
    buf.write_str(NEW_SHELL_X, 0, NEW_SHELL_LABEL, crate::cell::Rgba::rgb(255, 255, 255), t.accent);
    buf.write_str(ASSIST_X, 0, ASSIST_LABEL, t.accent, t.active_bg);
    // Power button (the host name + ▾), right-aligned, with a button-like fill.
    let px = (width - power_label.chars().count() as i32).max(0);
    // Status-tray segments occupy the right side, just left of the power button.
    let tray_left = segments.iter().map(|s| s.rect.x).min().unwrap_or(px);
    for s in segments {
        buf.write_str(s.rect.x, 0, &s.text, t.text, t.menubar_bg);
    }
    // Focused-app name sits between the brand and the tray, truncated so it can
    // never overwrite a tray segment or the power button on a narrow bar.
    let right_limit = tray_left.min(px);
    let avail = (right_limit - 1 - APP_X).max(0) as usize;
    let app: String = focused_app.chars().take(avail).collect();
    buf.write_str(APP_X, 0, &app, t.dim, t.menubar_bg);
    buf.write_str(px, 0, power_label, t.text, t.active_bg);
    Layer { z: 1000, origin: Point::new(0, 0), buf, opacity: 1.0, scissor: None }
}

/// Screen-space hit region for the menubar brand ("tuiui") button, used to open
/// the launcher dropdown. Top-left of the menubar.
pub fn menubar_brand_region() -> Rect {
    Rect::new(0, 0, GO_LABEL.chars().count() as i32, 1)
}

/// Screen-space hit region for the menubar "+" (new shell) button (just right of
/// the brand; it swapped places with the view-mode toggle, now on the dock).
pub fn menubar_new_shell_region() -> Rect {
    Rect::new(NEW_SHELL_X, 0, NEW_SHELL_LABEL.chars().count() as i32, 1)
}

/// Screen-space hit region for the assistant (✦) button — opens the AI chat panel.
pub fn menubar_assistant_region() -> Rect {
    Rect::new(ASSIST_X, 0, ASSIST_LABEL.chars().count() as i32, 1)
}

/// Screen-space hit region for the menubar power button (the host name + ▾, top
/// row, right side), used to open the Exit/Restart/Shutdown menu. `power_label`
/// must be the same string passed to [`render_menubar`] so the widths match.
///
/// Returned in the same coordinate space as [`dock_hit_regions`] so input
/// routing can detect the click without coupling to chrome rendering.
pub fn menubar_power_region(width: i32, power_label: &str) -> Rect {
    let pw = power_label.chars().count() as i32;
    let px = (width - pw).max(0);
    Rect::new(px, 0, width - px, 1)
}

/// The "new shell" quick-launch button (menubar, next to the brand).
const NEW_SHELL_LABEL: &str = " + ";

/// Screen-space hit region for the dock's view-mode toggle (bottom-left; shows
/// the CURRENT mode, click to switch desktop ⊞ / full-screen-single-app ▦).
pub fn dock_mode_region(height: i32) -> Rect {
    Rect::new(0, height - 1, MODE_DESKTOP.chars().count() as i32, 1)
}

/// Build a compositor [`Layer`] for the bottom dock row.
///
/// The layer is 1 row tall, positioned at `(0, height - 1)`.
pub fn render_dock(width: i32, height: i32, items: &[DockItem], simple: bool) -> Layer {
    let t = crate::theme::current();
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.dock_bg, attrs: Default::default() });
    // The view-mode toggle, bottom-left (swapped places with new-shell "+").
    let mode = if simple { MODE_SIMPLE } else { MODE_DESKTOP };
    buf.write_str(0, 0, mode, t.accent, t.active_bg);
    for (i, (_idx, r, badge_x, label_text)) in dock_layout(items).into_iter().enumerate() {
        let item = &items[i];
        let bg = if item.focused { t.active_bg } else { t.dock_bg };
        // Write the full pill background first
        buf.write_str(r.x, 0, &label_text, t.text, bg);
        // Bell-notification dot at the pill's right edge.
        if item.attention {
            buf.set(r.x + r.w - 1, 0, crate::cell::Cell {
                ch: '•',
                fg: t.accent,
                bg,
                attrs: Default::default(),
            });
        }
        // Overwrite the badge cell (first char of label_text) with badge color
        buf.set(badge_x, 0, crate::cell::Cell {
            ch: item.badge_letter,
            fg: crate::cell::Rgba::rgb(255, 255, 255),
            bg: item.badge_color,
            attrs: Default::default(),
        });
    }
    Layer { z: 1000, origin: Point::new(0, height - 1), buf, opacity: 1.0, scissor: None }
}

/// Return `(pill_index, Rect)` hit regions in *screen* coordinates (bottom row).
///
/// The caller uses these to translate a dock click into a pill index, then
/// looks up `items[pill_index].kind` to decide what to do.
pub fn dock_hit_regions(_width: i32, height: i32, items: &[DockItem]) -> Vec<(usize, Rect)> {
    dock_layout(items).into_iter()
        .map(|(idx, r, _, _)| (idx, Rect::new(r.x, height - 1, r.w, 1)))
        .collect()
}

/// Render a small bordered popup ABOVE the dock for a window-group chooser.
///
/// Returns `(layers, row_rects)` where each entry in `row_rects` is the
/// screen-space rect of one window row (for hit-testing).
pub fn render_dock_popup(
    width: i32,
    height: i32,
    pill_x: i32,
    pill_w: i32,
    rows: &[(WindowId, char, crate::cell::Rgba, String)], // (id, badge_letter, badge_color, label)
    focused_id: Option<WindowId>,
) -> (Vec<Layer>, Vec<(WindowId, Rect)>) {
    let t = crate::theme::current();
    let n = rows.len() as i32;
    let box_h = n + 2; // border rows
    let max_label_w = rows.iter().map(|(_, _, _, l)| l.chars().count()).max().unwrap_or(4) as i32;
    let box_w = (max_label_w + 5).max(12).min(width); // badge + space + label + borders + padding
    // Anchor left edge at pill_x but clamp to screen
    let bx = pill_x.min(width - box_w).max(0);
    // Place above the dock row
    let by = (height - 1 - box_h).max(0);
    let rect = Rect::new(bx, by, box_w, box_h);

    let mut buf = CellBuffer::new(rect.w, rect.h);
    buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });

    // Border
    let b = |ch: char| crate::cell::Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
    for x in 0..rect.w {
        buf.set(x, 0, b('─'));
        buf.set(x, rect.h - 1, b('─'));
    }
    for y in 0..rect.h {
        buf.set(0, y, b('│'));
        buf.set(rect.w - 1, y, b('│'));
    }
    buf.set(0, 0, b('╭'));
    buf.set(rect.w - 1, 0, b('╮'));
    buf.set(0, rect.h - 1, b('╰'));
    buf.set(rect.w - 1, rect.h - 1, b('╯'));

    let mut row_rects = Vec::new();
    for (ri, (win_id, badge_letter, badge_color, label)) in rows.iter().enumerate() {
        let y = 1 + ri as i32;
        let is_focused = Some(*win_id) == focused_id;
        let row_bg = if is_focused { t.active_bg } else { t.window_bg };
        // Fill row background inside borders
        for x in 1..rect.w - 1 {
            buf.set(x, y, crate::cell::Cell { ch: ' ', fg: t.text, bg: row_bg, attrs: Default::default() });
        }
        // Badge cell
        buf.set(1, y, crate::cell::Cell {
            ch: *badge_letter,
            fg: crate::cell::Rgba::rgb(255, 255, 255),
            bg: *badge_color,
            attrs: Default::default(),
        });
        // Space + label
        let avail = (rect.w - 4).max(0) as usize;
        let lbl: String = label.chars().take(avail).collect();
        buf.write_str(3, y, &lbl, t.text, row_bg);

        let screen_row = Rect::new(rect.x, rect.y + y, rect.w, 1);
        row_rects.push((*win_id, screen_row));
    }

    let _ = (pill_w, focused_id); // suppress unused warnings
    let layer = Layer { z: 5200, origin: Point::new(rect.x, rect.y), buf, opacity: 1.0, scissor: None };
    (vec![layer], row_rects)
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Superscript digit suffix for group counts.
fn count_suffix(n: usize) -> String {
    const SUP: [char; 10] = ['⁰','¹','²','³','⁴','⁵','⁶','⁷','⁸','⁹'];
    if n <= 1 { String::new() }
    else if n <= 9 { format!(" {}", SUP[n]) }
    else { format!(" \u{00B7}{n}") }
}

/// Compute the local (y = 0) layout for dock items.
///
/// Each item is rendered as `"B label"` (badge cell + space + label + count suffix),
/// padded with spaces. Returns `(pill_index, local_rect, badge_x, full_label_string)`.
fn dock_layout(items: &[DockItem]) -> Vec<(usize, Rect, i32, String)> {
    let mut out = Vec::new();
    // Pills start after the bottom-left view-mode toggle.
    let mut x = MODE_DESKTOP.chars().count() as i32 + 1;
    for (i, it) in items.iter().enumerate() {
        let suffix = count_suffix(it.count);
        // Pill format: " B label[suffix] "
        // We store the rendered string (badge placeholder + label), badge_x tracks
        // where the badge cell is in the pill.
        let label_part = format!(" {} {}{} ", it.badge_letter, it.label, suffix);
        let w = label_part.chars().count() as i32;
        // badge is at position x (the ' B' — second char, x+1 after leading space)
        let badge_x = x + 1;
        out.push((i, Rect::new(x, 0, w, 1), badge_x, label_part));
        x += w + 1;
    }
    out
}
