//! The working-directory picker: a browsable, lazily-loaded directory tree shown
//! as a daemon-side overlay when launching an app flagged `requires_cwd`.
//!
//! The tree logic is pure and testable: directory listing goes through the
//! [`DirLister`] trait, so unit tests inject a fake filesystem.

use std::path::{Path, PathBuf};

/// The app launch deferred until the user picks a directory.
#[derive(Clone, Debug, Default)]
pub struct PendingLaunch {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// Lists and creates sub-directories of a path. The real impl touches the
/// filesystem; tests inject a fake.
pub trait DirLister: Send {
    /// Sub-directories of `path` as `(name, full_path)`, hidden ones included
    /// only when `show_hidden`.
    fn list_dirs(&self, path: &Path, show_hidden: bool) -> Vec<(String, PathBuf)>;

    /// Create directory `name` inside `parent`, returning its full path.
    fn create_dir(&self, parent: &Path, name: &str) -> std::io::Result<PathBuf>;
}

/// The real filesystem lister.
pub struct FsLister;
impl DirLister for FsLister {
    fn list_dirs(&self, path: &Path, show_hidden: bool) -> Vec<(String, PathBuf)> {
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir(path) {
            for e in rd.flatten() {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = e.file_name().to_string_lossy().to_string();
                    if !show_hidden && name.starts_with('.') {
                        continue;
                    }
                    v.push((name, e.path()));
                }
            }
        }
        v.sort_by_key(|(name, _)| name.to_lowercase());
        v
    }

    fn create_dir(&self, parent: &Path, name: &str) -> std::io::Result<PathBuf> {
        let path = parent.join(name);
        std::fs::create_dir(&path)?;
        Ok(path)
    }
}

struct Node {
    name: String,
    path: PathBuf,
    depth: usize,
    expanded: bool,
    /// Arena indices of loaded children; `None` until first expanded.
    children: Option<Vec<usize>>,
}

/// One row in the flattened, currently-visible tree.
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleRow {
    pub name: String,
    pub path: PathBuf,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
}

/// The directory picker: an arena-backed tree with a selected visible row.
pub struct DirPicker {
    lister: Box<dyn DirLister>,
    arena: Vec<Node>,
    roots: Vec<usize>,
    selected: usize,
    show_hidden: bool,
    pending: PendingLaunch,
    /// `Some(name_buffer)` while the user is typing a new folder name.
    creating: Option<String>,
}

impl DirPicker {
    /// Create a picker rooted at `root`, using the real filesystem.
    pub fn new(root: PathBuf, pending: PendingLaunch) -> Self {
        Self::with_lister(root, pending, Box::new(FsLister))
    }

    /// Create a picker with a custom [`DirLister`] (for tests).
    pub fn with_lister(root: PathBuf, pending: PendingLaunch, lister: Box<dyn DirLister>) -> Self {
        let mut p = Self {
            lister,
            arena: Vec::new(),
            roots: Vec::new(),
            selected: 0,
            show_hidden: false,
            pending,
            creating: None,
        };
        p.roots = p.load_children(&root, 0);
        p
    }

    /// Whether the new-folder name input is active.
    pub fn is_creating(&self) -> bool {
        self.creating.is_some()
    }

    /// Begin creating a new folder inside the highlighted directory.
    pub fn begin_create(&mut self) {
        self.creating = Some(String::new());
    }

    /// Append a character to the new-folder name.
    pub fn create_type(&mut self, c: char) {
        if let Some(buf) = self.creating.as_mut() {
            if c != '/' {
                buf.push(c);
            }
        }
    }

    /// Delete the last character of the new-folder name.
    pub fn create_backspace(&mut self) {
        if let Some(buf) = self.creating.as_mut() {
            buf.pop();
        }
    }

    /// Abandon the new-folder input.
    pub fn cancel_create(&mut self) {
        self.creating = None;
    }

    /// Create the typed folder inside the highlighted directory (or the root when
    /// the tree is empty), then expand the parent and select the new folder.
    pub fn commit_create(&mut self) {
        let Some(name) = self.creating.take() else { return };
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        match self.selected_arena_idx() {
            Some(idx) => {
                let (parent, depth) = (self.arena[idx].path.clone(), self.arena[idx].depth + 1);
                if let Ok(newp) = self.lister.create_dir(&parent, &name) {
                    let kids = self.load_children(&parent, depth);
                    self.arena[idx].children = Some(kids);
                    self.arena[idx].expanded = true;
                    self.select_path(&newp);
                }
            }
            None => {
                let parent = self.root_path();
                if let Ok(newp) = self.lister.create_dir(&parent, &name) {
                    self.arena.clear();
                    self.roots = self.load_children(&parent, 0);
                    self.select_path(&newp);
                }
            }
        }
    }

    /// Move the selection to the visible row with `path`, if present.
    fn select_path(&mut self, path: &Path) {
        if let Some(i) = self.visible().iter().position(|r| r.path == path) {
            self.selected = i;
        }
    }

    /// Load `path`'s sub-directories into the arena, returning their indices.
    fn load_children(&mut self, path: &Path, depth: usize) -> Vec<usize> {
        let listed = self.lister.list_dirs(path, self.show_hidden);
        let mut idxs = Vec::with_capacity(listed.len());
        for (name, p) in listed {
            self.arena.push(Node { name, path: p, depth, expanded: false, children: None });
            idxs.push(self.arena.len() - 1);
        }
        idxs
    }

    /// Pre-order walk of the visible tree.
    fn visible_indices(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for &r in &self.roots {
            self.push_visible(r, &mut out);
        }
        out
    }

    fn push_visible(&self, idx: usize, out: &mut Vec<usize>) {
        out.push(idx);
        let n = &self.arena[idx];
        if n.expanded {
            if let Some(children) = &n.children {
                for &c in children {
                    self.push_visible(c, out);
                }
            }
        }
    }

    /// The flattened, currently-visible rows.
    pub fn visible(&self) -> Vec<VisibleRow> {
        self.visible_indices()
            .into_iter()
            .map(|i| {
                let n = &self.arena[i];
                // Unloaded dirs are assumed expandable; loaded-empty are leaves.
                let has_children = n.children.as_ref().map(|c| !c.is_empty()).unwrap_or(true);
                VisibleRow {
                    name: n.name.clone(),
                    path: n.path.clone(),
                    depth: n.depth,
                    expanded: n.expanded,
                    has_children,
                }
            })
            .collect()
    }

    fn selected_arena_idx(&self) -> Option<usize> {
        self.visible_indices().get(self.selected).copied()
    }

    /// Path of the highlighted directory (falls back to the root when empty).
    pub fn selected_path(&self) -> PathBuf {
        self.selected_arena_idx()
            .map(|i| self.arena[i].path.clone())
            .unwrap_or_default()
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        let n = self.visible_indices().len();
        if n > 0 && self.selected + 1 < n {
            self.selected += 1;
        }
    }

    /// Expand the selected directory (lazy-loading its children on first open).
    pub fn expand(&mut self) {
        let Some(idx) = self.selected_arena_idx() else { return };
        if self.arena[idx].children.is_none() {
            let (path, depth) = (self.arena[idx].path.clone(), self.arena[idx].depth + 1);
            let kids = self.load_children(&path, depth);
            self.arena[idx].children = Some(kids);
        }
        self.arena[idx].expanded = true;
    }

    /// Collapse the selected directory.
    pub fn collapse(&mut self) {
        if let Some(idx) = self.selected_arena_idx() {
            self.arena[idx].expanded = false;
        }
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        // Reload the whole tree under the new visibility.
        let root = self.root_path();
        self.arena.clear();
        self.roots = self.load_children(&root, 0);
        self.selected = 0;
    }

    fn root_path(&self) -> PathBuf {
        self.roots
            .first()
            .and_then(|&i| self.arena.get(i))
            .and_then(|n| n.path.parent().map(Path::to_path_buf))
            .unwrap_or_default()
    }

    /// Resolve the picker: the deferred launch and the chosen directory.
    pub fn confirm(&self) -> (PendingLaunch, PathBuf) {
        (self.pending.clone(), self.selected_path())
    }

    /// The deferred launch (for cancel handling).
    pub fn pending(&self) -> &PendingLaunch {
        &self.pending
    }
}

// ── Rendering + hit-testing ───────────────────────────────────────────────────

use crate::buffer::CellBuffer;
use crate::cell::Cell;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};

/// Maximum tree rows shown at once (the view scrolls to keep the selection in).
const MAX_ROWS: usize = 14;

impl DirPicker {
    /// Box origin, width, height, scroll offset, and number of rows shown for a
    /// `w × h` screen. Shared by `render` and `row_at` so they never disagree.
    fn layout(&self, w: i32, h: i32) -> (Point, i32, i32, usize, usize) {
        let vis_len = self.visible_indices().len();
        let shown = vis_len.min(MAX_ROWS);
        let offset = if self.selected >= MAX_ROWS { self.selected + 1 - MAX_ROWS } else { 0 };
        let box_w = 54.min(w - 4).max(30);
        let box_h = shown as i32 + 4;
        let origin = Point::new((w - box_w) / 2, ((h - box_h) / 2).max(1));
        (origin, box_w, box_h, offset, shown)
    }

    /// Screen rect of visible row `i` (absolute index), if currently on-screen.
    pub fn row_rect(&self, i: usize, w: i32, h: i32) -> Option<Rect> {
        let (origin, box_w, _bh, offset, shown) = self.layout(w, h);
        if i < offset || i >= offset + shown {
            return None;
        }
        let y = origin.y + 2 + (i - offset) as i32;
        Some(Rect::new(origin.x + 1, y, box_w - 2, 1))
    }

    /// The visible-row index a click at `p` lands on, if any.
    pub fn row_at(&self, p: Point, w: i32, h: i32) -> Option<usize> {
        let (origin, box_w, _bh, offset, shown) = self.layout(w, h);
        if p.x < origin.x || p.x >= origin.x + box_w {
            return None;
        }
        let row = p.y - (origin.y + 2);
        if row >= 0 && (row as usize) < shown {
            Some(offset + row as usize)
        } else {
            None
        }
    }

    /// Set the selection to visible row `i` (clamped).
    pub fn select(&mut self, i: usize) {
        let n = self.visible_indices().len();
        if n > 0 {
            self.selected = i.min(n - 1);
        }
    }

    /// Render the picker overlay into compositor layers.
    pub fn render(&self, w: i32, h: i32) -> Vec<Layer> {
        let t = crate::theme::current();
        let (origin, box_w, box_h, offset, shown) = self.layout(w, h);
        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h, &t);

        buf.write_str(2, 0, " Working directory ", t.accent, t.title_focus);
        // Row 1: the new-folder input when creating, else the path breadcrumb.
        if let Some(name) = &self.creating {
            buf.write_str(2, 1, &format!("New folder: {name}█"), t.accent, t.window_bg);
        } else {
            let crumb = self.selected_path();
            let crumb = crumb.to_string_lossy();
            let crumb: String = crumb.chars().rev().take(box_w as usize - 4).collect::<Vec<_>>().into_iter().rev().collect();
            buf.write_str(2, 1, &crumb, t.dim, t.window_bg);
        }

        let vis = self.visible();
        for row in 0..shown {
            let i = offset + row;
            let Some(r) = vis.get(i) else { break };
            let y = 2 + row as i32;
            let sel = i == self.selected;
            let (fg, bg) = if sel { (t.title_fg, t.active_bg) } else { (t.text, t.window_bg) };
            for x in 1..box_w - 1 {
                buf.set(x, y, Cell { ch: ' ', fg, bg, attrs: Default::default() });
            }
            let twig = if r.has_children { if r.expanded { "▾" } else { "▸" } } else { " " };
            let indent = 1 + r.depth as i32 * 2;
            buf.write_str(indent, y, twig, t.accent, bg);
            buf.write_str(indent + 2, y, "📁", t.accent, bg);
            let name_x = indent + 4;
            let avail = (box_w - 1 - name_x).max(1) as usize;
            let name: String = r.name.chars().take(avail).collect();
            buf.write_str(name_x, y, &name, fg, bg);
        }

        let hint = if self.creating.is_some() {
            " Enter create folder · Esc cancel "
        } else {
            " Enter open · →← expand · n new folder · Esc cancel "
        };
        buf.write_str(2, box_h - 1, hint, t.dim, t.window_bg);
        vec![Layer { z: 5200, origin, buf, opacity: 1.0, scissor: None }]
    }
}

/// Fill a buffer with the window background and a rounded border + title bar.
fn fill_box(buf: &mut CellBuffer, w: i32, h: i32, t: &crate::theme::Theme) {
    buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
    for x in 0..w {
        buf.set(x, 0, Cell { ch: ' ', fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
    }
    let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
    for y in 1..h {
        buf.set(0, y, b('│'));
        buf.set(w - 1, y, b('│'));
    }
    for x in 0..w {
        buf.set(x, h - 1, b('─'));
    }
    buf.set(0, h - 1, b('╰'));
    buf.set(w - 1, h - 1, b('╯'));
}
