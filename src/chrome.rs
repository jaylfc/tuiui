use crate::buffer::CellBuffer;
use crate::cell::Rgba;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};
use crate::window::WindowId;

// ── Theme constants ────────────────────────────────────────────────────────────
// Named consts so a future theme system can swap them without hunting literals.

const MENUBAR_BG: Rgba = Rgba { r: 22, g: 27, b: 39, a: 255 };
const DOCK_BG: Rgba    = Rgba { r: 22, g: 27, b: 39, a: 255 };
const TEXT: Rgba       = Rgba { r: 200, g: 208, b: 220, a: 255 };
const BRAND: Rgba      = Rgba { r: 108, g: 182, b: 255, a: 255 };
const ACTIVE_BG: Rgba  = Rgba { r: 45, g: 58, b: 85, a: 255 };

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
pub fn render_menubar(width: i32, focused_app: &str) -> Layer {
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: TEXT, bg: MENUBAR_BG, attrs: Default::default() });
    buf.write_str(1, 0, "\u{2726} Tuiui", BRAND, MENUBAR_BG);
    buf.write_str(10, 0, focused_app, TEXT, MENUBAR_BG);
    Layer { z: 1000, origin: Point::new(0, 0), buf, opacity: 1.0, scissor: None }
}

/// Build a compositor [`Layer`] for the bottom dock row.
///
/// The layer is 1 row tall, positioned at `(0, height - 1)`.
pub fn render_dock(width: i32, height: i32, items: &[DockItem]) -> Layer {
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: TEXT, bg: DOCK_BG, attrs: Default::default() });
    for (i, (_, r, label)) in dock_layout(items).into_iter().enumerate() {
        let bg = if items[i].focused { ACTIVE_BG } else { DOCK_BG };
        buf.write_str(r.x, 0, &label, TEXT, bg);
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
