use crate::geometry::{Point, Rect};

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
    /// Filling the entire work area (via maximize).
    Maximized,
    /// Tiled into grid cell `(row, col)`.
    Tiled { row: u8, col: u8 },
}

/// A clickable control button in a window's titlebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WinControl {
    /// Hide the window to the dock.
    Minimize,
    /// Toggle fill-work-area / restore.
    Maximize,
    /// Close the window.
    Close,
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
    /// Saved `rect` to restore when un-snapping or un-maximizing.
    pub restore_rect: Rect,
    /// When `true` the window is hidden from the desktop but kept in the dock.
    pub minimized: bool,
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

    /// Local x (within `rect`) of each titlebar control glyph: `(min, max, close)`.
    ///
    /// Buttons are right-aligned: close at `w-3`, maximize at `w-5`, minimize at
    /// `w-7`, each followed by a one-cell gap. Shared by rendering and hit-testing
    /// so they never drift apart.
    pub fn control_columns(&self) -> (i32, i32, i32) {
        let w = self.rect.w;
        (w - 7, w - 5, w - 3)
    }

    /// Return which titlebar control (if any) the screen point `p` hits.
    ///
    /// Each button has a 2-cell-wide target (glyph + trailing gap) so it is
    /// comfortably clickable. Windows narrower than 9 cells expose only Close.
    pub fn control_at(&self, p: Point) -> Option<WinControl> {
        if p.y != self.rect.y {
            return None;
        }
        let lx = p.x - self.rect.x;
        let w = self.rect.w;
        if w < 9 {
            return if lx == w - 2 { Some(WinControl::Close) } else { None };
        }
        let (minc, maxc, closec) = self.control_columns();
        if lx == minc || lx == minc + 1 {
            Some(WinControl::Minimize)
        } else if lx == maxc || lx == maxc + 1 {
            Some(WinControl::Maximize)
        } else if lx == closec || lx == closec + 1 {
            Some(WinControl::Close)
        } else {
            None
        }
    }
}
