//! A window that displays an image. It renders a cell **placeholder** (border +
//! filename + dimensions) — the universal fallback for terminals without graphics
//! support — and reports its `ImageId` so the session can attach a Kitty graphics
//! placement over the same rect.

use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::imagestore::ImageId;

pub struct ImageView {
    path: String,
    id: Option<ImageId>,
    dims: (u32, u32),
}

impl ImageView {
    pub fn new(path: String, id: Option<ImageId>, dims: (u32, u32)) -> Self {
        Self { path, id, dims }
    }

    /// The cached image id for this view (None when the file couldn't be decoded).
    pub fn image_id(&self) -> Option<ImageId> {
        self.id
    }

    /// Render the placeholder cells (centered filename + dimensions, or an error).
    pub fn render(&self, w: i32, h: i32) -> CellBuffer {
        let t = crate::theme::current();
        let bg = t.window_bg;
        let mut buf = CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: t.dim, bg, attrs: Default::default() });
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        let label = if self.id.is_some() {
            format!("\u{1F5BC}  {}  ({}\u{00D7}{})", name, self.dims.0, self.dims.1)
        } else {
            format!("cannot display  {name}")
        };
        let x = ((w - label.chars().count() as i32) / 2).max(0);
        let y = (h / 2).max(0);
        let fg = Rgba { r: 200, g: 208, b: 220, a: 255 };
        buf.write_str(x, y, &label, fg, bg);
        buf
    }
}
