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

    /// Change directory, pushing onto history (truncating any forward entries).
    fn navigate_to(&mut self, dir: PathBuf) {
        self.cwd = dir.clone();
        self.history.truncate(self.hpos + 1);
        self.history.push(dir);
        self.hpos = self.history.len() - 1;
        self.cursor = 0;
        self.reload();
    }

    /// Enter the focused entry: navigate into a directory, or request an open
    /// action for a file (via `openwith::resolve`).
    pub fn activate(&mut self) {
        let Some(entry) = self.entries.get(self.cursor) else { return; };
        let path = entry.path.clone();
        let is_dir = entry.is_dir;
        match resolve(&path, is_dir, &self.handlers) {
            OpenAction::Navigate => self.navigate_to(path),
            OpenAction::Builtin("@image") => {
                self.action = Some(FileManagerAction::OpenImage(path));
            }
            OpenAction::Builtin(_) => {}
            OpenAction::RunApp { command, args } => {
                self.action = Some(FileManagerAction::RunApp { command, args });
            }
            OpenAction::OpenWithMenu => {
                self.overlay = Some(Overlay::OpenWith { idx: self.cursor, sel: 0 });
            }
        }
    }

    pub fn go_parent(&mut self) {
        if let Some(parent) = self.cwd.parent() {
            self.navigate_to(parent.to_path_buf());
        }
    }

    pub fn go_back(&mut self) {
        if self.hpos > 0 {
            self.hpos -= 1;
            self.cwd = self.history[self.hpos].clone();
            self.cursor = 0;
            self.reload();
        }
    }

    pub fn go_forward(&mut self) {
        if self.hpos + 1 < self.history.len() {
            self.hpos += 1;
            self.cwd = self.history[self.hpos].clone();
            self.cursor = 0;
            self.reload();
        }
    }
}

use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::geometry::Point;

const BG: Rgba = Rgba { r: 17, g: 20, b: 29, a: 255 };
const FG: Rgba = Rgba { r: 200, g: 208, b: 220, a: 255 };
const DIM: Rgba = Rgba { r: 120, g: 130, b: 150, a: 255 };
const SEL_BG: Rgba = Rgba { r: 45, g: 58, b: 85, a: 255 };
const ACCENT: Rgba = Rgba { r: 108, g: 182, b: 255, a: 255 };

const SIDEBAR_W: i32 = 16; // left shortcuts column
const TOOLBAR_Y: i32 = 0;  // breadcrumb/toolbar row
const LIST_TOP: i32 = 2;   // first entry row (below toolbar + spacer)
const TILE_W: i32 = 14;    // icon-grid tile width
const TILE_H: i32 = 3;     // icon-grid tile height (glyph row + name row + gap)

/// A click target inside the file-manager content area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Entry(usize),
    Sidebar(usize),
    Back,
    Forward,
    Up,
    ToggleView,
    Crumb(usize),
}

impl<F: FsOps> FileManager<F> {
    /// Sidebar shortcut destinations (label, path), home-relative.
    fn sidebar(&self) -> Vec<(String, PathBuf)> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        [
            ("Home", home.clone()),
            ("Desktop", home.join("Desktop")),
            ("Documents", home.join("Documents")),
            ("Downloads", home.join("Downloads")),
            ("Pictures", home.join("Pictures")),
        ]
        .iter()
        .map(|(l, p)| (l.to_string(), p.clone()))
        .collect()
    }

    fn glyph(entry: &Entry) -> char {
        use crate::openwith::Role::*;
        match entry.role {
            Directory => '\u{1F4C1}', // 📁
            Image => '\u{1F5BC}',     // 🖼
            Audio => '\u{1F3B5}',     // 🎵
            Video => '\u{1F3AC}',     // 🎬
            Archive => '\u{1F4E6}',   // 📦
            Pdf => '\u{1F4D5}',       // 📕
            Code => '\u{1F4C4}',      // 📄
            _ => '\u{1F4C4}',         // 📄
        }
    }

    pub fn render(&self, w: i32, h: i32) -> CellBuffer {
        let mut buf = CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: FG, bg: BG, attrs: Default::default() });

        // Toolbar: back/forward/up + breadcrumb + view toggle.
        buf.write_str(0, TOOLBAR_Y, "\u{25C2} \u{25B8} \u{25B2}", ACCENT, BG); // ◂ ▸ ▲
        let crumb = self.cwd.to_string_lossy().to_string();
        buf.write_str(8, TOOLBAR_Y, &crumb, FG, BG);
        let toggle = match self.view { ViewMode::Icon => "[grid]", ViewMode::List => "[list]" };
        buf.write_str((w - toggle.len() as i32 - 1).max(0), TOOLBAR_Y, toggle, DIM, BG);

        // Sidebar.
        for (i, (label, _)) in self.sidebar().iter().enumerate() {
            buf.write_str(0, LIST_TOP + i as i32, label, DIM, BG);
        }

        let area_x = SIDEBAR_W;
        let area_w = (w - SIDEBAR_W).max(1);

        match self.view {
            ViewMode::List => {
                for (i, e) in self.entries.iter().enumerate() {
                    let y = LIST_TOP + i as i32;
                    if y >= h - 1 { break; }
                    let selected = self.selection.contains(&i);
                    let focused = i == self.cursor;
                    let bg = if selected || focused { SEL_BG } else { BG };
                    for x in area_x..w { buf.set(x, y, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() }); }
                    let mark = if e.is_dir { '\u{1F4C1}' } else { Self::glyph(e) };
                    buf.write_str(area_x, y, &format!("{mark} {}", e.name), if focused { ACCENT } else { FG }, bg);
                }
            }
            ViewMode::Icon => {
                let cols = (area_w / TILE_W).max(1);
                self.cols_per_row.set(cols);
                for (i, e) in self.entries.iter().enumerate() {
                    let col = i as i32 % cols;
                    let row = i as i32 / cols;
                    let x = area_x + col * TILE_W;
                    let y = LIST_TOP + row * TILE_H;
                    if y >= h - 1 { break; }
                    let selected = self.selection.contains(&i);
                    let focused = i == self.cursor;
                    let bg = if selected || focused { SEL_BG } else { BG };
                    buf.set(x + TILE_W / 2, y, Cell { ch: Self::glyph(e), fg: FG, bg, attrs: Default::default() });
                    let name: String = e.name.chars().take((TILE_W - 1) as usize).collect();
                    buf.write_str(x, y + 1, &name, if focused { ACCENT } else { FG }, bg);
                }
            }
        }

        // Status line.
        if !self.status.is_empty() {
            buf.write_str(0, h - 1, &self.status, ACCENT, BG);
        } else {
            let info = format!("{} items", self.entries.len());
            buf.write_str(0, h - 1, &info, DIM, BG);
        }

        // Overlays render on top (Task 7 adds NewFolder/Rename/Confirm/Context/OpenWith/Error).
        self.render_overlay(&mut buf, w, h);
        buf
    }

    /// Map a content-local click to a target. Mirrors the render layout.
    pub fn hit_test(&self, p: Point, w: i32, _h: i32) -> Option<Target> {
        if p.y == TOOLBAR_Y {
            return match p.x {
                0 => Some(Target::Back),
                2 => Some(Target::Forward),
                4 => Some(Target::Up),
                x if x >= w - 7 => Some(Target::ToggleView),
                _ => None,
            };
        }
        if p.x < SIDEBAR_W && p.y >= LIST_TOP {
            let i = (p.y - LIST_TOP) as usize;
            if i < self.sidebar().len() { return Some(Target::Sidebar(i)); }
            return None;
        }
        // Entry area.
        let area_x = SIDEBAR_W;
        let area_w = (w - SIDEBAR_W).max(1);
        match self.view {
            ViewMode::List => {
                let i = (p.y - LIST_TOP) as usize;
                if p.y >= LIST_TOP && i < self.entries.len() { Some(Target::Entry(i)) } else { None }
            }
            ViewMode::Icon => {
                let cols = (area_w / TILE_W).max(1);
                if p.y < LIST_TOP { return None; }
                let col = (p.x - area_x) / TILE_W;
                let row = (p.y - LIST_TOP) / TILE_H;
                if col < 0 || col >= cols { return None; }
                let i = (row * cols + col) as usize;
                if i < self.entries.len() { Some(Target::Entry(i)) } else { None }
            }
        }
    }

    /// Handle a content-local left click. `ctrl`/`shift` modify selection; a click
    /// on a toolbar/sidebar target navigates. Returns true if anything changed.
    pub fn handle_click(&mut self, p: Point, w: i32, h: i32, ctrl: bool, shift: bool) -> bool {
        match self.hit_test(p, w, h) {
            Some(Target::Entry(i)) => { self.select_at(i, ctrl, shift); true }
            Some(Target::Back) => { self.go_back(); true }
            Some(Target::Forward) => { self.go_forward(); true }
            Some(Target::Up) => { self.go_parent(); true }
            Some(Target::ToggleView) => {
                self.view = match self.view { ViewMode::Icon => ViewMode::List, ViewMode::List => ViewMode::Icon };
                true
            }
            Some(Target::Sidebar(i)) => {
                if let Some((_, path)) = self.sidebar().get(i) {
                    if path.is_dir() { self.navigate_to(path.clone()); }
                }
                true
            }
            Some(Target::Crumb(_)) | None => false,
        }
    }

    /// Placeholder overlay renderer (real overlays land in Task 7).
    fn render_overlay(&self, _buf: &mut CellBuffer, _w: i32, _h: i32) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tuiui-fmu-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn render_returns_sized_buffer() {
        let d = tmp("render");
        fs::write(d.join("a.txt"), b"x").unwrap();
        let fm = FileManager::new(d.clone(), BTreeMap::new());
        let buf = fm.render(80, 24);
        assert_eq!(buf.width(), 80);
        assert_eq!(buf.height(), 24);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn view_toggle_switches_modes() {
        let d = tmp("toggle");
        let mut fm = FileManager::new(d.clone(), BTreeMap::new());
        assert_eq!(fm.view(), ViewMode::Icon);
        fm.set_view(ViewMode::List);
        assert_eq!(fm.view(), ViewMode::List);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn click_on_entry_selects_it() {
        let d = tmp("click");
        for n in ["a", "b", "c"] { fs::write(d.join(n), b"").unwrap(); }
        let mut fm = FileManager::new(d.clone(), BTreeMap::new());
        fm.set_view(ViewMode::List); // deterministic 1-per-row layout
        let _ = fm.render(80, 24);   // establish layout rects
        // List rows start at LIST_TOP; second row → index 1. The click x must be
        // inside the entry area (x >= SIDEBAR_W); x < SIDEBAR_W hits the sidebar.
        let target = crate::geometry::Point::new(SIDEBAR_W + 2, LIST_TOP + 1);
        let hit = fm.hit_test(target, 80, 24);
        assert_eq!(hit, Some(Target::Entry(1)));
        let _ = fs::remove_dir_all(&d);
    }
}
