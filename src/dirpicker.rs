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

/// Lists the sub-directories of a path. The real impl reads the filesystem;
/// tests inject a fake.
pub trait DirLister: Send {
    /// Sub-directories of `path` as `(name, full_path)`, hidden ones included
    /// only when `show_hidden`.
    fn list_dirs(&self, path: &Path, show_hidden: bool) -> Vec<(String, PathBuf)>;
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
        v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        v
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
        };
        p.roots = p.load_children(&root, 0);
        p
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
