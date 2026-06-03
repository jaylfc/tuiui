use crate::geometry::Rect;

/// Opaque handle that uniquely identifies a window within a [`WindowManager`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

/// Stacking/layout state of a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    /// Freely positioned by the user.
    Floating,
    /// Occupying the left half of the work area (via snap).
    SnappedLeft,
    /// Occupying the right half of the work area (via snap).
    SnappedRight,
}

/// A single managed window.
///
/// `rect` is the **outer** bounding rectangle including the 1-row titlebar and
/// 1-column borders on left, right, and bottom.  The content area is inset by
/// 1 cell on every side (use [`Window::content_rect`] to obtain it).
#[derive(Clone, Debug)]
pub struct Window {
    /// Stable identifier for this window.
    pub id: WindowId,
    /// Human-readable title shown in the titlebar.
    pub title: String,
    /// Outer rectangle in screen coordinates (includes titlebar + borders).
    pub rect: Rect,
    /// Stacking z-index — higher value renders on top.
    pub z: i32,
    /// Current layout state.
    pub state: WindowState,
    /// Saved `rect` to restore when un-snapping.
    pub restore_rect: Rect,
}

impl Window {
    /// Inner content rectangle (excludes the 1-row titlebar and 1-column borders).
    ///
    /// Width and height are clamped to 0 so callers never get a negative size.
    pub fn content_rect(&self) -> Rect {
        Rect::new(
            self.rect.x + 1,
            self.rect.y + 1,
            (self.rect.w - 2).max(0),
            (self.rect.h - 2).max(0),
        )
    }

    /// The single-row rectangle occupied by the titlebar (top row of `rect`).
    pub fn titlebar_rect(&self) -> Rect {
        Rect::new(self.rect.x, self.rect.y, self.rect.w, 1)
    }
}
