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

    /// Clamp every window (minimized ones too — their rect is what a restore
    /// brings back) into the current work area: shrink rects larger than the
    /// work area, then move the rect so it lies fully inside. Returns the ids
    /// of windows that changed, so the session can resize their PTYs.
    ///
    /// Window state (snapped / tiled / maximized) is deliberately left alone:
    /// the clamp is involuntary (the terminal shrank), not a user move.
    pub fn clamp_all_into_work(&mut self) -> Vec<WindowId> {
        let work = self.work;
        let mut changed = Vec::new();
        for w in &mut self.windows {
            let before = w.rect;
            w.rect.w = w.rect.w.min(work.w);
            w.rect.h = w.rect.h.min(work.h);
            w.rect.x = w.rect.x.clamp(work.x, work.x + work.w - w.rect.w);
            w.rect.y = w.rect.y.clamp(work.y, work.y + work.h - w.rect.h);
            if w.rect != before {
                changed.push(w.id);
            }
        }
        changed
    }

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
    /// The position is clamped so the titlebar always stays reachable within the
    /// work area (a window can never be dragged fully off-screen and lost).
    pub fn move_to(&mut self, id: WindowId, x: i32, y: i32) {
        let work = self.work;
        if let Some(w) = self.get_mut(id) {
            const KEEP_VISIBLE: i32 = 6; // cells of the titlebar that must stay on-screen
            let min_x = work.x - w.rect.w + KEEP_VISIBLE;
            let max_x = work.x + work.w - KEEP_VISIBLE;
            let max_y = work.y + work.h - 1; // titlebar row never below the work area
            w.rect.x = x.clamp(min_x, max_x.max(min_x));
            w.rect.y = y.clamp(work.y, max_y.max(work.y));
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

    /// Place window `id` into grid cell `(row, col)`, saving its floating rect
    /// for restore. Resizes/positions it to the cell and records `Tiled` state.
    pub fn send_to_cell(&mut self, id: WindowId, grid: crate::geometry::Grid, row: u8, col: u8, gap: i32) {
        let work = self.work;
        if let Some(w) = self.get_mut(id) {
            if w.state == WindowState::Floating {
                w.restore_rect = w.rect;
            }
            w.rect = grid.cell_rect(work, row, col, gap);
            w.state = WindowState::Tiled { row, col };
        }
    }

    /// Arrange all non-minimized windows into the grid in z-order (row-major).
    /// Windows beyond `grid.cells()` are left untouched (they float on top).
    pub fn tile_all(&mut self, grid: crate::geometry::Grid, gap: i32) {
        let mut v: Vec<&Window> = self.windows.iter().filter(|w| !w.minimized).collect();
        v.sort_by_key(|w| w.z);
        let ids: Vec<WindowId> = v.into_iter().map(|w| w.id).collect();
        for (i, id) in ids.iter().enumerate() {
            if i as u8 >= grid.cells() {
                break;
            }
            let (row, col) = grid.row_col(i as u8);
            self.send_to_cell(*id, grid, row, col, gap);
        }
    }

    /// Swap the rects and tiled-states of two windows (auto-tile drag swap).
    pub fn swap_cells(&mut self, a: WindowId, b: WindowId) {
        let (ra, sa) = match self.get(a) {
            Some(w) => (w.rect, w.state),
            None => return,
        };
        let (rb, sb) = match self.get(b) {
            Some(w) => (w.rect, w.state),
            None => return,
        };
        if let Some(w) = self.get_mut(a) {
            w.rect = rb;
            w.state = sb;
        }
        if let Some(w) = self.get_mut(b) {
            w.rect = ra;
            w.state = sa;
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

    /// Update the display title of window `id` (used by the rename command; does
    /// not affect `app_keys` so grouping remains stable).
    pub fn rename_window(&mut self, id: WindowId, new_title: String) {
        if let Some(w) = self.get_mut(id) {
            w.title = new_title;
        }
    }
}

// ── Window rendering ──────────────────────────────────────────────────────────

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
    let t = crate::theme::current();
    let r = win.rect;
    let base_z = 10 + win.z * 2;

    // Shadow: a translucent block offset by (1, 1) behind the window.
    let shadow_layer = shadows.then(|| {
        let mut shadow = CellBuffer::new(r.w, r.h);
        shadow.fill(Cell {
            ch: ' ',
            fg: Rgba::TRANSPARENT,
            bg: t.shadow,
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
        fg: t.text,
        bg: t.window_bg,
        attrs: Default::default(),
    });

    // Titlebar row (y = 0 within the buffer).
    let tbg = if focused { t.title_focus } else { t.title_blur };
    for x in 0..r.w {
        buf.set(x, 0, Cell { ch: ' ', fg: t.title_fg, bg: tbg, attrs: Default::default() });
    }
    // Title, truncated so it never runs under the control buttons on the right.
    let title_limit = if r.w >= 9 { (r.w - 10).max(0) } else { (r.w - 4).max(0) } as usize;
    let title: String = win.title.chars().take(title_limit).collect();
    buf.write_str(2, 0, &title, t.title_fg, tbg);
    // Titlebar control buttons. Wide windows get minimize / maximize / close;
    // very narrow windows get just a close glyph (matching `Window::control_at`).
    if r.w >= 9 {
        let (minc, maxc, closec) = win.control_columns();
        let max_glyph = if win.state == WindowState::Maximized { '\u{2750}' } else { '\u{25A2}' };
        buf.set(minc,   0, Cell { ch: '\u{2013}', fg: t.ctrl_fg,  bg: tbg, attrs: Default::default() });
        buf.set(maxc,   0, Cell { ch: max_glyph,  fg: t.ctrl_fg,  bg: tbg, attrs: Default::default() });
        buf.set(closec, 0, Cell { ch: '\u{2715}', fg: t.close_fg, bg: tbg, attrs: Default::default() });
    } else if r.w >= 2 {
        buf.set(r.w - 2, 0, Cell { ch: '\u{2715}', fg: t.close_fg, bg: tbg, attrs: Default::default() });
    }

    // Left and right borders (rows 1..h).
    for y in 1..r.h {
        buf.set(0,       y, Cell { ch: '│', fg: t.border, bg: t.window_bg, attrs: Default::default() });
        buf.set(r.w - 1, y, Cell { ch: '│', fg: t.border, bg: t.window_bg, attrs: Default::default() });
    }
    // Bottom border.
    for x in 0..r.w {
        buf.set(x, r.h - 1, Cell { ch: '─', fg: t.border, bg: t.window_bg, attrs: Default::default() });
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
