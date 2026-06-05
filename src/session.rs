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
use crate::geometry::{Point, Rect, SnapZone};
use crate::input::{route_mouse, MouseKind, Hit, Action};
use crate::launcher::Launcher;
use crate::ptyhost::AppInstance;
use crate::settings::Settings;
use crate::store::{self, Store};
use crate::window::WindowId;
use crate::wm::{WindowManager, render_window};
use std::collections::HashMap;

/// The content hosted by a window: a PTY-backed app or a native Tuiui widget.
///
/// This is the [`WindowContent`](../../docs) seam — the window manager, chrome,
/// and input routing operate on windows uniformly, while the content type varies.
enum WinContent {
    /// A child process in a pseudo-terminal.
    App(AppInstance),
    /// The native app store browser.
    Store(Store),
    /// The native settings panel.
    Settings(Settings),
}

impl WinContent {
    fn render(&self, w: i32, h: i32) -> crate::buffer::CellBuffer {
        match self {
            WinContent::App(a) => a.snapshot(),
            WinContent::Store(s) => s.render(w, h),
            WinContent::Settings(s) => s.render(w, h),
        }
    }
    fn resize(&mut self, w: i32, h: i32) {
        if let WinContent::App(a) = self {
            a.resize(w, h);
        }
    }
    fn write_input(&mut self, bytes: &[u8]) {
        if let WinContent::App(a) = self {
            a.write_input(bytes);
        }
    }
    fn is_alive(&mut self) -> bool {
        match self {
            WinContent::App(a) => a.is_alive(),
            WinContent::Store(_) | WinContent::Settings(_) => true,
        }
    }
    fn kill(&mut self) {
        if let WinContent::App(a) = self {
            a.kill();
        }
    }
}

// ── Public message type ───────────────────────────────────────────────────────

/// All input the front-end (or a future daemon client) can send to the core.
///
/// This enum is intentionally minimal — exactly the surface needed for Slice 1.
/// Additional variants (e.g. scroll, touch, IPC commands) belong in later
/// slices once the socket transport is defined.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
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
    /// Tile all windows into the configured grid (one-shot).
    TileAll,
    /// Toggle auto-tile mode.
    ToggleAutoTile,
    /// Send the focused window to grid cell N (1-based, row-major).
    SendToCell(u8),
    /// Open/close the launcher dropdown menu (keyboard command).
    ToggleMenu,
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
    /// Open the store window (or focus it if already open).
    OpenStore,
    /// Store: move selection up / down.
    StoreUp,
    StoreDown,
    /// Store: previous / next category.
    StorePrevCategory,
    StoreNextCategory,
    /// Store: type into / edit the search query.
    StoreChar(char),
    StoreBackspace,
    /// Store: install or launch the selected app (Enter).
    StoreActivate,
    /// Store: close the store window (Escape).
    StoreClose,
    /// Open the settings window (or focus it if already open).
    OpenSettings,
    /// Settings: move selection up / down.
    SettingsUp,
    SettingsDown,
    /// Settings: previous / next section.
    SettingsPrevSection,
    SettingsNextSection,
    /// Settings: decrease / increase / toggle the selected setting.
    SettingsLeft,
    SettingsRight,
    SettingsToggle,
    /// Settings (Apps add form): type a character into the focused field.
    SettingsChar(char),
    /// Settings (Apps add form): delete the last character of the focused field.
    SettingsBackspace,
    /// Settings (Apps add form): abandon the form without saving (Escape).
    SettingsCancelEdit,
    /// Settings: close the settings window (Escape).
    SettingsClose,
    /// Toggle the keyboard-shortcut help overlay.
    ToggleHelp,
    /// Working-directory picker: navigation, expand/collapse, confirm, cancel.
    DirPickerUp,
    DirPickerDown,
    DirPickerExpand,
    DirPickerCollapse,
    DirPickerConfirm,
    DirPickerCancel,
    DirPickerToggleHidden,
    /// Picker: start the new-folder name input.
    DirPickerNewFolder,
    /// Picker: type / delete a character of the new-folder name.
    DirPickerChar(char),
    DirPickerBackspace,
    /// Shut down the daemon entirely (kills all apps). Sent by `tuiui kill`.
    Shutdown,
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
    contents: HashMap<WindowId, WinContent>,
    /// The store window's id, if open (so it can be re-focused, not re-opened).
    store_win: Option<WindowId>,
    /// The settings window's id, if open.
    settings_win: Option<WindowId>,
    /// Dock-ordered list of (id, display-name) pairs.
    titles: Vec<(WindowId, String)>,
    cfg: Config,
    w: i32,
    h: i32,
    drag: Option<Hit>,
    cursor: Point,
    /// Set when the user clicks the menubar quit button; polled by the loop.
    /// In daemon mode this means "detach", not "shut down".
    quit: bool,
    /// Set by `ClientMsg::Shutdown` — the daemon should exit and kill all apps.
    shutdown: bool,
    /// The app launcher (menubar dropdown + Spotlight overlay).
    launcher: Launcher,
    /// Shared host-state snapshot refreshed by the daemon's `SystemPoller`.
    tray_state: std::sync::Arc<std::sync::RwLock<crate::system::SystemState>>,
    /// The menubar status tray (open popover state + hit-testing).
    tray: crate::tray::Tray,
    /// The OS backend that applies tray control intents (volume/wifi/bluetooth).
    backend: Box<dyn crate::system::Backend>,
    /// Target-cell highlight shown while dragging a window near an edge.
    drag_preview: Option<Rect>,
    /// The working-directory picker, open while a flagged launch awaits a dir.
    dirpicker: Option<crate::dirpicker::DirPicker>,
    /// Whether the keyboard-shortcut help overlay is showing.
    help_open: bool,
}

impl SessionCore {
    /// Create a new session for a terminal of size `w` × `h` cells.
    ///
    /// The work area is set to exclude the single-row menubar at the top and
    /// the single-row dock at the bottom, i.e. `Rect::new(0, 1, w, h - 2)`.
    pub fn new(w: i32, h: i32, cfg: Config) -> Self {
        let work = Rect::new(0, 1, w, h - 2);
        let launcher = Launcher::new(Self::build_launcher_apps(&cfg));
        Self {
            wm: WindowManager::new(work),
            contents: HashMap::new(),
            store_win: None,
            settings_win: None,
            titles: Vec::new(),
            cfg,
            w,
            h,
            drag: None,
            cursor: Point::new(w / 2, h / 2),
            quit: false,
            shutdown: false,
            launcher,
            tray_state: std::sync::Arc::new(std::sync::RwLock::new(crate::system::SystemState::default())),
            tray: crate::tray::Tray::new(),
            backend: crate::system::backend(),
            drag_preview: None,
            dirpicker: None,
            help_open: false,
        }
    }

    /// Whether the working-directory picker overlay is open.
    pub fn dirpicker_open(&self) -> bool {
        self.dirpicker.is_some()
    }

    /// Whether the picker's new-folder name input is active.
    pub fn dirpicker_creating(&self) -> bool {
        self.dirpicker.as_ref().map(|d| d.is_creating()).unwrap_or(false)
    }

    /// Whether the keyboard-shortcut help overlay is showing.
    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// The current tray segments, laid out from the live snapshot.
    fn tray_segments_now(&self) -> Vec<crate::tray::Segment> {
        let st = self.tray_state.read().unwrap();
        crate::tray::tray_segments(&st, self.w)
    }

    /// Apply a tray control intent: optimistically update the cached snapshot so
    /// the UI responds immediately, then run the (timeout-guarded) backend call.
    fn apply_intent(&mut self, intent: crate::system::ControlIntent) {
        use crate::system::ControlIntent as I;
        if let Ok(mut s) = self.tray_state.write() {
            match &intent {
                I::VolumeUp => s.volume.level = s.volume.level.saturating_add(5).min(100),
                I::VolumeDown => s.volume.level = s.volume.level.saturating_sub(5),
                I::VolumeSet(l) => s.volume.level = (*l).min(100),
                I::ToggleMute => s.volume.muted = !s.volume.muted,
                I::WifiSetEnabled(on) => { if let Some(w) = s.wifi.as_mut() { w.enabled = *on; } }
                I::BtSetEnabled(on) => s.bluetooth.enabled = *on,
                _ => {}
            }
        }
        self.backend.apply(&intent);
    }

    /// Attach the daemon's shared system snapshot (written by the `SystemPoller`)
    /// so the menubar tray reflects live host state.
    pub fn attach_tray_state(
        &mut self,
        state: std::sync::Arc<std::sync::RwLock<crate::system::SystemState>>,
    ) {
        self.tray_state = state;
    }

    /// Whether `tuiui kill` requested a full daemon shutdown.
    pub fn shutdown_requested(&self) -> bool { self.shutdown }

    /// Clear the detach (quit) flag — called by the daemon after a client detaches
    /// so the next client doesn't immediately detach again.
    pub fn clear_quit(&mut self) { self.quit = false; }

    /// Build the launcher's app list: the configured apps (with categories filled
    /// in from the catalog where missing), plus any known TUIs detected on `$PATH`
    /// that aren't already listed.
    fn build_launcher_apps(cfg: &Config) -> Vec<AppEntry> {
        // Pinned tuiui actions first (open the store / settings windows).
        let mut apps = vec![
            AppEntry { name: "Store".into(), command: "@store".into(), args: vec![], category: Some("tuiui".into()), requires_cwd: None, cwd: None },
            AppEntry { name: "Settings".into(), command: "@settings".into(), args: vec![], category: Some("tuiui".into()), requires_cwd: None, cwd: None },
        ];
        apps.extend(cfg.launcher_apps());
        for a in &mut apps {
            if a.category.is_none() {
                a.category = crate::catalog::category_for(&a.name)
                    .or_else(|| crate::catalog::category_for(&a.command));
            }
        }
        for detected in crate::catalog::detect_installed() {
            if !apps.iter().any(|a| a.name.eq_ignore_ascii_case(&detected.name)) {
                apps.push(detected);
            }
        }
        apps
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
    pub fn window_count(&self) -> usize { self.contents.len() }

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
                    if let Some(c) = self.contents.get_mut(&id) {
                        c.write_input(&bytes);
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
                self.auto_tile_if_enabled();
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
            ClientMsg::TileAll => {
                let grid = self.grid();
                self.wm.tile_all(grid, self.cfg.tile_gap);
                self.sync_all_app_sizes();
            }
            ClientMsg::ToggleAutoTile => {
                self.cfg.auto_tile = !self.cfg.auto_tile;
                let _ = self.cfg.save();
                if self.cfg.auto_tile {
                    let grid = self.grid();
                    self.wm.tile_all(grid, self.cfg.tile_gap);
                    self.sync_all_app_sizes();
                }
            }
            ClientMsg::SendToCell(n) => {
                let grid = self.grid();
                if n >= 1 && n <= grid.cells() {
                    if let Some(id) = self.wm.focused() {
                        let (row, col) = grid.row_col(n - 1);
                        self.wm.send_to_cell(id, grid, row, col, self.cfg.tile_gap);
                        self.sync_app_size(id);
                    }
                }
            }
            ClientMsg::ToggleMenu => self.launcher.toggle_menu(),
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
            ClientMsg::OpenStore => self.open_store(),
            ClientMsg::StoreUp => { if let Some(s) = self.focused_store_mut() { s.move_up(); } }
            ClientMsg::StoreDown => { if let Some(s) = self.focused_store_mut() { s.move_down(); } }
            ClientMsg::StorePrevCategory => { if let Some(s) = self.focused_store_mut() { s.prev_category(); } }
            ClientMsg::StoreNextCategory => { if let Some(s) = self.focused_store_mut() { s.next_category(); } }
            ClientMsg::StoreChar(c) => { if let Some(s) = self.focused_store_mut() { s.type_char(c); } }
            ClientMsg::StoreBackspace => { if let Some(s) = self.focused_store_mut() { s.backspace(); } }
            ClientMsg::StoreActivate => self.store_activate(),
            ClientMsg::StoreClose => {
                if let Some(id) = self.wm.focused() {
                    if matches!(self.contents.get(&id), Some(WinContent::Store(_))) {
                        self.close(id);
                    }
                }
            }
            ClientMsg::OpenSettings => self.open_settings(),
            ClientMsg::SettingsUp => { if let Some(s) = self.focused_settings_mut() { s.move_up(); } }
            ClientMsg::SettingsDown => { if let Some(s) = self.focused_settings_mut() { s.move_down(); } }
            ClientMsg::SettingsPrevSection => { if let Some(s) = self.focused_settings_mut() { s.prev_section(); } }
            ClientMsg::SettingsNextSection => { if let Some(s) = self.focused_settings_mut() { s.next_section(); } }
            ClientMsg::SettingsLeft => { if let Some(s) = self.focused_settings_mut() { s.left(); } self.sync_settings(); }
            ClientMsg::SettingsRight => { if let Some(s) = self.focused_settings_mut() { s.right(); } self.sync_settings(); }
            ClientMsg::SettingsToggle => {
                if let Some(s) = self.focused_settings_mut() {
                    s.toggle();
                }
                match self.focused_settings_mut().and_then(|s| s.take_action()) {
                    Some(crate::settings::SettingsAction::CheckUpdates) => {
                        let msg = check_for_updates();
                        if let Some(s) = self.focused_settings_mut() {
                            s.set_update_status(msg);
                        }
                    }
                    Some(crate::settings::SettingsAction::InstallUpdate) => {
                        let cmd = format!(
                            "clear; echo 'Updating tuiui from {repo} …'; echo; \
cargo install --git {repo} --force; echo; echo '────'; \
echo 'Done. Quit (\u{2715} Quit) then run:  tuiui kill ; tuiui'; exec \"$SHELL\"",
                            repo = crate::REPO_URL,
                        );
                        self.launch("update tuiui".into(), "sh".into(), vec!["-lc".into(), cmd]);
                    }
                    None => {}
                }
                self.sync_settings();
            }
            ClientMsg::SettingsChar(c) => { if let Some(s) = self.focused_settings_mut() { s.type_char(c); } }
            ClientMsg::SettingsBackspace => { if let Some(s) = self.focused_settings_mut() { s.backspace(); } }
            ClientMsg::SettingsCancelEdit => { if let Some(s) = self.focused_settings_mut() { s.cancel_edit(); } }
            ClientMsg::SettingsClose => {
                if let Some(id) = self.wm.focused() {
                    if matches!(self.contents.get(&id), Some(WinContent::Settings(_))) {
                        self.close(id);
                    }
                }
            }
            ClientMsg::ToggleHelp => self.help_open = !self.help_open,
            ClientMsg::DirPickerUp => { if let Some(d) = self.dirpicker.as_mut() { d.move_up(); } }
            ClientMsg::DirPickerDown => { if let Some(d) = self.dirpicker.as_mut() { d.move_down(); } }
            ClientMsg::DirPickerExpand => { if let Some(d) = self.dirpicker.as_mut() { d.expand(); } }
            ClientMsg::DirPickerCollapse => { if let Some(d) = self.dirpicker.as_mut() { d.collapse(); } }
            ClientMsg::DirPickerToggleHidden => { if let Some(d) = self.dirpicker.as_mut() { d.toggle_hidden(); } }
            ClientMsg::DirPickerNewFolder => { if let Some(d) = self.dirpicker.as_mut() { d.begin_create(); } }
            ClientMsg::DirPickerChar(c) => { if let Some(d) = self.dirpicker.as_mut() { d.create_type(c); } }
            ClientMsg::DirPickerBackspace => { if let Some(d) = self.dirpicker.as_mut() { d.create_backspace(); } }
            ClientMsg::DirPickerCancel => {
                // Cancel the new-folder input first; only then close the picker.
                match self.dirpicker.as_mut() {
                    Some(d) if d.is_creating() => d.cancel_create(),
                    _ => self.dirpicker = None,
                }
            }
            ClientMsg::DirPickerConfirm => {
                // Enter commits the new-folder name, else launches in the dir.
                if self.dirpicker.as_ref().map(|d| d.is_creating()).unwrap_or(false) {
                    if let Some(d) = self.dirpicker.as_mut() { d.commit_create(); }
                } else {
                    self.confirm_dirpicker();
                }
            }
            ClientMsg::Shutdown => self.shutdown = true,
        }
    }

    /// Resolve the open picker: launch its pending app in the chosen directory
    /// and record the directory in the recent list.
    fn confirm_dirpicker(&mut self) {
        let Some(picker) = self.dirpicker.take() else { return };
        let (pending, path) = picker.confirm();
        // Record in the MRU (most-recent first, deduped, capped at 10).
        let p = path.to_string_lossy().to_string();
        self.cfg.recent_dirs.retain(|d| d != &p);
        self.cfg.recent_dirs.insert(0, p);
        self.cfg.recent_dirs.truncate(10);
        let _ = self.cfg.save();
        self.launch_in(pending.name, pending.command, pending.args, Some(path));
    }

    /// Launch an app, first opening the working-directory picker when it is
    /// flagged `requires_cwd` and has no fixed directory.
    fn launch_maybe_cwd(&mut self, name: String, command: String, args: Vec<String>, requires_cwd: bool, fixed: Option<String>) {
        if let Some(dir) = fixed {
            self.launch_in(name, command, args, Some(expand_tilde(&dir)));
        } else if requires_cwd {
            self.dirpicker = Some(crate::dirpicker::DirPicker::new(
                self.picker_root(),
                crate::dirpicker::PendingLaunch { name, command, args },
            ));
        } else {
            self.launch(name, command, args);
        }
    }

    /// The directory the picker opens at: the configured project dir (tilde
    /// expanded) or the user's home.
    fn picker_root(&self) -> std::path::PathBuf {
        self.cfg
            .default_project_dir
            .as_deref()
            .map(expand_tilde)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("/"))
    }

    /// Open the settings window, or focus it if it's already open.
    fn open_settings(&mut self) {
        if let Some(id) = self.settings_win {
            if self.contents.contains_key(&id) {
                self.wm.unminimize(id);
                return;
            }
        }
        let w = 60.min((self.w - 4).max(40));
        let h = 16.min((self.h - 4).max(10));
        let rect = Rect::new((self.w - w) / 2, 3, w, h);
        let id = self.wm.add_window("Settings".into(), rect);
        self.contents.insert(id, WinContent::Settings(Settings::new(self.cfg.clone())));
        self.titles.push((id, "Settings".into()));
        self.settings_win = Some(id);
    }

    /// `true` when the focused window hosts the settings panel.
    pub fn focused_is_settings(&self) -> bool {
        self.wm
            .focused()
            .and_then(|id| self.contents.get(&id))
            .map(|c| matches!(c, WinContent::Settings(_)))
            .unwrap_or(false)
    }

    /// `true` when the focused settings panel is in a text-entry field (Apps add
    /// form), so the client should forward typed characters rather than treat
    /// them as navigation.
    pub fn settings_editing(&self) -> bool {
        matches!(
            self.wm.focused().and_then(|id| self.contents.get(&id)),
            Some(WinContent::Settings(s)) if s.is_editing()
        )
    }

    fn focused_settings_mut(&mut self) -> Option<&mut Settings> {
        let id = self.wm.focused()?;
        match self.contents.get_mut(&id)? {
            WinContent::Settings(s) => Some(s),
            _ => None,
        }
    }

    /// Copy the focused settings panel's edited config into the live config and
    /// persist it to disk. Changes (snapping, shadows) then take effect next frame.
    fn sync_settings(&mut self) {
        let cfg = match self.wm.focused().and_then(|id| self.contents.get(&id)) {
            Some(WinContent::Settings(s)) => Some(s.config().clone()),
            _ => None,
        };
        if let Some(cfg) = cfg {
            // Rebuilding the launcher rescans $PATH, so only do it when the
            // custom-app list actually changed (not on every shadow/theme tweak).
            let launcher_changed = cfg.launcher != self.cfg.launcher;
            self.cfg = cfg;
            crate::theme::set(&self.cfg.theme);
            let _ = self.cfg.save();
            if launcher_changed {
                self.launcher = Launcher::new(Self::build_launcher_apps(&self.cfg));
            }
        }
    }

    /// Open the store window, or focus it if it's already open.
    fn open_store(&mut self) {
        if let Some(id) = self.store_win {
            if self.contents.contains_key(&id) {
                self.wm.unminimize(id);
                return;
            }
        }
        let w = 84.min((self.w - 4).max(40));
        let h = 28.min((self.h - 4).max(12));
        let rect = Rect::new((self.w - w) / 2, 2, w, h);
        let id = self.wm.add_window("Store".into(), rect);
        self.contents.insert(id, WinContent::Store(Store::new()));
        self.titles.push((id, "Store".into()));
        self.store_win = Some(id);
    }

    /// `true` when the focused window hosts the store browser (the front-end
    /// routes keyboard input to the store in this case).
    pub fn focused_is_store(&self) -> bool {
        self.wm
            .focused()
            .and_then(|id| self.contents.get(&id))
            .map(|c| matches!(c, WinContent::Store(_)))
            .unwrap_or(false)
    }

    fn focused_store_mut(&mut self) -> Option<&mut Store> {
        let id = self.wm.focused()?;
        match self.contents.get_mut(&id)? {
            WinContent::Store(s) => Some(s),
            _ => None,
        }
    }

    /// Enter on a store row: launch the app if installed, else install it (the
    /// install command runs visibly in a new shell window).
    fn store_activate(&mut self) {
        let app = self.focused_store_mut().and_then(|s| s.selected_app());
        let Some(app) = app else { return };
        if crate::catalog::is_installed(&app.bin) {
            // Coding agents (flagged, or in the AI category) prompt for a dir.
            let requires_cwd = crate::catalog::recipe(&app.name).map(|r| r.requires_cwd).unwrap_or(false)
                || app.category == "AI";
            self.launch_maybe_cwd(app.name.clone(), app.bin.clone(), Vec::new(), requires_cwd, None);
        } else {
            let cmd = store::install_command(app);
            self.launch(format!("install: {}", app.name), "sh".into(), vec!["-lc".into(), cmd]);
        }
    }

    /// Activate a launcher entry: open the store/settings for the pinned tuiui
    /// actions, otherwise spawn the app (prompting for a working directory when
    /// the entry is flagged `requires_cwd` and has no fixed `cwd`).
    fn launch_entry(&mut self, e: AppEntry) {
        match e.command.as_str() {
            "@store" => self.open_store(),
            "@settings" => self.open_settings(),
            _ => self.launch_maybe_cwd(e.name, e.command, e.args, e.requires_cwd.unwrap_or(false), e.cwd),
        }
    }

    /// Spawn a new PTY-backed window.
    ///
    /// If `AppInstance::spawn` fails, the window is removed and no dock entry
    /// is added (silently drops the launch request — the caller can surface an
    /// error later via a `CoreMsg` notification once that protocol exists).
    fn launch(&mut self, name: String, command: String, args: Vec<String>) {
        self.launch_in(name, command, args, None);
    }

    /// Spawn a new PTY-backed window, starting the child in `cwd` (or the user's
    /// home when `None`).
    fn launch_in(&mut self, name: String, command: String, args: Vec<String>, cwd: Option<std::path::PathBuf>) {
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
        match AppInstance::spawn(&command, &args, content.w.max(1), content.h.max(1), cwd.as_deref()) {
            Ok(app) => {
                self.contents.insert(id, WinContent::App(app));
                self.titles.push((id, name));
                self.auto_tile_if_enabled();
            }
            Err(_) => {
                self.wm.close(id);
            }
        }
    }

    /// Route a mouse event through dock hit-testing then the WM input router.
    fn handle_mouse(&mut self, kind: MouseKind, p: Point) {
        // The help overlay is modal: any click dismisses it.
        if kind == MouseKind::Down && self.help_open {
            self.help_open = false;
            return;
        }

        // The working-directory picker captures clicks while open: a click on a
        // row selects + expands it; a click outside the box cancels.
        if kind == MouseKind::Down && self.dirpicker.is_some() {
            let (w, h) = (self.w, self.h);
            let hit = self.dirpicker.as_ref().and_then(|d| d.row_at(p, w, h));
            match hit {
                Some(i) => {
                    if let Some(d) = self.dirpicker.as_mut() {
                        d.select(i);
                        d.expand();
                    }
                }
                None => self.dirpicker = None,
            }
            return;
        }

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

        // An open tray popover captures the next click: apply its intent, or
        // dismiss when the click misses every hot-zone.
        if kind == MouseKind::Down && self.tray.open().is_some() {
            let rendered = { let st = self.tray_state.read().unwrap(); self.tray.render(self.w, self.h, &st) };
            if let Some(intent) = self.tray.on_popover_click(p, &rendered) {
                self.apply_intent(intent);
                return;
            }
            self.tray.close();
            return;
        }

        // Menubar brand opens the launcher dropdown; quit button, tray segments,
        // and dock clicks are checked before normal window routing.
        if kind == MouseKind::Down {
            if menubar_brand_region().contains(p) {
                self.launcher.toggle_menu();
                return;
            }
            if menubar_quit_region(self.w).contains(p) {
                self.quit = true;
                return;
            }
            let segs = self.tray_segments_now();
            if self.tray.on_menubar_click(p, &segs) {
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
                // Show a target-cell highlight when the pointer nears an edge.
                let work = Rect::new(0, 1, self.w, self.h - 2);
                self.drag_preview = if self.cfg.snapping_enabled && near_edge(p, work, self.cfg.snap_threshold) {
                    let grid = self.grid();
                    let (row, col) = grid.cell_at(work, p);
                    Some(grid.cell_rect(work, row, col, self.cfg.tile_gap))
                } else {
                    None
                };
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
                // The store handles content clicks (category/row/install); PTY apps
                // are keyboard-first for now, so raising is enough for them.
                let cr = self.wm.get(id).map(|w| w.content_rect());
                let mut store_activate = false;
                let mut settings_changed = false;
                if let Some(cr) = cr {
                    match self.contents.get_mut(&id) {
                        Some(WinContent::Store(s)) => store_activate = s.handle_click(local, cr.w, cr.h),
                        Some(WinContent::Settings(s)) => settings_changed = s.handle_click(local, cr.w, cr.h),
                        _ => {}
                    }
                }
                if store_activate {
                    self.store_activate();
                }
                if settings_changed {
                    self.sync_settings();
                }
            }
            Action::EndDrag => {
                if let Some(Hit::Moving { id, .. }) = self.drag {
                    let work = Rect::new(0, 1, self.w, self.h - 2);
                    if self.cfg.snapping_enabled && near_edge(p, work, self.cfg.snap_threshold) {
                        let grid = self.grid();
                        let (row, col) = grid.cell_at(work, p);
                        // In auto-tile mode, dropping onto an occupied cell swaps
                        // the two windows; otherwise place into the target cell.
                        match self.window_in_cell(grid, row, col, id) {
                            Some(other) if self.cfg.auto_tile => {
                                self.wm.swap_cells(id, other);
                                self.sync_app_size(id);
                                self.sync_app_size(other);
                            }
                            _ => {
                                self.wm.send_to_cell(id, grid, row, col, self.cfg.tile_gap);
                                self.sync_app_size(id);
                            }
                        }
                    }
                }
                self.drag = None;
                self.drag_preview = None;
            }
            Action::None => {}
        }
    }

    /// The configured tiling grid (clamped to 1..=6 on each axis).
    fn grid(&self) -> crate::geometry::Grid {
        crate::geometry::Grid {
            rows: self.cfg.grid_rows.clamp(1, 6),
            cols: self.cfg.grid_cols.clamp(1, 6),
        }
    }

    /// The id of a non-`except` window currently tiled in cell `(row, col)`.
    fn window_in_cell(&self, _grid: crate::geometry::Grid, row: u8, col: u8, except: WindowId) -> Option<WindowId> {
        self.wm
            .z_ordered()
            .into_iter()
            .find(|w| w.id != except && w.state == crate::window::WindowState::Tiled { row, col })
            .map(|w| w.id)
    }

    /// Re-tile all windows into the grid when auto-tile is on (called after a
    /// window opens, closes, or the screen resizes).
    fn auto_tile_if_enabled(&mut self) {
        if self.cfg.auto_tile {
            let grid = self.grid();
            self.wm.tile_all(grid, self.cfg.tile_gap);
            self.sync_all_app_sizes();
        }
    }

    /// Resize every window's hosted app to its current content rect (after a
    /// bulk re-tile).
    fn sync_all_app_sizes(&mut self) {
        let ids: Vec<WindowId> = self.wm.z_ordered().iter().map(|w| w.id).collect();
        for id in ids {
            self.sync_app_size(id);
        }
    }

    /// Tell the app instance for `id` to resize to match the window's current
    /// content rect.
    fn sync_app_size(&mut self, id: WindowId) {
        if let Some(w) = self.wm.get(id) {
            let c = w.content_rect();
            if let Some(content) = self.contents.get_mut(&id) {
                content.resize(c.w.max(1), c.h.max(1));
            }
        }
    }

    /// Kill a window's content, remove its dock entry, and close the WM window.
    fn close(&mut self, id: WindowId) {
        if let Some(mut content) = self.contents.remove(&id) {
            content.kill();
        }
        if self.store_win == Some(id) {
            self.store_win = None;
        }
        if self.settings_win == Some(id) {
            self.settings_win = None;
        }
        self.titles.retain(|(i, _)| *i != id);
        self.wm.close(id);
        self.auto_tile_if_enabled();
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
            let cr = w.content_rect();
            let content = self.contents.get(&w.id)
                .map(|c| c.render(cr.w, cr.h))
                .unwrap_or_else(|| crate::buffer::CellBuffer::new(cr.w, cr.h));
            layers.extend(render_window(w, &content, Some(w.id) == focused, self.cfg.window_shadows));
        }

        // Drag-to-cell preview: a translucent highlight of the target cell,
        // above the windows but below the chrome.
        if let Some(r) = self.drag_preview {
            let t = crate::theme::current();
            let mut buf = crate::buffer::CellBuffer::new(r.w, r.h);
            let mut tint = t.accent;
            tint.a = 70; // translucent so the windows below show through
            buf.fill(crate::cell::Cell { ch: ' ', fg: crate::cell::Rgba::TRANSPARENT, bg: tint, attrs: Default::default() });
            layers.push(Layer { z: 900, origin: Point::new(r.x, r.y), buf, opacity: 1.0, scissor: None });
        }

        let app_name = focused
            .and_then(|id| self.titles.iter().find(|(i, _)| *i == id))
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        let segs = {
            let st = self.tray_state.read().unwrap();
            crate::tray::tray_segments(&st, self.w)
        };
        layers.push(render_menubar(self.w, &app_name, &segs));
        layers.push(render_dock(self.w, self.h, &self.dock_items()));

        // Launcher (dropdown / Spotlight) renders above all chrome.
        layers.extend(self.launcher.render(self.w, self.h).layers);

        // An open tray popover renders above everything else.
        {
            let st = self.tray_state.read().unwrap();
            layers.extend(self.tray.render(self.w, self.h, &st).layers);
        }

        // The working-directory picker renders on top of the whole desktop.
        if let Some(d) = &self.dirpicker {
            layers.extend(d.render(self.w, self.h));
        }

        // The help overlay is the topmost layer of all.
        if self.help_open {
            layers.extend(crate::help::render_help(self.w, self.h));
        }

        Frame { layers, cursor: Some(self.cursor) }
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Remove windows whose PTY child has exited.
    ///
    /// Call this once per render loop tick to keep the session consistent with
    /// process state.
    pub fn reap_dead(&mut self) {
        let dead: Vec<WindowId> = self.contents
            .iter_mut()
            .filter_map(|(id, c)| if !c.is_alive() { Some(*id) } else { None })
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
        for (_, content) in self.contents.iter_mut() {
            content.kill();
        }
        self.contents.clear();
    }
}

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(s: &str) -> std::path::PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    std::path::PathBuf::from(s)
}

/// True when `p` is within `threshold` cells of any edge of `work` — the band in
/// which a drag engages grid-cell snapping (interior drags stay floating).
fn near_edge(p: Point, work: Rect, threshold: i32) -> bool {
    p.x - work.x < threshold
        || work.right() - p.x < threshold
        || p.y - work.y < threshold
        || work.bottom() - p.y < threshold
}

/// Check the upstream repository for a newer commit than this build.
///
/// Uses `curl` against the GitHub API with a hard timeout so the call can never
/// hang the desktop. Returns a short human-readable status string.
fn check_for_updates() -> String {
    let short = |s: &str| s.chars().take(7).collect::<String>();
    let api = format!(
        "https://api.github.com/repos/{}/commits/main",
        crate::REPO_URL.trim_start_matches("https://github.com/")
    );
    let out = std::process::Command::new("curl")
        .args(["-fsS", "--max-time", "6", "-H", "User-Agent: tuiui", &api])
        .output();
    let latest = out
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| v.get("sha").and_then(|s| s.as_str()).map(str::to_string));
    match latest {
        Some(sha) => {
            let cur = crate::GIT_SHA;
            if cur == "unknown" {
                format!("Latest is {} — reinstall to update", short(&sha))
            } else if sha.starts_with(cur) || cur.starts_with(&short(&sha)) {
                format!("Up to date ({})", short(cur))
            } else {
                format!("Update available: {} → {}", short(cur), short(&sha))
            }
        }
        None => "Couldn't check (offline?)".to_string(),
    }
}
