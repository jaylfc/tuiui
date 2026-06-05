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
    Columns,
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
    GetInfo { idx: usize },
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
    /// Entry index → loaded thumbnail ImageId (filled by the session).
    thumbs: std::collections::HashMap<usize, u64>,
    /// Whether the right-hand preview pane is open.
    preview: bool,
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
            thumbs: std::collections::HashMap::new(),
            preview: false,
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
        self.thumbs.clear();
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.reload();
    }

    pub fn set_view(&mut self, v: ViewMode) { self.view = v; }

    /// Cycle Icon -> List -> Columns -> Icon.
    pub fn cycle_view(&mut self) {
        self.view = match self.view {
            ViewMode::Icon => ViewMode::List,
            ViewMode::List => ViewMode::Columns,
            ViewMode::Columns => ViewMode::Icon,
        };
    }

    pub fn preview_open(&self) -> bool { self.preview }
    pub fn toggle_preview(&mut self) { self.preview = !self.preview; }

    /// Width (cells) reserved for the preview pane when it is open.
    fn preview_reserve(&self, w: i32) -> i32 {
        if self.preview { (w / 3).clamp(20, 48) } else { 0 }
    }

    /// The preview body for the focused entry (≤ `max` lines).
    pub fn preview_lines(&self, max: usize) -> Vec<String> {
        let Some(e) = self.entries.get(self.cursor) else { return vec![]; };
        use crate::openwith::Role::*;
        match e.role {
            Text | Code => read_head(&e.path, max),
            Pdf => pdf_preview(&e.path, max),
            Directory => vec![format!("{} \u{2014} folder", e.name)],
            _ => {
                let mut v = vec![
                    format!("Name: {}", e.name),
                    format!("Kind: {}", e.role.label()),
                    format!("Size: {} bytes", e.size),
                ];
                v.truncate(max);
                v
            }
        }
    }

    /// Image entries (index, path) in the current view that should have a thumbnail.
    pub fn thumbnail_requests(&self) -> Vec<(usize, std::path::PathBuf)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.role == crate::openwith::Role::Image)
            .map(|(i, e)| (i, e.path.clone()))
            .collect()
    }

    /// Record a loaded thumbnail id for entry `idx`.
    pub fn set_thumb(&mut self, idx: usize, id: u64) {
        self.thumbs.insert(idx, id);
    }

    /// Placements for loaded thumbnails, in the Icon view's tile grid, offset into
    /// `content` (the window content rect, cells). Only on-screen tiles are emitted.
    pub fn thumbnail_placements(
        &self,
        content: crate::geometry::Rect,
        visible: bool,
    ) -> Vec<crate::protocol::ImagePlacement> {
        // Thumbnails only render in Icon view (List shows glyphs).
        if self.view != ViewMode::Icon {
            return Vec::new();
        }
        let area_x = SIDEBAR_W;
        let area_w = (content.w - SIDEBAR_W - self.preview_reserve(content.w)).max(1);
        let cols = (area_w / TILE_W).max(1);
        let mut out = Vec::new();
        for (&idx, &id) in &self.thumbs {
            if idx >= self.entries.len() {
                continue;
            }
            let col = idx as i32 % cols;
            let row = idx as i32 / cols;
            let cx = content.x + area_x + col * TILE_W;
            let cy = content.y + LIST_TOP + row * TILE_H;
            if cy + 1 >= content.y + content.h {
                continue; // below the viewport
            }
            let rect = crate::geometry::Rect::new(cx, cy, (TILE_W - 1).max(1), 1);
            out.push(crate::protocol::ImagePlacement {
                id,
                rect,
                cols: rect.w.max(1) as u16,
                rows: rect.h.max(1) as u16,
                visible,
            });
        }
        out.sort_by_key(|p| p.id); // deterministic order
        out
    }

    /// Move the cursor by (dx tiles, dy rows). In List view any nonzero delta is
    /// collapsed to a single ±1 step.
    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        if self.entries.is_empty() { return; }
        let n = self.entries.len() as i32;
        let cur = self.cursor as i32;
        let next = match self.view {
            ViewMode::List | ViewMode::Columns => cur + dx.signum() + dy.signum(),
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

    // ---- overlay lifecycle -------------------------------------------------

    pub fn begin_new_folder(&mut self) { self.overlay = Some(Overlay::NewFolder { name: String::new() }); }

    pub fn begin_rename(&mut self) {
        if let Some(e) = self.entries.get(self.cursor) {
            self.overlay = Some(Overlay::Rename { idx: self.cursor, name: e.name.clone() });
        }
    }

    pub fn begin_delete(&mut self) {
        let count = if self.selection.is_empty() { 1 } else { self.selection.len() };
        self.overlay = Some(Overlay::ConfirmDelete { count });
    }

    pub fn begin_context(&mut self) {
        self.overlay = Some(Overlay::Context { idx: self.cursor });
    }

    pub fn begin_get_info(&mut self) {
        self.overlay = Some(Overlay::GetInfo { idx: self.cursor });
    }

    pub fn cancel_overlay(&mut self) { self.overlay = None; }

    pub fn overlay_char(&mut self, c: char) {
        match &mut self.overlay {
            Some(Overlay::NewFolder { name }) | Some(Overlay::Rename { name, .. }) => name.push(c),
            _ => {}
        }
    }

    pub fn overlay_backspace(&mut self) {
        match &mut self.overlay {
            Some(Overlay::NewFolder { name }) | Some(Overlay::Rename { name, .. }) => { name.pop(); }
            _ => {}
        }
    }

    /// Commit a NewFolder or Rename overlay.
    pub fn overlay_commit(&mut self) {
        match self.overlay.take() {
            Some(Overlay::NewFolder { name }) if !name.trim().is_empty() => {
                if let Err(e) = self.fs.mkdir(&self.cwd, name.trim()) {
                    self.status = format!("New folder failed: {e}");
                }
                self.reload();
            }
            Some(Overlay::Rename { idx, name }) if !name.trim().is_empty() => {
                if let Some(entry) = self.entries.get(idx) {
                    if let Err(e) = self.fs.rename(&entry.path.clone(), name.trim()) {
                        self.status = format!("Rename failed: {e}");
                    }
                }
                self.reload();
            }
            _ => {}
        }
    }

    // ---- clipboard ---------------------------------------------------------

    fn selected_paths(&self) -> Vec<PathBuf> {
        if self.selection.is_empty() {
            self.entries.get(self.cursor).map(|e| vec![e.path.clone()]).unwrap_or_default()
        } else {
            self.selection.iter().filter_map(|&i| self.entries.get(i)).map(|e| e.path.clone()).collect()
        }
    }

    pub fn copy_selection(&mut self) {
        let paths = self.selected_paths();
        if !paths.is_empty() { self.clipboard = Some(Clipboard { paths, cut: false }); }
    }

    pub fn cut_selection(&mut self) {
        let paths = self.selected_paths();
        if !paths.is_empty() { self.clipboard = Some(Clipboard { paths, cut: true }); }
    }

    pub fn paste(&mut self) {
        let Some(cb) = self.clipboard.clone() else { return; };
        for src in &cb.paths {
            let r = if cb.cut { self.fs.move_to(src, &self.cwd) } else { self.fs.copy(src, &self.cwd) };
            if let Err(e) = r { self.status = format!("Paste failed: {e}"); }
        }
        if cb.cut { self.clipboard = None; }
        self.reload();
    }

    // ---- delete ------------------------------------------------------------

    pub fn confirm_delete(&mut self) {
        if !matches!(self.overlay, Some(Overlay::ConfirmDelete { .. })) { return; }
        self.overlay = None;
        for src in self.selected_paths() {
            if let Err(e) = self.fs.trash(&src) { self.status = format!("Trash failed: {e}"); }
        }
        self.reload();
    }
}

fn read_head(path: &std::path::Path, max: usize) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => s.lines().take(max).map(|l| l.chars().take(200).collect()).collect(),
        Err(_) => vec!["(binary or unreadable)".into()],
    }
}

fn pdf_preview(path: &std::path::Path, max: usize) -> Vec<String> {
    for tool in ["pdftotext", "mutool"] {
        if crate::catalog::is_installed(tool) {
            let args: Vec<String> = if tool == "pdftotext" {
                vec![path.to_string_lossy().into(), "-".into()]
            } else {
                vec!["draw".into(), "-F".into(), "txt".into(), path.to_string_lossy().into()]
            };
            if let Ok(out) = std::process::Command::new(tool).args(&args).output() {
                let text = String::from_utf8_lossy(&out.stdout);
                let lines: Vec<String> =
                    text.lines().take(max).map(|l| l.chars().take(200).collect()).collect();
                if !lines.is_empty() {
                    return lines;
                }
            }
        }
    }
    vec!["PDF (install pdftotext or mutool for a text preview)".into()]
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
        let toggle = match self.view {
            ViewMode::Icon => "[grid]",
            ViewMode::List => "[list]",
            ViewMode::Columns => "[cols]",
        };
        buf.write_str((w - toggle.len() as i32 - 1).max(0), TOOLBAR_Y, toggle, DIM, BG);

        // Sidebar.
        for (i, (label, _)) in self.sidebar().iter().enumerate() {
            buf.write_str(0, LIST_TOP + i as i32, label, DIM, BG);
        }

        let area_x = SIDEBAR_W;
        let preview_w = self.preview_reserve(w);
        let area_right = (w - preview_w).max(area_x + 1);
        let area_w = (area_right - SIDEBAR_W).max(1);

        match self.view {
            ViewMode::List => {
                for (i, e) in self.entries.iter().enumerate() {
                    let y = LIST_TOP + i as i32;
                    if y >= h - 1 { break; }
                    let selected = self.selection.contains(&i);
                    let focused = i == self.cursor;
                    let bg = if selected || focused { SEL_BG } else { BG };
                    for x in area_x..area_right { buf.set(x, y, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() }); }
                    let mark = if e.is_dir { '\u{1F4C1}' } else { Self::glyph(e) };
                    buf.write_str(area_x, y, &format!("{mark} {}", e.name), if focused { ACCENT } else { FG }, bg);
                }
            }
            ViewMode::Columns => {
                self.render_columns(&mut buf, area_x, area_right, h);
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

        // Preview pane (Icon / List): a vertical separator then the entry's body.
        if preview_w > 0 && self.view != ViewMode::Columns {
            let sep_x = area_right;
            for y in LIST_TOP..h - 1 {
                buf.set(sep_x, y, Cell { ch: '\u{2502}', fg: DIM, bg: BG, attrs: Default::default() });
            }
            let pane_x = sep_x + 2;
            let max = (h - LIST_TOP - 1).max(0) as usize;
            for (i, line) in self.preview_lines(max).iter().enumerate() {
                let y = LIST_TOP + i as i32;
                if y >= h - 1 { break; }
                let text: String = line.chars().take((preview_w - 2).max(1) as usize).collect();
                buf.write_str(pane_x, y, &text, FG, BG);
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

    /// Render the Miller-columns layout: parent | current | preview-of-focused.
    fn render_columns(&self, buf: &mut CellBuffer, area_x: i32, area_right: i32, h: i32) {
        let area_w = (area_right - area_x).max(1);
        let col_w = (area_w / 3).max(1);
        let mid_x = area_x + col_w;
        let right_x = area_x + 2 * col_w;

        // Left: parent directory listing, the current folder highlighted.
        if let Some(parent) = self.cwd.parent() {
            if let Ok(es) = self.fs.list(parent, self.show_hidden) {
                for (i, e) in es.iter().enumerate() {
                    let y = LIST_TOP + i as i32;
                    if y >= h - 1 { break; }
                    let here = e.path == self.cwd;
                    let bg = if here { SEL_BG } else { BG };
                    for x in area_x..mid_x { buf.set(x, y, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() }); }
                    let name: String = e.name.chars().take((col_w - 1).max(1) as usize).collect();
                    buf.write_str(area_x, y, &name, if here { ACCENT } else { DIM }, bg);
                }
            }
        }

        // Middle: current entries, cursor highlighted.
        for (i, e) in self.entries.iter().enumerate() {
            let y = LIST_TOP + i as i32;
            if y >= h - 1 { break; }
            let selected = self.selection.contains(&i);
            let focused = i == self.cursor;
            let bg = if selected || focused { SEL_BG } else { BG };
            for x in mid_x..right_x { buf.set(x, y, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() }); }
            let mark = if e.is_dir { '\u{1F4C1}' } else { Self::glyph(e) };
            let label = format!("{mark} {}", e.name);
            let label: String = label.chars().take((col_w - 1).max(1) as usize).collect();
            buf.write_str(mid_x, y, &label, if focused { ACCENT } else { FG }, bg);
        }

        // Right: preview of the focused entry.
        for x in right_x - 1..right_x { buf.set(x, LIST_TOP, Cell { ch: ' ', fg: FG, bg: BG, attrs: Default::default() }); }
        let max = (h - LIST_TOP - 1).max(0) as usize;
        for (i, line) in self.preview_lines(max).iter().enumerate() {
            let y = LIST_TOP + i as i32;
            if y >= h - 1 { break; }
            let text: String = line.chars().take((area_right - right_x).max(1) as usize).collect();
            buf.write_str(right_x, y, &text, FG, BG);
        }
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
        let area_right = (w - self.preview_reserve(w)).max(area_x + 1);
        let area_w = (area_right - SIDEBAR_W).max(1);
        match self.view {
            ViewMode::List => {
                let i = (p.y - LIST_TOP) as usize;
                if p.y >= LIST_TOP && i < self.entries.len() { Some(Target::Entry(i)) } else { None }
            }
            ViewMode::Columns => {
                // Only the middle column maps to entries; left/right are visual.
                let col_w = (area_w / 3).max(1);
                let mid_x = area_x + col_w;
                if p.y < LIST_TOP || p.x < mid_x || p.x >= mid_x + col_w {
                    return None;
                }
                let i = (p.y - LIST_TOP) as usize;
                if i < self.entries.len() { Some(Target::Entry(i)) } else { None }
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
            Some(Target::ToggleView) => { self.cycle_view(); true }
            Some(Target::Sidebar(i)) => {
                if let Some((_, path)) = self.sidebar().get(i) {
                    if path.is_dir() { self.navigate_to(path.clone()); }
                }
                true
            }
            Some(Target::Crumb(_)) | None => false,
        }
    }

    fn render_overlay(&self, buf: &mut CellBuffer, w: i32, h: i32) {
        let Some(ov) = &self.overlay else { return; };
        match ov {
            Overlay::NewFolder { name } => {
                self.draw_box(buf, w, h, "New folder", &[format!("Name: {name}\u{2588}")]);
            }
            Overlay::Rename { name, .. } => {
                self.draw_box(buf, w, h, "Rename", &[format!("Name: {name}\u{2588}")]);
            }
            Overlay::ConfirmDelete { count } => {
                self.draw_box(
                    buf,
                    w,
                    h,
                    "Move to Trash",
                    &[format!("Trash {count} item(s)? [Enter] Yes  [Esc] No")],
                );
            }
            Overlay::Context { .. } => {
                self.draw_box(buf, w, h, "Actions", &["Open  Rename  Copy  Cut  Delete".to_string()]);
            }
            Overlay::OpenWith { .. } => {
                self.draw_box(buf, w, h, "Open with", &["Pick an app (Enter), Esc to cancel".to_string()]);
            }
            Overlay::Error { message } => {
                self.draw_box(buf, w, h, "Error", std::slice::from_ref(message));
            }
            Overlay::GetInfo { idx } => {
                let Some(e) = self.entries.get(*idx) else { return; };
                let mut lines = vec![format!("Name: {}", e.name)];
                if let Ok(info) = crate::fileops::info(&e.path) {
                    lines.push(format!("Path: {}", info.path.display()));
                    lines.push(format!("Size: {} bytes", info.size));
                    let kind = if info.is_dir { "Folder" } else { e.role.label() };
                    lines.push(format!("Kind: {kind}"));
                    lines.push(format!(
                        "Permissions: {} ({:o})",
                        crate::fileops::mode_rwx(info.mode),
                        info.mode & 0o777
                    ));
                    if let Some(t) = &info.link_target {
                        lines.push(format!("Symlink \u{2192} {}", t.display()));
                    }
                }
                lines.push("[Esc] close".into());
                self.draw_box(buf, w, h, "Get Info", &lines);
            }
        }
    }

    /// Draw a centered modal box `lines.len()+2` rows tall with a title and body.
    fn draw_box(&self, buf: &mut CellBuffer, w: i32, h: i32, title: &str, lines: &[String]) {
        let widest = lines.iter().map(|l| l.len()).max().unwrap_or(0);
        let bw = (title.len().max(widest) as i32 + 4).min(w - 2).max(4);
        let bh = lines.len() as i32 + 2;
        let bx = (w - bw) / 2;
        let by = (h - bh) / 2;
        for y in by..by + bh {
            for x in bx..bx + bw {
                buf.set(x, y, Cell { ch: ' ', fg: FG, bg: SEL_BG, attrs: Default::default() });
            }
        }
        buf.write_str(bx + 2, by, title, ACCENT, SEL_BG);
        for (i, line) in lines.iter().enumerate() {
            buf.write_str(bx + 2, by + 2 + i as i32, line, FG, SEL_BG);
        }
    }
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
        fm.cycle_view();
        assert_eq!(fm.view(), ViewMode::List);
        fm.cycle_view();
        assert_eq!(fm.view(), ViewMode::Columns);
        fm.cycle_view();
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
