use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::geometry::{Point, Rect};

/// A single renderable plane: a `CellBuffer` positioned at `origin` in screen space,
/// with a z-index for stacking order, an opacity modifier, and an optional scissor rect
/// that clips which screen cells the layer can paint.
pub struct Layer {
    /// Stacking order — lower z is rendered first (behind higher z).
    pub z: i32,
    /// Top-left corner of the layer in screen (compositor) coordinates.
    pub origin: Point,
    /// Source cell grid for this layer.
    pub buf: CellBuffer,
    /// Uniform opacity applied to the layer's `bg` alpha before blending (0.0–1.0).
    pub opacity: f32,
    /// If `Some`, only screen cells inside the rect are painted by this layer.
    pub scissor: Option<Rect>,
}

/// A single cell that changed between the last committed frame and the current composite.
/// Used by the terminal backend to emit only the minimal ANSI update.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CellChange {
    /// Screen column.
    pub x: i32,
    /// Screen row.
    pub y: i32,
    /// New cell value to write.
    pub cell: Cell,
}


/// Double-buffered compositor: composites z-ordered [`Layer`]s (with alpha blending),
/// overlays a block cursor, and diffs successive frames into a minimal [`CellChange`] list.
///
/// # Lifecycle
/// 1. Call [`composite`](Compositor::composite) to produce the new back buffer.
/// 2. Call [`diff`](Compositor::diff) to get only the cells that changed.
/// 3. Write those changes to the terminal.
/// 4. Call [`commit`](Compositor::commit) to promote back → front.
///
/// This type has no knowledge of PTYs, terminals, or windows — it only knows layers and cells.
pub struct Compositor {
    w: i32,
    h: i32,
    /// Last committed frame — what the physical terminal currently shows.
    front: CellBuffer,
    /// Most recent composite result — not yet committed to the terminal.
    back: CellBuffer,
}

impl Compositor {
    /// Create a compositor for a screen of `w` × `h` cells.
    pub fn new(w: i32, h: i32) -> Self {
        Self {
            w,
            h,
            front: CellBuffer::new(w, h),
            back: CellBuffer::new(w, h),
        }
    }

    /// Resize the compositor, discarding both buffers (caller must re-composite).
    pub fn resize(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.front = CellBuffer::new(w, h);
        self.back = CellBuffer::new(w, h);
    }

    /// Screen width in cells.
    pub fn width(&self) -> i32 { self.w }

    /// Screen height in cells.
    pub fn height(&self) -> i32 { self.h }

    /// Composite `layers` (sorted by z here — caller need not pre-sort) onto the back buffer,
    /// then overlay the cursor at `cursor` (if any) by toggling `attrs.inverse`.
    ///
    /// Returns a shared reference to the resulting back buffer.
    pub fn composite(&mut self, layers: &[Layer], cursor: Option<Point>) -> &CellBuffer {
        let t = crate::theme::current();
        // Fill back buffer with the desktop background.
        let base = Cell {
            ch: ' ',
            fg: Rgba::rgb(90, 100, 120),
            bg: t.desktop_bg,
            attrs: Default::default(),
        };
        self.back.fill(base);

        // Sort layers by z (lowest first = rendered underneath).
        let mut order: Vec<&Layer> = layers.iter().collect();
        order.sort_by_key(|l| l.z);

        for layer in order {
            for ly in 0..layer.buf.height() {
                for lx in 0..layer.buf.width() {
                    let gx = layer.origin.x + lx;
                    let gy = layer.origin.y + ly;
                    // Clip to compositor bounds.
                    if gx < 0 || gy < 0 || gx >= self.w || gy >= self.h {
                        continue;
                    }
                    // Apply scissor rect if set.
                    if let Some(s) = layer.scissor {
                        if !s.contains(Point::new(gx, gy)) {
                            continue;
                        }
                    }
                    let src = *layer.buf.get(lx, ly).unwrap();
                    let dst = *self.back.get(gx, gy).unwrap();
                    self.back.set(gx, gy, blend_cell(src, dst, layer.opacity));
                }
            }
        }

        // Overlay cursor by toggling the inverse attribute.
        if let Some(p) = cursor {
            if let Some(c) = self.back.get(p.x, p.y) {
                let mut c = *c;
                c.attrs.inverse = !c.attrs.inverse;
                self.back.set(p.x, p.y, c);
            }
        }

        &self.back
    }

    /// Return every cell that differs between the current back buffer and the last committed
    /// front buffer. Feed this to the terminal backend to minimise ANSI output.
    pub fn diff(&self) -> Vec<CellChange> {
        let mut out = Vec::new();
        for y in 0..self.h {
            for x in 0..self.w {
                let b = self.back.get(x, y).unwrap();
                let f = self.front.get(x, y).unwrap();
                if b != f {
                    out.push(CellChange { x, y, cell: *b });
                }
            }
        }
        out
    }

    /// Promote the back buffer to the front (mark it as "what the terminal shows").
    /// Call this after the terminal backend has written all changes from [`diff`](Self::diff).
    pub fn commit(&mut self) {
        self.front = self.back.clone();
    }
}

/// Composite a single source cell `src` over a destination cell `dst`, modulating
/// `src.bg` alpha by `opacity` before blending.
///
/// Three cases:
/// 1. `src` has a glyph → ink it over the blended background.
/// 2. `src` is blank with an **opaque** background → a solid space that *covers*
///    whatever was beneath (a normal window's empty area must hide lower windows).
/// 3. `src` is blank with a **translucent** background → a shadow/tint: blend the
///    background but keep the glyph from below showing through.
fn blend_cell(src: Cell, dst: Cell, opacity: f32) -> Cell {
    let src_bg = src.bg.with_opacity(opacity);
    let out_bg = src_bg.over(dst.bg);
    if src.ch != ' ' {
        Cell {
            ch: src.ch,
            fg: src.fg.over(out_bg),
            bg: out_bg,
            attrs: src.attrs,
        }
    } else if src_bg.a == 255 {
        // Opaque blank cell: clears the content beneath it.
        Cell {
            ch: ' ',
            fg: src.fg,
            bg: out_bg,
            attrs: src.attrs,
        }
    } else {
        // Translucent blank cell (shadow / tint): keep the lower glyph.
        Cell {
            ch: dst.ch,
            fg: dst.fg,
            bg: out_bg,
            attrs: dst.attrs,
        }
    }
}
