//! Session core — the `ClientMsg`-in / `Frame`-out boundary.
//!
//! [`SessionCore`] is the integration layer that owns:
//! - the [`WindowManager`] (window geometry, z-order, focus), and
//! - the live [`AppInstance`] map (PTY-backed child processes).
//!
//! All external control flows through [`ClientMsg`] variants; the core
//! produces a [`Frame`] (ordered compositor layers + cursor position) via
//! [`SessionCore::build_frame`].  No I/O, no terminal types, no renderer
//! details cross this boundary — it is the seam that a future daemon will
//! expose on a socket.

use crate::chrome::{
    render_menubar, render_dock, dock_hit_regions, menubar_brand_region, menubar_quit_region, DockItem,
};
use crate::compositor::Layer;
use crate::config::{AppEntry, Config};
use crate::geometry::{Point, Rect, SnapZone, snap_zone};
use crate::input::{route_mouse, MouseKind, Hit, Action};
use crate::launcher::Launcher;
use crate::ptyhost::AppInstance;
use crate::window::WindowId;
use crate::wm::{WindowManager, render_window};
use std::collections::HashMap;

// ── Public message type ───────────────────────────────────────────────────────

/// All input the front-end (or a future daemon client) can send to the core.
///
/// This enum is intentionally minimal — exactly the surface needed for Slice 1.
/// Additional variants (e.g. scroll, touch, IPC commands) belong in later
/// slices once the socket transport is defined.
#[derive(Debug)]
pub enum ClientMsg {
    /// Spawn a new PTY-backed application window.
    Launch {
        /// Short human-readable name shown in the titlebar and dock.
        name: String,
        /// Executable to run (passed to the PTY host verbatim).
        command: String,
        /// Additional arguments for the child process.
        args: Vec<String>,
    },
    /// Left-button press at screen coordinates `p`.
    MouseDown(Point),
    /// Left-button drag (button still held) at screen coordinates `p`.
    MouseDrag(Point),
    /// Left-button release at screen coordinates `p`.
    MouseUp(Point),
    /// Raw input bytes to forward to the focused app.
    Key(Vec<u8>),
    /// Terminal was resized to `w` × `h` cells.
    Resize { w: i32, h: i32 },
    /// Toggle maximize / restore on the focused window (keyboard command).
    MaximizeFocused,
    /// Minimize the focused window to the dock (keyboard command).
    MinimizeFocused,
    /// Snap the focused window to a screen half (keyboard command).
    SnapFocused(SnapZone),
    /// Open/close the Spotlight search overlay (keyboard command).
    ToggleSpotlight,
    /// Type a character into the Spotlight query.
    LauncherChar(char),
    /// Delete the last character of the Spotlight query.
    LauncherBackspace,
    /// Move the launcher highlight up.
    LauncherUp,
    /// Move the launcher highlight down.
    LauncherDown,
    /// Launch the highlighted entry (Enter).
    LauncherEnter,
    /// Dismiss the launcher (Escape).
    LauncherEsc,
}

// ── Output frame type ─────────────────────────────────────────────────────────

/// One rendered desktop frame, ready for the compositor.
///
/// The layers are already in z-ordered form (chrome on top of windows on top of
/// desktop background).  The cursor position, if `Some`, should be rendered by
/// the compositor as an inverse-video overlay.
pub struct Frame {
    /// Compositor layers ordered bottom-to-top.
    pub layers: Vec<Layer>,
    /// Screen-space cursor position, or `None` if the cursor is hidden.
    pub cursor: Option<Point>,
}

// ── Session core ──────────────────────────────────────────────────────────────

/// Owns the window manager and all running app instances.
///
/// `SessionCore` is the clean `ClientMsg`-in / `Frame`-out boundary.
/// All internal state (window geometry, PTY handles, drag tracking) is
/// fully encapsulated; callers interact only through [`apply`](Self::apply)
/// and [`build_frame`](Self::build_frame).
///
/// A future daemon will serialise `ClientMsg` over a socket and deliver the
/// resulting `Frame` layers to a remote renderer — keeping this struct on the
/// server side.
pub struct SessionCore {
    wm: WindowManager,
    apps: HashMap<WindowId, AppInstance>,
    /// Dock-ordered list of (id, display-name) pairs.
    titles: Vec<(WindowId, String)>,
    cfg: Config,
    w: i32,
    h: i32,
    drag: Option<Hit>,
    cursor: Point,
    /// Set when the user clicks the menubar quit button; polled by the loop.
    quit: bool,
    /// The app launcher (menubar dropdown + Spotlight overlay).
    launcher: Launcher,
}

impl SessionCore {
    /// Create a new session for a terminal of size `w` × `h` cells.
    ///
    /// The work area is set to exclude the single-row menubar at the top and
    /// the single-row dock at the bottom, i.e. `Rect::new(0, 1, w, h - 2)`.
    pub fn new(w: i32, h: i32, cfg: Config) -> Self {
        let work = Rect::new(0, 1, w, h - 2);
        let launcher = Launcher::new(cfg.launcher_apps());
        Self {
            wm: WindowManager::new(work),
            apps: HashMap::new(),
            titles: Vec::new(),
            cfg,
            w,
            h,
            drag: None,
            cursor: Point::new(w / 2, h / 2),
            quit: false,
            launcher,
        }
    }

    /// Whether the launcher (menu or Spotlight) is currently open.
    pub fn launcher_open(&self) -> bool {
        self.launcher.is_open()
    }

    /// Whether the Spotlight overlay specifically is open (the loop routes typed
    /// characters to the query only in this mode).
    pub fn spotlight_open(&self) -> bool {
        self.launcher.mode() == Some(crate::launcher::LauncherMode::Spotlight)
    }

    /// Whether the user has requested quit (clicked the menubar quit button).
    /// The render loop polls this each tick and exits when it returns `true`.
    pub fn quit_requested(&self) -> bool { self.quit }

    /// Return the number of live windows (app instances spawned successfully).
    pub fn window_count(&self) -> usize { self.apps.len() }

    /// Return the currently focused [`WindowId`], if any.
    pub fn focused(&self) -> Option<WindowId> { self.wm.focused() }

    /// Return the screen-space hit regions for every dock item.
    ///
    /// Each tuple is `(WindowId, Rect)` where the rect is a 1-row slice on the
    /// bottom screen row.  Used by callers that need to detect dock clicks
    /// without going through the full mouse-routing path.
    pub fn dock_regions(&self) -> Vec<(WindowId, Rect)> {
        let items = self.dock_items();
        dock_hit_regions(self.w, self.h, &items)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Build the current list of dock items from the titles registry.
    fn dock_items(&self) -> Vec<DockItem> {
        let f = self.wm.focused();
        self.titles
            .iter()
            .map(|(id, t)| DockItem {
                id: *id,
                label: t.clone(),
                focused: Some(*id) == f,
            })
            .collect()
    }

    // ── Public apply ─────────────────────────────────────────────────────────

    /// Apply a single client message, mutating internal state.
    ///
    /// This is the **only** way external code drives the session.  The method
    /// dispatches to private sub-handlers so that none of the internal
    /// machinery (WM, PTY handles, drag state) leaks through the public API.
    pub fn apply(&mut self, msg: ClientMsg) {
        match msg {
            ClientMsg::Launch { name, command, args } => {
                self.launch(name, command, args);
            }
            ClientMsg::MouseDown(p) => {
                self.cursor = p;
                self.handle_mouse(MouseKind::Down, p);
            }
            ClientMsg::MouseDrag(p) => {
                self.cursor = p;
                self.handle_mouse(MouseKind::Drag, p);
            }
            ClientMsg::MouseUp(p) => {
                self.cursor = p;
                self.handle_mouse(MouseKind::Up, p);
            }
            ClientMsg::Key(bytes) => {
                if let Some(id) = self.wm.focused() {
                    if let Some(app) = self.apps.get_mut(&id) {
                        app.write_input(&bytes);
                    }
                }
            }
            ClientMsg::Resize { w, h } => {
                self.w = w;
                self.h = h;
                self.wm.set_work_area(Rect::new(0, 1, w, h - 2));
                // Re-fit any maximized window and its app to the new work area.
                if let Some(id) = self.wm.focused() {
                    self.sync_app_size(id);
                }
            }
            ClientMsg::MaximizeFocused => {
                if let Some(id) = self.wm.focused() {
                    self.wm.maximize_toggle(id);
                    self.sync_app_size(id);
                }
            }
            ClientMsg::MinimizeFocused => {
                if let Some(id) = self.wm.focused() {
                    self.wm.minimize(id);
                }
            }
            ClientMsg::SnapFocused(zone) => {
                if let Some(id) = self.wm.focused() {
                    self.wm.snap(id, zone);
                    self.sync_app_size(id);
                }
            }
            ClientMsg::ToggleSpotlight => self.launcher.toggle_spotlight(),
            ClientMsg::LauncherChar(c) => self.launcher.type_char(c),
            ClientMsg::LauncherBackspace => self.launcher.backspace(),
            ClientMsg::LauncherUp => self.launcher.move_up(),
            ClientMsg::LauncherDown => self.launcher.move_down(),
            ClientMsg::LauncherEnter => {
                if let Some(e) = self.launcher.selected_entry() {
                    self.launcher.close();
                    self.launch_entry(e);
                }
            }
            ClientMsg::LauncherEsc => self.launcher.close(),
        }
    }

    /// Spawn a launcher entry as a new window and bring it to the front.
    fn launch_entry(&mut self, e: AppEntry) {
        self.launch(e.name, e.command, e.args);
    }

    /// Spawn a new PTY-backed window.
    ///
    /// If `AppInstance::spawn` fails, the window is removed and no dock entry
    /// is added (silently drops the launch request — the caller can surface an
    /// error later via a `CoreMsg` notification once that protocol exists).
    fn launch(&mut self, name: String, command: String, args: Vec<String>) {
        // Cascade new windows with a generous offset so each one is clearly
        // visible (not buried under the previous window), clamped so the whole
        // window stays on-screen within the work area.
        let n = self.titles.len() as i32;
        // Default large enough that demanding apps (e.g. btop needs 80×24
        // content → 82×26 outer) fit without complaint, clamped to the screen.
        let win_w = 84.min((self.w - 4).max(20));
        let win_h = 30.min((self.h - 4).max(6));
        let max_x = (self.w - win_w - 1).max(0);
        let max_y = (self.h - 1 - win_h).max(1); // keep above the dock row
        let x = (2 + n * 6).min(max_x);
        let y = (2 + n * 3).min(max_y);
        let rect = Rect::new(x, y, win_w, win_h);
        let id = self.wm.add_window(name.clone(), rect);
        let content = self.wm.get(id).unwrap().content_rect();
        match AppInstance::spawn(&command, &args, content.w.max(1), content.h.max(1)) {
            Ok(app) => {
                self.apps.insert(id, app);
                self.titles.push((id, name));
            }
            Err(_) => {
                self.wm.close(id);
            }
        }
    }

    /// Route a mouse event through dock hit-testing then the WM input router.
    fn handle_mouse(&mut self, kind: MouseKind, p: Point) {
        // An open launcher captures the next click: launch an item, or dismiss.
        if kind == MouseKind::Down && self.launcher.is_open() {
            let rendered = self.launcher.render(self.w, self.h);
            for (entry, r) in rendered.items {
                if r.contains(p) {
                    self.launcher.close();
                    self.launch_entry(entry);
                    return;
                }
            }
            self.launcher.close();
            return;
        }

        // Menubar brand opens the launcher dropdown; quit button and dock clicks
        // are checked before normal window routing.
        if kind == MouseKind::Down {
            if menubar_brand_region().contains(p) {
                self.launcher.toggle_menu();
                return;
            }
            if menubar_quit_region(self.w).contains(p) {
                self.quit = true;
                return;
            }
            for (id, r) in self.dock_regions() {
                if r.contains(p) {
                    // Restore (un-minimize) and raise the clicked window.
                    self.wm.unminimize(id);
                    return;
                }
            }
        }
        // Minimized windows are hidden, so they never receive mouse events.
        let windows: Vec<_> = self
            .wm
            .z_ordered()
            .into_iter()
            .filter(|w| !w.minimized)
            .cloned()
            .collect();
        let action = route_mouse(kind, p, &windows, self.drag);
        self.exec(action, p);
    }

    /// Execute a resolved [`Action`] against the window manager and app state.
    fn exec(&mut self, action: Action, p: Point) {
        match action {
            Action::BeginMove(id) => {
                self.wm.raise(id);
                let r = self.wm.get(id).unwrap().rect;
                self.drag = Some(Hit::Moving {
                    id,
                    grab_dx: p.x - r.x,
                    grab_dy: p.y - r.y,
                });
            }
            Action::BeginResize(id) => {
                self.wm.raise(id);
                self.drag = Some(Hit::Resizing { id });
            }
            Action::MoveTo { id, x, y } => {
                self.wm.move_to(id, x, y);
            }
            Action::ResizeTo { id, w, h } => {
                let r = self.wm.get(id).unwrap().rect;
                self.wm.resize_to(id, w - r.x + 1, h - r.y + 1);
                self.sync_app_size(id);
            }
            Action::Close(id) => self.close(id),
            Action::Minimize(id) => self.wm.minimize(id),
            Action::ToggleMaximize(id) => {
                self.wm.maximize_toggle(id);
                self.sync_app_size(id);
            }
            Action::FocusAndForward { id, local } => {
                self.wm.raise(id);
                // Mouse-forwarding into apps is keyboard-first in Slice 1; raise is enough.
                let _ = local;
            }
            Action::EndDrag => {
                if let Some(Hit::Moving { id, .. }) = self.drag {
                    if self.cfg.snapping_enabled {
                        let work = Rect::new(0, 1, self.w, self.h - 2);
                        if let Some(z) = snap_zone(p, work, self.cfg.snap_threshold) {
                            self.wm.snap(id, z);
                            self.sync_app_size(id);
                        }
                    }
                }
                self.drag = None;
            }
            Action::None => {}
        }
    }

    /// Tell the app instance for `id` to resize to match the window's current
    /// content rect.
    fn sync_app_size(&mut self, id: WindowId) {
        if let Some(w) = self.wm.get(id) {
            let c = w.content_rect();
            if let Some(app) = self.apps.get_mut(&id) {
                app.resize(c.w.max(1), c.h.max(1));
            }
        }
    }

    /// Kill a window's PTY, remove its dock entry, and close the WM window.
    fn close(&mut self, id: WindowId) {
        if let Some(mut app) = self.apps.remove(&id) {
            app.kill();
        }
        self.titles.retain(|(i, _)| *i != id);
        self.wm.close(id);
    }

    // ── Frame builder ─────────────────────────────────────────────────────────

    /// Build a complete [`Frame`] from the current session state.
    ///
    /// The frame contains (bottom to top):
    /// 1. Window shadow + body layers for every open window (z-ordered).
    /// 2. The menubar layer (z = 1000).
    /// 3. The dock layer (z = 1000).
    ///
    /// The cursor is set to the last known mouse position.
    pub fn build_frame(&self) -> Frame {
        let mut layers: Vec<Layer> = Vec::new();
        let focused = self.wm.focused();

        for w in self.wm.z_ordered() {
            if w.minimized {
                continue; // hidden to the dock
            }
            let content = self.apps.get(&w.id)
                .map(|a| a.snapshot())
                .unwrap_or_else(|| {
                    let cr = w.content_rect();
                    crate::buffer::CellBuffer::new(cr.w, cr.h)
                });
            layers.extend(render_window(w, &content, Some(w.id) == focused));
        }

        let app_name = focused
            .and_then(|id| self.titles.iter().find(|(i, _)| *i == id))
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        layers.push(render_menubar(self.w, &app_name));
        layers.push(render_dock(self.w, self.h, &self.dock_items()));

        // Launcher (dropdown / Spotlight) renders above all chrome.
        layers.extend(self.launcher.render(self.w, self.h).layers);

        Frame { layers, cursor: Some(self.cursor) }
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Remove windows whose PTY child has exited.
    ///
    /// Call this once per render loop tick to keep the session consistent with
    /// process state.
    pub fn reap_dead(&mut self) {
        let dead: Vec<WindowId> = self.apps
            .iter_mut()
            .filter_map(|(id, a)| if !a.is_alive() { Some(*id) } else { None })
            .collect();
        for id in dead {
            self.close(id);
        }
    }

    /// Kill all running apps and clear the session.
    ///
    /// Must be called before dropping the session to ensure no child processes
    /// are orphaned.
    pub fn shutdown(&mut self) {
        for (_, app) in self.apps.iter_mut() {
            app.kill();
        }
        self.apps.clear();
    }
}
