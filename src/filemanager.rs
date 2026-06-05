//! The native file-manager widget: directory state, selection, navigation,
//! overlays, and operation dispatch. All disk access goes through `FsOps`.

use crate::fileops::{Entry, FsOps, StdFs};
use crate::openwith::{resolve, OpenAction};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Icon,
    List,
}

/// What the widget asks the session to do (the session owns windows/PTYs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileManagerAction {
    /// Open the builtin image viewer for this path.
    OpenImage(PathBuf),
    /// Launch a TUI app with these args (the file path already appended).
    RunApp { command: String, args: Vec<String> },
}

/// Modal overlays drawn on top of the listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    NewFolder { name: String },
    Rename { idx: usize, name: String },
    ConfirmDelete { count: usize },
    Context { idx: usize },          // right-click menu for entry `idx`
    OpenWith { idx: usize, sel: usize },
    Error { message: String },
}

#[derive(Clone, Debug)]
struct Clipboard {
    paths: Vec<PathBuf>,
    cut: bool,
}

/// The file manager. Generic over the filesystem backend for testability.
pub struct FileManager<F: FsOps = StdFs> {
    fs: F,
    cwd: PathBuf,
    entries: Vec<Entry>,
    cursor: usize,
    selection: BTreeSet<usize>,
    view: ViewMode,
    show_hidden: bool,
    history: Vec<PathBuf>,
    hpos: usize,
    scroll: i32,
    clipboard: Option<Clipboard>,
    overlay: Option<Overlay>,
    handlers: BTreeMap<String, String>,
    status: String,
    action: Option<FileManagerAction>,
    /// Tiles per row in the last Icon render; navigation uses it. Updated by render.
    cols_per_row: std::cell::Cell<i32>,
}

impl FileManager<StdFs> {
    /// Open a file manager rooted at `cwd` with the given `[default_apps]` map.
    pub fn new(cwd: PathBuf, handlers: BTreeMap<String, String>) -> Self {
        Self::with_fs(StdFs, cwd, handlers)
    }
}

impl<F: FsOps> FileManager<F> {
    pub fn with_fs(fs: F, cwd: PathBuf, handlers: BTreeMap<String, String>) -> Self {
        let mut me = Self {
            fs,
            cwd: cwd.clone(),
            entries: Vec::new(),
            cursor: 0,
            selection: BTreeSet::new(),
            view: ViewMode::Icon,
            show_hidden: false,
            history: vec![cwd],
            hpos: 0,
            scroll: 0,
            clipboard: None,
            overlay: None,
            handlers,
            status: String::new(),
            action: None,
            cols_per_row: std::cell::Cell::new(1),
        };
        me.reload();
        me
    }

    pub fn cwd(&self) -> &Path { &self.cwd }
    pub fn entries(&self) -> &[Entry] { &self.entries }
    pub fn cursor(&self) -> usize { self.cursor }
    pub fn view(&self) -> ViewMode { self.view }
    pub fn status(&self) -> &str { &self.status }
    pub fn overlay(&self) -> Option<&Overlay> { self.overlay.as_ref() }
    pub fn is_editing(&self) -> bool {
        matches!(self.overlay, Some(Overlay::NewFolder { .. }) | Some(Overlay::Rename { .. }))
    }
    pub fn selection_indices(&self) -> Vec<usize> { self.selection.iter().copied().collect() }

    /// Take a pending action requested by the user (cleared on read).
    pub fn take_action(&mut self) -> Option<FileManagerAction> { self.action.take() }

    /// Re-list the current directory, clamping cursor/selection.
    pub fn reload(&mut self) {
        match self.fs.list(&self.cwd, self.show_hidden) {
            Ok(es) => {
                self.entries = es;
                self.status.clear();
            }
            Err(e) => {
                self.entries.clear();
                self.status = format!("Cannot read {}: {}", self.cwd.display(), e);
            }
        }
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        self.selection.clear();
        self.scroll = 0;
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.reload();
    }

    pub fn set_view(&mut self, v: ViewMode) { self.view = v; }

    /// Move the cursor by (dx tiles, dy rows). In List view any nonzero delta is
    /// collapsed to a single ±1 step.
    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        if self.entries.is_empty() { return; }
        let n = self.entries.len() as i32;
        let cur = self.cursor as i32;
        let next = match self.view {
            ViewMode::List => cur + dx.signum() + dy.signum(),
            ViewMode::Icon => {
                let cols = self.cols_per_row.get().max(1);
                cur + dx.signum() + dy.signum() * cols
            }
        };
        self.cursor = next.clamp(0, n - 1) as usize;
    }

    /// Select entry `idx`. `ctrl` toggles it into the set; `shift` selects the
    /// range from the current cursor; neither makes it the sole selection.
    pub fn select_at(&mut self, idx: usize, ctrl: bool, shift: bool) {
        if idx >= self.entries.len() { return; }
        if shift {
            let (lo, hi) = if idx >= self.cursor { (self.cursor, idx) } else { (idx, self.cursor) };
            self.selection = (lo..=hi).collect();
        } else if ctrl {
            if !self.selection.remove(&idx) { self.selection.insert(idx); }
        } else {
            self.selection.clear();
            self.selection.insert(idx);
        }
        self.cursor = idx;
    }
}
