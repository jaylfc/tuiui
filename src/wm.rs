use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::compositor::Layer;
use crate::geometry::{Point, Rect, SnapZone, snapped_rect};
use crate::window::{Window, WindowId, WindowState};

/// Minimum outer window width enforced by [`WindowManager::resize_to`].
pub const MIN_W: i32 = 8;

/// Minimum outer window height enforced by [`WindowManager::resize_to`].
pub const MIN_H: i32 = 3;

/// Manages a collection of floating windows: z-order, focus, and the
/// move / resize / snap operations.
///
/// This type is pure: it contains no I/O, no rendering, and no process state.
/// Use [`render_window`] to turn a `Window` into compositor [`Layer`]s.
pub struct WindowManager {
    work: Rect,
    /// Unordered backing store; `z` field is the stacking truth.
    windows: Vec<Window>,
    focus: Option<WindowId>,
    next_id: u64,
    next_z: i32,
}

impl WindowManager {
    /// Create a new `WindowManager` with the given usable work area.
    pub fn new(work: Rect) -> Self {
        Self {
            work,
            windows: Vec::new(),
            focus: None,
            next_id: 1,
            next_z: 1,
        }
    }

    /// Return the current work area rectangle.
    pub fn work_area(&self) -> Rect { self.work }

    /// Replace the work area (e.g. after a terminal resize).
    pub fn set_work_area(&mut self, r: Rect) { self.work = r; }

    /// Add a new floating window with the given title and outer rect.
    ///
    /// The new window receives the highest z-index and becomes the focused window.
    /// Returns the new window's [`WindowId`].
    pub fn add_window(&mut self, title: String, rect: Rect) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;
        let z = self.next_z;
        self.next_z += 1;
        self.windows.push(Window {
            id,
            title,
            rect,
            z,
            state: WindowState::Floating,
            restore_rect: rect,
            minimized: false,
        });
        self.focus = Some(id);
        id
    }

    /// Look up a window by id (read-only).
    pub fn get(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Return the currently focused window id, if any.
    pub fn focused(&self) -> Option<WindowId> { self.focus }

    /// Return all windows sorted bottom-to-top by z (suitable for rendering).
    pub fn z_ordered(&self) -> Vec<&Window> {
        let mut v: Vec<&Window> = self.windows.iter().collect();
        v.sort_by_key(|w| w.z);
        v
    }

    /// Return the id of the topmost window whose outer rect contains `p`, or `None`.
    pub fn topmost_at(&self, p: Point) -> Option<WindowId> {
        self.windows
            .iter()
            .filter(|w| w.rect.contains(p))
            .max_by_key(|w| w.z)
            .map(|w| w.id)
    }

    /// Bring window `id` to the top of the stack and give it focus.
    pub fn raise(&mut self, id: WindowId) {
        let z = self.next_z;
        self.next_z += 1;
        if let Some(w) = self.get_mut(id) {
            w.z = z;
        }
        self.focus = Some(id);
    }

    /// Translate window `id` by `(dx, dy)` cells; un-snaps it to `Floating`.
    pub fn move_by(&mut self, id: WindowId, dx: i32, dy: i32) {
        if let Some(w) = self.get_mut(id) {
            w.rect.x += dx;
            w.rect.y += dy;
            if w.state != WindowState::Floating {
                w.state = WindowState::Floating;
            }
        }
    }

    /// Move a window so its top-left is exactly `(x, y)`; un-snaps it to `Floating`.
    pub fn move_to(&mut self, id: WindowId, x: i32, y: i32) {
        if let Some(w) = self.get_mut(id) {
            w.rect.x = x;
            w.rect.y = y;
            w.state = WindowState::Floating;
        }
    }

    /// Resize window `id` to `(w_new, h_new)`, clamping to [`MIN_W`] × [`MIN_H`].
    pub fn resize_to(&mut self, id: WindowId, w_new: i32, h_new: i32) {
        if let Some(win) = self.get_mut(id) {
            win.rect.w = w_new.max(MIN_W);
            win.rect.h = h_new.max(MIN_H);
            win.state = WindowState::Floating;
        }
    }

    /// Snap window `id` to `zone`, saving the current rect for future restore.
    ///
    /// If the window is already floating its current rect is stored in `restore_rect`.
    pub fn snap(&mut self, id: WindowId, zone: SnapZone) {
        let work = self.work;
        if let Some(w) = self.get_mut(id) {
            if w.state == WindowState::Floating {
                w.restore_rect = w.rect;
            }
            w.rect = snapped_rect(zone, work);
            w.state = match zone {
                SnapZone::Left => WindowState::SnappedLeft,
                SnapZone::Right => WindowState::SnappedRight,
            };
        }
    }

    /// Toggle maximize for window `id`: fill the work area, or restore the
    /// previous rect if already maximized.
    ///
    /// The pre-maximize rect is saved in `restore_rect` (only when coming from a
    /// floating state, so repeated toggles don't lose the original geometry).
    pub fn maximize_toggle(&mut self, id: WindowId) {
        let work = self.work;
        if let Some(w) = self.get_mut(id) {
            if w.state == WindowState::Maximized {
                w.rect = w.restore_rect;
                w.state = WindowState::Floating;
            } else {
                if w.state == WindowState::Floating {
                    w.restore_rect = w.rect;
                }
                w.rect = work;
                w.state = WindowState::Maximized;
            }
        }
    }

    /// Hide window `id` to the dock. If it was focused, focus moves to the
    /// topmost remaining visible window.
    pub fn minimize(&mut self, id: WindowId) {
        if let Some(w) = self.get_mut(id) {
            w.minimized = true;
        }
        if self.focus == Some(id) {
            self.focus = self
                .windows
                .iter()
                .filter(|w| !w.minimized)
                .max_by_key(|w| w.z)
                .map(|w| w.id);
        }
    }

    /// Restore window `id` from the dock and raise/focus it.
    pub fn unminimize(&mut self, id: WindowId) {
        if let Some(w) = self.get_mut(id) {
            w.minimized = false;
        }
        self.raise(id);
    }

    /// Remove window `id` and focus the next highest-z visible window, if any.
    pub fn close(&mut self, id: WindowId) {
        self.windows.retain(|w| w.id != id);
        if self.focus == Some(id) {
            self.focus = self
                .windows
                .iter()
                .filter(|w| !w.minimized)
                .max_by_key(|w| w.z)
                .map(|w| w.id);
        }
    }
}

// ── Window rendering ──────────────────────────────────────────────────────────

const TITLE_BG_FOCUS: Rgba = Rgba { r: 29,  g: 36,  b: 51,  a: 255 };
const TITLE_BG_BLUR:  Rgba = Rgba { r: 20,  g: 24,  b: 34,  a: 255 };
const TITLE_FG:       Rgba = Rgba { r: 143, g: 183, b: 255, a: 255 };
const BORDER:         Rgba = Rgba { r: 58,  g: 68,  b: 88,  a: 255 };
const WIN_BG:         Rgba = Rgba { r: 17,  g: 20,  b: 29,  a: 255 };
const SHADOW:         Rgba = Rgba { r: 0,   g: 0,   b: 0,   a: 110 };
const CTRL_FG:        Rgba = Rgba { r: 150, g: 165, b: 190, a: 255 };
const CLOSE_FG:       Rgba = Rgba { r: 255, g: 107, b: 107, a: 255 };

/// Render a window and its content into compositor layers.
///
/// Returns `[shadow_layer, window_layer]`.  The shadow is offset by (1, 1) at
/// `win.z * 2 + 10`; the window body is one z above that.
///
/// This function is **pure**: it takes only a [`Window`] descriptor and a
/// pre-rendered `content` [`CellBuffer`] — it has no knowledge of PTYs,
/// terminals, or any I/O.  Callers snapshot app content independently and
/// pass it in; this keeps rendering and process-hosting cleanly separated.
#[must_use]
pub fn render_window(win: &Window, content: &CellBuffer, focused: bool, shadows: bool) -> Vec<Layer> {
    let r = win.rect;
    let base_z = 10 + win.z * 2;

    // Shadow: a translucent block offset by (1, 1) behind the window.
    let shadow_layer = shadows.then(|| {
        let mut shadow = CellBuffer::new(r.w, r.h);
        shadow.fill(Cell {
            ch: ' ',
            fg: Rgba::TRANSPARENT,
            bg: SHADOW,
            attrs: Default::default(),
        });
        Layer {
            z: base_z,
            origin: Point::new(r.x + 1, r.y + 1),
            buf: shadow,
            opacity: 1.0,
            scissor: None,
        }
    });

    // Window body.
    let mut buf = CellBuffer::new(r.w, r.h);
    buf.fill(Cell {
        ch: ' ',
        fg: Rgba::rgb(200, 208, 220),
        bg: WIN_BG,
        attrs: Default::default(),
    });

    // Titlebar row (y = 0 within the buffer).
    let tbg = if focused { TITLE_BG_FOCUS } else { TITLE_BG_BLUR };
    for x in 0..r.w {
        buf.set(x, 0, Cell { ch: ' ', fg: TITLE_FG, bg: tbg, attrs: Default::default() });
    }
    // Title, truncated so it never runs under the control buttons on the right.
    let title_limit = if r.w >= 9 { (r.w - 10).max(0) } else { (r.w - 4).max(0) } as usize;
    let title: String = win.title.chars().take(title_limit).collect();
    buf.write_str(2, 0, &title, TITLE_FG, tbg);
    // Titlebar control buttons. Wide windows get minimize / maximize / close;
    // very narrow windows get just a close glyph (matching `Window::control_at`).
    if r.w >= 9 {
        let (minc, maxc, closec) = win.control_columns();
        let max_glyph = if win.state == WindowState::Maximized { '\u{2750}' } else { '\u{25A2}' };
        buf.set(minc,   0, Cell { ch: '\u{2013}', fg: CTRL_FG,  bg: tbg, attrs: Default::default() });
        buf.set(maxc,   0, Cell { ch: max_glyph,  fg: CTRL_FG,  bg: tbg, attrs: Default::default() });
        buf.set(closec, 0, Cell { ch: '\u{2715}', fg: CLOSE_FG, bg: tbg, attrs: Default::default() });
    } else if r.w >= 2 {
        buf.set(r.w - 2, 0, Cell { ch: '\u{2715}', fg: CLOSE_FG, bg: tbg, attrs: Default::default() });
    }

    // Left and right borders (rows 1..h).
    for y in 1..r.h {
        buf.set(0,       y, Cell { ch: '│', fg: BORDER, bg: WIN_BG, attrs: Default::default() });
        buf.set(r.w - 1, y, Cell { ch: '│', fg: BORDER, bg: WIN_BG, attrs: Default::default() });
    }
    // Bottom border.
    for x in 0..r.w {
        buf.set(x, r.h - 1, Cell { ch: '─', fg: BORDER, bg: WIN_BG, attrs: Default::default() });
    }

    // Blit content into the inner rect starting at (1, 1).
    for cy in 0..content.height().min(r.h - 2) {
        for cx in 0..content.width().min(r.w - 2) {
            buf.set(1 + cx, 1 + cy, *content.get(cx, cy).unwrap());
        }
    }

    let win_layer = Layer {
        z: base_z + 1,
        origin: Point::new(r.x, r.y),
        buf,
        opacity: 1.0,
        scissor: None,
    };

    match shadow_layer {
        Some(shadow) => vec![shadow, win_layer],
        None => vec![win_layer],
    }
}
