//! The confirm-close dialog: a modal "are you sure?" shown when the user clicks
//! the titlebar ✕ on an **app** window, because closing it kills the running
//! process. Built-in panels (Store / Settings / File Manager) close without a
//! prompt and never open this dialog.
//!
//! Like [`crate::powermenu`], the widget is pure UI state + geometry — it never
//! touches the session. A confirmed click/key reports the [`WindowId`] to close.

use crate::buffer::CellBuffer;
use crate::cell::Cell;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};
use crate::window::WindowId;

/// Modal confirm-close dialog. Holds the window awaiting confirmation (with its
/// title for the message), or `None` when nothing is pending.
#[derive(Default)]
pub struct ConfirmClose {
    target: Option<(WindowId, String)>,
}

// ── Geometry (shared by render + hit-test so they can never drift) ──────────────

/// The centered dialog box.
fn dialog_rect(w: i32, h: i32) -> Rect {
    let box_w = 52.min((w - 2).max(20));
    let box_h = 7.min((h - 1).max(5));
    Rect::new((w - box_w) / 2, ((h - box_h) / 2).max(0), box_w, box_h)
}

/// Screen rects of the dialog's `(Cancel, Close)` buttons.
fn dialog_buttons(w: i32, h: i32) -> (Rect, Rect) {
    let d = dialog_rect(w, h);
    let by = d.y + d.h - 2;
    let cancel = Rect::new(d.x + 2, by, 10, 1);
    let close_w = 14;
    let close = Rect::new(d.x + d.w - 2 - close_w, by, close_w, 1);
    (cancel, close)
}

impl ConfirmClose {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the dialog is showing. While true it is modal and the session
    /// routes clicks/keys to it.
    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    /// Open the dialog for `id` (showing `title` in the message).
    pub fn open(&mut self, id: WindowId, title: String) {
        self.target = Some((id, title));
    }

    /// Dismiss without closing the window.
    pub fn close(&mut self) {
        self.target = None;
    }

    /// The window awaiting confirmation, if any.
    pub fn target(&self) -> Option<WindowId> {
        self.target.as_ref().map(|(id, _)| *id)
    }

    /// Route a click while open. Returns the window to close when the Close
    /// button is hit; otherwise the click is consumed (Cancel, or a click
    /// outside the box, dismisses the dialog).
    pub fn on_click(&mut self, p: Point, w: i32, h: i32) -> Option<WindowId> {
        let id = match self.target {
            Some((id, _)) => id,
            None => return None,
        };
        let (cancel, close) = dialog_buttons(w, h);
        if close.contains(p) {
            self.close();
            return Some(id);
        }
        if cancel.contains(p) || !dialog_rect(w, h).contains(p) {
            self.close();
        }
        None
    }

    /// Keyboard confirm (Enter / y). Returns the window to close.
    pub fn confirm(&mut self) -> Option<WindowId> {
        let id = self.target();
        self.close();
        id
    }

    /// Keyboard cancel (Esc / n). Dismisses without closing.
    pub fn cancel(&mut self) {
        self.close();
    }

    /// Compositor layer for the dialog (empty when closed). Same visual style as
    /// the power-menu confirm dialog so the two read as one design.
    pub fn render(&self, w: i32, h: i32) -> Vec<Layer> {
        let (_, title) = match &self.target {
            Some(t) => t,
            None => return Vec::new(),
        };
        let t = crate::theme::current();
        let d = dialog_rect(w, h);
        let mut buf = CellBuffer::new(d.w, d.h);
        buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
        // Title bar.
        for x in 0..d.w {
            buf.set(x, 0, Cell { ch: ' ', fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
        }
        buf.write_str(2, 0, " tuiui ", t.title_fg, t.title_focus);
        // Border.
        let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
        for y in 1..d.h {
            buf.set(0, y, b('│'));
            buf.set(d.w - 1, y, b('│'));
        }
        for x in 0..d.w {
            buf.set(x, d.h - 1, b('─'));
        }
        buf.set(0, d.h - 1, b('╰'));
        buf.set(d.w - 1, d.h - 1, b('╯'));
        // Message: "Close "<title>"? This quits the app." — truncate a long title
        // so the message fits inside the box.
        let avail = (d.w as usize).saturating_sub(4);
        let suffix = "? This quits the app.";
        let head = "Close \"";
        let budget = avail.saturating_sub(head.len() + 1 + suffix.len());
        let shown: String = if title.chars().count() > budget {
            let mut s: String = title.chars().take(budget.saturating_sub(1)).collect();
            s.push('…');
            s
        } else {
            title.clone()
        };
        let msg = format!("{head}{shown}\"{suffix}");
        buf.write_str(2, 2, &msg, t.text, t.window_bg);
        // Buttons (local coords; pad labels to fill their hit rects).
        let (cancel, close) = dialog_buttons(w, h);
        let cancel_s = format!("{:^width$}", "Cancel", width = cancel.w as usize);
        buf.write_str(cancel.x - d.x, cancel.y - d.y, &cancel_s, t.text, t.active_bg);
        let close_s = format!("{:^width$}", "Close", width = close.w as usize);
        buf.write_str(close.x - d.x, close.y - d.y, &close_s, t.close_fg, t.accent);
        vec![Layer { z: 6000, origin: Point::new(d.x, d.y), buf, opacity: 1.0, scissor: None }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(r: Rect) -> Point {
        Point::new(r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn opens_and_reports_target() {
        let mut c = ConfirmClose::new();
        assert!(!c.is_open());
        c.open(WindowId(7), "vim".into());
        assert!(c.is_open());
        assert_eq!(c.target(), Some(WindowId(7)));
    }

    #[test]
    fn click_close_returns_window_and_dismisses() {
        let (w, h) = (120, 40);
        let mut c = ConfirmClose::new();
        c.open(WindowId(3), "htop".into());
        let (_, close) = dialog_buttons(w, h);
        assert_eq!(c.on_click(center(close), w, h), Some(WindowId(3)));
        assert!(!c.is_open());
    }

    #[test]
    fn click_cancel_dismisses_without_closing() {
        let (w, h) = (120, 40);
        let mut c = ConfirmClose::new();
        c.open(WindowId(3), "htop".into());
        let (cancel, _) = dialog_buttons(w, h);
        assert_eq!(c.on_click(center(cancel), w, h), None);
        assert!(!c.is_open());
    }

    #[test]
    fn click_outside_dismisses_without_closing() {
        let (w, h) = (120, 40);
        let mut c = ConfirmClose::new();
        c.open(WindowId(3), "htop".into());
        assert_eq!(c.on_click(Point::new(0, 0), w, h), None);
        assert!(!c.is_open());
    }

    #[test]
    fn keyboard_confirm_and_cancel() {
        let mut c = ConfirmClose::new();
        c.open(WindowId(9), "bash".into());
        assert_eq!(c.confirm(), Some(WindowId(9)));
        assert!(!c.is_open());

        c.open(WindowId(9), "bash".into());
        c.cancel();
        assert!(!c.is_open());
    }

    #[test]
    fn render_is_empty_when_closed_and_present_when_open() {
        let c0 = ConfirmClose::new();
        assert!(c0.render(120, 40).is_empty());
        let mut c = ConfirmClose::new();
        c.open(WindowId(1), "a-very-long-window-title-that-needs-truncating-to-fit".into());
        assert_eq!(c.render(120, 40).len(), 1);
    }
}
