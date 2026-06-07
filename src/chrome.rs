use crate::buffer::CellBuffer;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};
use crate::window::WindowId;

// ── Theme constants ────────────────────────────────────────────────────────────

/// Label drawn for the top-left launcher button (opens the app launcher).
const GO_LABEL: &str = " Go ";

/// Label drawn for the top-right power button (opens the Exit/Restart/Shutdown
/// menu). Kept as a const so its width and the matching hit region stay in sync.
const POWER_LABEL: &str = " tuiui \u{25be} ";

// ── Public types ───────────────────────────────────────────────────────────────

/// One entry in the dock bar — corresponds to an open window.
pub struct DockItem {
    /// The window this item represents.
    pub id: WindowId,
    /// Short text shown in the dock pill.
    pub label: String,
    /// Whether this window is currently focused.
    pub focused: bool,
}

// ── Public render functions ────────────────────────────────────────────────────

/// Build a compositor [`Layer`] for the top menubar row.
///
/// The layer is 1 row tall, `width` columns wide, positioned at `(0, 0)`.
/// It displays the brand name on the left and `focused_app` at a fixed offset.
pub fn render_menubar(width: i32, focused_app: &str, segments: &[crate::tray::Segment]) -> Layer {
    let t = crate::theme::current();
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.menubar_bg, attrs: Default::default() });
    buf.write_str(0, 0, GO_LABEL, t.accent, t.active_bg);
    // Power button, right-aligned, with a button-like fill.
    let px = (width - POWER_LABEL.chars().count() as i32).max(0);
    // Status-tray segments occupy the right side, just left of the power button.
    let tray_left = segments.iter().map(|s| s.rect.x).min().unwrap_or(px);
    for s in segments {
        buf.write_str(s.rect.x, 0, &s.text, t.text, t.menubar_bg);
    }
    // Focused-app name sits between the brand and the tray, truncated so it can
    // never overwrite a tray segment or the power button on a narrow bar.
    const APP_X: i32 = 10;
    let right_limit = tray_left.min(px);
    let avail = (right_limit - 1 - APP_X).max(0) as usize;
    let app: String = focused_app.chars().take(avail).collect();
    buf.write_str(APP_X, 0, &app, t.dim, t.menubar_bg);
    buf.write_str(px, 0, POWER_LABEL, t.text, t.active_bg);
    Layer { z: 1000, origin: Point::new(0, 0), buf, opacity: 1.0, scissor: None }
}

/// Screen-space hit region for the menubar "Go" button, used to open the
/// launcher dropdown. Top-left of the menubar.
pub fn menubar_brand_region() -> Rect {
    Rect::new(0, 0, GO_LABEL.chars().count() as i32, 1)
}

/// Screen-space hit region for the menubar power button ("tuiui ▾", top row,
/// right side), used to open the Exit/Restart/Shutdown menu.
///
/// Returned in the same coordinate space as [`dock_hit_regions`] so input
/// routing can detect the click without coupling to chrome rendering.
pub fn menubar_power_region(width: i32) -> Rect {
    let pw = POWER_LABEL.chars().count() as i32;
    let px = (width - pw).max(0);
    Rect::new(px, 0, width - px, 1)
}

/// Build a compositor [`Layer`] for the bottom dock row.
///
/// The layer is 1 row tall, positioned at `(0, height - 1)`.
pub fn render_dock(width: i32, height: i32, items: &[DockItem]) -> Layer {
    let t = crate::theme::current();
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.dock_bg, attrs: Default::default() });
    for (i, (_, r, label)) in dock_layout(items).into_iter().enumerate() {
        let bg = if items[i].focused { t.active_bg } else { t.dock_bg };
        buf.write_str(r.x, 0, &label, t.text, bg);
    }
    Layer { z: 1000, origin: Point::new(0, height - 1), buf, opacity: 1.0, scissor: None }
}

/// Return `(WindowId, Rect)` hit regions in *screen* coordinates (bottom row).
///
/// The caller uses these to translate a dock click into a `WindowId` without
/// coupling input routing to chrome rendering.
pub fn dock_hit_regions(_width: i32, height: i32, items: &[DockItem]) -> Vec<(WindowId, Rect)> {
    dock_layout(items).into_iter()
        .map(|(id, r, _)| (id, Rect::new(r.x, height - 1, r.w, 1)))
        .collect()
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Compute the local (y = 0) layout for dock items.
///
/// Each item is rendered as `" label "` with one space of separation.
/// Returns `(WindowId, local_rect, formatted_label)` tuples.
fn dock_layout(items: &[DockItem]) -> Vec<(WindowId, Rect, String)> {
    let mut out = Vec::new();
    let mut x = 1;
    for it in items {
        let label = format!(" {} ", it.label);
        let w = label.chars().count() as i32;
        out.push((it.id, Rect::new(x, 0, w, 1), label));
        x += w + 1;
    }
    out
}
