use crate::geometry::Point;
use crate::window::{Window, WindowId, WinControl};

/// The kind of mouse event being reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    /// Button pressed.
    Down,
    /// Button released.
    Up,
    /// Button held and pointer moved.
    Drag,
    /// Pointer moved without a button held.
    Move,
}

/// Drag-in-progress state the event loop carries between successive events.
///
/// The loop stores this after a `BeginMove` or `BeginResize` action and
/// passes it back into `route_mouse` on subsequent events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The user is dragging a window by its titlebar.
    Moving {
        id: WindowId,
        /// `cursor_x - window_origin_x` at the moment the drag started.
        grab_dx: i32,
        /// `cursor_y - window_origin_y` at the moment the drag started.
        grab_dy: i32,
    },
    /// The user is dragging the bottom-right resize corner.
    Resizing { id: WindowId },
}

/// The decision produced by [`route_mouse`] — the event loop executes it.
///
/// `route_mouse` is a **pure** function; it never mutates state or performs I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// No meaningful action (click on empty desktop, non-Down event with no drag, …).
    None,
    /// Start a move drag on the given window (loop stores `Hit::Moving`).
    BeginMove(WindowId),
    /// Start a resize drag on the given window (loop stores `Hit::Resizing`).
    BeginResize(WindowId),
    /// Move the window's top-left corner to the absolute screen position `(x, y)`.
    MoveTo { id: WindowId, x: i32, y: i32 },
    /// Resize the window so its bottom-right corner lands at the absolute
    /// screen position `(w, h)` — the loop converts to `(w - rect.x + 1, h - rect.y + 1)`.
    ResizeTo { id: WindowId, w: i32, h: i32 },
    /// Close the window.
    Close(WindowId),
    /// Minimize the window to the dock.
    Minimize(WindowId),
    /// Toggle maximize / restore for the window.
    ToggleMaximize(WindowId),
    /// Raise and focus the window, then forward the click at `local` coords
    /// (relative to the window's content origin) to the app.
    FocusAndForward { id: WindowId, local: Point },
    /// A drag sequence has ended (mouse button released).
    EndDrag,
    /// Begin focus cycle (Alt+Tab). Used by Wayland compositor for window switching.
    BeginFocusCycle,
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Return the window with the highest `z` that contains `p`, or `None`.
fn topmost_at(p: Point, windows: &[Window]) -> Option<&Window> {
    windows.iter().filter(|w| w.rect.contains(p)).max_by_key(|w| w.z)
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Map a raw mouse event into a high-level [`Action`].
///
/// This is a **pure** decision function — it reads `windows` and `drag` but
/// never mutates them; the event loop is responsible for executing the action.
///
/// # Arguments
/// * `kind`    – what kind of mouse event occurred.
/// * `p`       – screen-coordinate position of the cursor.
/// * `windows` – current snapshot of all open windows (any order).
/// * `drag`    – in-progress drag state, if any.
pub fn route_mouse(kind: MouseKind, p: Point, windows: &[Window], drag: Option<Hit>) -> Action {
    // Continue an in-progress drag first.
    if let Some(h) = drag {
        match (kind, h) {
            (MouseKind::Drag, Hit::Moving { id, grab_dx, grab_dy }) =>
                return Action::MoveTo { id, x: p.x - grab_dx, y: p.y - grab_dy },
            (MouseKind::Drag, Hit::Resizing { id }) => {
                // Resize so that the bottom-right follows the cursor.
                // The loop interprets (w, h) as the new bottom-right corner.
                return Action::ResizeTo { id, w: p.x, h: p.y };
            }
            (MouseKind::Up, _) => return Action::EndDrag,
            _ => {}
        }
    }

    if kind != MouseKind::Down { return Action::None; }

    let w = match topmost_at(p, windows) { Some(w) => w, None => return Action::None };
    let r = w.rect;

    // Titlebar control buttons (minimize / maximize / close) take precedence.
    if let Some(ctrl) = w.control_at(p) {
        return match ctrl {
            WinControl::Close => Action::Close(w.id),
            WinControl::Minimize => Action::Minimize(w.id),
            WinControl::Maximize => Action::ToggleMaximize(w.id),
        };
    }
    // Rest of the titlebar row → begin move.
    if p.y == r.y { return Action::BeginMove(w.id); }
    // Right column or bottom row (the window borders) → begin resize.
    // A whole-edge target makes resizing easy to grab, unlike a 1-cell corner.
    if p.x == r.right() || p.y == r.bottom() { return Action::BeginResize(w.id); }
    // Content area → focus + forward local coordinates.
    let local = Point::new(p.x - (r.x + 1), p.y - (r.y + 1));
    Action::FocusAndForward { id: w.id, local }
}
