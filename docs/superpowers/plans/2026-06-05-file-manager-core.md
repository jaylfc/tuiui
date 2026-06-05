# File Manager Core (C, stage 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native, mouse-and-keyboard file manager window (`@files`) that lists a directory as an icon grid or list, navigates folders, opens files via the Default Apps engine (`openwith::resolve`), and performs the core operations — new folder, rename, copy/cut/paste, move, and delete-to-Trash — with no hard deletes.

**Architecture:** A daemon-side `fileops` module (an `FsOps` trait + `StdFs` impl) owns all disk I/O behind a testable seam. A `filemanager` widget owns UI state (cwd, entries, cursor, selection, view, clipboard, overlay) and renders to a `CellBuffer`, mirroring the existing `Store`/`Settings` widget contract. `session.rs` embeds it as `WinContent::FileManager`, routes `ClientMsg::FileManager*` messages to it, and turns its `take_action()` requests into real effects (navigate / open builtin / launch app). `client.rs` routes keys when `Flags.filemanager_focused`.

**Tech Stack:** Rust; `std::fs`; existing `openwith` (B, done), `config`, `buffer`/`cell`, `geometry`, `session`/`protocol`/`client`. No new crates.

**Reference spec:** `docs/superpowers/specs/2026-06-05-file-manager-default-apps-design.md` (sections "C — File manager UI", "Interactions", "File operations & Trash", "Open-with flow", "Session / protocol wiring"). This plan implements build-sequence steps 2–7 + the session/protocol/client wiring (step 13). Thumbnails, tabs, preview pane, Columns view, and Get-Info are **stage 2** (`file-manager-rich`), explicitly out of scope here.

**Out of scope for this plan (stage 2):** image thumbnails (A1 placements), tabs, the preview pane, the Miller-columns view, and the Get-Info/permissions overlay. Core ships Icon + List views only.

---

## File Structure

- **Create `src/fileops.rs`** — `Entry`, `FsOps` trait, `StdFs`, pure helpers (`unique_destination`, `trash_dir`). One responsibility: filesystem listing + mutation behind a trait.
- **Create `src/filemanager.rs`** — the `FileManager` widget: state, Icon/List render, hit-testing, navigation, selection, overlays, operation dispatch, and a `FileManagerAction` request enum. No direct disk I/O except through an `FsOps`.
- **Modify `src/lib.rs`** — `pub mod fileops;` and `pub mod filemanager;`.
- **Modify `src/protocol.rs`** — add `Flags.filemanager_focused: bool`.
- **Modify `src/session.rs`** — `WinContent::FileManager`, `ClientMsg::FileManager*` variants, `filemanager_win` field, `open_filemanager()`, focus helpers, `apply()` branches, `take_action` → effect dispatch, `@files` launcher pin, `Flags` population in the daemon.
- **Modify `src/client.rs`** — `} else if f.filemanager_focused { … }` key routing + scroll.
- **Modify `src/config.rs`** — `filemanager_view: Option<String>` (persist default view) — small, optional.
- **Tests:** `tests/fileops_tests.rs`, `tests/filemanager_tests.rs`, and additions to `tests/session_tests.rs`; plus inline `#[cfg(test)] mod tests` in `filemanager.rs`.

---

## Conventions (read before starting)

- Set PATH before any cargo command: `export PATH="$HOME/.cargo/bin:$PATH"`.
- Build before commit. Quality bar per task: `cargo build --offline` clean, the task's tests pass. After the final task run the full gate: `cargo build --offline && cargo test --offline && cargo clippy --offline --all-targets` → 0 warnings, all green.
- Commit per task with the exact message given. No AI attribution, no Co-Authored-By lines. Branch: `main`.
- Tests that touch disk use a unique temp dir under `std::env::temp_dir()` keyed by `std::process::id()` and clean up with `let _ = std::fs::remove_dir_all(&dir);` — follow the pattern already in `tests/session_tests.rs` (the `image_window_emits_a_visible_placement` test).
- `openwith` public API (already built): `Role` (Image/Video/Audio/Text/Code/Archive/Pdf/Directory/Executable/Other), `classify(&Path, is_dir) -> Role`, `resolve(&Path, is_dir, &BTreeMap<String,String>) -> OpenAction`, `OpenAction::{Navigate, Builtin(&'static str), RunApp{command,args}, OpenWithMenu}`, `candidates(Role) -> Vec<String>`.

---

### Task 1: `fileops` — `Entry`, `FsOps` trait, `StdFs::list`

**Files:** Create `src/fileops.rs`; Modify `src/lib.rs`; Test `tests/fileops_tests.rs`.

- [ ] **Step 1: Write the failing test** (`tests/fileops_tests.rs`):

```rust
use std::fs;
use tuiui::fileops::{FsOps, StdFs};

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tuiui-fo-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn list_sorts_dirs_first_then_name_and_hides_dotfiles() {
    let d = tmp("list");
    fs::create_dir(d.join("zeta")).unwrap();
    fs::write(d.join("alpha.txt"), b"hi").unwrap();
    fs::write(d.join(".secret"), b"x").unwrap();
    fs::create_dir(d.join("apps")).unwrap();

    let fs_ops = StdFs;
    let shown = fs_ops.list(&d, false).unwrap();
    let names: Vec<&str> = shown.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["apps", "zeta", "alpha.txt"]); // dirs first (name-sorted), then files; dotfile hidden

    let all = fs_ops.list(&d, true).unwrap();
    assert!(all.iter().any(|e| e.name == ".secret"));

    let alpha = shown.iter().find(|e| e.name == "alpha.txt").unwrap();
    assert!(!alpha.is_dir);
    assert_eq!(alpha.size, 2);
    assert_eq!(alpha.role, tuiui::openwith::Role::Text);

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn list_of_unreadable_or_missing_dir_is_err_not_panic() {
    let fs_ops = StdFs;
    assert!(fs_ops.list(std::path::Path::new("/no/such/dir/here"), false).is_err());
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --test fileops_tests`): module missing.

- [ ] **Step 3: Implement `src/fileops.rs`:**

```rust
//! Filesystem listing and mutation behind a testable `FsOps` trait. The real
//! `StdFs` impl talks to disk; the file manager only ever calls through the
//! trait, so its logic is unit-testable and disk I/O is isolated here.

use crate::openwith::{classify, Role};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A single directory entry the file manager displays.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub role: Role,
}

/// All filesystem effects the file manager needs, behind a trait so the UI is
/// testable with a fake and the real impl is the only thing touching disk.
pub trait FsOps {
    /// List `dir`, directories first then files, each group sorted case-insensitively
    /// by name. Hidden (dot) entries are included only when `show_hidden`.
    fn list(&self, dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>>;
    /// Create `name` under `parent`; returns the new path.
    fn mkdir(&self, parent: &Path, name: &str) -> io::Result<PathBuf>;
    /// Rename `path` to `new_name` (same parent); returns the new path.
    fn rename(&self, path: &Path, new_name: &str) -> io::Result<PathBuf>;
    /// Copy `src` into `dst_dir` (recursive for directories), de-duping the name.
    fn copy(&self, src: &Path, dst_dir: &Path) -> io::Result<PathBuf>;
    /// Move `src` into `dst_dir` (rename, falling back to copy+remove across devices).
    fn move_to(&self, src: &Path, dst_dir: &Path) -> io::Result<PathBuf>;
    /// Move `path` to the OS Trash. Never hard-deletes.
    fn trash(&self, path: &Path) -> io::Result<()>;
}

/// The production filesystem implementation.
pub struct StdFs;

impl FsOps for StdFs {
    fn list(&self, dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        for ent in std::fs::read_dir(dir)? {
            let ent = ent?;
            let name = ent.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let meta = ent.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta.as_ref().and_then(|m| m.modified().ok());
            let path = ent.path();
            let role = classify(&path, is_dir);
            out.push(Entry { name, path, is_dir, size, modified, role });
        }
        out.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir) // dirs first (true > false)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    fn mkdir(&self, parent: &Path, name: &str) -> io::Result<PathBuf> {
        let p = parent.join(name);
        std::fs::create_dir(&p)?;
        Ok(p)
    }

    fn rename(&self, path: &Path, new_name: &str) -> io::Result<PathBuf> {
        let parent = path.parent().unwrap_or(Path::new("."));
        let dst = parent.join(new_name);
        std::fs::rename(path, &dst)?;
        Ok(dst)
    }

    fn copy(&self, src: &Path, dst_dir: &Path) -> io::Result<PathBuf> {
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item")
            .to_string();
        let dst = unique_destination(dst_dir, &name);
        copy_recursive(src, &dst)?;
        Ok(dst)
    }

    fn move_to(&self, src: &Path, dst_dir: &Path) -> io::Result<PathBuf> {
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item")
            .to_string();
        let dst = unique_destination(dst_dir, &name);
        match std::fs::rename(src, &dst) {
            Ok(()) => Ok(dst),
            Err(_) => {
                // Cross-device: copy then remove the source.
                copy_recursive(src, &dst)?;
                if src.is_dir() {
                    std::fs::remove_dir_all(src)?;
                } else {
                    std::fs::remove_file(src)?;
                }
                Ok(dst)
            }
        }
    }

    fn trash(&self, path: &Path) -> io::Result<()> {
        let dir = trash_dir().ok_or_else(|| io::Error::other("no trash directory"))?;
        std::fs::create_dir_all(&dir)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item")
            .to_string();
        let dst = unique_destination(&dir, &name);
        match std::fs::rename(path, &dst) {
            Ok(()) => Ok(()),
            Err(_) => {
                copy_recursive(path, &dst)?;
                if path.is_dir() {
                    std::fs::remove_dir_all(path)?;
                } else {
                    std::fs::remove_file(path)?;
                }
                Ok(())
            }
        }
    }
}

/// Recursively copy a file or directory tree from `src` to `dst`.
fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for ent in std::fs::read_dir(src)? {
            let ent = ent?;
            copy_recursive(&ent.path(), &dst.join(ent.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// A non-colliding destination path for `name` inside `dir`: `name`, then
/// `name copy`, `name copy 2`, … (preserving the extension for files).
pub fn unique_destination(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = split_name(name);
    for n in 1.. {
        let suffix = if n == 1 { " copy".to_string() } else { format!(" copy {n}") };
        let trial = if ext.is_empty() {
            format!("{stem}{suffix}")
        } else {
            format!("{stem}{suffix}.{ext}")
        };
        let p = dir.join(trial);
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

/// Split a filename into (stem, extension-without-dot). Dotfiles with no further
/// extension keep their whole name as the stem.
fn split_name(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (name.to_string(), String::new()),
    }
}

/// The OS Trash directory for moved-not-deleted files.
pub fn trash_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    if cfg!(target_os = "macos") {
        Some(home.join(".Trash"))
    } else {
        Some(home.join(".local/share/Trash/files"))
    }
}
```

Register in `src/lib.rs`: add `pub mod fileops;` (near `pub mod openwith;`).

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test fileops_tests`).

- [ ] **Step 5: Commit:**

```bash
git add src/fileops.rs src/lib.rs tests/fileops_tests.rs
git commit -m "fileops: Entry + FsOps trait + StdFs::list (dirs-first, dotfile filter)"
```

---

### Task 2: `fileops` mutations — mkdir, rename, copy, move, unique naming

**Files:** `src/fileops.rs` (already implemented in Task 1); Test `tests/fileops_tests.rs` (append).

The implementations exist from Task 1; this task adds the tests that lock their behaviour. (If any test fails, fix the impl in `fileops.rs`.)

- [ ] **Step 1: Append tests:**

```rust
#[test]
fn mkdir_rename_copy_move_roundtrip() {
    let d = tmp("mut");
    let fs_ops = StdFs;

    let made = fs_ops.mkdir(&d, "Projects").unwrap();
    assert!(made.is_dir());

    fs::write(d.join("a.txt"), b"hello").unwrap();
    let renamed = fs_ops.rename(&d.join("a.txt"), "b.txt").unwrap();
    assert!(renamed.ends_with("b.txt"));
    assert!(!d.join("a.txt").exists());

    // copy b.txt into Projects, then copy again → de-duped name
    let c1 = fs_ops.copy(&d.join("b.txt"), &made).unwrap();
    assert_eq!(c1.file_name().unwrap(), "b.txt");
    let c2 = fs_ops.copy(&d.join("b.txt"), &made).unwrap();
    assert_eq!(c2.file_name().unwrap(), "b copy.txt");

    // move b.txt into Projects (now original gone from root)
    let moved = fs_ops.move_to(&d.join("b.txt"), &made).unwrap();
    assert!(moved.exists());
    assert!(!d.join("b.txt").exists());

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn copy_is_recursive_for_directories() {
    let d = tmp("rec");
    let fs_ops = StdFs;
    let src = fs_ops.mkdir(&d, "tree").unwrap();
    fs::create_dir(src.join("sub")).unwrap();
    fs::write(src.join("sub/leaf.txt"), b"x").unwrap();
    let into = fs_ops.mkdir(&d, "dest").unwrap();

    let copied = fs_ops.copy(&src, &into).unwrap();
    assert!(copied.join("sub/leaf.txt").exists());

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn unique_destination_suffixes_extension_correctly() {
    use tuiui::fileops::unique_destination;
    let d = tmp("uniq");
    fs::write(d.join("note.md"), b"x").unwrap();
    let p = unique_destination(&d, "note.md");
    assert_eq!(p.file_name().unwrap(), "note copy.md");

    let p2 = unique_destination(&d, "fresh.md");
    assert_eq!(p2.file_name().unwrap(), "fresh.md"); // no collision → unchanged

    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → PASS** (`cargo test --offline --test fileops_tests`). Fix `fileops.rs` if any fail.

- [ ] **Step 3: Commit:**

```bash
git add tests/fileops_tests.rs
git commit -m "fileops: tests for mkdir/rename/copy/move + unique naming"
```

---

### Task 3: `fileops` Trash routing

**Files:** `src/fileops.rs` (impl from Task 1); Test `tests/fileops_tests.rs` (append).

- [ ] **Step 1: Append tests:**

```rust
#[test]
fn trash_dir_is_os_appropriate() {
    use tuiui::fileops::trash_dir;
    let p = trash_dir().expect("home dir resolvable in test env");
    let s = p.to_string_lossy();
    if cfg!(target_os = "macos") {
        assert!(s.ends_with("/.Trash"), "macOS trash is ~/.Trash, got {s}");
    } else {
        assert!(s.ends_with("Trash/files"), "Linux trash is XDG Trash/files, got {s}");
    }
}

#[test]
fn trash_moves_file_out_of_source_dir() {
    // Verify trash() removes the source without hard-deleting. We point at the
    // real OS trash but immediately clean up our marker file from it.
    let d = tmp("trash");
    let fs_ops = StdFs;
    let marker = format!("tuiui-trash-marker-{}.txt", std::process::id());
    let victim = d.join(&marker);
    fs::write(&victim, b"bye").unwrap();

    fs_ops.trash(&victim).unwrap();
    assert!(!victim.exists(), "source file should be gone after trashing");

    // Clean our marker out of the real trash so we don't litter.
    if let Some(td) = tuiui::fileops::trash_dir() {
        let _ = fs::remove_file(td.join(&marker));
        let _ = fs::remove_file(td.join(format!("{marker} copy")));
    }
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → PASS** (`cargo test --offline --test fileops_tests`).

- [ ] **Step 3: Commit:**

```bash
git add tests/fileops_tests.rs
git commit -m "fileops: Trash routing tests (OS-appropriate, no hard delete)"
```

---

### Task 4: `FileManager` model — state, listing, cursor, selection

**Files:** Create `src/filemanager.rs`; Modify `src/lib.rs`; Test `tests/filemanager_tests.rs`.

The widget is generic over `FsOps` so tests can drive it against a temp dir with `StdFs`. It holds the role→handler map (a clone of `config.default_apps`) so it can `resolve()` without the session.

- [ ] **Step 1: Write the failing test** (`tests/filemanager_tests.rs`):

```rust
use std::collections::BTreeMap;
use std::fs;
use tuiui::filemanager::{FileManager, ViewMode};

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tuiui-fm-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn new_lists_root_dirs_first() {
    let d = tmp("new");
    fs::create_dir(d.join("sub")).unwrap();
    fs::write(d.join("a.txt"), b"x").unwrap();

    let fm = FileManager::new(d.clone(), BTreeMap::new());
    assert_eq!(fm.cwd(), d.as_path());
    let names: Vec<&str> = fm.entries().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["sub", "a.txt"]);
    assert_eq!(fm.cursor(), 0);
    assert_eq!(fm.view(), ViewMode::Icon);

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn cursor_moves_and_clamps() {
    let d = tmp("cursor");
    fs::write(d.join("a"), b"").unwrap();
    fs::write(d.join("b"), b"").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    assert_eq!(fm.cursor(), 0);
    fm.move_cursor(1, 0); // down one (dx, dy) — see note below
    assert_eq!(fm.cursor(), 1);
    fm.move_cursor(5, 0); // clamps
    assert_eq!(fm.cursor(), 1);
    fm.move_cursor(-9, 0);
    assert_eq!(fm.cursor(), 0);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn selection_single_ctrl_and_clear() {
    let d = tmp("sel");
    for n in ["a", "b", "c"] { fs::write(d.join(n), b"").unwrap(); }
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.select_at(0, false, false); // click a → {0}
    assert_eq!(fm.selection_indices(), vec![0]);
    fm.select_at(2, true, false);  // ctrl-click c → {0,2}
    assert_eq!(fm.selection_indices(), vec![0, 2]);
    fm.select_at(1, false, false); // plain click b → {1}
    assert_eq!(fm.selection_indices(), vec![1]);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn toggle_hidden_reloads() {
    let d = tmp("hidden");
    fs::write(d.join(".dot"), b"").unwrap();
    fs::write(d.join("v"), b"").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    assert_eq!(fm.entries().len(), 1);
    fm.toggle_hidden();
    assert_eq!(fm.entries().len(), 2);
    let _ = fs::remove_dir_all(&d);
}
```

> Note on `move_cursor(dx, dy)`: in Icon view a row holds several tiles, so movement is 2-D; `dy != 0` moves by a whole row (`cols_per_row`), `dx != 0` moves by one tile. List view treats every move as ±1. Tests above only use `dx` so they hold in both views.

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --test filemanager_tests`).

- [ ] **Step 3: Implement `src/filemanager.rs`** (model portion; render/hit-test come in Task 5–6). Use `StdFs` as the default backend:

```rust
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
```

Register in `src/lib.rs`: add `pub mod filemanager;`.

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test filemanager_tests`).

- [ ] **Step 5: Commit:**

```bash
git add src/filemanager.rs src/lib.rs tests/filemanager_tests.rs
git commit -m "filemanager: model — listing, cursor, selection, hidden toggle"
```

---

### Task 5: Navigation + open-with resolution

**Files:** `src/filemanager.rs` (append to `impl<F: FsOps>`); Test `tests/filemanager_tests.rs` (append).

- [ ] **Step 1: Append tests:**

```rust
#[test]
fn enter_directory_navigates_and_back_returns() {
    let d = tmp("nav");
    fs::create_dir(d.join("sub")).unwrap();
    fs::write(d.join("sub/inner.txt"), b"x").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    // cursor on "sub" (dirs first) → activate navigates in
    fm.activate();
    assert_eq!(fm.cwd(), d.join("sub"));
    assert_eq!(fm.entries().len(), 1);
    fm.go_back();
    assert_eq!(fm.cwd(), d.as_path());
    fm.go_forward();
    assert_eq!(fm.cwd(), d.join("sub"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn parent_navigates_up() {
    let d = tmp("up");
    fs::create_dir(d.join("child")).unwrap();
    let mut fm = FileManager::new(d.join("child"), BTreeMap::new());
    fm.go_parent();
    assert_eq!(fm.cwd(), d.as_path());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn activate_file_requests_open_action() {
    use tuiui::filemanager::FileManagerAction;
    let d = tmp("open");
    fs::write(d.join("notes.md"), b"# hi").unwrap();
    let mut handlers = BTreeMap::new();
    handlers.insert("text".to_string(), "vi".to_string());
    let mut fm = FileManager::new(d.clone(), handlers);
    // only one entry, the file
    fm.activate();
    match fm.take_action() {
        Some(FileManagerAction::RunApp { command, args }) => {
            assert_eq!(command, "vi");
            assert!(args.last().unwrap().ends_with("notes.md"));
        }
        other => panic!("expected RunApp, got {other:?}"),
    }
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn activate_image_requests_open_image() {
    use tuiui::filemanager::FileManagerAction;
    let d = tmp("img");
    fs::write(d.join("p.png"), b"\x89PNG").unwrap(); // ext is enough for classify
    let mut handlers = BTreeMap::new();
    handlers.insert("image".to_string(), "@image".to_string());
    let mut fm = FileManager::new(d.clone(), handlers);
    fm.activate();
    assert!(matches!(fm.take_action(), Some(FileManagerAction::OpenImage(_))));
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Append navigation to `impl<F: FsOps>` in `src/filemanager.rs`:**

```rust
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
```

> Implementation note: `navigate_to` calls `reload()` which clears `selection` and resets `scroll` — keep it that way so a fresh directory starts clean.

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/filemanager.rs tests/filemanager_tests.rs
git commit -m "filemanager: navigation (enter/back/forward/parent) + open-with resolve"
```

---

### Task 6: Render (Icon + List) + hit-testing + view toggle

**Files:** `src/filemanager.rs` (append render/hit-test); inline `#[cfg(test)] mod tests`.

The render mirrors `Store`/`Settings`: top toolbar/breadcrumb row, a left sidebar of shortcuts, the entry area, and a status line. Hit-testing maps a content-local `Point` to a target. `render` records `cols_per_row` for Icon navigation.

- [ ] **Step 1: Write the failing inline test** (append a `mod tests` at the bottom of `src/filemanager.rs`):

```rust
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
        // List rows start at LIST_TOP; second row → index 1.
        let target = crate::geometry::Point::new(4, LIST_TOP + 1);
        let hit = fm.hit_test(target, 80, 24);
        assert_eq!(hit, Some(Target::Entry(1)));
        let _ = fs::remove_dir_all(&d);
    }
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --lib filemanager`).

- [ ] **Step 3: Implement render + hit-testing.** Append to `src/filemanager.rs`:

```rust
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
```

- [ ] **Step 4: Run → PASS** (`cargo test --offline --lib filemanager`).

- [ ] **Step 5: Commit:**

```bash
git add src/filemanager.rs
git commit -m "filemanager: Icon + List render, hit-testing, view toggle, click nav"
```

---

### Task 7: Operations + overlays (new folder, rename, copy/cut/paste, delete→Trash, context, open-with)

**Files:** `src/filemanager.rs`; Test `tests/filemanager_tests.rs` (append).

- [ ] **Step 1: Append tests:**

```rust
#[test]
fn new_folder_overlay_creates_directory() {
    let d = tmp("mkdir");
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.begin_new_folder();
    assert!(fm.is_editing());
    for c in "Projects".chars() { fm.overlay_char(c); }
    fm.overlay_commit();
    assert!(!fm.is_editing());
    assert!(d.join("Projects").is_dir());
    assert!(fm.entries().iter().any(|e| e.name == "Projects"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn rename_overlay_renames_focused() {
    let d = tmp("rename");
    fs::write(d.join("old.txt"), b"x").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.select_at(0, false, false);
    fm.begin_rename();
    for _ in 0.."old.txt".len() { fm.overlay_backspace(); }
    for c in "new.txt".chars() { fm.overlay_char(c); }
    fm.overlay_commit();
    assert!(d.join("new.txt").exists());
    assert!(!d.join("old.txt").exists());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn copy_paste_duplicates_into_cwd() {
    let d = tmp("paste");
    fs::write(d.join("f.txt"), b"x").unwrap();
    let sub = d.join("sub");
    fs::create_dir(&sub).unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    // select f.txt (index 1: sub dir first, then f.txt)
    let i = fm.entries().iter().position(|e| e.name == "f.txt").unwrap();
    fm.select_at(i, false, false);
    fm.copy_selection();
    // enter sub and paste
    let si = fm.entries().iter().position(|e| e.name == "sub").unwrap();
    fm.select_at(si, false, false);
    fm.activate();
    fm.paste();
    assert!(sub.join("f.txt").exists());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn delete_moves_to_trash_after_confirm() {
    let d = tmp("del");
    let marker = format!("tuiui-fm-del-{}.txt", std::process::id());
    fs::write(d.join(&marker), b"x").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.select_at(0, false, false);
    fm.begin_delete();
    assert!(matches!(fm.overlay(), Some(tuiui::filemanager::Overlay::ConfirmDelete { .. })));
    fm.confirm_delete();
    assert!(!d.join(&marker).exists());
    if let Some(td) = tuiui::fileops::trash_dir() {
        let _ = fs::remove_file(td.join(&marker));
    }
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement operations.** Append to `impl<F: FsOps>` in `src/filemanager.rs`:

```rust
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
```

Now wire the real overlay renderer — replace the placeholder `render_overlay` body:

```rust
    fn render_overlay(&self, buf: &mut CellBuffer, w: i32, h: i32) {
        let Some(ov) = &self.overlay else { return; };
        let (title, body): (String, String) = match ov {
            Overlay::NewFolder { name } => ("New folder".into(), format!("Name: {name}\u{2588}")),
            Overlay::Rename { name, .. } => ("Rename".into(), format!("Name: {name}\u{2588}")),
            Overlay::ConfirmDelete { count } => ("Move to Trash".into(), format!("Trash {count} item(s)? [Enter] Yes  [Esc] No")),
            Overlay::Context { .. } => ("Actions".into(), "Open  Rename  Copy  Cut  Delete".into()),
            Overlay::OpenWith { .. } => ("Open with".into(), "Pick an app (Enter), Esc to cancel".into()),
            Overlay::Error { message } => ("Error".into(), message.clone()),
        };
        let bw = (title.len().max(body.len()) as i32 + 4).min(w - 2);
        let bx = (w - bw) / 2;
        let by = h / 2 - 1;
        for y in by..by + 4 {
            for x in bx..bx + bw {
                buf.set(x, y, Cell { ch: ' ', fg: FG, bg: SEL_BG, attrs: Default::default() });
            }
        }
        buf.write_str(bx + 2, by, &title, ACCENT, SEL_BG);
        buf.write_str(bx + 2, by + 2, &body, FG, SEL_BG);
    }
```

> Implementation note: clippy may flag `entry.path.clone()` inside the `rename` arm — borrow ends before `self.fs.rename`, but cloning is simplest and avoids a borrow conflict with `&mut self`. Keep the clone.

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test filemanager_tests` and `--lib filemanager`).

- [ ] **Step 5: Commit:**

```bash
git add src/filemanager.rs tests/filemanager_tests.rs
git commit -m "filemanager: operations + overlays (mkdir/rename/copy/cut/paste/trash)"
```

---

### Task 8: Protocol + session wiring (`WinContent::FileManager`, `@files`, effect dispatch)

**Files:** `src/protocol.rs`, `src/session.rs`, `src/config.rs`; Test `tests/session_tests.rs` (append).

- [ ] **Step 1: Append the failing session test** (`tests/session_tests.rs`):

```rust
#[test]
fn open_file_manager_creates_focused_window_and_is_single_instance() {
    let mut core = SessionCore::new(120, 40, Config::default());
    assert!(!core.focused_is_filemanager());
    core.apply(ClientMsg::OpenFileManager);
    assert!(core.focused_is_filemanager());
    let n = core.window_count();
    core.apply(ClientMsg::OpenFileManager);
    assert_eq!(core.window_count(), n); // re-focus, not a second window
    core.apply(ClientMsg::FileManagerClose);
    assert!(!core.focused_is_filemanager());
    core.shutdown();
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --test session_tests open_file_manager`).

- [ ] **Step 3: Implement the wiring.**

(a) `src/protocol.rs` — add to `Flags` (after `settings_editing` or near the other focus flags):

```rust
    /// The file-manager window is focused; the client routes navigation keys to it.
    pub filemanager_focused: bool,
    /// The file manager has a text overlay open (new-folder / rename); forward chars.
    pub filemanager_editing: bool,
```

(b) `src/session.rs` — add `ClientMsg` variants (in the enum, near the Settings block):

```rust
    OpenFileManager,
    FileManagerUp,
    FileManagerDown,
    FileManagerLeft,
    FileManagerRight,
    FileManagerActivate,
    FileManagerBack,
    FileManagerParent,
    FileManagerToggleView,
    FileManagerToggleHidden,
    FileManagerNewFolder,
    FileManagerRename,
    FileManagerDelete,
    FileManagerCopy,
    FileManagerCut,
    FileManagerPaste,
    FileManagerChar(char),
    FileManagerBackspace,
    FileManagerCommit,
    FileManagerCancel,
    FileManagerClose,
```

(c) `src/session.rs` — add the `WinContent` variant and render/lifecycle arms:

```rust
    FileManager(crate::filemanager::FileManager),
```
In `WinContent::render`: `WinContent::FileManager(f) => f.render(w, h),`
In resize: no-op (the widget reads w/h at render time, like Settings). In `is_alive`: `true`. In `kill`/`write_input`: no-op.

(d) `src/session.rs` — add a field on `SessionCore`: `filemanager_win: Option<WindowId>,` (initialize `None` in `SessionCore::new`).

(e) `open_filemanager()` and focus helpers (mirror `open_store`):

```rust
fn open_filemanager(&mut self) {
    if let Some(id) = self.filemanager_win {
        if self.contents.contains_key(&id) {
            self.wm.unminimize(id);
            self.wm.focus(id);
            return;
        }
    }
    let w = 90.min((self.w - 4).max(40));
    let h = 30.min((self.h - 4).max(12));
    let rect = Rect::new((self.w - w) / 2, 2, w, h);
    let id = self.wm.add_window("Files".into(), rect);
    let root = self.picker_root(); // ~ or configured default
    self.contents.insert(
        id,
        WinContent::FileManager(crate::filemanager::FileManager::new(root, self.cfg.default_apps.clone())),
    );
    self.titles.push((id, "Files".into()));
    self.filemanager_win = Some(id);
}

pub fn focused_is_filemanager(&self) -> bool {
    matches!(
        self.wm.focused().and_then(|id| self.contents.get(&id)),
        Some(WinContent::FileManager(_))
    )
}

fn focused_filemanager_mut(&mut self) -> Option<&mut crate::filemanager::FileManager> {
    match self.wm.focused().and_then(|id| self.contents.get_mut(&id)) {
        Some(WinContent::FileManager(f)) => Some(f),
        _ => None,
    }
}

fn filemanager_editing(&self) -> bool {
    matches!(
        self.wm.focused().and_then(|id| self.contents.get(&id)),
        Some(WinContent::FileManager(f)) if f.is_editing()
    )
}
```

> `self.wm.focus(id)` — confirm the WM exposes a focus method; the existing code uses `self.wm.unminimize(id)` and relies on `add_window` focusing. If there is no public `focus`, drop that line (unminimize + being the most-recently-added is enough; match `open_store` exactly, which does NOT call focus).

(f) `apply()` branches — route each message and, after activating, drain the widget's `take_action`:

```rust
ClientMsg::OpenFileManager => self.open_filemanager(),
ClientMsg::FileManagerUp => { if let Some(f) = self.focused_filemanager_mut() { f.move_cursor(0, -1); } }
ClientMsg::FileManagerDown => { if let Some(f) = self.focused_filemanager_mut() { f.move_cursor(0, 1); } }
ClientMsg::FileManagerLeft => { if let Some(f) = self.focused_filemanager_mut() { f.move_cursor(-1, 0); } }
ClientMsg::FileManagerRight => { if let Some(f) = self.focused_filemanager_mut() { f.move_cursor(1, 0); } }
ClientMsg::FileManagerActivate => { if let Some(f) = self.focused_filemanager_mut() { f.activate(); } self.drain_fm_action(); }
ClientMsg::FileManagerBack => { if let Some(f) = self.focused_filemanager_mut() { f.go_back(); } }
ClientMsg::FileManagerParent => { if let Some(f) = self.focused_filemanager_mut() { f.go_parent(); } }
ClientMsg::FileManagerToggleView => { if let Some(f) = self.focused_filemanager_mut() {
    let v = match f.view() { crate::filemanager::ViewMode::Icon => crate::filemanager::ViewMode::List, _ => crate::filemanager::ViewMode::Icon };
    f.set_view(v);
} }
ClientMsg::FileManagerToggleHidden => { if let Some(f) = self.focused_filemanager_mut() { f.toggle_hidden(); } }
ClientMsg::FileManagerNewFolder => { if let Some(f) = self.focused_filemanager_mut() { f.begin_new_folder(); } }
ClientMsg::FileManagerRename => { if let Some(f) = self.focused_filemanager_mut() { f.begin_rename(); } }
ClientMsg::FileManagerDelete => { if let Some(f) = self.focused_filemanager_mut() { f.begin_delete(); } }
ClientMsg::FileManagerCopy => { if let Some(f) = self.focused_filemanager_mut() { f.copy_selection(); } }
ClientMsg::FileManagerCut => { if let Some(f) = self.focused_filemanager_mut() { f.cut_selection(); } }
ClientMsg::FileManagerPaste => { if let Some(f) = self.focused_filemanager_mut() { f.paste(); } }
ClientMsg::FileManagerChar(c) => { if let Some(f) = self.focused_filemanager_mut() { f.overlay_char(c); } }
ClientMsg::FileManagerBackspace => { if let Some(f) = self.focused_filemanager_mut() { f.overlay_backspace(); } }
ClientMsg::FileManagerCommit => {
    // Commit either an edit overlay or a delete confirmation.
    if let Some(f) = self.focused_filemanager_mut() {
        match f.overlay() {
            Some(crate::filemanager::Overlay::ConfirmDelete { .. }) => f.confirm_delete(),
            _ => f.overlay_commit(),
        }
    }
}
ClientMsg::FileManagerCancel => { if let Some(f) = self.focused_filemanager_mut() { f.cancel_overlay(); } }
ClientMsg::FileManagerClose => {
    if let Some(id) = self.wm.focused() {
        if matches!(self.contents.get(&id), Some(WinContent::FileManager(_))) {
            self.close(id);
        }
    }
}
```

(g) The effect dispatch helper (turns a `FileManagerAction` into a real window):

```rust
fn drain_fm_action(&mut self) {
    let action = self.focused_filemanager_mut().and_then(|f| f.take_action());
    match action {
        Some(crate::filemanager::FileManagerAction::OpenImage(path)) => {
            self.open_image(path.to_string_lossy().to_string());
        }
        Some(crate::filemanager::FileManagerAction::RunApp { command, args }) => {
            let name = args.last().and_then(|a| a.rsplit('/').next()).unwrap_or(&command).to_string();
            self.launch_in(name, command, args, self.focused_fm_cwd());
        }
        None => {}
    }
}

/// The cwd of the focused file manager (for launching an app in-place).
fn focused_fm_cwd(&self) -> Option<std::path::PathBuf> {
    match self.wm.focused().and_then(|id| self.contents.get(&id)) {
        Some(WinContent::FileManager(f)) => Some(f.cwd().to_path_buf()),
        _ => None,
    }
}
```

> Borrow note: compute `action` first (releasing the `&mut self` borrow) before calling `self.open_image`/`self.launch_in`. The code above does this. `focused_fm_cwd` takes `&self`, so call it before the `&mut` methods or inline the PathBuf — adjust to satisfy the borrow checker (e.g. capture `let cwd = self.focused_fm_cwd();` before `launch_in`).

(h) `@files` launcher action — in `launch_entry`, add: `"@files" => self.open_filemanager(),`. And pin it in the launcher's built-in entries next to `@store`/`@settings` (find where `@store`/`@settings` `AppEntry`s are constructed — around the launcher seed — and add `AppEntry { name: "Files".into(), command: "@files".into(), args: vec![], category: Some("System".into()), requires_cwd: None, cwd: None }`).

(i) `src/config.rs` — add `filemanager_view: Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` (persisted default view; wired fully in stage 2 — for now just store the field so the config round-trips).

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test session_tests`).

- [ ] **Step 5: Commit:**

```bash
git add src/protocol.rs src/session.rs src/config.rs tests/session_tests.rs
git commit -m "filemanager: session wiring — WinContent, @files, effect dispatch"
```

---

### Task 9: Daemon Flags + client key routing + final gate

**Files:** `src/daemon.rs`, `src/client.rs`; final verification.

- [ ] **Step 1: Populate the new Flags in the daemon.** In `src/daemon.rs`, where `Flags` is built per frame (alongside `settings_focused`, `dirpicker_open`, etc.), add:

```rust
    filemanager_focused: core.focused_is_filemanager(),
    filemanager_editing: core.filemanager_editing(),
```

> `filemanager_editing()` is currently private (`fn`); make it `pub fn` in session.rs so the daemon can read it (mirror `settings_editing` which is `pub`).

- [ ] **Step 2: Route keys in `src/client.rs`.** Add a branch BEFORE the `ctrl_alt` window-manager branch and AFTER the settings branches. Two sub-cases: editing (overlay text input) vs. navigating.

```rust
} else if f.filemanager_focused && f.filemanager_editing {
    match k.code {
        KeyCode::Esc => send(&mut out_stream, &ClientMsg::FileManagerCancel)?,
        KeyCode::Enter => send(&mut out_stream, &ClientMsg::FileManagerCommit)?,
        KeyCode::Backspace => send(&mut out_stream, &ClientMsg::FileManagerBackspace)?,
        KeyCode::Char(c) => send(&mut out_stream, &ClientMsg::FileManagerChar(c))?,
        _ => {}
    }
} else if f.filemanager_focused {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL) || k.modifiers.contains(KeyModifiers::SUPER);
    match (k.code, ctrl) {
        (KeyCode::Esc, _) => send(&mut out_stream, &ClientMsg::FileManagerClose)?,
        (KeyCode::Up, _) => send(&mut out_stream, &ClientMsg::FileManagerUp)?,
        (KeyCode::Down, _) => send(&mut out_stream, &ClientMsg::FileManagerDown)?,
        (KeyCode::Left, _) => send(&mut out_stream, &ClientMsg::FileManagerLeft)?,
        (KeyCode::Right, _) => send(&mut out_stream, &ClientMsg::FileManagerRight)?,
        (KeyCode::Enter, _) => send(&mut out_stream, &ClientMsg::FileManagerActivate)?,
        (KeyCode::Backspace, _) => send(&mut out_stream, &ClientMsg::FileManagerParent)?,
        (KeyCode::Char('c'), true) => send(&mut out_stream, &ClientMsg::FileManagerCopy)?,
        (KeyCode::Char('x'), true) => send(&mut out_stream, &ClientMsg::FileManagerCut)?,
        (KeyCode::Char('v'), true) => send(&mut out_stream, &ClientMsg::FileManagerPaste)?,
        (KeyCode::Char('n'), true) => send(&mut out_stream, &ClientMsg::FileManagerNewFolder)?,
        (KeyCode::Delete, _) => send(&mut out_stream, &ClientMsg::FileManagerDelete)?,
        (KeyCode::F(2), _) => send(&mut out_stream, &ClientMsg::FileManagerRename)?,
        (KeyCode::Char('1'), false) => send(&mut out_stream, &ClientMsg::FileManagerToggleView)?,
        (KeyCode::Char('2'), false) => send(&mut out_stream, &ClientMsg::FileManagerToggleView)?,
        (KeyCode::Char('.'), false) => send(&mut out_stream, &ClientMsg::FileManagerToggleHidden)?,
        _ => {}
    }
}
```

> Match the project's existing import of `KeyModifiers` in client.rs; if `SUPER` isn't already referenced, use whatever the codebase uses for Cmd (check the existing `ctrl_alt` computation and reuse its modifier logic). Keep `1`/`2` both toggling for now (List/Icon); a precise mapping lands in stage 2 with the third view.

- [ ] **Step 3: Mouse scroll** — in the `Event::Mouse` arm of `client.rs`, add wheel routing alongside the store's:

```rust
        MouseEventKind::ScrollUp if f.filemanager_focused => send(&mut out_stream, &ClientMsg::FileManagerUp)?,
        MouseEventKind::ScrollDown if f.filemanager_focused => send(&mut out_stream, &ClientMsg::FileManagerDown)?,
```

> Left-click selection/navigation in the FM goes through the existing `MouseDown` → session `handle_mouse` path. In session's mouse handler, when the click lands on a focused `FileManager`'s content rect, translate to content-local coords and call `f.handle_click(local, w, h, ctrl, shift)`. Find where `Store::handle_click`/`Settings::handle_click` are dispatched in the session mouse handler and add the `FileManager` case identically (modifiers default to false if the mouse path doesn't carry them — single-click select is enough for v1; double-click-to-open can map to a second MouseDown that calls `activate()` + `drain_fm_action()`). If the existing mouse plumbing doesn't carry double-click, wire `MouseDown` on an already-selected entry to `activate()`; otherwise leave open-by-mouse to a follow-up and ensure Enter works.

- [ ] **Step 4: Final gate.**

Run:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --offline && cargo test --offline && cargo clippy --offline --all-targets
```
Expected: builds, all tests pass, **0 clippy warnings**. Fix anything that isn't.

- [ ] **Step 5: Commit:**

```bash
git add src/daemon.rs src/client.rs src/session.rs
git commit -m "filemanager: daemon flags + client key/scroll routing"
```

---

### Task 10: Manual verification + docs

- [ ] **Step 1:** `cargo install --path . --root ~/.local --force`; on the host, `tuiui kill ; tuiui`. Open the launcher → **Files** (or `@files`). Verify: navigate folders (Enter/Backspace/click), Icon↔List toggle (`1`/`2` or the toolbar), open a text file (launches `$EDITOR`), open a PNG (image viewer), new folder (`Ctrl+N`), rename (`F2`), copy/paste (`Ctrl+C`/`Ctrl+V`), delete→Trash (`Delete`, confirm), `.` toggles hidden.
- [ ] **Step 2:** Update `README.md` — add the file manager to the features list and its keyboard shortcuts to the shortcuts section (Enter open, Backspace up, `Ctrl+C/X/V`, `Delete` trash, `F2` rename, `Ctrl+N` new folder, `1`/`2` views, `.` hidden, `Esc` close). Commit:

```bash
git add README.md
git commit -m "docs: file manager features + shortcuts in README"
```

---

## Notes for the implementer

- The `FileManager<F: FsOps = StdFs>` default type param lets the session hold `FileManager` (= `FileManager<StdFs>`) while tests can use `with_fs`. Confirm the session's `WinContent::FileManager(crate::filemanager::FileManager)` resolves the default param — if Rust complains, write `WinContent::FileManager(crate::filemanager::FileManager<StdFs>)` explicitly and re-export `StdFs`.
- `take_action`/`drain_fm_action` is the seam that keeps disk-launch effects in the session (where windows/PTYs live) and pure resolution in the widget — do not launch PTYs from inside `filemanager.rs`.
- Trash is the only delete path. No `remove_file`/`remove_dir_all` of user data anywhere in `filemanager.rs`.
- Stage 2 (`file-manager-rich`) adds: `thumb: Option<ImageId>` on `Entry` + thumbnail `ImagePlacement`s in `build_frame`, `tabs: Vec<Tab>`, the preview pane, Columns view, and the Get-Info overlay. Keep this stage's structs easy to extend (e.g. don't make `view` exhaustive-match in places that stage 2 must touch — already handled).
