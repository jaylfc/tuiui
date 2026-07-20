//! Session core — the `ClientMsg`-in / `Frame`-out boundary.
//!
//! [`SessionCore`] is the integration layer that owns:
//! - the [`WindowManager`] (window geometry, z-order, focus), and
//! - the [`LocalAppHost`] (PTY-backed child processes).
//!
//! All external control flows through [`ClientMsg`] variants; the core
//! produces a [`Frame`] (ordered compositor layers + cursor position) via
//! [`SessionCore::build_frame`].  No I/O, no terminal types, no renderer
//! details cross this boundary — it is the seam that a future daemon will
//! expose on a socket.

use crate::chrome::{
    render_menubar, render_dock, dock_hit_regions, menubar_assistant_region, menubar_brand_region, menubar_mode_region, menubar_power_region, DockItem, DockKind,
};
use crate::powermenu::{PowerClick, PowerMenu, PowerOutcome};
use crate::confirmclose::ConfirmClose;
use crate::launchwarn::LaunchWarn;
use crate::compositor::Layer;
use crate::config::{AppEntry, Config};
use crate::geometry::{Point, Rect, SnapZone};
use crate::input::{route_mouse, MouseKind, Hit, Action};
use crate::apphost::{AppHost, AppId, LocalAppHost};
use crate::launcher::Launcher;
use crate::settings::Settings;
use crate::activity::Activity;
use crate::store::{self, Store};
use crate::window::WindowId;
use crate::wm::{WindowManager, render_window};
use std::collections::HashMap;

/// Opaque per-app window state the frontend stashes in the apphost so a fresh
/// frontend (after `reload` or a crash) can rebuild the window in place.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Debug)]
struct WinMeta {
    rect: crate::geometry::Rect,
    title: String,
    z: i32,
    minimized: bool,
    /// Immutable grouping key (the launch name, set at spawn time). If absent in
    /// older stored meta, the title is used as a fallback.
    #[serde(default)]
    app_key: String,
}

/// A file-manager window's backend identity: `None` browses the local disk,
/// `Some((ssh_target, port))` a saved remote system.
type FmBackend = Option<(String, Option<u16>)>;

/// The content hosted by a window: a PTY-backed app or a native Tuiui widget.
///
/// This is the [`WindowContent`](../../docs) seam — the window manager, chrome,
/// and input routing operate on windows uniformly, while the content type varies.
enum WinContent {
    /// A PTY-backed child process, identified by its `AppId` in the session's `LocalAppHost`.
    App(AppId),
    /// The native app store browser.
    Store(Store),
    /// The native settings panel (boxed: by far the largest variant, and there
    /// is at most one settings window).
    Settings(Box<Settings>),
    /// A native image viewer (placeholder cells + a Kitty graphics placement).
    ImageView(crate::imageview::ImageView),
    /// The native file manager (local disk or a remote system over ssh).
    FileManager(crate::filemanager::DynFileManager),
    /// The native log viewer (tail of ~/tuiui-debug.log + clipboard copy).
    Logs(crate::logsview::LogsView),
    /// The activity monitor — a live table of hosted apps with kill-app controls.
    Activity(Activity),
}

impl WinContent {
    fn render(&self, host: &dyn AppHost, w: i32, h: i32) -> crate::buffer::CellBuffer {
        match self {
            WinContent::App(id) => host.snapshot(*id).unwrap_or_else(|| crate::buffer::CellBuffer::new(w, h)),
            WinContent::Store(s) => s.render(w, h),
            WinContent::Settings(s) => s.render(w, h),
            WinContent::ImageView(v) => v.render(w, h),
            WinContent::FileManager(f) => f.render(w, h),
            WinContent::Logs(l) => l.render(w, h),
            WinContent::Activity(a) => a.render(w, h),
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
    /// Collapse the cascade one level (Menu mode, ←).
    LauncherLeft,
    /// Descend into the focused submenu (Menu mode, →).
    LauncherRight,
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
    /// Open the file-manager window (or focus it if already open).
    OpenFileManager,
    /// File manager: move the cursor.
    FileManagerUp,
    FileManagerDown,
    FileManagerLeft,
    FileManagerRight,
    /// File manager: enter the focused entry (navigate / open).
    FileManagerActivate,
    /// File manager: history back.
    FileManagerBack,
    /// File manager: navigate to the parent directory.
    FileManagerParent,
    /// File manager: cycle Icon / List / Columns view.
    FileManagerToggleView,
    /// File manager: set the view directly.
    FileManagerViewIcon,
    FileManagerViewList,
    FileManagerViewColumns,
    /// File manager: toggle the preview pane.
    FileManagerTogglePreview,
    /// File manager: toggle hidden (dot) entries.
    FileManagerToggleHidden,
    /// File manager: begin the new-folder overlay.
    FileManagerNewFolder,
    /// File manager: begin the rename overlay.
    FileManagerRename,
    /// File manager: begin the delete confirmation.
    FileManagerDelete,
    /// File manager: copy / cut the selection to the clipboard.
    FileManagerCopy,
    FileManagerCut,
    /// File manager: paste the clipboard into the current directory.
    FileManagerPaste,
    /// File manager: type / delete a character in an overlay text field.
    FileManagerChar(char),
    FileManagerBackspace,
    /// File manager: commit the active overlay (Enter).
    FileManagerCommit,
    /// File manager: cancel the active overlay (Escape).
    FileManagerCancel,
    /// File manager: close the window (Escape with no overlay).
    FileManagerClose,
    /// File manager: open a new tab on the current folder.
    FileManagerNewTab,
    /// File manager: close the active tab.
    FileManagerCloseTab,
    /// File manager: focus the next tab.
    FileManagerNextTab,
    /// Open the activity-monitor window (or focus it if already open).
    OpenActivity,
    /// Activity monitor: move selection up / down.
    ActivityUp,
    ActivityDown,
    /// Activity monitor: request to kill the selected app (Enter / `k`).
    /// Returns the requested kill via the in-app confirm overlay when the app
    /// is alive, or kills immediately when it's already dead.
    ActivityKill,
    /// Activity monitor: confirm a pending kill (`Enter` / `y` from confirm).
    ActivityConfirmKill,
    /// Activity monitor: cancel a pending kill (`Esc` / `n` from confirm).
    ActivityCancelKill,
    /// Activity monitor: kill every row currently in the `dead` state.
    ActivityKillDead,
    /// Activity monitor: force a refresh of the row list (`r`).
    ActivityRefresh,
    /// Activity monitor: close the panel (`Esc` when no kill is pending).
    ActivityClose,
    /// Toggle the keyboard-shortcut help overlay.
    ToggleHelp,
    /// Open an image file in a native image-viewer window.
    OpenImage(String),
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
    /// Left-button double-click at screen coordinates `p` (desktop: open icon).
    MouseDouble(Point),
    /// Right-button press at screen coordinates `p` (desktop: context menu).
    MouseRightDown(Point),
    /// Desktop overlay (rename / new-folder): type a character.
    DesktopChar(char),
    /// Desktop overlay: delete the last character.
    DesktopBackspace,
    /// Desktop overlay: commit (Enter).
    DesktopCommit,
    /// Desktop overlay: cancel (Escape).
    DesktopCancel,
    /// Begin renaming the currently focused window (keyboard shortcut / double-click).
    RenameFocused,
    /// Type a character into the window rename buffer.
    RenameChar(char),
    /// Delete the last character from the window rename buffer.
    RenameBackspace,
    /// Commit the window rename (sets the new title; empty buffer = cancel).
    RenameCommit,
    /// Cancel the window rename without changing the title.
    RenameCancel,
    /// Confirm the close-window dialog (Enter / y): close the pending window.
    ConfirmCloseYes,
    /// Dismiss the close-window dialog (Esc / n) without closing.
    ConfirmCloseNo,
    /// Confirm the launch-warning dialog (Enter / y): launch the pending entry.
    LaunchWarnYes,
    /// Dismiss the launch-warning dialog (Esc / n) without launching.
    LaunchWarnNo,
    /// Shut down the daemon entirely (kills all apps). Sent by `tuiui kill`.
    Shutdown,
    /// Restart the frontend only, keeping the apphost (and apps) alive.
    Reload,
    /// A raw mouse event destined for the focused app's PTY (passthrough).
    MouseInput(crate::mouse::MouseInput),
    /// Scroll the scrollback of the PTY window under `p` by `lines`
    /// (+ = back into history). Sent by the wheel when no overlay claims it.
    ScrollAt { p: Point, lines: i32 },
    /// Power-menu Add Remote form: type a character into the focused field.
    PowerFormChar(char),
    /// Power-menu form: delete the last character of the focused field.
    PowerFormBackspace,
    /// Power-menu form: focus the next / previous field (Tab / Shift-Tab, ↓ / ↑).
    PowerFormNext,
    PowerFormPrev,
    /// Power-menu form: cycle the theme field (← / →).
    PowerFormLeft,
    PowerFormRight,
    /// Power-menu form: Enter — advance, submitting from the last field.
    PowerFormCommit,
    /// Power-menu form: Esc — back to the Systems submenu.
    PowerFormCancel,
    /// Apply (and persist) a theme by name. Sent by the client on attach when
    /// `TUIUI_THEME` is set — how a per-system theme rides over ssh.
    SetTheme(String),
    /// Logs viewer: scroll one line / one page.
    LogsUp,
    LogsDown,
    LogsPageUp,
    LogsPageDown,
    /// Logs viewer: copy the log tail to the host clipboard (OSC 52).
    LogsCopy,
    /// Logs viewer: re-read the log file.
    LogsRefresh,
    /// Logs viewer: close the window.
    LogsClose,
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
    /// Image placements (Kitty graphics) for the visible ImageView windows.
    pub images: Vec<crate::protocol::ImagePlacement>,
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
    apphost: Box<dyn AppHost>,
    /// The store window's id, if open (so it can be re-focused, not re-opened).
    store_win: Option<WindowId>,
    /// The settings window's id, if open.
    settings_win: Option<WindowId>,
    /// The file-manager window's id, if open.
    filemanager_win: Option<WindowId>,
    /// Remote file-manager windows: window → (system name, ssh target, port).
    remote_fms: HashMap<WindowId, (String, String, Option<u16>)>,
    /// Cross-window file clipboard for system↔system transfers: the source
    /// window's ssh backend (`None` = local disk) and the copied paths.
    transfer: Option<(FmBackend, Vec<std::path::PathBuf>)>,
    /// Post-update safety dialog: the running apphost predates this binary's
    /// minimum-compatible protocol, so a full app-server restart (which closes
    /// the user's apps) is needed for everything to work. The dialog gives
    /// them a chance to save work first.
    compat_dialog: bool,
    /// Set once the user was warned (so Settings can show the restart row even
    /// after the dialog is dismissed).
    apphost_outdated: bool,
    /// The logs-viewer window's id, if open.
    logs_win: Option<WindowId>,
    /// The activity-monitor window's id, if open.
    activity_win: Option<WindowId>,
    /// Text to put on the HOST terminal's clipboard (OSC 52), shipped to the
    /// client in the next frame: log copies and app OSC-52 stores.
    pending_clipboard: Option<String>,
    /// Dock-ordered list of (id, display-name) pairs.
    titles: Vec<(WindowId, String)>,
    /// Immutable grouping key per window (set at spawn/open; never mutated).
    app_keys: HashMap<WindowId, String>,
    /// The app_key of the dock group whose chooser popup is open (`None` = closed).
    dock_popup: Option<String>,
    /// Dock right-click context menu: `Some((target window, anchor x of the
    /// clicked pill))` while open, `None` when closed.
    dock_ctx: Option<(WindowId, i32)>,
    /// Menubar power-button label: the host name + ▾ (computed once at startup).
    power_label: String,
    /// Active window rename: `Some((id, buffer))` while the user is typing a new
    /// name. `None` when no rename is in progress.
    rename: Option<(WindowId, String)>,
    cfg: Config,
    w: i32,
    h: i32,
    drag: Option<Hit>,
    /// Pointer position where the current drag was pressed, and whether it has
    /// passed the drag threshold. A plain click (or sub-threshold jitter) on a
    /// titlebar must NOT move/untile the window — only a real drag does.
    drag_start: Option<Point>,
    drag_armed: bool,
    cursor: Point,
    /// Set when the user clicks the menubar quit button; polled by the loop.
    /// In daemon mode this means "detach", not "shut down".
    quit: bool,
    /// Set by `ClientMsg::Shutdown` — the daemon should exit and kill all apps.
    shutdown: bool,
    /// Set by `ClientMsg::Reload` — the daemon should restart the frontend only,
    /// keeping the apphost (and all apps) alive.
    reload: bool,
    /// The app launcher (menubar dropdown + Spotlight overlay).
    launcher: Launcher,
    /// The top-right power menu (Exit / Restart / Shutdown + confirm dialogs).
    power_menu: PowerMenu,
    /// Modal "are you sure?" shown when closing an app window (which kills it).
    confirm_close: ConfirmClose,
    /// Modal "are you sure?" shown before launching an app entry flagged `warn`.
    launch_warn: LaunchWarn,
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
    /// Whether the full-screen "simple" view mode is active.
    simple: bool,
    /// Decoded-image cache for ImageView windows (the native image layer).
    images: crate::imagestore::ImageStore,
    /// The wallpaper-level desktop icons (merged `~/Desktop` + pins).
    desktop: crate::desktop::DesktopIcons,
    /// When [`poll_desktop_dir`](Self::poll_desktop_dir) last actually stat'd
    /// the desktop folder (throttle so every tick doesn't hit the filesystem).
    desktop_last_poll: std::time::Instant,
    /// The desktop folder's modified-time as of the last poll, so a changed
    /// mtime (folder/file created or removed outside tuiui) is detected.
    desktop_last_mtime: Option<std::time::SystemTime>,
    /// Maps a hosted app's Kitty image id `(window, kitty_id)` to the `ImageStore`
    /// id its PNG was loaded under (populated by [`refresh_app_graphics`]).
    app_image_ids: HashMap<(WindowId, u32), u64>,
    /// Background loader for file-manager / desktop thumbnails (off the desktop
    /// loop so a slow/offloaded image never freezes the UI).
    thumb_loader: crate::thumbnail::ThumbLoader,
    /// Maps a source image path to its loaded thumbnail `ImageStore` id.
    thumb_ids: HashMap<std::path::PathBuf, u64>,
    /// Pre-generated large file-type icons (one per role) → `ImageStore` id.
    role_icon_ids: HashMap<crate::openwith::Role, u64>,
    /// Last `WinMeta` blob pushed to the apphost per app window (change-gate so
    /// we only send `set_meta` when a window actually moved/retitled/minimized).
    last_meta: HashMap<WindowId, Vec<u8>>,
    /// Saved remote systems shown in the power menu's Systems submenu.
    systems: Vec<crate::systems::RemoteSystem>,
    /// Set when the user picked a system to switch to: shipped to the client in
    /// the next frame (alongside `quit`), which exits and ssh-es over.
    switch_to: Option<crate::systems::SwitchSpec>,
}

impl SessionCore {
    /// Create a new session for a terminal of size `w` × `h` cells.
    ///
    /// The work area is set to exclude the single-row menubar at the top and
    /// the single-row dock at the bottom, i.e. `Rect::new(0, 1, w, h - 2)`.
    pub fn new(w: i32, h: i32, cfg: Config) -> Self {
        Self::with_apphost(w, h, cfg, Box::new(LocalAppHost::new()))
    }

    /// Construct a session backed by a specific [`AppHost`]. The daemon injects
    /// a `RemoteAppHost` here (Phase 2b); tests and in-process use get the
    /// default `LocalAppHost` via [`new`](Self::new).
    pub fn with_apphost(w: i32, h: i32, cfg: Config, apphost: Box<dyn AppHost>) -> Self {
        let work = Rect::new(0, 1, w, h - 2);
        let systems = crate::systems::load();
        let launcher = Launcher::new(Self::build_launcher_apps(&cfg, &systems));
        let desktop_dir = dirs::home_dir().map(|h| h.join("Desktop")).unwrap_or_default();
        let mut core = Self {
            wm: WindowManager::new(work),
            contents: HashMap::new(),
            apphost,
            store_win: None,
            settings_win: None,
            filemanager_win: None,
            remote_fms: HashMap::new(),
            transfer: None,
            compat_dialog: false,
            apphost_outdated: false,
            logs_win: None,
            activity_win: None,
            pending_clipboard: None,
            titles: Vec::new(),
            app_keys: HashMap::new(),
            dock_popup: None,
            dock_ctx: None,
            power_label: Self::host_power_label(),
            rename: None,
            cfg,
            w,
            h,
            drag: None,
            drag_start: None,
            drag_armed: false,
            cursor: Point::new(w / 2, h / 2),
            quit: false,
            shutdown: false,
            reload: false,
            launcher,
            power_menu: PowerMenu::new(),
            confirm_close: ConfirmClose::new(),
            launch_warn: LaunchWarn::new(),
            tray_state: std::sync::Arc::new(std::sync::RwLock::new(crate::system::SystemState::default())),
            tray: crate::tray::Tray::new(),
            backend: crate::system::backend(),
            drag_preview: None,
            dirpicker: None,
            help_open: false,
            simple: false,
            images: crate::imagestore::ImageStore::new(),
            desktop: crate::desktop::DesktopIcons::new(desktop_dir),
            desktop_last_poll: std::time::Instant::now(),
            desktop_last_mtime: None,
            app_image_ids: HashMap::new(),
            thumb_loader: crate::thumbnail::ThumbLoader::new(),
            thumb_ids: HashMap::new(),
            role_icon_ids: HashMap::new(),
            last_meta: HashMap::new(),
            systems,
            switch_to: None,
        };
        core.generate_role_icons();
        core.reload_desktop();
        core
    }

    /// Generate the large file-type icons once and cache them in the image store.
    fn generate_role_icons(&mut self) {
        use crate::openwith::Role::*;
        // Match the icon-image cell box aspect (cells are ~8×16px), at 2× for crispness.
        let w = ((crate::desktop::ICON_W - 2).max(2) * 16) as u32;
        let h = ((crate::desktop::ICON_H - 1).max(1) * 32) as u32;
        for role in [
            Image, Audio, Video, Text, Code, Archive, Pdf, Directory, Executable, Other,
        ] {
            if let Some(png) = crate::icons::role_icon_png(role, w, h) {
                let id = self.images.store_png(png, w, h);
                self.role_icon_ids.insert(role, id);
            }
        }
    }

    /// Rebuild the desktop icons from the configured pins + saved positions, lay
    /// them out for the current screen, and refresh any image thumbnails.
    fn reload_desktop(&mut self) {
        // layout() must run first: reload() assigns icons to free grid cells, which
        // needs the real column/row count (else everything piles onto cell (0,0)).
        self.desktop.layout(self.w, self.h);
        self.desktop.reload(&self.cfg.desktop_pins, &self.cfg.desktop_positions);
        crate::dbg_log(&format!("desktop reload: {} icons", self.desktop.icons().len()));
        self.refresh_desktop_thumbnails();
    }

    /// Throttled mtime watch for `~/Desktop`: at most every
    /// [`DESKTOP_POLL_INTERVAL`], stat the desktop folder and reload the
    /// desktop when its modified-time changed since the last observed scan —
    /// picks up a folder/file created via the file manager or a terminal
    /// without an FS-watcher dependency. Called once per daemon tick; cheap
    /// (a single `stat`, gated by an `Instant` so most ticks do nothing).
    pub fn poll_desktop_dir(&mut self) {
        let now = std::time::Instant::now();
        if !desktop_poll_due(self.desktop_last_poll, now) {
            return;
        }
        self.desktop_last_poll = now;
        let current = std::fs::metadata(self.desktop.dir()).and_then(|m| m.modified()).ok();
        if desktop_mtime_changed(self.desktop_last_mtime, current) {
            crate::dbg_log("desktop: folder mtime changed, auto-reloading");
            self.reload_desktop();
        }
        self.desktop_last_mtime = current;
    }

    /// Assign already-loaded thumbnails to the desktop's image icons and queue any
    /// not-yet-loaded ones on the background loader. Never blocks on file I/O.
    fn refresh_desktop_thumbnails(&mut self) {
        for (idx, path) in self.desktop.thumbnail_requests() {
            if let Some(&id) = self.thumb_ids.get(&path) {
                self.desktop.set_thumb(idx, id);
            } else {
                // Icon tile is 21 cells wide, 4 tall; cells are ~8x16px.
                self.thumb_loader.request(path, 13 * 8, 4 * 16);
            }
        }
    }

    /// Drain finished thumbnails into the image store, then (re)assign them to the
    /// desktop and the open file manager. Called once per frame; cheap (no I/O).
    pub fn pump_thumbnails(&mut self) {
        for r in self.thumb_loader.drain() {
            let id = self.images.store_png(r.png, r.w, r.h);
            self.thumb_ids.insert(r.path, id);
        }
        self.refresh_desktop_thumbnails();
        self.refresh_fm_thumbnails();
    }

    /// Drain app events captured by the PTY emulators: bell rings become dock/
    /// tray notifications (for unfocused or minimized windows), and OSC-52
    /// clipboard stores are forwarded to the host terminal. Called once per
    /// daemon tick; cheap (drains in-memory counters).
    pub fn pump_app_events(&mut self) {
        let focused = self.wm.focused();
        let minimized: std::collections::HashSet<WindowId> =
            self.wm.z_ordered().iter().filter(|w| w.minimized).map(|w| w.id).collect();
        let now = self.tray_state.read().map(|st| st.clock.time.clone()).unwrap_or_default();
        let wins: Vec<(WindowId, AppId, String)> = self
            .titles
            .iter()
            .filter_map(|(id, title)| match self.contents.get(id) {
                Some(WinContent::App(aid)) => Some((*id, *aid, title.clone())),
                _ => None,
            })
            .collect();
        for (id, aid, title) in wins {
            let bells = self.apphost.take_bells(aid);
            if bells > 0 && (focused != Some(id) || minimized.contains(&id)) {
                crate::dbg_log(&format!("notify: '{title}' rang the bell x{bells}"));
                self.tray.notify(id.0, title.clone(), now.clone());
            }
            if let Some(text) = self.apphost.take_clipboard(aid) {
                crate::dbg_log(&format!("clipboard: forwarding {} bytes from '{title}' (OSC 52)", text.len()));
                self.pending_clipboard = Some(text);
            }
        }
    }

    /// Point the desktop at a specific directory and reload (integration tests).
    #[doc(hidden)]
    pub fn set_desktop_dir_for_test(&mut self, dir: std::path::PathBuf) {
        self.desktop = crate::desktop::DesktopIcons::new(dir);
        self.reload_desktop();
        // Re-baseline the mtime watch against the new folder so a stale mtime
        // from the previous desktop dir doesn't trigger a spurious reload.
        self.desktop_last_mtime = None;
        self.desktop_last_poll = std::time::Instant::now();
    }

    /// The number of currently-selected desktop icons (integration tests).
    #[doc(hidden)]
    pub fn desktop_selection_len_for_test(&self) -> usize {
        self.desktop.selection().len()
    }

    /// The screen tile rect of desktop icon `idx` (integration tests).
    #[doc(hidden)]
    pub fn desktop_icon_tile_for_test(&self, idx: usize) -> crate::geometry::Rect {
        self.desktop.tile_rect(self.desktop.icons()[idx].cell)
    }

    /// Begin the desktop new-folder overlay directly (integration tests).
    #[doc(hidden)]
    pub fn begin_desktop_new_folder_for_test(&mut self) {
        self.desktop.begin_new_folder();
    }

    /// Whether the desktop has a rename / new-folder overlay open (so the client
    /// forwards typed characters as desktop overlay input).
    pub fn desktop_editing(&self) -> bool {
        self.desktop.is_editing()
    }

    /// Whether a window rename is in progress (so the client forwards typed
    /// characters to the rename buffer rather than the focused app).
    pub fn renaming(&self) -> bool {
        self.rename.is_some()
    }

    /// The menubar power-button label: the machine's host name + a ▾ chevron
    /// (the short host name, domain stripped, capped so it can't crowd the bar).
    fn host_power_label() -> String {
        let host = sysinfo::System::host_name().unwrap_or_else(|| "tuiui".into());
        let short: String = host.split('.').next().unwrap_or(&host).chars().take(20).collect();
        let short = if short.is_empty() { "tuiui".to_string() } else { short };
        format!(" {short} \u{25be} ")
    }

    /// True when no non-minimized window's rect contains `p` (a click here falls
    /// through to the desktop).
    fn window_at_is_none(&self, p: Point) -> bool {
        !self.wm.z_ordered().iter().any(|w| !w.minimized && w.rect.contains(p))
    }

    /// Return the id of the topmost non-minimized window whose **titlebar row**
    /// contains `p`, provided `p` is NOT on a control button (min/max/close).
    /// Used by the double-click handler to start a rename.
    fn topmost_window_titlebar_at(&self, p: Point) -> Option<WindowId> {
        // z_ordered is bottom-to-top; last match is the topmost.
        self.wm
            .z_ordered()
            .iter()
            .filter(|w| {
                !w.minimized
                    && w.rect.y == p.y
                    && p.x >= w.rect.x
                    && p.x < w.rect.x + w.rect.w
                    && w.control_at(p).is_none()
            })
            .map(|w| w.id)
            .next_back()
    }

    /// The topmost non-minimized window whose **content area** contains `p`, with
    /// that content rect. `z_ordered` is bottom-to-top, so the last match is on top.
    fn topmost_window_content_at(&self, p: Point) -> Option<(WindowId, Rect)> {
        self.wm
            .z_ordered()
            .iter()
            .filter(|w| !w.minimized && w.content_rect().contains(p))
            .map(|w| (w.id, w.content_rect()))
            .next_back()
    }

    /// Fire any pending Updates-section action (Check / Install), from either
    /// the keyboard or the mouse path.
    fn drain_settings_action(&mut self) {
        match self.focused_settings_mut().and_then(|s| s.take_action()) {
            Some(crate::settings::SettingsAction::CheckUpdates) => {
                let branch = self.cfg.update_branch.clone();
                let msg = check_for_updates(&branch);
                crate::dbg_log(&format!(
                    "update: check (branch {branch}, running v{}) -> {msg}",
                    crate::VERSION
                ));
                if let Some(s) = self.focused_settings_mut() {
                    s.set_update_status(msg);
                }
            }
            Some(crate::settings::SettingsAction::RestartApphost) => {
                self.restart_apphost();
            }
            Some(crate::settings::SettingsAction::InstallUpdate) => {
                // Remember to reopen Settings on the Updates screen after the
                // reload, so the user isn't dumped back on the bare desktop.
                write_reopen_hint(Ui_SETTINGS_UPDATES);
                let cmd = update_command(&self.cfg.update_branch);
                crate::dbg_log(&format!(
                    "update: install requested (branch {}, running v{}, exe {:?}); running in a window",
                    self.cfg.update_branch,
                    crate::VERSION,
                    std::env::current_exe().ok()
                ));
                self.launch("update tuiui".into(), "sh".into(), vec!["-lc".into(), cmd]);
            }
            None => {}
        }
    }

    /// Called by the daemon after connecting to the apphost: if the running app
    /// server predates this binary's minimum-compatible protocol AND owns live
    /// apps, raise the safety dialog instead of silently misbehaving — the user
    /// gets a chance to save and close work before restarting the app server.
    pub fn check_apphost_compat(&mut self) {
        let proto = self.apphost.proto_version();
        let min = crate::apphost::proto::MIN_COMPAT;
        if needs_apphost_restart(proto, min, self.host_app_count()) {
            crate::dbg_log(&format!(
                "compat: apphost proto {proto} < required {min} with {} app(s) — raising safety dialog",
                self.host_app_count()
            ));
            self.compat_dialog = true;
            self.apphost_outdated = true;
        }
    }

    /// Restart the app server (CLOSES every running app — only ever offered via
    /// the safety dialog / Settings row, after the user was warned): stop the
    /// apphost and reload the frontend; the fresh daemon spawns a new-binary
    /// apphost.
    fn restart_apphost(&mut self) {
        crate::dbg_log("compat: user confirmed app-server restart (apps will close)");
        write_reopen_hint(Ui_SETTINGS_UPDATES);
        self.apphost.shutdown_host();
        self.compat_dialog = false;
        self.apphost_outdated = false;
        self.reload = true;
    }

    /// Whether the post-update compat dialog is showing (modal).
    pub fn compat_dialog_open(&self) -> bool {
        self.compat_dialog
    }

    /// Render the dock context menu (empty when closed). Uses the same
    /// geometry fns as the hit-testing so they can never drift.
    fn render_dock_ctx(&self) -> Vec<crate::compositor::Layer> {
        let Some((_, ax)) = self.dock_ctx else { return Vec::new() };
        let t = crate::theme::current();
        let d = dock_ctx_rect(ax, self.w, self.h);
        let mut buf = crate::buffer::CellBuffer::new(d.w, d.h);
        buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
        let b = |ch: char| crate::cell::Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
        for x in 0..d.w {
            buf.set(x, 0, b('\u{2500}'));
            buf.set(x, d.h - 1, b('\u{2500}'));
        }
        for y in 0..d.h {
            buf.set(0, y, b('\u{2502}'));
            buf.set(d.w - 1, y, b('\u{2502}'));
        }
        buf.set(0, 0, b('\u{256D}'));
        buf.set(d.w - 1, 0, b('\u{256E}'));
        buf.set(0, d.h - 1, b('\u{2570}'));
        buf.set(d.w - 1, d.h - 1, b('\u{256F}'));
        for (i, label) in DOCK_CTX_ROWS.iter().enumerate() {
            let fg = if i == 2 { t.close_fg } else { t.text };
            buf.write_str(1, 1 + i as i32, &format!(" {label}"), fg, t.window_bg);
        }
        vec![crate::compositor::Layer { z: 5300, origin: Point::new(d.x, d.y), buf, opacity: 1.0, scissor: None }]
    }

    /// Render the post-update safety dialog (empty when closed).
    fn render_compat_dialog(&self) -> Vec<crate::compositor::Layer> {
        if !self.compat_dialog {
            return Vec::new();
        }
        let t = crate::theme::current();
        let d = compat_dialog_rect(self.w, self.h);
        let mut buf = crate::buffer::CellBuffer::new(d.w, d.h);
        buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
        for x in 0..d.w {
            buf.set(x, 0, crate::cell::Cell { ch: ' ', fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
        }
        buf.write_str(2, 0, " tuiui update ", t.title_fg, t.title_focus);
        let b = |ch: char| crate::cell::Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
        for y in 1..d.h {
            buf.set(0, y, b('\u{2502}'));
            buf.set(d.w - 1, y, b('\u{2502}'));
        }
        for x in 0..d.w {
            buf.set(x, d.h - 1, b('\u{2500}'));
        }
        buf.set(0, d.h - 1, b('\u{2570}'));
        buf.set(d.w - 1, d.h - 1, b('\u{256F}'));
        let n = self.host_app_count();
        buf.write_str(2, 2, "This update needs to restart the app server.", t.text, t.window_bg);
        buf.write_str(2, 3, &format!("Your {n} running app(s) will CLOSE when it restarts."), t.close_fg, t.window_bg);
        buf.write_str(2, 5, "Save your work first, then restart - or keep apps", t.dim, t.window_bg);
        buf.write_str(2, 6, "running for now (restart later in Settings > Updates).", t.dim, t.window_bg);
        let (keep, restart) = compat_dialog_buttons(self.w, self.h);
        let keep_s = format!("{:^width$}", "Keep apps", width = keep.w as usize);
        buf.write_str(keep.x - d.x, keep.y - d.y, &keep_s, t.text, t.active_bg);
        let restart_s = format!("{:^width$}", "Restart app server", width = restart.w as usize);
        buf.write_str(restart.x - d.x, restart.y - d.y, &restart_s, t.close_fg, t.accent);
        vec![crate::compositor::Layer { z: 6500, origin: Point::new(d.x, d.y), buf, opacity: 1.0, scissor: None }]
    }

    /// On startup (fresh or post-reload), honour a one-shot reopen hint left by
    /// the in-app updater so Settings comes back on the Updates screen.
    pub fn reopen_ui_from_hint(&mut self) {
        if take_reopen_hint() == Some(Ui_SETTINGS_UPDATES) {
            self.open_settings();
            if let Some(s) = self.focused_settings_mut() {
                s.show_updates_section();
            }
            self.sync_settings();
        }
    }

    /// Scroll the scrollback of the PTY app window under `p`. No-op when the
    /// pointer isn't over an app window (native widgets handle their own scroll).
    fn scroll_app_at(&mut self, p: Point, lines: i32) {
        if let Some((id, _)) = self.topmost_window_content_at(p) {
            if let Some(WinContent::App(aid)) = self.contents.get(&id) {
                self.apphost.scroll(*aid, lines);
            }
        }
    }

    /// Persist the desktop's current icon positions to the config.
    fn persist_desktop_positions(&mut self) {
        self.cfg.desktop_positions = self.desktop.positions();
        let _ = self.cfg.save();
    }

    /// Open an image file in a new ImageView window.
    fn open_image(&mut self, path: String) {
        crate::dbg_log(&format!("open_image: {}", path));
        let expanded = expand_tilde(&path);
        // Bound the decode to a screen-sized image (assume an 8×16 px cell).
        let id = self.images.load(&expanded, (self.w.max(1) as u32) * 8, (self.h.max(1) as u32) * 16);
        let dims = id.and_then(|i| self.images.dimensions(i)).unwrap_or((0, 0));
        let w = 60.min((self.w - 4).max(20));
        let h = 24.min((self.h - 4).max(8));
        let rect = Rect::new((self.w - w) / 2, 2, w, h);
        let label = format!("image: {}", path.rsplit('/').next().unwrap_or(&path));
        let id_win = self.wm.add_window(label.clone(), rect);
        self.app_keys.insert(id_win, label.clone());
        self.contents.insert(id_win, WinContent::ImageView(crate::imageview::ImageView::new(path, id, dims)));
        self.titles.push((id_win, label));
    }

    /// Whether the working-directory picker overlay is open.
    pub fn dirpicker_open(&self) -> bool {
        self.dirpicker.is_some()
    }

    /// Whether `win`'s content rect is fully unobstructed by any higher window
    /// (used to decide if an image placement is visible).
    fn fully_unobstructed(&self, win: &crate::window::Window) -> bool {
        let cr = win.content_rect();
        !self.wm.z_ordered().iter().any(|o| {
            !o.minimized && o.z > win.z && o.rect.intersect(cr).is_some()
        })
    }

    /// PNG bytes for an image id currently held in the store (for the daemon's
    /// blob bookkeeping).
    pub fn image_png(&self, id: u64) -> Option<Vec<u8>> {
        self.images.png_bytes(id).map(|b| b.to_vec())
    }

    /// Load any not-yet-loaded PNGs that hosted apps have transmitted into the
    /// shared `ImageStore`, mapping each `(window, kitty_id)` to its store id.
    ///
    /// Called by the daemon once per frame, before [`build_frame`](Self::build_frame).
    /// `build_frame` itself is `&self` and only reads the resulting map, so the
    /// `&mut self` loading happens here.
    pub fn refresh_app_graphics(&mut self) {
        // First pass: collect the (window, kitty_id, png) we still need to load,
        // cloning the PNG bytes so the app's graphics lock is released before we
        // mutably borrow `self.images`.
        let mut needed: Vec<(WindowId, u32, Vec<u8>)> = Vec::new();
        for (id, content) in &self.contents {
            if let WinContent::App(aid) = content {
                for pl in self.apphost.placements(*aid) {
                    if self.app_image_ids.contains_key(&(*id, pl.image_id)) {
                        continue;
                    }
                    if let Some(png) = self.apphost.image_png(*aid, pl.image_id) {
                        needed.push((*id, pl.image_id, png));
                    }
                }
            }
        }
        // Second pass: load + record (no app graphics borrow held).
        let bound_w = (self.w.max(1) as u32) * 8;
        let bound_h = (self.h.max(1) as u32) * 16;
        for (win, kitty_id, png) in needed {
            if let Some(img) = self.images.load_bytes(&png, bound_w, bound_h) {
                self.app_image_ids.insert((win, kitty_id), img);
            }
        }
    }

    /// Push each app window's current geometry/state to the apphost as opaque
    /// `meta`, but only when it changed. Called once per frame by the daemon.
    /// For the in-process `LocalAppHost` this just updates a local map (cheap);
    /// for `RemoteAppHost` it sends `SetMeta` so a restarted frontend can restore.
    pub fn sync_app_meta(&mut self) {
        let mut updates: Vec<(AppId, WindowId, Vec<u8>)> = Vec::new();
        for w in self.wm.z_ordered() {
            if let Some(WinContent::App(aid)) = self.contents.get(&w.id) {
                let title = self
                    .titles
                    .iter()
                    .find(|(i, _)| *i == w.id)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_default();
                let app_key = self.app_keys.get(&w.id).cloned().unwrap_or_default();
                let meta = WinMeta { rect: w.rect, title, z: w.z, minimized: w.minimized, app_key };
                let bytes = serde_json::to_vec(&meta).unwrap_or_default();
                if self.last_meta.get(&w.id) != Some(&bytes) {
                    updates.push((*aid, w.id, bytes));
                }
            }
        }
        for (aid, win, bytes) in updates {
            self.apphost.set_meta(aid, bytes.clone());
            self.last_meta.insert(win, bytes);
        }
    }

    /// Number of entries in the last-meta cache (for tests).
    #[doc(hidden)]
    pub fn app_meta_count_for_test(&self) -> usize {
        self.last_meta.len()
    }

    /// Inject a placement + image directly into the most-recently-launched app's
    /// graphics state, then refresh the image map so a subsequent `build_frame`
    /// emits it (integration tests).
    #[doc(hidden)]
    pub fn inject_app_graphics_for_test(&mut self, png: &[u8]) {
        let app_id = self
            .titles
            .iter()
            .rev()
            .map(|(id, _)| *id)
            .find_map(|id| match self.contents.get(&id) {
                Some(WinContent::App(aid)) => Some(*aid),
                _ => None,
            });
        if let Some(aid) = app_id {
            self.apphost.inject_test_image(aid, png);
        }
        self.refresh_app_graphics();
    }

    /// Rebuild a window for every app the apphost already owns (after a frontend
    /// reload or crash). Returns the number of windows restored. Apps with no
    /// stored `meta` are skipped (they will still be reaped/closed normally if
    /// the user never had a window for them).
    pub fn restore_windows_from_host(&mut self) -> usize {
        // Snapshot ids first so we don't borrow the host across mutations.
        let ids: Vec<AppId> = self.apphost.list();
        let mut restored = 0;
        for aid in ids {
            // Skip if we already have a window bound to this app.
            if self.contents.values().any(|c| matches!(c, WinContent::App(a) if *a == aid)) {
                continue;
            }
            let Some(bytes) = self.apphost.meta(aid) else { continue };
            let Ok(meta) = serde_json::from_slice::<WinMeta>(&bytes) else { continue };
            let id = self.wm.add_window(meta.title.clone(), meta.rect);
            if meta.minimized {
                self.wm.minimize(id);
            }
            // Use stored app_key, falling back to title for old meta blobs.
            let key = if meta.app_key.is_empty() { meta.title.clone() } else { meta.app_key.clone() };
            self.app_keys.insert(id, key);
            self.contents.insert(id, WinContent::App(aid));
            self.titles.push((id, meta.title));
            self.last_meta.insert(id, bytes);
            restored += 1;
        }
        if restored > 0 {
            self.auto_tile_if_enabled();
        }
        restored
    }

    /// Drop all window bookkeeping while leaving the apphost's apps alive — used
    /// to simulate a fresh frontend in restore tests.
    #[doc(hidden)]
    pub fn forget_windows_for_test(&mut self) {
        let ids: Vec<WindowId> = self.contents.keys().copied().collect();
        for id in ids {
            self.wm.close(id);
        }
        self.contents.clear();
        self.titles.clear();
        self.app_keys.clear();
        self.last_meta.clear();
    }

    /// Whether the picker's new-folder name input is active.
    pub fn dirpicker_creating(&self) -> bool {
        self.dirpicker.as_ref().map(|d| d.is_creating()).unwrap_or(false)
    }

    /// Whether the keyboard-shortcut help overlay is showing.
    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// Whether the full-screen "simple" view mode is active.
    pub fn simple_mode(&self) -> bool { self.simple }

    /// The work-area rect a full-screen app fills in simple mode (between the
    /// top menubar row and the bottom dock row).
    fn simple_content_rect(&self) -> crate::geometry::Rect {
        crate::geometry::Rect::new(0, 1, self.w.max(1), (self.h - 2).max(1))
    }

    /// Toggle between desktop and simple view. Resizes the focused app so it
    /// fills the screen (entering simple) or returns to its window size (leaving).
    pub fn toggle_simple(&mut self) {
        self.simple = !self.simple;
        // Re-sync every app window: in simple mode only the focused one becomes
        // full-screen (handled by sync_app_size); the rest go to their window size.
        let ids: Vec<WindowId> = self.contents.keys().copied().collect();
        for id in ids {
            self.sync_app_size(id);
        }
    }

    /// The current tray segments, laid out from the live snapshot.
    fn tray_segments_now(&self) -> Vec<crate::tray::Segment> {
        let st = self.tray_state.read().unwrap();
        crate::tray::tray_segments(&st, self.w, self.power_label.chars().count() as i32, self.tray.notif_count())
    }

    /// Apply a tray control intent: optimistically update the cached snapshot so
    /// the UI responds immediately, then run the (timeout-guarded) backend call.
    fn apply_intent(&mut self, intent: crate::system::ControlIntent) {
        use crate::system::ControlIntent as I;
        // Calendar navigation and notifications are session/tray UI state, not
        // OS controls.
        match intent {
            I::CalendarPrev => return self.tray.calendar_step(-1),
            I::CalendarNext => return self.tray.calendar_step(1),
            I::NotifFocus(raw) => {
                let id = WindowId(raw);
                self.tray.clear_notifs_for(raw);
                self.tray.close();
                if self.contents.contains_key(&id) {
                    self.wm.unminimize(id);
                    if self.simple { self.sync_app_size(id); }
                }
                return;
            }
            I::NotifClear => {
                self.tray.clear_notifs();
                self.tray.close();
                return;
            }
            _ => {}
        }
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

    /// Whether a frontend-only reload was requested (apps stay alive).
    pub fn reload_requested(&self) -> bool { self.reload }

    /// Number of apps the host currently owns (regardless of whether the
    /// frontend has a window for each). The daemon uses this to decide whether
    /// this is a fresh start (auto-launch configured apps) or a reload/recovery
    /// against an apphost that already has apps (restore, don't re-launch).
    pub fn host_app_count(&self) -> usize {
        self.apphost.list().len()
    }

    /// Whether every app window has its grid available yet. After a reload the
    /// daemon waits on this so the first painted frame already shows app content
    /// (the apphost streams frames asynchronously into the host cache, so a
    /// freshly-restored window is briefly blank until its first frame lands).
    /// `true` when there are no app windows.
    pub fn app_windows_ready(&self) -> bool {
        self.contents.values().all(|c| match c {
            WinContent::App(aid) => self.apphost.snapshot(*aid).is_some(),
            _ => true,
        })
    }

    /// Clear the detach (quit) flag — called by the daemon after a client detaches
    /// so the next client doesn't immediately detach again.
    pub fn clear_quit(&mut self) {
        self.quit = false;
        self.switch_to = None;
    }

    /// The pending system switch, if the user picked one in the Systems menu.
    /// Shipped to the client in the frame that also carries the detach flag.
    pub fn switch_spec(&self) -> Option<crate::systems::SwitchSpec> {
        self.switch_to.clone()
    }

    /// Whether the power menu's Add Remote form is open (the client forwards
    /// typed characters / field navigation to it).
    pub fn power_form_editing(&self) -> bool {
        self.power_menu.form_open()
    }

    /// Carry out a confirmed power-menu outcome (button click or form submit).
    fn apply_power_outcome(&mut self, outcome: PowerOutcome) {
        match outcome {
            PowerOutcome::Detach => self.quit = true,
            PowerOutcome::Reload => self.reload = true,
            PowerOutcome::Shutdown => self.shutdown = true,
            PowerOutcome::Switch(spec) => {
                crate::dbg_log(&format!("systems: switch to '{}' ({})", spec.name, spec.host));
                self.switch_to = Some(spec);
                self.quit = true; // detach locally; apps keep running
            }
            PowerOutcome::AddAndConnect { system, password } => {
                let mut spec = crate::systems::SwitchSpec::connect(&system);
                spec.setup = true;
                spec.password = password; // setup-only; never persisted
                crate::dbg_log(&format!(
                    "systems: add '{}' ({}) theme={:?} → saving + first-time setup",
                    system.name, system.host, system.theme
                ));
                self.systems.retain(|s| s.name != system.name);
                self.systems.push(system);
                if let Err(e) = crate::systems::save(&self.systems) {
                    crate::dbg_log(&format!("systems: save failed: {e}"));
                }
                self.switch_to = Some(spec);
                self.quit = true;
            }
            PowerOutcome::Forget { name, revoke } => {
                crate::dbg_log(&format!("systems: forget '{name}' (revoke={})", revoke.is_some()));
                self.systems.retain(|s| s.name != name);
                if let Err(e) = crate::systems::save(&self.systems) {
                    crate::dbg_log(&format!("systems: save failed: {e}"));
                }
                // Optionally strip our key from the remote — best-effort, on a
                // background thread so an unreachable host can't stall the UI.
                if let Some(sys) = revoke {
                    Self::revoke_remote_key(sys);
                }
            }
        }
    }

    /// Strip this machine's SSH key from a forgotten system's remote
    /// `authorized_keys` (the inverse of the key copy that adding it performed).
    /// Runs on a detached thread: ssh can take seconds or hang on an unreachable
    /// host, and the render loop must never wait on the network. Best-effort —
    /// every outcome is logged for the in-app Logs viewer, nothing is surfaced.
    fn revoke_remote_key(sys: crate::systems::RemoteSystem) {
        std::thread::spawn(move || {
            crate::dbg_log(&format!("systems: revoking key on '{}' ({})", sys.name, sys.host));
            let script = crate::systems::revoke_script(&sys.host, sys.port);
            match crate::system::run_capped("sh", &["-c", &script], 15) {
                Some(out) => {
                    let tail = out.trim();
                    crate::dbg_log(&format!(
                        "systems: revoke on '{}' done{}",
                        sys.name,
                        if tail.is_empty() { String::new() } else { format!(" — {tail}") }
                    ));
                }
                None => crate::dbg_log(&format!(
                    "systems: revoke on '{}' did not complete (host unreachable, auth refused, \
or a remote-side error — its authorized_keys was left untouched)",
                    sys.name
                )),
            }
        });
    }

    /// Build the launcher's app list: the configured apps (with categories filled
    /// in from the catalog where missing), plus any known TUIs detected on `$PATH`
    /// that aren't already listed.
    fn build_launcher_apps(cfg: &Config, systems: &[crate::systems::RemoteSystem]) -> Vec<AppEntry> {
        // Pinned tuiui actions first (open the store / settings windows).
        let mut apps = vec![
            AppEntry { name: "Store".into(), command: "@store".into(), args: vec![], category: Some("tuiui".into()), requires_cwd: None, cwd: None, cli: None, warn: None },
            AppEntry { name: "Settings".into(), command: "@settings".into(), args: vec![], category: Some("tuiui".into()), requires_cwd: None, cwd: None, cli: None, warn: None },
            AppEntry { name: "Files".into(), command: "@files".into(), args: vec![], category: Some("tuiui".into()), requires_cwd: None, cwd: None, cli: None, warn: None },
            AppEntry { name: "Logs".into(), command: "@logs".into(), args: vec![], category: Some("tuiui".into()), requires_cwd: None, cwd: None, cli: None, warn: None },
            AppEntry { name: "Activity".into(), command: "@activity".into(), args: vec![], category: Some("tuiui".into()), requires_cwd: None, cwd: None, cli: None, warn: None },
        ];
        // One remote file browser per saved system (Systems category).
        for sys in systems {
            apps.push(AppEntry {
                name: format!("Files on {}", sys.name),
                command: "@rfiles".into(),
                args: vec![sys.name.clone()],
                category: Some("Systems".into()),
                requires_cwd: None,
                cwd: None,
                cli: None,
                warn: None,
            });
        }
        apps.extend(cfg.launcher_apps());
        for a in &mut apps {
            if a.category.is_none() {
                a.category = crate::catalog::category_for(&a.name)
                    .or_else(|| crate::catalog::category_for(&a.command));
            }
            // Backfill the cwd-prompt flag from the catalog (by name, then binary)
            // when a config entry doesn't set it explicitly, so a known cwd-app
            // gets its working-directory picker without the user remembering the flag.
            if a.requires_cwd.is_none() {
                a.requires_cwd = crate::catalog::requires_cwd_for(&a.name)
                    .or_else(|| crate::catalog::requires_cwd_for(&a.command));
            }
            // Backfill the CLI flag from the catalog the same way, so a config
            // entry that just names a known CLI tool still gets the badge and
            // the `sh -lc` launch wrapper without repeating the flag by hand.
            if a.cli.is_none() {
                a.cli = Some(
                    crate::catalog::is_cli(&a.name) || crate::catalog::is_cli(&a.command),
                );
            }
        }
        for detected in crate::catalog::detect_installed() {
            if !apps.iter().any(|a| a.name.eq_ignore_ascii_case(&detected.name)) {
                apps.push(detected);
            }
        }
        apps
    }

    /// Re-scan `$PATH` and rebuild the launcher's app list when it is about to
    /// open, so a newly-installed app appears without a daemon restart.
    fn refresh_launcher_if_closed(&mut self) {
        if !self.launcher.is_open() {
            crate::catalog::refresh_installed();
            self.launcher.set_items(Self::build_launcher_apps(&self.cfg, &self.systems));
        }
    }

    /// Whether the launcher (menu or Spotlight) is currently open.
    pub fn launcher_open(&self) -> bool {
        self.launcher.is_open()
    }

    /// Whether the launcher is open (integration tests).
    #[doc(hidden)]
    pub fn launcher_open_for_test(&self) -> bool {
        self.launcher.is_open()
    }

    /// Whether the Spotlight overlay specifically is open (the loop routes typed
    /// characters to the query only in this mode).
    pub fn spotlight_open(&self) -> bool {
        self.launcher.mode() == Some(crate::launcher::LauncherMode::Spotlight)
    }

    /// Whether the user has requested quit (Exit from the power menu).
    /// The render loop polls this each tick and exits when it returns `true`.
    pub fn quit_requested(&self) -> bool { self.quit }

    /// Whether the top-right power menu (dropdown or confirm dialog) is open.
    pub fn power_menu_open(&self) -> bool { self.power_menu.is_open() }

    /// Whether the confirm-close dialog is showing (the client routes Enter/Esc
    /// and y/n to it while open).
    pub fn confirm_close_open(&self) -> bool { self.confirm_close.is_open() }

    /// Whether the launch-warning dialog is showing (the client routes
    /// Enter/Esc and y/n to it while open).
    pub fn launch_warn_open(&self) -> bool { self.launch_warn.is_open() }

    /// Return the number of live windows (app instances spawned successfully).
    pub fn window_count(&self) -> usize { self.contents.len() }

    /// Return the currently focused [`WindowId`], if any.
    pub fn focused(&self) -> Option<WindowId> { self.wm.focused() }

    /// Test helper: the focused window's outer rect (titlebar + borders).
    #[doc(hidden)]
    pub fn focused_window_rect_for_test(&self) -> Option<crate::geometry::Rect> {
        self.wm.focused().and_then(|id| self.wm.get(id)).map(|w| w.rect)
    }

    /// Return the screen-space hit regions for every dock pill.
    ///
    /// Each tuple is `(pill_index, Rect)` where the rect is a 1-row slice on the
    /// bottom screen row.  Used by callers that need to detect dock clicks
    /// without going through the full mouse-routing path.
    pub fn dock_regions(&self) -> Vec<(usize, Rect)> {
        let items = self.dock_items();
        dock_hit_regions(self.w, self.h, &items)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Build the current list of dock pills, grouping windows by app_key.
    fn dock_items(&self) -> Vec<DockItem> {
        let focused = self.wm.focused();
        let mut order: Vec<String> = Vec::new();
        let mut groups: std::collections::HashMap<String, Vec<WindowId>> = std::collections::HashMap::new();
        for (id, _) in &self.titles {
            let key = self.app_keys.get(id).cloned().unwrap_or_else(|| {
                self.titles.iter().find(|(i, _)| i == id).map(|(_, t)| t.clone()).unwrap_or_default()
            });
            if !groups.contains_key(&key) { order.push(key.clone()); }
            groups.entry(key).or_default().push(*id);
        }
        order.into_iter().map(|key| {
            let wins = groups.remove(&key).unwrap_or_default();
            let badge = crate::badge::badge_for(&key, &self.cfg.dock_badges);
            let is_focused = wins.iter().any(|w| Some(*w) == focused);
            let attention = wins.iter().any(|w| self.tray.has_notif_for(w.0));
            if wins.len() == 1 {
                let id = wins[0];
                let label = self.titles.iter().find(|(i, _)| *i == id).map(|(_, t)| t.clone()).unwrap_or_else(|| key.clone());
                DockItem { kind: DockKind::Single(id), label, count: 1, badge_letter: badge.letter, badge_color: badge.color, focused: is_focused, attention }
            } else {
                let count = wins.len();
                DockItem { kind: DockKind::Group(key.clone(), wins), label: key, count, badge_letter: badge.letter, badge_color: badge.color, focused: is_focused, attention }
            }
        }).collect()
    }

    /// Test helper: number of dock pills (after grouping).
    #[doc(hidden)]
    pub fn dock_pill_count_for_test(&self) -> usize {
        self.dock_items().len()
    }

    /// Test helper: the full dock-item list (after grouping).
    #[doc(hidden)]
    pub fn dock_items_for_test(&self) -> Vec<DockItem> {
        self.dock_items()
    }

    /// Test helper: the focused window's current dock/title label.
    #[doc(hidden)]
    pub fn focused_label_for_test(&self) -> String {
        self.wm
            .focused()
            .and_then(|id| self.titles.iter().find(|(i, _)| *i == id))
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    }

    /// Test helper: the focused window's actual spawned command + args, as
    /// recorded by the (in-process) apphost — lets launch tests assert on the
    /// real `sh -lc …` wrapper rewrite for CLI-flagged apps.
    #[doc(hidden)]
    pub fn focused_app_launch_cmd_for_test(&self) -> Option<(String, Vec<String>)> {
        let id = self.wm.focused()?;
        match self.contents.get(&id)? {
            WinContent::App(aid) => Some(self.apphost_launch_cmd(*aid)),
            _ => None,
        }
    }

    /// Collect the rows for the open dock-group popup (badge, label per window).
    fn dock_popup_rows(&self) -> Vec<(WindowId, char, crate::cell::Rgba, String)> {
        let Some(ref key) = self.dock_popup else { return Vec::new() };
        let focused = self.wm.focused();
        let badge = crate::badge::badge_for(key, &self.cfg.dock_badges);
        self.titles
            .iter()
            .filter(|(id, _)| self.app_keys.get(id).map(|k| k == key).unwrap_or(false))
            .map(|(id, label)| {
                let _ = focused;
                (*id, badge.letter, badge.color, label.clone())
            })
            .collect()
    }

    /// Compute the popup position and call `render_dock_popup`, returning layers
    /// + row rects for hit-testing. Used in both `handle_mouse` and `build_frame`.
    fn render_popup_for_hit_test(
        &self,
        rows: &[(WindowId, char, crate::cell::Rgba, String)],
    ) -> (Vec<crate::compositor::Layer>, Vec<(WindowId, Rect)>) {
        let Some(ref key) = self.dock_popup else {
            return (Vec::new(), Vec::new());
        };
        // Find the pill's x position so the popup anchors to it
        let items = self.dock_items();
        let regions = dock_hit_regions(self.w, self.h, &items);
        let pill_x = items
            .iter()
            .enumerate()
            .find(|(_, it)| match &it.kind {
                DockKind::Group(k, _) => k == key,
                DockKind::Single(_) => false,
            })
            .and_then(|(i, it)| {
                regions.iter().find(|(idx, _)| *idx == i).map(|(_, r)| (r.x, it.count))
            });
        let (px, pill_w) = pill_x.map(|(x, c)| (x, c as i32)).unwrap_or((0, 2));
        let focused = self.wm.focused();
        crate::chrome::render_dock_popup(self.w, self.h, px, pill_w, rows, focused)
    }

    // ── Public apply ─────────────────────────────────────────────────────────

    /// Apply a single client message, mutating internal state.
    ///
    /// This is the **only** way external code drives the session.  The method
    /// dispatches to private sub-handlers so that none of the internal
    /// machinery (WM, PTY handles, drag state) leaks through the public API.
    pub fn apply(&mut self, msg: ClientMsg) {
        if !matches!(
            msg,
            ClientMsg::MouseDown(_)
                | ClientMsg::MouseUp(_)
                | ClientMsg::MouseDrag(_)
                | ClientMsg::MouseInput(_)
                | ClientMsg::Resize { .. }
                | ClientMsg::Key(_)
                | ClientMsg::RenameChar(_)
                | ClientMsg::RenameBackspace
        ) {
            crate::dbg_log(&format!("apply {:?}", msg));
        }
        match msg {
            ClientMsg::Launch { name, command, args } => {
                // The `tuiui launch` escape hatch (CLI / assistant / dock pins).
                // A bare launch of a catalog-flagged CLI tool would open a window
                // that prints usage and instantly dies, so give it the same
                // help-then-shell wrapper as the launcher menu. Explicit args mean
                // an intentional invocation (`tuiui launch gum choose a b`) —
                // run those as given.
                let (command, args) = if args.is_empty()
                    && (crate::catalog::is_cli(&command) || crate::catalog::is_cli(&name))
                {
                    cli_wrap(&command, &args)
                } else {
                    (command, args)
                };
                self.launch(name, command, args);
            }
            ClientMsg::MouseDown(p) => {
                self.cursor = p;
                self.handle_mouse(MouseKind::Down, p);
            }
            ClientMsg::MouseDrag(p) => {
                // While dragging a window, drop a "teleport" report — a motion
                // event that jumps more than half the screen vertically in one
                // step. You can't teleport mid-drag, so this is a spurious/garbage
                // terminal mouse report that would fling the window off-screen.
                // (Only applied during an active drag — a plain click on a bottom-
                // row control like the dock "+" must always go through.)
                if self.drag.is_some() && self.is_spurious_jump(p) {
                    return; // keep the last good cursor; ignore this event
                }
                self.cursor = p;
                self.handle_mouse(MouseKind::Drag, p);
            }
            ClientMsg::MouseUp(p) => {
                // End a drag at the last good position if the release coordinate
                // is itself a spurious teleport (don't snap the window to it).
                let p = if self.drag.is_some() && self.is_spurious_jump(p) { self.cursor } else { p };
                self.cursor = p;
                self.handle_mouse(MouseKind::Up, p);
            }
            ClientMsg::Key(bytes) => {
                if let Some(id) = self.wm.focused() {
                    if let Some(WinContent::App(aid)) = self.contents.get(&id) {
                        let aid = *aid;
                        self.apphost.input(aid, &bytes);
                    }
                }
            }
            ClientMsg::Resize { w, h } => {
                self.w = w;
                self.h = h;
                self.wm.set_work_area(Rect::new(0, 1, w, h - 2));
                // Re-fit the desktop grid so icons stay anchored top-right.
                self.desktop.layout(w, h);
                // Re-fit any maximized window and its app to the new work area.
                if let Some(id) = self.wm.focused() {
                    self.sync_app_size(id);
                }
                self.auto_tile_if_enabled();
                if self.simple {
                    if let Some(fid) = self.wm.focused() {
                        self.sync_app_size(fid);
                    }
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
            ClientMsg::ToggleMenu => {
                self.refresh_launcher_if_closed();
                self.launcher.toggle_menu();
            }
            ClientMsg::ToggleSpotlight => {
                self.refresh_launcher_if_closed();
                self.launcher.toggle_spotlight();
            }
            ClientMsg::LauncherChar(c) => self.launcher.type_char(c),
            ClientMsg::LauncherBackspace => self.launcher.backspace(),
            ClientMsg::LauncherUp => self.launcher.move_up(),
            ClientMsg::LauncherDown => self.launcher.move_down(),
            ClientMsg::LauncherLeft => self.launcher.collapse(),
            ClientMsg::LauncherRight => self.launcher.expand(),
            ClientMsg::LauncherEnter => {
                match self.launcher.mode() {
                    Some(crate::launcher::LauncherMode::Menu) => {
                        if let Some(app) = self.launcher.activate() {
                            self.launcher.close();
                            self.launch_entry(app);
                        }
                    }
                    Some(crate::launcher::LauncherMode::Spotlight) => {
                        if let Some(app) = self.launcher.selected_entry() {
                            self.launcher.close();
                            self.launch_entry(app);
                        }
                    }
                    None => {}
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
                self.drain_settings_action();
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
            ClientMsg::OpenFileManager => self.open_filemanager(),
            ClientMsg::FileManagerUp => {
                if let Some(f) = self.focused_filemanager_mut() {
                    // While the context menu is open, Up/Down move its
                    // highlight instead of the file cursor underneath.
                    if f.context_menu_open() { f.context_menu_up(); } else { f.move_cursor(0, -1); }
                }
            }
            ClientMsg::FileManagerDown => {
                if let Some(f) = self.focused_filemanager_mut() {
                    if f.context_menu_open() { f.context_menu_down(); } else { f.move_cursor(0, 1); }
                }
            }
            ClientMsg::FileManagerLeft => { if let Some(f) = self.focused_filemanager_mut() { f.move_cursor(-1, 0); } }
            ClientMsg::FileManagerRight => { if let Some(f) = self.focused_filemanager_mut() { f.move_cursor(1, 0); } }
            ClientMsg::FileManagerActivate => {
                if let Some(f) = self.focused_filemanager_mut() { f.activate(); }
                self.drain_fm_action();
            }
            ClientMsg::FileManagerBack => { if let Some(f) = self.focused_filemanager_mut() { f.go_back(); } }
            ClientMsg::FileManagerParent => { if let Some(f) = self.focused_filemanager_mut() { f.go_parent(); } }
            ClientMsg::FileManagerToggleView => {
                if let Some(f) = self.focused_filemanager_mut() { f.cycle_view(); }
            }
            ClientMsg::FileManagerViewIcon => {
                if let Some(f) = self.focused_filemanager_mut() {
                    f.set_view(crate::filemanager::ViewMode::Icon);
                }
            }
            ClientMsg::FileManagerViewList => {
                if let Some(f) = self.focused_filemanager_mut() {
                    f.set_view(crate::filemanager::ViewMode::List);
                }
            }
            ClientMsg::FileManagerViewColumns => {
                if let Some(f) = self.focused_filemanager_mut() {
                    f.set_view(crate::filemanager::ViewMode::Columns);
                }
            }
            ClientMsg::FileManagerTogglePreview => {
                if let Some(f) = self.focused_filemanager_mut() { f.toggle_preview(); }
            }
            ClientMsg::FileManagerToggleHidden => { if let Some(f) = self.focused_filemanager_mut() { f.toggle_hidden(); } }
            ClientMsg::FileManagerNewFolder => { if let Some(f) = self.focused_filemanager_mut() { f.begin_new_folder(); } }
            ClientMsg::FileManagerRename => { if let Some(f) = self.focused_filemanager_mut() { f.begin_rename(); } }
            ClientMsg::FileManagerDelete => { if let Some(f) = self.focused_filemanager_mut() { f.begin_delete(); } }
            ClientMsg::FileManagerCopy => {
                // Record the copy in the session-level transfer clipboard too, so
                // pasting into a window on a DIFFERENT system scp's the files.
                let backend = self.focused_fm_backend();
                if let Some(f) = self.focused_filemanager_mut() {
                    f.copy_selection();
                    let paths = f.selected_paths();
                    if !paths.is_empty() {
                        self.transfer = Some((backend, paths));
                    }
                }
            }
            ClientMsg::FileManagerCut => { if let Some(f) = self.focused_filemanager_mut() { f.cut_selection(); } }
            ClientMsg::FileManagerPaste => self.fm_paste(),
            ClientMsg::FileManagerChar(c) => { if let Some(f) = self.focused_filemanager_mut() { f.overlay_char(c); } }
            ClientMsg::FileManagerBackspace => { if let Some(f) = self.focused_filemanager_mut() { f.overlay_backspace(); } }
            ClientMsg::FileManagerCommit => {
                // Commit an edit overlay, a delete confirmation, or (Enter on
                // an open context menu) the highlighted menu action.
                if let Some(f) = self.focused_filemanager_mut() {
                    match f.overlay() {
                        Some(crate::filemanager::Overlay::ConfirmDelete { .. }) => f.confirm_delete(),
                        Some(crate::filemanager::Overlay::Context { .. }) => f.context_menu_commit(),
                        _ => f.overlay_commit(),
                    }
                }
                // The "Open" menu action can queue an OpenImage/RunApp action,
                // same as FileManagerActivate.
                self.drain_fm_action();
            }
            ClientMsg::FileManagerCancel => { if let Some(f) = self.focused_filemanager_mut() { f.cancel_overlay(); } }
            ClientMsg::FileManagerClose => {
                if let Some(id) = self.wm.focused() {
                    if matches!(self.contents.get(&id), Some(WinContent::FileManager(_))) {
                        self.close(id);
                    }
                }
            }
            ClientMsg::FileManagerNewTab => { if let Some(f) = self.focused_filemanager_mut() { f.new_tab(); } }
            ClientMsg::FileManagerCloseTab => { if let Some(f) = self.focused_filemanager_mut() { f.close_tab(); } }
            ClientMsg::FileManagerNextTab => { if let Some(f) = self.focused_filemanager_mut() { f.next_tab(); } }
            ClientMsg::OpenActivity => self.open_activity(),
            ClientMsg::ActivityUp => { if let Some(a) = self.focused_activity_mut() { a.move_up(); } }
            ClientMsg::ActivityDown => { if let Some(a) = self.focused_activity_mut() { a.move_down(); } }
            ClientMsg::ActivityKill => {
                if let Some(a) = self.focused_activity_mut() {
                    if let Some(app_id) = a.request_kill_selected() {
                        self.apphost.kill(AppId(app_id));
                    }
                }
            }
            ClientMsg::ActivityConfirmKill => {
                if let Some(a) = self.focused_activity_mut() {
                    if let Some(app_id) = a.request_kill_selected() {
                        self.apphost.kill(AppId(app_id));
                    }
                }
            }
            ClientMsg::ActivityCancelKill => {
                if let Some(a) = self.focused_activity_mut() { a.cancel_kill(); }
            }
            ClientMsg::ActivityKillDead => {
                if let Some(a) = self.focused_activity_mut() {
                    for app_id in a.kill_dead() {
                        self.apphost.kill(AppId(app_id));
                    }
                }
            }
            ClientMsg::ActivityRefresh => self.refresh_activity(),
            ClientMsg::ActivityClose => {
                if let Some(id) = self.activity_win.take() {
                    if matches!(self.contents.get(&id), Some(WinContent::Activity(_))) {
                        self.close(id);
                    }
                }
            }
            ClientMsg::ToggleHelp => self.help_open = !self.help_open,
            ClientMsg::OpenImage(p) => self.open_image(p),
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
            ClientMsg::MouseRightDown(p) => {
                self.cursor = p;
                // Right-clicking a dock pill opens its context menu (minimise /
                // maximise / close / reset size). Groups target the focused
                // window of the group, else the first. Only when no other
                // menu/modal is already showing: its click-capture block runs
                // before the other modal routing, so opening it underneath a
                // higher-z overlay would let a hidden menu silently eat the
                // next click. Same overlay set as `app_mouse_area`.
                let overlay_open = self.launcher.is_open()
                    || self.help_open
                    || self.dirpicker.is_some()
                    || self.power_menu.is_open()
                    || self.confirm_close.is_open()
                    || self.launch_warn.is_open()
                    || self.compat_dialog
                    || self.rename.is_some()
                    || self.tray.open().is_some()
                    || self.desktop.overlay_rect().is_some();
                if !overlay_open && p.y == self.h - 1 {
                    let items = self.dock_items();
                    let hit = crate::chrome::dock_hit_regions(self.w, self.h, &items)
                        .into_iter()
                        .find(|(_, r)| r.contains(p));
                    if let Some((idx, r)) = hit {
                        let focused = self.wm.focused();
                        let target = match &items[idx].kind {
                            DockKind::Single(id) => Some(*id),
                            DockKind::Group(_, wins) => wins
                                .iter()
                                .copied()
                                .find(|w| Some(*w) == focused)
                                .or_else(|| wins.first().copied()),
                        };
                        if let Some(id) = target {
                            self.dock_popup = None;
                            self.dock_ctx = Some((id, r.x));
                        }
                        return;
                    }
                }
                // Right-clicking inside a file-manager window's content area
                // opens its context menu (rename / delete / …) on the entry
                // under the cursor. Same overlay guard as the dock branch
                // above; a non-entry hit (toolbar/sidebar/empty area) does
                // nothing (v1) rather than falling through to the desktop.
                if !overlay_open {
                    if let Some((id, cr)) = self.topmost_window_content_at(p) {
                        if matches!(self.contents.get(&id), Some(WinContent::FileManager(_))) {
                            self.wm.raise(id);
                            let local = Point::new(p.x - cr.x, p.y - cr.y);
                            if let Some(WinContent::FileManager(f)) = self.contents.get_mut(&id) {
                                if let Some(crate::filemanager::Target::Entry(i)) = f.hit_test(local, cr.w, cr.h) {
                                    // Anchor the menu at the exact click point (mirrors the
                                    // desktop's right_click), not the tile origin.
                                    f.begin_context_at(i, local);
                                }
                            }
                            return;
                        }
                    }
                }
                self.handle_desktop_right(p);
            }
            ClientMsg::MouseDouble(p) => {
                self.cursor = p;
                // If the launcher is open, route the double-click to it (a second
                // fast click on the brand dismisses the menu rather than falling
                // through to titlebar/desktop handling — and must not launch the
                // selected row).
                if self.launcher.is_open() {
                    if let Some(app) = self.launcher.click(p) {
                        self.launcher.close();
                        self.launch_entry(app);
                    } else {
                        self.launcher.close();
                    }
                    return;
                }
                // Double-click on a window's titlebar (not on a control button)
                // starts a rename of that window. Check this before desktop/content.
                if let Some(id) = self.topmost_window_titlebar_at(p) {
                    // Start with an empty buffer — type the new name fresh; an
                    // empty commit (or Esc) keeps the current name.
                    self.rename = Some((id, String::new()));
                } else if self.cfg.desktop_enabled && self.window_at_is_none(p) {
                    self.desktop.double_click(p);
                    self.drain_desktop_action();
                } else if let Some((id, cr)) = self.topmost_window_content_at(p) {
                    // Double-click inside a file-manager window opens the entry
                    // under the cursor (navigate into a folder / open a file).
                    let local = Point::new(p.x - cr.x, p.y - cr.y);
                    let mut acted = false;
                    if let Some(WinContent::FileManager(f)) = self.contents.get_mut(&id) {
                        acted = f.double_click(local, cr.w, cr.h);
                    }
                    if acted {
                        self.drain_fm_action();
                    }
                }
            }
            ClientMsg::DesktopChar(c) => self.desktop.overlay_char(c),
            ClientMsg::DesktopBackspace => self.desktop.overlay_backspace(),
            ClientMsg::DesktopCommit => self.desktop_commit(),
            ClientMsg::DesktopCancel => self.desktop.cancel_overlay(),
            ClientMsg::RenameFocused => {
                if let Some(id) = self.wm.focused() {
                    self.rename = Some((id, String::new()));
                }
            }
            ClientMsg::RenameChar(c) => {
                if !c.is_control() {
                    if let Some((_, buf)) = &mut self.rename {
                        buf.push(c);
                    }
                }
            }
            ClientMsg::RenameBackspace => {
                if let Some((_, buf)) = &mut self.rename {
                    buf.pop();
                }
            }
            ClientMsg::RenameCommit => {
                if let Some((id, buf)) = self.rename.take() {
                    if !buf.is_empty() {
                        // Update the dock label.
                        if let Some((_, label)) = self.titles.iter_mut().find(|(i, _)| *i == id) {
                            *label = buf.clone();
                        }
                        // Update the window titlebar title.
                        self.wm.rename_window(id, buf);
                    }
                }
            }
            ClientMsg::RenameCancel => {
                self.rename = None;
            }
            ClientMsg::ConfirmCloseYes => {
                if let Some(id) = self.confirm_close.confirm() {
                    self.close(id);
                }
            }
            ClientMsg::ConfirmCloseNo => self.confirm_close.cancel(),
            ClientMsg::LaunchWarnYes => {
                if let Some(l) = self.launch_warn.confirm() {
                    self.launch_resolved(l.name, l.command, l.args, l.cli, l.requires_cwd, l.cwd);
                }
            }
            ClientMsg::LaunchWarnNo => self.launch_warn.cancel(),
            ClientMsg::Shutdown => self.shutdown = true,
            ClientMsg::Reload => self.reload = true,
            ClientMsg::MouseInput(m) => self.forward_mouse_to_app(m),
            ClientMsg::ScrollAt { p, lines } => self.scroll_app_at(p, lines),
            ClientMsg::PowerFormChar(c) => self.power_menu.form_char(c),
            ClientMsg::PowerFormBackspace => self.power_menu.form_backspace(),
            ClientMsg::PowerFormNext => self.power_menu.form_next(),
            ClientMsg::PowerFormPrev => self.power_menu.form_prev(),
            ClientMsg::PowerFormLeft => self.power_menu.form_left(),
            ClientMsg::PowerFormRight => self.power_menu.form_right(),
            ClientMsg::PowerFormCommit => {
                if let PowerClick::Act(outcome) = self.power_menu.form_commit() {
                    self.apply_power_outcome(outcome);
                }
            }
            ClientMsg::PowerFormCancel => self.power_menu.form_cancel(),
            ClientMsg::LogsUp => { if let Some(l) = self.focused_logs_mut() { l.scroll_by(-1); } }
            ClientMsg::LogsDown => { if let Some(l) = self.focused_logs_mut() { l.scroll_by(1); } }
            ClientMsg::LogsPageUp => {
                if let Some(l) = self.focused_logs_mut() {
                    let n = l.page_size() as i32;
                    l.scroll_by(-n);
                }
            }
            ClientMsg::LogsPageDown => {
                if let Some(l) = self.focused_logs_mut() {
                    let n = l.page_size() as i32;
                    l.scroll_by(n);
                }
            }
            ClientMsg::LogsCopy => {
                if let Some(l) = self.focused_logs_mut() {
                    let payload = l.copy_payload();
                    self.pending_clipboard = Some(payload);
                }
            }
            ClientMsg::LogsRefresh => { if let Some(l) = self.focused_logs_mut() { l.reload(); } }
            ClientMsg::LogsClose => {
                if let Some(id) = self.wm.focused() {
                    if matches!(self.contents.get(&id), Some(WinContent::Logs(_))) {
                        self.close(id);
                    }
                }
            }
            ClientMsg::SetTheme(name) => {
                crate::dbg_log(&format!("theme: set '{name}' (client attach / TUIUI_THEME)"));
                crate::theme::set(&name);
                self.cfg.theme = name;
                if let Err(e) = self.cfg.save() {
                    crate::dbg_log(&format!("theme: config save failed: {e}"));
                }
            }
        }
        // Refresh thumbnails after any message that may have changed the focused
        // file manager's listing (cheap: ImageStore loads are content-hash cached).
        if self.focused_is_filemanager() {
            self.refresh_fm_thumbnails();
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
        self.app_keys.insert(id, "Settings".into());
        let mut panel = Settings::new(self.cfg.clone());
        panel.set_apphost_outdated(self.apphost_outdated);
        self.contents.insert(id, WinContent::Settings(Box::new(panel)));
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
                self.launcher = Launcher::new(Self::build_launcher_apps(&self.cfg, &self.systems));
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
        self.app_keys.insert(id, "Store".into());
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

    /// Assign already-loaded thumbnails to the focused file manager's image
    /// entries and queue any not-yet-loaded ones on the background loader. Never
    /// blocks on file I/O (a slow/offloaded image can no longer freeze the loop).
    /// Non-image entries get the same pre-generated per-role tile the desktop
    /// uses (`self.role_icon_ids`) — no disk I/O, no per-entry PNG generation.
    fn refresh_fm_thumbnails(&mut self) {
        let (reqs, role_reqs) = match self.focused_filemanager_mut() {
            Some(f) => (f.thumbnail_requests(), f.role_icon_requests()),
            None => return,
        };
        let mut ready: Vec<(usize, u64)> = Vec::new();
        for (idx, path) in reqs {
            if let Some(&id) = self.thumb_ids.get(&path) {
                ready.push((idx, id));
            } else {
                // Icon tile's image area is (TILE_W - 2) cells wide by
                // (TILE_H - 1) tall; cells are ~8x16px. Sized to match the
                // enlarged Icon-view tile (was a 1-row-tall request before
                // the icon tile grew to mirror the desktop's).
                let iw = (crate::filemanager::TILE_W - 2).max(2) * 8;
                let ih = (crate::filemanager::TILE_H - 1).max(1) * 16;
                self.thumb_loader.request(path, iw as u32, ih as u32);
            }
        }
        for (idx, role) in role_reqs {
            if let Some(&id) = self.role_icon_ids.get(&role) {
                ready.push((idx, id));
            }
        }
        if let Some(f) = self.focused_filemanager_mut() {
            for (idx, id) in ready {
                f.set_thumb(idx, id);
            }
        }
    }

    /// The assistant panel's window, if one exists (found by app key so it
    /// survives frontend reloads, where the window is restored from the apphost
    /// roster).
    fn assistant_window(&self) -> Option<WindowId> {
        self.app_keys.iter().find(|(_, k)| k.as_str() == "Assistant").map(|(id, _)| *id)
    }

    /// The ✦ menubar button: toggle the assistant chat panel. Focused → hide
    /// (minimize; the agent keeps running); hidden/blurred → show + focus;
    /// absent → spawn the configured agent CLI in a right-docked panel.
    fn toggle_assistant(&mut self) {
        if let Some(id) = self.assistant_window() {
            let minimized = self.wm.get(id).map(|w| w.minimized).unwrap_or(false);
            if self.wm.focused() == Some(id) && !minimized {
                self.wm.minimize(id);
            } else {
                self.wm.unminimize(id);
                if self.simple {
                    self.sync_app_size(id);
                }
            }
            return;
        }
        self.open_assistant();
    }

    /// Spawn the assistant agent (opencode by default) in a persistent
    /// right-side panel. The agent is a normal apphost app (survives
    /// detach/reload); its briefing is stamped as `AGENTS.md` in a dedicated
    /// working directory, which opencode reads on startup.
    fn open_assistant(&mut self) {
        let Some((command, args)) =
            crate::assistant::resolve_agent(self.cfg.assistant_command.as_deref(), &self.cfg.assistant_args)
        else {
            crate::dbg_log(&format!(
                "assistant: '{}' not found (install it, or set assistant_command in config.toml)",
                crate::assistant::DEFAULT_AGENT
            ));
            // Point the user at the AI category instead of failing silently.
            self.open_store();
            return;
        };
        let Some(dir) = crate::assistant::workdir() else { return };
        let host = self.power_label.trim().trim_end_matches('\u{25be}').trim().to_string();
        if let Err(e) = crate::assistant::write_briefing(&dir, &host, &self.systems) {
            crate::dbg_log(&format!("assistant: briefing write failed: {e}"));
        }
        crate::dbg_log(&format!(
            "assistant: starting '{command}' ({}) in {}",
            self.cfg.assistant_mode,
            dir.display()
        ));
        // "panel": right-docked, full work height, 2/5 width (clamped readable).
        // "window": a regular floating window — the popped-out mode. Either way
        // it's a normal WM window, so the user can drag/resize/maximize it.
        let rect = if self.cfg.assistant_mode == "window" {
            let w = 84.min((self.w - 4).max(30));
            let h = 30.min((self.h - 4).max(8));
            Rect::new((self.w - w) / 2, ((self.h - h) / 2).max(1), w, h)
        } else {
            let panel_w = (self.w * 2 / 5).clamp(34.min(self.w), 70);
            Rect::new((self.w - panel_w).max(0), 1, panel_w, (self.h - 2).max(4))
        };
        let id = self.wm.add_window("Assistant".into(), rect);
        let content = self.wm.get(id).unwrap().content_rect();
        match self.apphost.spawn(&command, &args, Some(&dir), content.w.max(1), content.h.max(1)) {
            Ok(app_id) => {
                self.app_keys.insert(id, "Assistant".into());
                self.contents.insert(id, WinContent::App(app_id));
                self.titles.push((id, "Assistant".into()));
                if self.simple {
                    self.sync_app_size(id);
                }
            }
            Err(e) => {
                crate::dbg_log(&format!("assistant: spawn '{command}' failed: {e}"));
                self.wm.close(id);
            }
        }
    }

    /// Open (or re-focus) the Logs viewer window.
    fn open_logs(&mut self) {
        if let Some(id) = self.logs_win {
            if self.contents.contains_key(&id) {
                self.wm.unminimize(id);
                if let Some(WinContent::Logs(l)) = self.contents.get_mut(&id) {
                    l.reload();
                }
                return;
            }
        }
        let w = 100.min((self.w - 4).max(40));
        let h = 28.min((self.h - 4).max(10));
        let rect = Rect::new((self.w - w) / 2, 1, w, h);
        let id = self.wm.add_window("Logs".into(), rect);
        self.app_keys.insert(id, "Logs".into());
        self.contents.insert(id, WinContent::Logs(crate::logsview::LogsView::new()));
        self.titles.push((id, "Logs".into()));
        self.logs_win = Some(id);
    }

    /// `true` when the focused window hosts the logs viewer (keyboard routing).
    pub fn focused_is_logs(&self) -> bool {
        matches!(
            self.wm.focused().and_then(|id| self.contents.get(&id)),
            Some(WinContent::Logs(_))
        )
    }

    fn focused_logs_mut(&mut self) -> Option<&mut crate::logsview::LogsView> {
        let id = self.wm.focused()?;
        match self.contents.get_mut(&id)? {
            WinContent::Logs(l) => Some(l),
            _ => None,
        }
    }

    /// Text the client should put on the HOST terminal clipboard via OSC 52
    /// (one-shot; cleared on take).
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.pending_clipboard.take()
    }

    fn open_filemanager(&mut self) {
        let root = self.picker_root();
        self.open_filemanager_root(root);
    }

    /// Open (or re-focus) the file manager rooted at `root`, then load thumbnails.
    fn open_filemanager_root(&mut self, root: std::path::PathBuf) {
        crate::dbg_log(&format!("open_filemanager_root: {}", root.display()));
        if let Some(id) = self.filemanager_win {
            if self.contents.contains_key(&id) {
                self.wm.unminimize(id);
                return;
            }
        }
        let w = 90.min((self.w - 4).max(40));
        let h = 30.min((self.h - 4).max(12));
        let rect = Rect::new((self.w - w) / 2, 2, w, h);
        let id = self.wm.add_window("Files".into(), rect);
        self.app_keys.insert(id, "Files".into());
        self.contents.insert(
            id,
            WinContent::FileManager(crate::filemanager::DynFileManager::new_local(root, self.cfg.default_apps.clone())),
        );
        self.titles.push((id, "Files".into()));
        self.filemanager_win = Some(id);
        self.refresh_fm_thumbnails();
    }

    /// Open the file manager rooted at an arbitrary directory (test support).
    #[doc(hidden)]
    pub fn open_filemanager_at(&mut self, dir: std::path::PathBuf) {
        // Replace any existing FM so the new root takes effect.
        if let Some(id) = self.filemanager_win.take() {
            self.contents.remove(&id);
            self.titles.retain(|(tid, _)| *tid != id);
            self.wm.close(id);
        }
        self.open_filemanager_root(dir);
    }

    /// Open (or re-focus) a remote file browser for the saved system `name`.
    /// Resolving the remote home blocks briefly (one capped ssh call); failures
    /// open nothing and log the reason.
    fn open_remote_files(&mut self, name: &str) {
        // Re-focus an existing browser for this system.
        if let Some((&id, _)) = self.remote_fms.iter().find(|(_, (n, _, _))| n == name) {
            if self.contents.contains_key(&id) {
                self.wm.unminimize(id);
                return;
            }
        }
        let Some(sys) = self.systems.iter().find(|s| s.name == name).cloned() else {
            crate::dbg_log(&format!("rfiles: unknown system '{name}'"));
            return;
        };
        crate::dbg_log(&format!("rfiles: opening {} ({})", sys.name, sys.host));
        let fs = crate::fileops::SshFs::new(sys.host.clone(), sys.port);
        let Some(home) = fs.remote_home() else {
            crate::dbg_log(&format!(
                "rfiles: {} unreachable (no key auth / offline) — set it up via Systems → Add Remote",
                sys.host
            ));
            return;
        };
        let w = 90.min((self.w - 4).max(40));
        let h = 30.min((self.h - 4).max(12));
        let rect = Rect::new((self.w - w) / 2, 2, w, h);
        let title = format!("Files on {}", sys.name);
        let id = self.wm.add_window(title.clone(), rect);
        self.app_keys.insert(id, title.clone());
        self.contents.insert(
            id,
            WinContent::FileManager(crate::filemanager::DynFileManager::new_remote(
                sys.host.clone(),
                sys.port,
                home,
                self.cfg.default_apps.clone(),
            )),
        );
        self.titles.push((id, title));
        self.remote_fms.insert(id, (sys.name.clone(), sys.host, sys.port));
    }

    /// The ssh backend of the focused file-manager window (`None` = local).
    fn focused_fm_backend(&self) -> FmBackend {
        let id = self.wm.focused()?;
        self.remote_fms.get(&id).map(|(_, target, port)| (target.clone(), *port))
    }

    /// `true` when the focused window hosts the file manager.
    pub fn focused_is_filemanager(&self) -> bool {
        matches!(
            self.wm.focused().and_then(|id| self.contents.get(&id)),
            Some(WinContent::FileManager(_))
        )
    }

    /// `true` when the focused file manager has a text overlay open (new-folder /
    /// rename), so the client forwards typed characters as overlay input.
    pub fn filemanager_editing(&self) -> bool {
        matches!(
            self.wm.focused().and_then(|id| self.contents.get(&id)),
            Some(WinContent::FileManager(f)) if f.is_editing()
        )
    }

    /// Whether the focused file manager has its context (right-click) menu
    /// open; the client redirects Up/Down/Enter/Esc to the menu instead of
    /// normal navigation while this is true.
    pub fn filemanager_context(&self) -> bool {
        matches!(
            self.wm.focused().and_then(|id| self.contents.get(&id)),
            Some(WinContent::FileManager(f))
                if matches!(f.overlay(), Some(crate::filemanager::Overlay::Context { .. }))
        )
    }

    /// Alias kept for integration-test readability.
    #[doc(hidden)]
    pub fn focused_fm_context_open_for_test(&self) -> bool {
        self.filemanager_context()
    }

    fn focused_filemanager_mut(&mut self) -> Option<&mut crate::filemanager::DynFileManager> {
        let id = self.wm.focused()?;
        match self.contents.get_mut(&id)? {
            WinContent::FileManager(f) => Some(f),
            _ => None,
        }
    }

    /// Paste into the focused file manager. Same backend → the manager's own
    /// clipboard; different backend (local↔remote) → background `scp -r`.
    fn fm_paste(&mut self) {
        let dest_backend = self.focused_fm_backend();
        let cross = match (&self.transfer, &dest_backend) {
            (Some((src, _)), dst) => src != dst,
            (None, _) => false,
        };
        if !cross {
            if let Some(f) = self.focused_filemanager_mut() {
                f.paste();
            }
            return;
        }
        let Some((source, paths)) = self.transfer.clone() else { return };
        let Some(f) = self.focused_filemanager_mut() else { return };
        let cwd = f.cwd().to_path_buf();
        // Build one scp invocation per path; remote↔remote uses scp -3 (via us).
        let mut cmds: Vec<Vec<String>> = Vec::new();
        for p in &paths {
            let mut args: Vec<String> = vec!["-r".into()];
            match (&source, &dest_backend) {
                (Some((st, sp)), None) => {
                    if let Some(port) = sp { args.extend(["-P".into(), port.to_string()]); }
                    args.push(format!("{}:{}", st, p.display()));
                    args.push(format!("{}/", cwd.display()));
                }
                (None, Some((dt, dp))) => {
                    if let Some(port) = dp { args.extend(["-P".into(), port.to_string()]); }
                    args.push(p.display().to_string());
                    args.push(format!("{}:{}/", dt, cwd.display()));
                }
                (Some((st, _)), Some((dt, _))) => {
                    args.insert(0, "-3".into());
                    args.push(format!("{}:{}", st, p.display()));
                    args.push(format!("{}:{}/", dt, cwd.display()));
                }
                (None, None) => continue, // handled by the non-cross path
            }
            cmds.push(args);
        }
        f.set_status(format!("transferring {} item(s) in the background (scp)…", cmds.len()));
        crate::dbg_log(&format!("scp transfer: {} item(s) → {}", cmds.len(), cwd.display()));
        std::thread::spawn(move || {
            for args in cmds {
                crate::dbg_log(&format!("scp {}", args.join(" ")));
                match std::process::Command::new("scp")
                    .args(&args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                {
                    Ok(st) if st.success() => crate::dbg_log("scp: ok"),
                    Ok(st) => crate::dbg_log(&format!("scp: FAILED ({st}) — args: {}", args.join(" "))),
                    Err(e) => crate::dbg_log(&format!("scp: could not run: {e}")),
                }
            }
        });
    }

    /// Open (or re-focus) the activity-monitor window, then immediately refresh
    /// its row list so the user sees live data on first render.
    fn open_activity(&mut self) {
        if let Some(id) = self.activity_win {
            if self.contents.contains_key(&id) {
                self.wm.unminimize(id);
                self.refresh_activity();
                return;
            }
        }
        let w = 84.min((self.w - 4).max(40));
        let h = 22.min((self.h - 4).max(10));
        let rect = Rect::new((self.w - w) / 2, 2, w, h);
        let id = self.wm.add_window("Activity Monitor".into(), rect);
        self.app_keys.insert(id, "Activity".into());
        self.contents.insert(id, WinContent::Activity(Activity::new()));
        self.titles.push((id, "Activity Monitor".into()));
        self.activity_win = Some(id);
        self.refresh_activity();
    }

    /// `true` when the focused window hosts the activity monitor.
    pub fn focused_is_activity(&self) -> bool {
        matches!(
            self.wm.focused().and_then(|id| self.contents.get(&id)),
            Some(WinContent::Activity(_))
        )
    }

    /// `true` when the activity monitor's kill-confirm overlay is up; the
    /// client uses this to route Enter / Esc / y / n to the panel.
    pub fn activity_confirming(&self) -> bool {
        self.focused_activity()
            .map(|a| a.has_pending_kill())
            .unwrap_or(false)
    }

    fn focused_activity_mut(&mut self) -> Option<&mut Activity> {
        let id = self.wm.focused()?;
        match self.contents.get_mut(&id)? {
            WinContent::Activity(a) => Some(a),
            _ => None,
        }
    }

    fn focused_activity(&self) -> Option<&Activity> {
        let id = self.wm.focused()?;
        match self.contents.get(&id)? {
            WinContent::Activity(a) => Some(a),
            _ => None,
        }
    }

    /// Rebuild the activity monitor's row list from the current `AppHost`.
    /// Called every frame so the table stays live — but only does any work when
    /// the panel is actually *visible*. Probing every hosted app each frame is
    /// costly (`apphost_dims` clones a whole app grid via `snapshot`), so we must
    /// not pay it when the rows aren't on screen: otherwise every render —
    /// including each step of a window drag — stalls behind N full grid copies,
    /// which is what makes dragging feel sticky/jerky. A minimized panel (skipped
    /// by `build_frame`), or a non-focused one in simple mode (hidden by
    /// `build_frame_simple`), counts as not visible.
    pub fn refresh_activity(&mut self) {
        let win = match self.activity_win {
            Some(id)
                if matches!(self.contents.get(&id), Some(WinContent::Activity(_)))
                    && self.wm.get(id).is_some_and(|w| {
                        !w.minimized && (!self.simple || self.wm.focused() == Some(id))
                    }) =>
            {
                id
            }
            _ => return,
        };
        let entries: Vec<crate::apphost::AppListEntry> = self
            .apphost
            .list()
            .into_iter()
            .map(|id| {
                let alive = self.apphost.is_alive(id);
                let pid = self.apphost.pid(id);
                let age_secs = self
                    .apphost
                    .spawn_time(id)
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                let (cols, rows) = self.apphost_dims(id);
                let (cmd, args) = self.apphost_launch_cmd(id);
                crate::apphost::AppListEntry {
                    app: id.0,
                    cmd,
                    args,
                    pid,
                    cols,
                    rows,
                    age_secs,
                    alive,
                }
            })
            .collect();
        if let Some(WinContent::Activity(a)) = self.contents.get_mut(&win) {
            a.set_rows(entries);
        }
    }

    /// Recover the launch command + args for `id` via the `AppHost` trait's
    /// own `launch_cmd` (both `LocalAppHost` and `RemoteAppHost` track it);
    /// empty strings when the host has no record (e.g. an unknown id).
    fn apphost_launch_cmd(&self, id: AppId) -> (String, Vec<String>) {
        if let Some((cmd, args)) = self.apphost.launch_cmd(id) {
            (cmd, args)
        } else {
            (String::new(), Vec::new())
        }
    }

    fn apphost_dims(&self, id: AppId) -> (i32, i32) {
        if let Some(snap) = self.apphost.snapshot(id) {
            (snap.width(), snap.height())
        } else {
            (0, 0)
        }
    }

    /// The cwd of the focused file manager (for launching an app in-place).
    fn focused_fm_cwd(&self) -> Option<std::path::PathBuf> {
        match self.wm.focused().and_then(|id| self.contents.get(&id)) {
            Some(WinContent::FileManager(f)) => Some(f.cwd().to_path_buf()),
            _ => None,
        }
    }

    /// Turn a pending [`FileManagerAction`] from the focused file manager into a
    /// real effect: open the builtin image viewer, or launch a TUI app in the
    /// file manager's current directory.
    fn drain_fm_action(&mut self) {
        // Compute everything that borrows `self` immutably first, so the
        // subsequent `&mut self` calls (open_image / launch_in) don't conflict.
        let action = self.focused_filemanager_mut().and_then(|f| f.take_action());
        let cwd = self.focused_fm_cwd();
        match action {
            Some(crate::filemanager::FileManagerAction::OpenImage(path)) => {
                crate::dbg_log(&format!("fm action: OpenImage {}", path.display()));
                self.open_image(path.to_string_lossy().to_string());
            }
            Some(crate::filemanager::FileManagerAction::RunApp { command, args }) => {
                crate::dbg_log(&format!("fm action: RunApp {}", command));
                let name = args
                    .last()
                    .and_then(|a| a.rsplit('/').next())
                    .unwrap_or(&command)
                    .to_string();
                self.launch_in(name, command, args, cwd);
            }
            None => {}
        }
    }

    /// Turn a pending [`DesktopAction`] from the desktop model into a real effect:
    /// open a folder in Files, open an image, launch an app for a file, run a pin,
    /// or unpin a shortcut. Mirrors [`drain_fm_action`](Self::drain_fm_action).
    fn drain_desktop_action(&mut self) {
        // Take the action first so the desktop borrow is dropped before the
        // subsequent `&mut self` effect calls.
        let action = self.desktop.take_action();
        match action {
            Some(crate::desktop::DesktopAction::Open(path)) => {
                crate::dbg_log(&format!("desktop action: Open {}", path.display()));
                let is_dir = path.is_dir();
                match crate::openwith::resolve(&path, is_dir, &self.cfg.default_apps) {
                    crate::openwith::OpenAction::Navigate => self.open_filemanager_root(path),
                    crate::openwith::OpenAction::Builtin("@image") => {
                        self.open_image(path.to_string_lossy().to_string());
                    }
                    crate::openwith::OpenAction::Builtin(_) => {}
                    crate::openwith::OpenAction::RunApp { command, args } => {
                        let name = args
                            .last()
                            .and_then(|a| a.rsplit('/').next())
                            .unwrap_or(&command)
                            .to_string();
                        let cwd = path.parent().map(|p| p.to_path_buf());
                        self.launch_in(name, command, args, cwd);
                    }
                    crate::openwith::OpenAction::OpenWithMenu => {}
                }
            }
            Some(crate::desktop::DesktopAction::Run { command, args }) => {
                crate::dbg_log(&format!("desktop action: Run {}", command));
                self.launch_entry(AppEntry {
                    name: command.clone(),
                    command,
                    args,
                    category: None,
                    requires_cwd: None,
                    cwd: None,
                    cli: None,
                    warn: None,
                });
            }
            Some(crate::desktop::DesktopAction::Unpin(cmd)) => {
                crate::dbg_log(&format!("desktop action: Unpin {}", cmd));
                self.cfg.desktop_pins.retain(|p| p.command != cmd);
                let _ = self.cfg.save();
                self.reload_desktop();
            }
            None => {}
        }
    }

    /// Right-click on the desktop: open a context (icon) or empty-desktop menu,
    /// but only for clicks that fall through to the desktop (no window hit).
    fn handle_desktop_right(&mut self, p: Point) {
        if self.cfg.desktop_enabled && self.window_at_is_none(p) {
            self.desktop.right_click(p);
        }
    }

    /// Commit the desktop's rename / new-folder overlay, reloading on success.
    fn desktop_commit(&mut self) {
        if self.desktop.overlay_commit() {
            self.reload_desktop();
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
            // CLI-flagged apps get the same `sh -lc … --help; exec $SHELL` wrapper
            // as launcher-driven launches (see `cli_wrap` / `launch_entry`).
            let (command, args) = if app.cli { cli_wrap(&app.bin, &[]) } else { (app.bin.clone(), Vec::new()) };
            self.launch_maybe_cwd(app.name.clone(), command, args, requires_cwd, None);
        } else {
            // Run the install visibly, then drop into a shell so the output (and
            // any errors) stay on screen. Closing the window triggers a $PATH
            // re-scan + launcher rebuild (see `reap_dead`).
            let cmd = store::install_command(app);
            let wrapped = format!(
                "{cmd}; echo; echo '── install finished — close this window (✕) to refresh ──'; exec \"$SHELL\""
            );
            self.launch(format!("install: {}", app.name), "sh".into(), vec!["-lc".into(), wrapped]);
        }
    }

    /// Activate a launcher entry: open the store/settings for the pinned tuiui
    /// actions, otherwise spawn the app (prompting for a working directory when
    /// the entry is flagged `requires_cwd` and has no fixed `cwd`).
    fn launch_entry(&mut self, e: AppEntry) {
        match e.command.as_str() {
            "@store" => self.open_store(),
            "@settings" => self.open_settings(),
            "@files" => self.open_filemanager(),
            "@logs" => self.open_logs(),
            "@rfiles" => {
                if let Some(name) = e.args.first().cloned() {
                    self.open_remote_files(&name);
                }
            }
            "@activity" => self.open_activity(),
            "@image" => { if let Some(p) = e.args.first().cloned() { self.open_image(p); } }
            _ => {
                let cli = e.cli.unwrap_or(false);
                let requires_cwd = e.requires_cwd.unwrap_or(false);
                if let Some(message) = e.warn {
                    // A warn-flagged entry (e.g. the Claude Code "skip
                    // permissions" variant) opens the modal first instead of
                    // launching; `LaunchWarnYes` replays the exact same launch
                    // via `launch_resolved`.
                    self.launch_warn.open(
                        crate::launchwarn::PendingLaunch {
                            name: e.name,
                            command: e.command,
                            args: e.args,
                            cli,
                            requires_cwd,
                            cwd: e.cwd,
                        },
                        message,
                    );
                } else {
                    self.launch_resolved(e.name, e.command, e.args, cli, requires_cwd, e.cwd);
                }
            }
        }
    }

    /// The tail of `launch_entry`'s default arm: apply the CLI
    /// help-then-shell wrapper (if flagged) and hand off to
    /// `launch_maybe_cwd`. Shared by the direct launch path and the
    /// launch-warning dialog's confirm action, so a warned launch proceeds
    /// exactly as it would have without the prompt.
    fn launch_resolved(&mut self, name: String, command: String, args: Vec<String>, cli: bool, requires_cwd: bool, cwd: Option<String>) {
        // CLI-flagged apps launch through the `sh -lc … --help; exec $SHELL`
        // wrapper instead of the bare binary (see `cli_wrap`); the window
        // title still shows the app name, and `requires_cwd`/`cwd` keep
        // working since the shell itself starts in the picked directory.
        let (command, args) = if cli { cli_wrap(&command, &args) } else { (command, args) };
        self.launch_maybe_cwd(name, command, args, requires_cwd, cwd)
    }

    /// Spawn a new PTY-backed window.
    ///
    /// If spawning fails, the window is removed and no dock entry
    /// is added (silently drops the launch request — the caller can surface an
    /// error later via a `CoreMsg` notification once that protocol exists).
    fn launch(&mut self, name: String, command: String, args: Vec<String>) {
        self.launch_in(name, command, args, None);
    }

    /// Open a new shell window (the dock "+" button / quick-launch). Uses `$SHELL`
    /// (falling back to `sh`); the window groups under the "Shell" app key.
    fn open_shell(&mut self) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
        self.launch("Shell".into(), shell, Vec::new());
    }

    /// Spawn a new PTY-backed window, starting the child in `cwd` (or the user's
    /// home when `None`).
    fn launch_in(&mut self, name: String, command: String, args: Vec<String>, cwd: Option<std::path::PathBuf>) {
        crate::dbg_log(&format!("launch_in {} cmd={}", name, command));
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
        // Optionally open maximized (fills the work area) before sizing the PTY.
        if self.cfg.launch_maximized {
            self.wm.maximize_toggle(id);
        }
        let content = self.wm.get(id).unwrap().content_rect();
        match self.apphost.spawn(&command, &args, cwd.as_deref(), content.w.max(1), content.h.max(1)) {
            Ok(app_id) => {
                self.app_keys.insert(id, name.clone());
                self.contents.insert(id, WinContent::App(app_id));
                self.titles.push((id, name));
                self.auto_tile_if_enabled();
                if self.simple {
                    self.sync_app_size(id);
                }
            }
            Err(_) => {
                self.wm.close(id);
            }
        }
    }

    /// Whether `p` is more than half the screen *vertically* away from the last
    /// cursor position — a physically impossible single-event jump that indicates
    /// a spurious terminal mouse report. (Vertical only: the observed bad reports
    /// teleport to the bottom row, and horizontal jumps are legit — e.g. clicking
    /// the far-left brand then the far-right power button on the menubar. Off-
    /// screen horizontal motion is handled by `wm::move_to` clamping instead.)
    fn is_spurious_jump(&self, p: Point) -> bool {
        (p.y - self.cursor.y).abs() > self.h / 2
    }

    /// Route a mouse event through dock hit-testing then the WM input router.
    fn handle_mouse(&mut self, kind: MouseKind, p: Point) {
        // The help overlay is modal: any click dismisses it.
        if kind == MouseKind::Down && self.help_open {
            self.help_open = false;
            return;
        }

        // The post-update compat dialog is the topmost modal of all: nothing
        // else reacts until the user picks "keep apps" or "restart app server".
        if kind == MouseKind::Down && self.compat_dialog {
            let (keep, restart) = compat_dialog_buttons(self.w, self.h);
            if restart.contains(p) {
                self.restart_apphost();
            } else if keep.contains(p) || !compat_dialog_rect(self.w, self.h).contains(p) {
                // "Keep apps" (or a click outside): dismiss; the restart stays
                // available in Settings → Updates once work is saved.
                crate::dbg_log("compat: user chose to keep apps for now");
                self.compat_dialog = false;
            }
            return;
        }

        // The dock context menu captures the next click: a row applies its
        // action; anywhere else dismisses (consuming the click, like the
        // other transient menus).
        if kind == MouseKind::Down && self.dock_ctx.is_some() {
            if let Some((id, ax)) = self.dock_ctx.take() {
                let row = (0..DOCK_CTX_ROWS.len())
                    .find(|&i| dock_ctx_row_rect(ax, self.w, self.h, i).contains(p));
                match row {
                    Some(0) => self.wm.minimize(id),
                    Some(1) => {
                        self.wm.unminimize(id);
                        self.wm.maximize_toggle(id);
                        self.sync_app_size(id);
                    }
                    Some(2) => self.request_close(id),
                    Some(3) => {
                        // Reset: centre at half the work area's width/height —
                        // the rescue hatch for a mis-sized or stranded window.
                        self.wm.unminimize(id);
                        let work = self.wm.work_area();
                        let (nw, nh) = ((work.w / 2).max(20), (work.h / 2).max(5));
                        self.wm.move_to(id, work.x + (work.w - nw) / 2, work.y + (work.h - nh) / 2);
                        self.wm.resize_to(id, nw, nh);
                        self.sync_app_size(id);
                    }
                    _ => {}
                }
            }
            return;
        }

        // The power menu (dropdown / confirm dialog) is modal while open: route
        // the click to it and act on a confirmed choice.
        if kind == MouseKind::Down && self.power_menu.is_open() {
            match self.power_menu.on_click(p, self.w, self.h, &self.systems) {
                PowerClick::Act(outcome) => self.apply_power_outcome(outcome),
                PowerClick::Consumed => {}
            }
            return;
        }

        // The confirm-close dialog is modal while open: route the click to it and
        // close the window if confirmed.
        if kind == MouseKind::Down && self.confirm_close.is_open() {
            if let Some(id) = self.confirm_close.on_click(p, self.w, self.h) {
                self.close(id);
            }
            return;
        }

        // The launch-warning dialog is modal while open: route the click to it
        // and run the pending launch if confirmed.
        if kind == MouseKind::Down && self.launch_warn.is_open() {
            if let Some(l) = self.launch_warn.on_click(p, self.w, self.h) {
                self.launch_resolved(l.name, l.command, l.args, l.cli, l.requires_cwd, l.cwd);
            }
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

        // An open launcher captures all clicks/moves: in Menu mode a move flies
        // out the hovered submenu and a click descends/launches; in Spotlight a
        // click launches the hit row. Nothing leaks through to window routing.
        if self.launcher.is_open() {
            match (self.launcher.mode(), kind) {
                (Some(crate::launcher::LauncherMode::Menu), MouseKind::Drag) => {
                    self.launcher.hover(p);
                    return;
                }
                (Some(crate::launcher::LauncherMode::Menu), MouseKind::Down) => {
                    if let Some(app) = self.launcher.click(p) {
                        self.launcher.close();
                        self.launch_entry(app);
                    } else if !self.launcher.point_in_menu(p) {
                        self.launcher.close();
                    }
                    return;
                }
                (Some(crate::launcher::LauncherMode::Menu), _) => return,
                (Some(crate::launcher::LauncherMode::Spotlight), MouseKind::Down) => {
                    let rendered = self.launcher.render(self.w, self.h);
                    let hit = rendered
                        .items
                        .into_iter()
                        .find(|(_, r)| r.contains(p))
                        .map(|(entry, _)| entry);
                    self.launcher.close();
                    if let Some(entry) = hit {
                        self.launch_entry(entry);
                    }
                    return;
                }
                _ => {}
            }
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
            if menubar_mode_region().contains(p) {
                self.launcher.close();
                self.power_menu.close();
                self.toggle_simple();
                return;
            }
            if menubar_assistant_region().contains(p) {
                self.launcher.close();
                self.power_menu.close();
                self.toggle_assistant();
                return;
            }
            if menubar_brand_region().contains(p) {
                self.power_menu.close();
                self.launcher.toggle_menu();
                return;
            }
            if menubar_power_region(self.w, &self.power_label).contains(p) {
                self.launcher.close();
                self.power_menu.toggle();
                return;
            }
            let segs = self.tray_segments_now();
            if self.tray.on_menubar_click(p, &segs) {
                return;
            }

            // If a dock group popup is open, hit-test it first (modal for
            // clicks while it is showing). A click on a row focuses that
            // window; a click anywhere else dismisses the popup.
            if self.dock_popup.is_some() {
                let popup_rows = self.dock_popup_rows();
                let (_layers, row_rects) = self.render_popup_for_hit_test(&popup_rows);
                for (win_id, row_rect) in &row_rects {
                    if row_rect.contains(p) {
                        let id = *win_id;
                        self.dock_popup = None;
                        self.wm.unminimize(id);
                        if self.simple { self.sync_app_size(id); }
                        return;
                    }
                }
                // Click outside popup → dismiss it, then continue normal routing
                self.dock_popup = None;
                return;
            }

            // The bottom-left "+" button opens a new shell window.
            if crate::chrome::dock_new_shell_region(self.h).contains(p) {
                self.dock_popup = None;
                self.open_shell();
                return;
            }

            let items = self.dock_items();
            for (idx, r) in self.dock_regions() {
                if r.contains(p) {
                    match &items[idx].kind {
                        DockKind::Single(id) => {
                            let id = *id;
                            self.dock_popup = None;
                            self.wm.unminimize(id);
                            if self.simple { self.sync_app_size(id); }
                        }
                        DockKind::Group(key, _) => {
                            let key = key.clone();
                            self.dock_popup = if self.dock_popup.as_deref() == Some(key.as_str()) {
                                None
                            } else {
                                Some(key)
                            };
                        }
                    }
                    return;
                }
            }
        }
        // An open desktop menu floats above the windows: a left press first tries
        // a menu item, else dismisses the menu, before any window routing.
        if kind == MouseKind::Down && self.cfg.desktop_enabled && self.desktop.overlay().is_some() {
            self.handle_desktop_menu_click(p);
            return;
        }

        // A desktop drag in progress captures motion + release (the icon drag is
        // tracked inside the desktop model, separate from window `self.drag`).
        if self.desktop.dragging() {
            match kind {
                MouseKind::Drag => {
                    self.desktop.drag_to(p);
                    return;
                }
                MouseKind::Up => {
                    if self.desktop.end_drag(p) {
                        self.persist_desktop_positions();
                    }
                    return;
                }
                MouseKind::Down | MouseKind::Move => {}
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
        // A left press that hits no window and no chrome falls through to the
        // desktop: begin a drag on an icon (which also selects it), else clear
        // the selection. A plain press-without-move still leaves the icon
        // selected; a drag moves + persists.
        if self.cfg.desktop_enabled
            && matches!(action, Action::None)
            && self.drag.is_none()
            && kind == MouseKind::Down
        {
            match self.desktop.icon_at(p) {
                Some(_) => self.desktop.begin_drag(p),
                None => self.desktop.click(p, false),
            }
            return;
        }
        self.exec(action, p);
    }

    /// A click while a desktop context / desktop menu is open: act on the menu
    /// item under `p`, or dismiss the menu if the click missed it.
    fn handle_desktop_menu_click(&mut self, p: Point) {
        let item = self.desktop.menu_item_at(p);
        let idx = self.desktop.context_idx();
        match item {
            Some(crate::desktop::DesktopMenuItem::Open) => {
                self.desktop.cancel_overlay();
                if let Some(i) = idx {
                    let r = self.desktop.tile_rect(self.desktop.icons()[i].cell);
                    self.desktop.double_click(Point::new(r.x + 1, r.y));
                    self.drain_desktop_action();
                }
            }
            Some(crate::desktop::DesktopMenuItem::OpenWith) => self.desktop.cancel_overlay(),
            Some(crate::desktop::DesktopMenuItem::Rename) => {
                if let Some(i) = idx {
                    self.desktop.begin_rename(i);
                }
            }
            Some(crate::desktop::DesktopMenuItem::Trash) => {
                if self.desktop.trash_selection() {
                    self.reload_desktop();
                }
            }
            Some(crate::desktop::DesktopMenuItem::Unpin) => {
                let cmd = idx.and_then(|i| self.desktop.icon_command(i));
                self.desktop.cancel_overlay();
                if let Some(cmd) = cmd {
                    self.cfg.desktop_pins.retain(|p| p.command != cmd);
                    let _ = self.cfg.save();
                    self.reload_desktop();
                }
            }
            Some(crate::desktop::DesktopMenuItem::NewFolder) => self.desktop.begin_new_folder(),
            Some(crate::desktop::DesktopMenuItem::CleanUp) => {
                self.desktop.cancel_overlay();
                self.desktop.clean_up();
                self.persist_desktop_positions();
            }
            None => self.desktop.cancel_overlay(),
        }
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
                self.drag_start = Some(p);
                self.drag_armed = false;
            }
            Action::BeginResize(id) => {
                self.wm.raise(id);
                self.drag = Some(Hit::Resizing { id });
                self.drag_start = Some(p);
                self.drag_armed = false;
            }
            Action::MoveTo { id, x, y } => {
                // Ignore sub-threshold motion so a plain click (or jitter) on a
                // titlebar never untiles/moves the window — only a real drag does.
                if !self.drag_armed {
                    let moved = self.drag_start
                        .map(|s| (p.x - s.x).abs() + (p.y - s.y).abs() >= DRAG_THRESHOLD)
                        .unwrap_or(true);
                    if !moved {
                        return;
                    }
                    self.drag_armed = true;
                }
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
                // Same drag threshold: a plain click on a window edge must not
                // resize/untile it; only a real drag does.
                if !self.drag_armed {
                    let moved = self.drag_start
                        .map(|s| (p.x - s.x).abs() + (p.y - s.y).abs() >= DRAG_THRESHOLD)
                        .unwrap_or(true);
                    if !moved {
                        return;
                    }
                    self.drag_armed = true;
                }
                let r = self.wm.get(id).unwrap().rect;
                self.wm.resize_to(id, w - r.x + 1, h - r.y + 1);
                self.sync_app_size(id);
            }
            Action::Close(id) => self.request_close(id),
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
                let mut fm_clicked = false;
                let mut activity_kill: Option<u64> = None;
                if let Some(cr) = cr {
                    match self.contents.get_mut(&id) {
                        Some(WinContent::Store(s)) => store_activate = s.handle_click(local, cr.w, cr.h),
                        Some(WinContent::Settings(s)) => settings_changed = s.handle_click(local, cr.w, cr.h),
                        // The mouse path carries no modifiers, so a click is a plain
                        // single-select / toolbar-nav (Ctrl/Shift-select and
                        // double-click-to-open are keyboard-driven for v1).
                        // While the context menu is open it captures every click
                        // (item → act, elsewhere → dismiss) before normal FM
                        // click handling ever sees it.
                        Some(WinContent::FileManager(f)) => {
                            fm_clicked = if f.context_menu_open() {
                                f.context_menu_click(local, cr.w, cr.h)
                            } else {
                                f.handle_click(local, cr.w, cr.h, false, false)
                            };
                        }
                        Some(WinContent::Activity(a)) => activity_kill = a.handle_click(local, cr.w),
                        _ => {}
                    }
                }
                if let Some(app_id) = activity_kill {
                    self.apphost.kill(AppId(app_id));
                }
                if store_activate {
                    self.store_activate();
                }
                if settings_changed {
                    // A click can have armed an Updates action (Check / Install);
                    // fire it on the mouse path too, not just the keyboard one.
                    self.drain_settings_action();
                    self.sync_settings();
                }
                if fm_clicked {
                    self.drain_fm_action();
                }
            }
            Action::EndDrag => {
                // Only consider drop-to-snap if the drag actually moved (armed);
                // a plain titlebar click leaves a tiled window exactly as it was.
                if let (Some(Hit::Moving { id, .. }), true) = (self.drag, self.drag_armed) {
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
                self.drag_start = None;
                self.drag_armed = false;
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
    /// content rect (or the full work area when in simple mode and focused).
    fn sync_app_size(&mut self, id: WindowId) {
        let target = if self.simple && self.wm.focused() == Some(id) {
            self.simple_content_rect()
        } else if let Some(w) = self.wm.get(id) {
            w.content_rect()
        } else {
            return;
        };
        if let Some(WinContent::App(aid)) = self.contents.get(&id) {
            let aid = *aid;
            self.apphost.resize(aid, target.w.max(1), target.h.max(1));
        }
    }

    /// Kill a window's content, remove its dock entry, and close the WM window.
    /// Handle a close request from the titlebar ✕. App windows (whose close kills
    /// a running process) raise a modal confirm dialog first; built-in panels
    /// (Store / Settings / File Manager) close immediately.
    fn request_close(&mut self, id: WindowId) {
        if matches!(self.contents.get(&id), Some(WinContent::App(_))) {
            let title = self
                .titles
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, t)| t.clone())
                .unwrap_or_default();
            self.confirm_close.open(id, title);
        } else {
            self.close(id);
        }
    }

    fn close(&mut self, id: WindowId) {
        // An install window is kept alive (it `exec`s a shell after the install so
        // the output stays readable), so it never trips `reap_dead`; instead we
        // refresh here when the user closes it, so the new app appears immediately.
        let was_install = self
            .titles
            .iter()
            .any(|(i, t)| *i == id && t.starts_with("install:"));
        crate::dbg_log(&format!(
            "close window {:?}{}",
            id,
            if was_install { " (install)" } else { "" }
        ));
        if let Some(WinContent::App(aid)) = self.contents.remove(&id) {
            self.apphost.kill(aid);
            self.apphost.remove(aid);
        }
        if self.store_win == Some(id) {
            self.store_win = None;
        }
        if self.settings_win == Some(id) {
            self.settings_win = None;
        }
        if self.filemanager_win == Some(id) {
            self.filemanager_win = None;
        }
        if self.logs_win == Some(id) {
            self.logs_win = None;
        }
        if self.activity_win == Some(id) {
            self.activity_win = None;
        }
        self.remote_fms.remove(&id);
        self.titles.retain(|(i, _)| *i != id);
        self.app_keys.remove(&id);
        // Close popup if its group no longer exists after this window closes.
        if let Some(ref key) = self.dock_popup.clone() {
            let still_has_group = self.app_keys.values().any(|k| k == key);
            if !still_has_group {
                self.dock_popup = None;
            }
        }
        self.wm.close(id);
        if was_install {
            self.refresh_installed_apps();
        }
        self.auto_tile_if_enabled();
    }

    /// Re-scan `$PATH` for newly-installed binaries and rebuild the launcher so a
    /// just-installed app appears without a daemon restart. Shared by `close`
    /// (install window dismissed) and `reap_dead` (install process exited).
    fn refresh_installed_apps(&mut self) {
        crate::catalog::refresh_installed();
        self.launcher = Launcher::new(Self::build_launcher_apps(&self.cfg, &self.systems));
    }

    // ── Frame builder ─────────────────────────────────────────────────────────

    /// Render the rename text-entry overlay over a window's titlebar row.
    ///
    /// Returns a single 1-row [`Layer`] at high z (2000) drawn over the title
    /// area of the window, showing the current buffer and a cursor block.
    fn render_rename_overlay(&self, win: &crate::window::Window, buf: &str) -> Vec<Layer> {
        let t = crate::theme::current();
        let r = win.rect;
        // Title area: skip the 2-cell left indent; leave room for control buttons.
        let btn_reserve = if r.w >= 9 { 9 } else { 4 };
        let title_start = r.x + 2;
        let title_w = (r.w - 2 - btn_reserve).max(1);
        let mut row = crate::buffer::CellBuffer::new(title_w, 1);
        // Fill background.
        row.fill(crate::cell::Cell { ch: ' ', fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
        // Draw the buffer text (truncated to fit).
        let display: String = buf.chars().take(title_w as usize).collect();
        for (i, ch) in display.chars().enumerate() {
            row.set(i as i32, 0, crate::cell::Cell { ch, fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
        }
        // Draw cursor block after the text.
        let cursor_x = display.chars().count().min((title_w - 1) as usize) as i32;
        row.set(cursor_x, 0, crate::cell::Cell { ch: ' ', fg: t.title_focus, bg: t.title_fg, attrs: Default::default() });
        vec![Layer { z: 2000, origin: Point::new(title_start, r.y), buf: row, opacity: 1.0, scissor: None }]
    }

    /// Build a complete [`Frame`] from the current session state.
    ///
    /// The frame contains (bottom to top):
    /// 1. Window shadow + body layers for every open window (z-ordered).
    /// 2. The menubar layer (z = 1000).
    /// 3. The dock layer (z = 1000).
    ///
    /// The cursor is set to the last known mouse position.
    pub fn build_frame(&self) -> Frame {
        if self.simple {
            return self.build_frame_simple();
        }
        let mut layers: Vec<Layer> = Vec::new();
        let focused = self.wm.focused();

        // The desktop icon layer sits at z=0, beneath every window (z≥1).
        if self.cfg.desktop_enabled {
            let buf = self.desktop.render(self.w, self.h);
            layers.push(Layer { z: 0, origin: Point::new(0, 0), buf, opacity: 1.0, scissor: None });
        }

        for w in self.wm.z_ordered() {
            if w.minimized {
                continue; // hidden to the dock
            }
            let cr = w.content_rect();
            let content = self.contents.get(&w.id)
                .map(|c| c.render(self.apphost.as_ref(), cr.w, cr.h))
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

        // Window rename overlay: draw the edit buffer over the target window's
        // titlebar row at high z (above the window chrome, below floating menus).
        if let Some((ren_id, ren_buf)) = &self.rename {
            if let Some(win) = self.wm.get(*ren_id) {
                layers.extend(self.render_rename_overlay(win, ren_buf));
            }
        }

        let app_name = focused
            .and_then(|id| self.titles.iter().find(|(i, _)| *i == id))
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        let segs = {
            let st = self.tray_state.read().unwrap();
            crate::tray::tray_segments(&st, self.w, self.power_label.chars().count() as i32, self.tray.notif_count())
        };
        layers.push(render_menubar(self.w, &app_name, &segs, false, &self.power_label));
        layers.push(render_dock(self.w, self.h, &self.dock_items()));
        layers.extend(self.render_dock_ctx());

        // The desktop context / rename menu floats above the windows (but below
        // the launcher / help overlays) on its own high-z layer.
        if self.cfg.desktop_enabled {
            if let Some(buf) = self.desktop.overlay_buffer(self.w, self.h) {
                layers.push(Layer { z: 850, origin: Point::new(0, 0), buf, opacity: 1.0, scissor: None });
            }
        }

        // From here on come the floating, panel-sized overlays (launcher, tray,
        // dir-picker, help, power menu). Record where they start so we can collect
        // their rects and suppress only the desktop icons they actually cover —
        // terminals draw the icon graphics over text, so an icon under a menu
        // would otherwise hide it. (Panel-sized layers give precise rects; the
        // full-screen desktop-overlay layer above is handled via overlay_rect().)
        let overlay_start = layers.len();

        // Dock group chooser popup (above dock, below the floating menus).
        if self.dock_popup.is_some() {
            let rows = self.dock_popup_rows();
            if !rows.is_empty() {
                let (popup_layers, _) = self.render_popup_for_hit_test(&rows);
                layers.extend(popup_layers);
            }
        }

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

        // The power menu (dropdown + confirm dialog) renders above all chrome.
        {
            let online = self.tray_state.read().map(|st| st.remotes_online.clone()).unwrap_or_default();
            layers.extend(self.power_menu.render(self.w, self.h, &self.systems, &online));
        }

        // The confirm-close / compat-restart / launch-warning dialogs stack on
        // top of everything else, in that order (launch-warning is the topmost
        // modal of all, by z).
        layers.extend(self.confirm_close.render(self.w, self.h));
        layers.extend(self.render_compat_dialog());
        layers.extend(self.launch_warn.render(self.w, self.h));

        // Screen rects of the floating overlays now showing (empty when none open).
        let overlay_rects: Vec<crate::geometry::Rect> = layers[overlay_start..]
            .iter()
            .map(|l| crate::geometry::Rect::new(l.origin.x, l.origin.y, l.buf.width(), l.buf.height()))
            .collect();

        // Image placements for ImageView windows (visible only when their content
        // rect is fully unobstructed; the placeholder cells show otherwise).
        let mut images = Vec::new();
        for w in self.wm.z_ordered() {
            if w.minimized {
                continue;
            }
            let id = match self.contents.get(&w.id) {
                Some(WinContent::ImageView(v)) => v.image_id(),
                _ => None,
            };
            if let Some(id) = id {
                let cr = w.content_rect();
                images.push(crate::protocol::ImagePlacement {
                    id,
                    rect: cr,
                    cols: cr.w.max(1) as u16,
                    rows: cr.h.max(1) as u16,
                    visible: self.fully_unobstructed(w),
                });
            }
        }

        // Thumbnail placements for visible file-manager windows (Icon view).
        for w in self.wm.z_ordered() {
            if w.minimized {
                continue;
            }
            if let Some(WinContent::FileManager(f)) = self.contents.get(&w.id) {
                let cr = w.content_rect();
                let vis = self.fully_unobstructed(w);
                images.extend(f.thumbnail_placements(cr, vis));
            }
        }

        // Image placements for desktop icons (photo thumbnails or generated
        // file-type icons) not covered by a window. (Menu/overlay overlap is
        // handled uniformly for ALL images by the suppression pass below.)
        if self.cfg.desktop_enabled {
            let occluded = |r: crate::geometry::Rect| {
                self.wm
                    .z_ordered()
                    .iter()
                    .any(|w| !w.minimized && w.rect.intersect(r).is_some())
            };
            images.extend(self.desktop.icon_placements(&self.role_icon_ids, |r| !occluded(r)));
        }

        // Image placements transmitted by hosted apps (A2 Kitty-graphics passthrough),
        // offset to the window's content rect and clipped to it. The PNGs were loaded
        // into `self.images` by `refresh_app_graphics` (called before this frame).
        for w in self.wm.z_ordered() {
            if w.minimized {
                continue;
            }
            let aid = match self.contents.get(&w.id) {
                Some(WinContent::App(aid)) => *aid,
                _ => continue,
            };
            let placements = self.apphost.placements(aid);
            if placements.is_empty() {
                continue;
            }
            let cr = w.content_rect();
            let vis = self.fully_unobstructed(w);
            for pl in &placements {
                if let Some(&img) = self.app_image_ids.get(&(w.id, pl.image_id)) {
                    let x = cr.x + pl.col as i32;
                    let y = cr.y + pl.row as i32;
                    if x >= cr.x + cr.w || y >= cr.y + cr.h {
                        continue;
                    }
                    let cols = pl.cols.min((cr.x + cr.w - x).max(1) as u16);
                    let rows = pl.rows.min((cr.y + cr.h - y).max(1) as u16);
                    images.push(crate::protocol::ImagePlacement {
                        id: img,
                        rect: crate::geometry::Rect::new(x, y, cols as i32, rows as i32),
                        cols,
                        rows,
                        visible: vis,
                    });
                }
            }
        }

        // Terminals composite images OVER text, so any image sitting under a
        // floating menu/overlay would hide it. Drop every image (desktop icon,
        // FM thumbnail, image viewer, or hosted-app graphic) that intersects an
        // open overlay's rect — the launcher/power-menu/help/tray panels and the
        // desktop context menu. Only icons actually under a menu are removed, so
        // a top-left launcher leaves the top-right icons as image tiles.
        let mut menu_rects = overlay_rects;
        if let Some(dm) = self.desktop.overlay_rect() {
            menu_rects.push(dm);
        }
        if !menu_rects.is_empty() {
            images.retain(|p| menu_rects.iter().all(|o| o.intersect(p.rect).is_none()));
        }
        sanitize_images(&mut images, self.w, self.h);

        Frame { layers, cursor: Some(self.cursor), images }
    }

    /// Build the frame for simple (full-screen single-app) view: the focused
    /// window's content fills the work area with no decorations; the menubar and
    /// dock stay; the desktop and other windows are hidden.
    fn build_frame_simple(&self) -> Frame {
        use crate::geometry::Point;
        let t = crate::theme::current();
        let wa = self.simple_content_rect();
        let mut layers: Vec<Layer> = Vec::new();
        let focused = self.wm.focused();

        // Focused window full-screen (no chrome), or a hint when nothing is open.
        if let Some(fid) = focused {
            if let Some(content) = self.contents.get(&fid) {
                let buf = content.render(self.apphost.as_ref(), wa.w, wa.h);
                layers.push(Layer { z: 1, origin: Point::new(wa.x, wa.y), buf, opacity: 1.0, scissor: None });
            }
        } else {
            let mut buf = crate::buffer::CellBuffer::new(wa.w, wa.h);
            buf.fill(crate::cell::Cell { ch: ' ', fg: t.dim, bg: t.desktop_bg, attrs: Default::default() });
            let hint = "Press Go to launch an app";
            let hx = ((wa.w - hint.chars().count() as i32) / 2).max(0);
            let hy = wa.h / 2;
            buf.write_str(hx, hy, hint, t.dim, t.desktop_bg);
            layers.push(Layer { z: 1, origin: Point::new(wa.x, wa.y), buf, opacity: 1.0, scissor: None });
        }

        // Chrome: menubar (simple glyph) + dock.
        let app_name = focused
            .and_then(|id| self.titles.iter().find(|(i, _)| *i == id))
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        let segs = {
            let st = self.tray_state.read().unwrap();
            crate::tray::tray_segments(&st, self.w, self.power_label.chars().count() as i32, self.tray.notif_count())
        };
        layers.push(render_menubar(self.w, &app_name, &segs, true, &self.power_label));
        layers.push(render_dock(self.w, self.h, &self.dock_items()));
        layers.extend(self.render_dock_ctx());

        // Overlays that must still work in simple mode.
        let overlay_start = layers.len();
        layers.extend(self.launcher.render(self.w, self.h).layers);
        {
            let st = self.tray_state.read().unwrap();
            layers.extend(self.tray.render(self.w, self.h, &st).layers);
        }
        if self.help_open {
            layers.extend(crate::help::render_help(self.w, self.h));
        }
        {
            let online = self.tray_state.read().map(|st| st.remotes_online.clone()).unwrap_or_default();
            layers.extend(self.power_menu.render(self.w, self.h, &self.systems, &online));
        }
        layers.extend(self.confirm_close.render(self.w, self.h));
        layers.extend(self.render_compat_dialog());

        // Images for the focused window only, mapped into the work area.
        let mut images = Vec::new();
        if let Some(fid) = focused {
            match self.contents.get(&fid) {
                Some(WinContent::App(aid)) => {
                    let aid = *aid;
                    for pl in self.apphost.placements(aid) {
                        if let Some(&img) = self.app_image_ids.get(&(fid, pl.image_id)) {
                            let x = wa.x + pl.col as i32;
                            let y = wa.y + pl.row as i32;
                            if x >= wa.x + wa.w || y >= wa.y + wa.h {
                                continue;
                            }
                            let cols = pl.cols.min((wa.x + wa.w - x).max(1) as u16);
                            let rows = pl.rows.min((wa.y + wa.h - y).max(1) as u16);
                            images.push(crate::protocol::ImagePlacement {
                                id: img,
                                rect: crate::geometry::Rect::new(x, y, cols as i32, rows as i32),
                                cols,
                                rows,
                                visible: true,
                            });
                        }
                    }
                }
                Some(WinContent::ImageView(v)) => {
                    if let Some(id) = v.image_id() {
                        images.push(crate::protocol::ImagePlacement {
                            id,
                            rect: wa,
                            cols: wa.w.max(1) as u16,
                            rows: wa.h.max(1) as u16,
                            visible: true,
                        });
                    }
                }
                Some(WinContent::FileManager(f)) => {
                    images.extend(f.thumbnail_placements(wa, true));
                }
                _ => {}
            }
        }

        // Drop any image under an open overlay (launcher / tray / help / power
        // menu / confirm-close dialog), since terminals composite graphics over
        // text and would otherwise bleed through the dialog.
        let overlay_rects: Vec<crate::geometry::Rect> = layers[overlay_start..]
            .iter()
            .map(|l| crate::geometry::Rect::new(l.origin.x, l.origin.y, l.buf.width(), l.buf.height()))
            .collect();
        images.retain(|p| overlay_rects.iter().all(|o| o.intersect(p.rect).is_none()));
        sanitize_images(&mut images, self.w, self.h);

        Frame { layers, cursor: Some(self.cursor), images }
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Remove windows whose PTY child has exited.
    ///
    /// Call this once per render loop tick to keep the session consistent with
    /// process state.
    pub fn reap_dead(&mut self) {
        let app_windows: Vec<(WindowId, AppId)> = self
            .contents
            .iter()
            .filter_map(|(id, c)| match c {
                WinContent::App(aid) => Some((*id, *aid)),
                _ => None,
            })
            .collect();
        let dead: Vec<WindowId> = app_windows
            .into_iter()
            .filter(|(_, aid)| !self.apphost.is_alive(*aid))
            .map(|(id, _)| id)
            .collect();
        // If an install window finished, re-scan $PATH and rebuild the launcher so
        // the newly-installed app shows up (and the store sees it as installed)
        // without a daemon restart.
        let install_finished = dead.iter().any(|id| {
            self.titles.iter().any(|(i, t)| i == id && t.starts_with("install:"))
        });
        for id in dead {
            self.close(id);
        }
        if install_finished {
            self.refresh_installed_apps();
        }
    }

    /// The focused app's content rect IFF that app currently captures the
    /// pointer (wants mouse, or alt-scroll). Used by the client to route
    /// in-app mouse events. `None` → all mouse stays on the chrome/WM path.
    pub fn app_mouse_area(&self) -> Option<crate::geometry::Rect> {
        // Any open overlay/menu (launcher, power menu, help, dir-picker, tray
        // popover, desktop menu) owns the mouse via the normal chrome path. Never
        // route clicks into the app while one is showing — otherwise clicking an
        // app in the Go launcher (drawn over a focused mouse-app) is swallowed.
        if self.launcher.is_open()
            || self.help_open
            || self.dirpicker.is_some()
            || self.power_menu.is_open()
            || self.confirm_close.is_open()
            || self.launch_warn.is_open()
            || self.tray.open().is_some()
            || self.desktop.overlay_rect().is_some()
            || self.dock_popup.is_some()
        {
            return None;
        }
        let fid = self.wm.focused()?;
        let aid = match self.contents.get(&fid)? {
            WinContent::App(aid) => *aid,
            _ => return None,
        };
        if !self.apphost.mouse_mode(aid).captures_pointer() {
            return None;
        }
        if self.simple {
            Some(self.simple_content_rect())
        } else {
            let w = self.wm.get(fid)?;
            if w.minimized { return None; }
            Some(w.content_rect())
        }
    }

    /// Forward a passthrough mouse event to the focused app's PTY.
    fn forward_mouse_to_app(&mut self, m: crate::mouse::MouseInput) {
        let Some(area) = self.app_mouse_area() else { return };
        if m.col < area.x || m.col >= area.x + area.w || m.row < area.y || m.row >= area.y + area.h {
            return;
        }
        let Some(fid) = self.wm.focused() else { return };
        let aid = match self.contents.get(&fid) { Some(WinContent::App(aid)) => *aid, _ => return };
        let mode = self.apphost.mouse_mode(aid);
        let local = crate::mouse::MouseInput { col: m.col - area.x, row: m.row - area.y, ..m };
        if let Some(bytes) = crate::mouse::encode(&local, &mode) {
            self.apphost.input(aid, &bytes);
        }
    }

    /// Kill all running apps and clear the session.
    ///
    /// Must be called before dropping the session to ensure no child processes
    /// are orphaned.
    pub fn shutdown(&mut self) {
        for aid in self.apphost.list() {
            self.apphost.kill(aid);
        }
        self.contents.clear();
        self.apphost.shutdown_host();
    }
}

/// Rewrite a CLI-flagged app's command/args into the `sh -lc` wrapper: run the
/// bare binary with `--help` so its usage is visible immediately, then drop
/// into the user's shell (tool still on `$PATH`, ready to invoke properly).
/// `requires_cwd`/`cwd` are untouched by this — the shell still starts wherever
/// the caller picked.
fn cli_wrap(command: &str, args: &[String]) -> (String, Vec<String>) {
    // Quote every word so an arg with spaces/quotes (from user config) can't
    // splice into the script — same rule as every other shell command we build.
    let mut invocation = crate::systems::sh_quote(command);
    for a in args {
        invocation.push(' ');
        invocation.push_str(&crate::systems::sh_quote(a));
    }
    let script = format!("{invocation} --help; exec \"${{SHELL:-sh}}\"");
    ("sh".into(), vec!["-lc".into(), script])
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

/// Manhattan distance (in cells) the pointer must travel from the press point
/// before a titlebar/edge drag starts moving/resizing a window. Below this, the
/// press is treated as a plain click (so it can't untile a window, and lets
/// double-click-to-rename work without nudging the window).
const DRAG_THRESHOLD: i32 = 3;

/// True when `p` is within `threshold` cells of any edge of `work` — the band in
/// which a drag engages grid-cell snapping (interior drags stay floating).
fn near_edge(p: Point, work: Rect, threshold: i32) -> bool {
    p.x - work.x < threshold
        || work.right() - p.x < threshold
        || p.y - work.y < threshold
        || work.bottom() - p.y < threshold
}

/// Drop image placements the client can't address on a `w`×`h` screen: a rect
/// with a negative origin would emit a CUP with a 0/negative parameter, which
/// terminals reject — the image would then be placed at whatever cell the
/// cursor happens to be on (an icon painted over an arbitrary spot, typically
/// seen right after a resize). Origins past the right/bottom edge are dropped
/// too; they would be entirely invisible anyway.
fn sanitize_images(images: &mut Vec<crate::protocol::ImagePlacement>, w: i32, h: i32) {
    images.retain(|p| p.rect.x >= 0 && p.rect.y >= 0 && p.rect.x < w && p.rect.y < h);
}

/// Check the upstream repository for a newer commit than this build.
///
/// Uses `curl` against the GitHub API with a hard timeout so the call can never
/// hang the desktop. Returns a short human-readable status string.
/// One-shot UI reopen hint codes, persisted across a frontend reload.
#[allow(non_upper_case_globals)]
const Ui_SETTINGS_UPDATES: u8 = 1;

/// Path of the reopen-hint file (per-user socket dir, so it's reload-scoped).
fn reopen_hint_path() -> std::path::PathBuf {
    crate::protocol::socket_dir().join("reopen-hint")
}

/// Persist a one-shot UI reopen hint for the next frontend start.
fn write_reopen_hint(code: u8) {
    let _ = std::fs::create_dir_all(crate::protocol::socket_dir());
    let _ = std::fs::write(reopen_hint_path(), [code]);
}

/// Read and consume the reopen hint (deleted after reading).
fn take_reopen_hint() -> Option<u8> {
    let p = reopen_hint_path();
    let code = std::fs::read(&p).ok().and_then(|b| b.first().copied());
    let _ = std::fs::remove_file(&p);
    code
}

/// Accept only well-formed git branch names; anything else falls back to
/// `main`. The branch comes from `config.update_branch`, which a hand-edited
/// config (or a typo) could set to junk — and it's interpolated into both a
/// shell command and a URL below, so a stray quote/space would otherwise
/// produce a broken updater run rather than a clean fallback.
fn sanitize_branch(branch: &str) -> String {
    let ok = !branch.is_empty()
        && branch.len() <= 100
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
        && !branch.starts_with('-')
        && !branch.contains("..");
    if ok { branch.to_string() } else { "main".to_string() }
}

/// The shell command that updates tuiui and reloads. On `main` it prefers the
/// prebuilt release binary (a fast download via `install.sh`, jumping straight
/// to the latest release), falling back to a source build; on any other branch
/// (the dev channel) it builds that branch from git. On success it reloads and
/// EXITS so the updater window auto-closes; only a failure drops to a shell.
fn update_command(branch: &str) -> String {
    let branch = sanitize_branch(branch);
    let branch = branch.as_str();
    let repo = crate::REPO_URL;
    let raw = "https://raw.githubusercontent.com/jaylfc/tuiui";
    // Keep the new binary where the running one lives (cargo bin vs ~/.local/bin).
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "$HOME/.local/bin".into());
    let root_flag = std::env::current_exe().map(|p| cargo_root_flag(&p)).unwrap_or_default();
    let dir = crate::systems::sh_quote(&exe_dir);
    // Reload via the freshly-installed binary's ABSOLUTE path, not a bare
    // `tuiui`. The updater runs in a non-interactive `sh -lc`, whose PATH may
    // not include the install dir (~/.local/bin is added by interactive zsh
    // config, not a login `sh`). If `tuiui reload` isn't found, the install
    // still succeeds but the daemon never restarts onto the new binary — so
    // the running version never changes and the update appears to "keep
    // failing". install.sh lands the binary in `exe_dir` (TUIUI_BIN_DIR), and
    // the cargo `--root` fallback targets the same dir, so `{exe_dir}/tuiui` is
    // the new binary in both paths.
    let reload = crate::systems::sh_quote(&format!("{exe_dir}/tuiui"));
    // Append each step to ~/tuiui-debug.log (same file as dbg_log) so a failed
    // update is visible in the log the user pastes — the install runs in a
    // window whose output is otherwise lost.
    let log = "\"$HOME/tuiui-debug.log\"";
    if branch == "main" {
        format!(
            "clear; echo 'Updating tuiui (latest release)…'; echo; \
echo \"update: install -> {dir}\" >> {log}; \
if TUIUI_BIN_DIR={dir} sh -c 'curl -fsSL {raw}/main/install.sh | sh'; then \
echo \"update: install.sh ok; reloading via {reload}\" >> {log}; echo; echo 'Reloading…'; {reload} reload; exit 0; \
elif cargo install --git {repo}{root_flag} --force; then \
echo \"update: cargo fallback ok; reloading via {reload}\" >> {log}; echo; echo 'Reloading…'; {reload} reload; exit 0; \
else echo \"update: FAILED (install.sh and cargo both failed)\" >> {log}; echo 'Update failed — tuiui not reloaded.'; exec \"$SHELL\"; fi",
        )
    } else {
        format!(
            "clear; echo 'Updating tuiui (branch {branch})…'; echo; \
echo \"update: cargo install (branch {branch})\" >> {log}; \
if cargo install --git {repo} --branch {branch}{root_flag} --force; then \
echo \"update: cargo ok; reloading via {reload}\" >> {log}; echo; echo 'Reloading…'; {reload} reload; exit 0; \
else echo \"update: FAILED (cargo branch {branch})\" >> {log}; echo 'Update failed — tuiui not reloaded.'; exec \"$SHELL\"; fi",
        )
    }
}

/// The ` --root <dir>` flag that steers `cargo install` to the running
/// binary's own `bin/` dir (its parent's parent — cargo appends `/bin` to the
/// root), or an empty string when the binary isn't in a `bin/` dir (leaving
/// cargo's default `~/.cargo/bin`). Co-locating the source build with the
/// binary the user actually runs avoids landing a shadowed copy in a different
/// dir, which would make the in-app update silently appear to do nothing.
fn cargo_root_flag(exe: &std::path::Path) -> String {
    match exe
        .parent()
        .filter(|d| d.file_name() == Some(std::ffi::OsStr::new("bin")))
        .and_then(|d| d.parent())
    {
        Some(root) => format!(" --root {}", crate::systems::sh_quote(&root.display().to_string())),
        None => String::new(),
    }
}

/// Whether a too-old apphost with live apps warrants the safety dialog: an
/// incompatible protocol is only worth interrupting the user for when there
/// are actually apps that would die in a restart.
fn needs_apphost_restart(host_proto: u32, min_compat: u32, app_count: usize) -> bool {
    host_proto < min_compat && app_count > 0
}

/// Minimum spacing between desktop-folder `stat` calls in
/// [`SessionCore::poll_desktop_dir`] — cheap, but no reason to hit the
/// filesystem every tick.
const DESKTOP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether enough time has passed since the last desktop-folder poll to do
/// another one. Pure so the throttle is testable without real sleeps.
fn desktop_poll_due(last_poll: std::time::Instant, now: std::time::Instant) -> bool {
    now.duration_since(last_poll) >= DESKTOP_POLL_INTERVAL
}

/// Whether the desktop folder's modified-time changed since the last observed
/// poll (so it should be rescanned). The very first poll (`last == None`)
/// never counts as a change — it just establishes the baseline, since the
/// desktop was already scanned at startup / by its own actions.
fn desktop_mtime_changed(last: Option<std::time::SystemTime>, current: Option<std::time::SystemTime>) -> bool {
    matches!((last, current), (Some(a), Some(b)) if a != b)
}

/// Rows of the dock right-click context menu (row 2, "Close", renders in the
/// close colour and routes through the confirm-close dialog for apps).
const DOCK_CTX_ROWS: [&str; 4] = ["Minimise", "Maximise", "Close", "Reset size"];

/// The dock context-menu box, just above the dock row, anchored to the clicked
/// pill's x (shared by render + hit-testing so they can never drift).
#[doc(hidden)]
pub fn dock_ctx_rect(anchor_x: i32, w: i32, h: i32) -> Rect {
    let box_w = 14;
    let box_h = DOCK_CTX_ROWS.len() as i32 + 2; // border rows
    let x = anchor_x.clamp(0, (w - box_w).max(0));
    // Sits just above the dock row, but never off the top: on a very short
    // terminal a negative y would push the rows off-screen (unclickable) — and
    // since render and hit-test share this fn, the clamp keeps them aligned.
    let y = (h - 1 - box_h).max(0);
    Rect::new(x, y, box_w, box_h)
}

/// Screen rect of dock context-menu row `i` (the clickable row).
#[doc(hidden)]
pub fn dock_ctx_row_rect(anchor_x: i32, w: i32, h: i32, i: usize) -> Rect {
    let d = dock_ctx_rect(anchor_x, w, h);
    Rect::new(d.x + 1, d.y + 1 + i as i32, d.w - 2, 1)
}

/// The post-update compat dialog box (shared by render + hit-testing).
fn compat_dialog_rect(w: i32, h: i32) -> Rect {
    let box_w = 58.min((w - 2).max(24));
    let box_h = 9.min((h - 1).max(7));
    Rect::new((w - box_w) / 2, ((h - box_h) / 2).max(0), box_w, box_h)
}

/// `(keep-apps, restart-app-server)` button rects of the compat dialog.
fn compat_dialog_buttons(w: i32, h: i32) -> (Rect, Rect) {
    let d = compat_dialog_rect(w, h);
    let by = d.y + d.h - 2;
    let keep = Rect::new(d.x + 2, by, 16, 1);
    let restart_w = 24;
    let restart = Rect::new(d.x + d.w - 2 - restart_w, by, restart_w, 1);
    (keep, restart)
}

/// Latest release tag via the web redirect, dodging the rate-limited REST API.
/// `github.com/OWNER/REPO/releases/latest` 302s to `.../releases/tag/vX.Y.Z`;
/// we read the tag straight out of the redirect's `Location` header. install.sh
/// resolves it the same way, for the same reason: the api.github.com endpoint is
/// rate-limited to 60 req/hour/IP and its 403 was surfacing as a false
/// "Couldn't check (offline?)" (and wedging the updater's install path).
fn latest_release_tag(repo: &str) -> Option<String> {
    let url = format!("https://github.com/{repo}/releases/latest");
    let out = std::process::Command::new("curl")
        .args(["-fsSI", "--max-time", "6", &url])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let headers = String::from_utf8_lossy(&out.stdout);
    let location = headers
        .lines()
        .rfind(|l| l.to_ascii_lowercase().starts_with("location:"))
        .and_then(|l| l.split_once(':').map(|(_, v)| v.trim()))?;
    if !location.contains("/releases/tag/") {
        return None;
    }
    let tag = location.rsplit('/').next()?.trim();
    (!tag.is_empty()).then(|| tag.to_string())
}

/// Check whether a newer build is available, comparing against whatever the
/// update channel would actually install, and reporting it in that channel's
/// own terms:
/// - **main** installs the latest prebuilt *release* (via `install.sh`), so we
///   compare the installed semver against the latest release tag and report
///   versions (`v0.2.0 → v0.2.1`). Comparing against the `main` branch tip is a
///   bug: any commit landed on `main` after the last release shows a permanent
///   "update available" that re-installing the same release can never clear,
///   i.e. an update loop.
/// - **dev** builds the branch tip from source — there is no version there, so
///   we compare the installed commit against the branch tip and report short
///   commit hashes.
fn check_for_updates(branch: &str) -> String {
    let branch = sanitize_branch(branch);
    let branch = branch.as_str();
    let repo = crate::REPO_URL.trim_start_matches("https://github.com/");
    let get_json = |path: &str| -> Option<serde_json::Value> {
        let url = format!("https://api.github.com/repos/{repo}/{path}");
        std::process::Command::new("curl")
            .args(["-fsS", "--max-time", "6", "-H", "User-Agent: tuiui", &url])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
    };
    let str_field = |v: &serde_json::Value, k: &str| {
        v.get(k).and_then(|s| s.as_str()).map(str::to_string)
    };

    if branch == "main" {
        // Release channel: compare versions, show versions. Resolve the tag via
        // the web redirect first (not rate-limited), falling back to the REST API
        // only when the redirect can't be parsed.
        let tag = match latest_release_tag(repo)
            .or_else(|| get_json("releases/latest").and_then(|v| str_field(&v, "tag_name")))
        {
            Some(t) => t,
            None => return "Couldn't check (offline?)".to_string(),
        };
        let latest = tag.trim_start_matches('v');
        let cur = crate::VERSION; // CARGO_PKG_VERSION, e.g. "0.2.1"
        // Only offer the update when the release is *strictly newer*: a plain
        // string compare would prompt a downgrade when the local build is ahead
        // of the latest tag — e.g. mid-release-cut, or a dev/source build whose
        // version outruns the published release. A release tag we can't parse is
        // reported as a check failure rather than silently treated as 0.0.0.
        match semver_tuple(latest) {
            Some(l) if Some(l) > semver_tuple(cur) => {
                format!("Update available: v{cur} → v{latest}")
            }
            Some(_) => format!("Up to date (v{cur})"),
            None => format!("Couldn't check (unexpected release tag '{tag}')"),
        }
    } else {
        // Dev channel: compare commits, show short hashes.
        let short = |s: &str| s.chars().take(7).collect::<String>();
        let latest = get_json(&format!("commits/{branch}")).and_then(|v| str_field(&v, "sha"));
        match latest {
            Some(sha) => {
                let cur = crate::GIT_SHA;
                if cur == "unknown" {
                    format!("Latest is {} on {branch} — reinstall to update", short(&sha))
                } else if sha.starts_with(cur) || cur.starts_with(&short(&sha)) {
                    format!("Up to date ({}) on {branch}", short(cur))
                } else {
                    format!("Update available on {branch}: {} → {}", short(cur), short(&sha))
                }
            }
            None => "Couldn't check (offline?)".to_string(),
        }
    }
}

/// Parse a `MAJOR.MINOR.PATCH` version (a leading `v` is tolerated) into a
/// comparable tuple, or `None` if it isn't exactly that. This is a numeric
/// `major.minor.patch` comparison, *not* full semver: any `-pre`/`+build`
/// suffix is dropped before parsing (tuiui only ever ships plain release
/// versions, so pre-release precedence never comes up). Anything that isn't
/// three numeric components — too few, too many, or non-numeric — is `None`, so
/// a malformed release tag surfaces as a check failure instead of silently
/// comparing as `0.0.0`.
fn semver_tuple(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next().unwrap_or("");
    let mut it = core.split('.');
    let major = it.next()?.trim().parse::<u64>().ok()?;
    let minor = it.next()?.trim().parse::<u64>().ok()?;
    let patch = it.next()?.trim().parse::<u64>().ok()?;
    if it.next().is_some() {
        return None; // a fourth component → not a MAJOR.MINOR.PATCH version
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compat_dialog_arms_only_for_old_hosts_with_apps() {
        // Dormant today: MIN_COMPAT is 0, nothing is older than that.
        assert!(!needs_apphost_restart(0, crate::apphost::proto::MIN_COMPAT, 5));
        // When a future release bumps MIN_COMPAT, old hosts with live apps arm
        // the dialog — but an empty apphost just restarts silently.
        assert!(needs_apphost_restart(0, 1, 3));
        assert!(!needs_apphost_restart(0, 1, 0), "no apps → nothing to protect");
        assert!(!needs_apphost_restart(1, 1, 3), "compatible host → no dialog");
        // The current binary pair is always compatible with itself (a const
        // block so a bad MIN_COMPAT bump fails the BUILD, not just this test).
        const {
            // MIN_COMPAT is 0 today, so this reads as "always true" to clippy —
            // but it's a deliberate build-time guard for future bumps.
            #[allow(clippy::absurd_extreme_comparisons)]
            {
                assert!(crate::apphost::proto::PROTO_VERSION >= crate::apphost::proto::MIN_COMPAT);
            }
        }
    }

    #[test]
    fn desktop_poll_is_throttled() {
        let t0 = std::time::Instant::now();
        assert!(!desktop_poll_due(t0, t0), "no time has passed yet");
        assert!(!desktop_poll_due(t0, t0 + std::time::Duration::from_secs(1)), "under the interval");
        assert!(desktop_poll_due(t0, t0 + DESKTOP_POLL_INTERVAL), "interval elapsed");
        assert!(desktop_poll_due(t0, t0 + std::time::Duration::from_secs(60)), "well past the interval");
    }

    #[test]
    fn desktop_mtime_change_detection() {
        let a = std::time::SystemTime::UNIX_EPOCH;
        let b = a + std::time::Duration::from_secs(1);
        assert!(!desktop_mtime_changed(None, None), "nothing observed yet");
        assert!(!desktop_mtime_changed(None, Some(a)), "first poll only sets the baseline");
        assert!(!desktop_mtime_changed(Some(a), Some(a)), "unchanged mtime");
        assert!(desktop_mtime_changed(Some(a), Some(b)), "mtime moved forward");
        assert!(!desktop_mtime_changed(Some(a), None), "folder vanished: no reload, just note it");
    }

    #[test]
    fn main_update_prefers_prebuilt_release() {
        let cmd = update_command("main");
        assert!(cmd.contains("install.sh"), "fast path is the prebuilt release: {cmd}");
        assert!(cmd.contains("cargo install --git"), "with a source fallback");
        assert!(cmd.contains("/tuiui' reload"), "reloads via the installed binary's absolute path: {cmd}");
        assert!(!cmd.contains("; tuiui reload"), "must not rely on PATH-resolving a bare `tuiui`: {cmd}");
        assert!(cmd.contains("tuiui-debug.log"), "logs each step to the debug log");
        assert!(cmd.contains("exit 0"), "exits so the updater window auto-closes");
        assert!(!cmd.contains("--branch"), "main needs no branch flag");
    }

    #[test]
    fn cargo_root_targets_the_running_bin_dir() {
        use std::path::Path;
        // A binary in a `bin/` dir → cargo is steered to the parent (it appends
        // `/bin`), so the source build lands next to the running binary.
        assert_eq!(cargo_root_flag(Path::new("/home/jay/.local/bin/tuiui")), " --root '/home/jay/.local'");
        assert_eq!(cargo_root_flag(Path::new("/home/jay/.cargo/bin/tuiui")), " --root '/home/jay/.cargo'");
        // Not in a `bin/` dir → no `--root` (leave cargo's default).
        assert_eq!(cargo_root_flag(Path::new("/opt/tuiui/tuiui")), "");
        assert_eq!(cargo_root_flag(Path::new("tuiui")), "");
    }

    #[test]
    fn dev_channel_builds_from_branch() {
        let cmd = update_command("dev");
        assert!(cmd.contains("cargo install --git"), "dev builds from source");
        assert!(cmd.contains("--branch dev"), "on the dev branch: {cmd}");
        assert!(!cmd.contains("install.sh"), "no prebuilt release off main");
    }

    #[test]
    fn junk_branch_falls_back_to_main() {
        // A hand-edited / typo'd config branch must never leak shell syntax or a
        // broken ref into the updater command or the check URL.
        for junk in ["x; rm -rf ~", "a b", "evil'", "--force", "..", ""] {
            assert_eq!(sanitize_branch(junk), "main", "rejected: {junk:?}");
            let cmd = update_command(junk);
            assert!(cmd.contains("install.sh"), "junk → the safe main path: {cmd}");
            assert!(!cmd.contains("rm -rf"), "no injected payload survives: {cmd}");
        }
        // Legitimate branch names are preserved.
        for ok in ["dev", "main", "feature/x", "release-1.2", "v0.2.0"] {
            assert_eq!(sanitize_branch(ok), ok);
        }
    }

    #[test]
    fn semver_tuple_orders_versions_and_never_downgrades() {
        // Newer release than installed → an update is offered.
        assert!(semver_tuple("0.2.2") > semver_tuple("0.2.1"));
        assert!(semver_tuple("0.3.0") > semver_tuple("0.2.9"));
        assert!(semver_tuple("1.0.0") > semver_tuple("0.9.9"));
        // Installed >= latest release → no downgrade prompt.
        assert!(semver_tuple("0.2.0") <= semver_tuple("0.2.1"), "local ahead of release");
        assert!(semver_tuple("0.2.1") <= semver_tuple("0.2.1"), "equal = up to date");
        // Leading v and (ignored) pre-release/build suffixes parse to the core.
        assert_eq!(semver_tuple("v0.2.1"), Some((0, 2, 1)));
        assert_eq!(semver_tuple("0.2.1-rc1"), Some((0, 2, 1)));
        // Anything that isn't exactly MAJOR.MINOR.PATCH is None (→ "couldn't
        // check"), not a silent 0.0.0 / truncation.
        assert_eq!(semver_tuple("0.2"), None, "too few components");
        assert_eq!(semver_tuple("0.2.1.4"), None, "too many components");
        assert_eq!(semver_tuple("not-a-version"), None);
        assert_eq!(semver_tuple(""), None);
        assert_eq!(semver_tuple("v"), None);
    }

    fn placement(x: i32, y: i32) -> crate::protocol::ImagePlacement {
        crate::protocol::ImagePlacement {
            id: 1,
            rect: Rect::new(x, y, 4, 3),
            cols: 4,
            rows: 3,
            visible: true,
        }
    }

    #[test]
    fn sanitize_drops_unaddressable_and_offscreen_placements() {
        let mut images = vec![
            placement(0, 0),     // kept: top-left corner
            placement(-2, 5),    // dropped: negative x (CUP param would be ≤ 0)
            placement(5, -1),    // dropped: negative y
            placement(100, 5),   // dropped: origin past the right edge
            placement(5, 30),    // dropped: origin past the bottom edge
            placement(99, 29),   // kept: last addressable cell
        ];
        sanitize_images(&mut images, 100, 30);
        let origins: Vec<(i32, i32)> = images.iter().map(|p| (p.rect.x, p.rect.y)).collect();
        assert_eq!(origins, vec![(0, 0), (99, 29)]);
    }
}
