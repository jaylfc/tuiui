# File Manager Rich (C, stage 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the "rich" Finder features on top of the stage-1 file manager: **Get-Info** (permissions + symlink target), **image thumbnails** (via the A1 image layer), a **preview pane** (text head / metadata / pdf), a **Miller-columns** view, and **tabs**.

**Architecture:** Builds directly on `src/filemanager.rs` and `src/fileops.rs` (stage 1, done) and the A1 image layer (`SessionCore::images: ImageStore`, `build_frame` placement emission). Disk metadata stays behind `FsOps`. Thumbnails are loaded into the **session's** existing `ImageStore` (so the daemon's blob-once bookkeeping and `image_png` keep working); the widget only reports which thumbnail id goes in which cell rect. Tabs are the final refactor: per-folder state moves onto a `Tab`, leaving `FileManager` as a thin tab container.

**Tech Stack:** Rust; `std::fs` + `std::os::unix::fs::PermissionsExt` for mode bits; existing `imagestore` (A1); existing compositor/session/protocol/client. Optional `pdftotext`/`mutool` shelled out only if present (graceful fallback). No new crates.

**Reference spec:** `docs/superpowers/specs/2026-06-05-file-manager-default-apps-design.md` (sections "Views", "Thumbnails", "Tabs", "Preview pane", "Get Info"). **Prereq:** `docs/superpowers/plans/2026-06-05-file-manager-core.md` (stage 1) is complete.

---

## Stage-1 surface this builds on (verified)

- `src/filemanager.rs`: `FileManager<F: FsOps = StdFs>` with fields `fs, cwd, entries: Vec<Entry>, cursor, selection: BTreeSet<usize>, view: ViewMode, show_hidden, history, hpos, scroll, clipboard, overlay: Option<Overlay>, handlers, status, action, cols_per_row: Cell<i32>`. Public: `new/with_fs`, `cwd()`, `entries()`, `cursor()`, `view()/set_view()`, `overlay()`, `is_editing()`, `reload()`, `move_cursor`, `select_at`, `activate`, `go_*`, overlay/clipboard ops, `render(w,h)->CellBuffer`, `hit_test`, `handle_click`, `take_action`. Consts: `BG/FG/DIM/SEL_BG/ACCENT`, `SIDEBAR_W=16`, `TOOLBAR_Y=0`, `LIST_TOP=2`, `TILE_W=14`, `TILE_H=3`. `ViewMode::{Icon, List}`. `Overlay::{NewFolder, Rename, ConfirmDelete, Context, OpenWith, Error}`. `Target::{Entry, Sidebar, Back, Forward, Up, ToggleView, Crumb}`. `render_overlay(&self, buf, w, h)` draws the modal box.
- `src/fileops.rs`: `Entry { name, path: PathBuf, is_dir, size: u64, modified: Option<SystemTime>, role: Role }`; `trait FsOps { list, mkdir, rename, copy, move_to, trash }`; `StdFs`; `unique_destination`, `trash_dir`.
- `src/session.rs`: `images: ImageStore`; `fn fully_unobstructed(&self, win) -> bool`; `pub fn image_png(&self, id) -> Option<Vec<u8>>`; `build_frame(&self)` emits `ImagePlacement`s for `ImageView` windows (loop at the end, before `Frame { layers, cursor, images }`); `focused_filemanager_mut()`, `open_filemanager()`, `drain_fm_action()`, the `ClientMsg::FileManager*` apply branches.
- `src/imagestore.rs`: `load(&mut self, path, max_w, max_h) -> Option<ImageId>` (u64), `png_bytes`, `dimensions`. `ImageId = u64`.
- `src/protocol.rs`: `ImagePlacement { id: u64, rect: Rect, cols: u16, rows: u16, visible: bool }`; `Flags` has `filemanager_focused`, `filemanager_editing`.

## Conventions (read before starting)

- `export PATH="$HOME/.cargo/bin:$PATH"` before cargo. Build before commit. Per-task: `cargo build --offline` clean + task tests pass. After the last task: `cargo build --offline && cargo test --offline && cargo clippy --offline --all-targets` → 0 warnings, all green.
- Commit per task with the exact message given. No AI attribution. Branch `main`.
- Disk tests use a unique temp dir under `std::env::temp_dir()` keyed by `std::process::id()`, cleaned up with `let _ = std::fs::remove_dir_all(&d);`.
- Mode bits are Unix-only. Guard `PermissionsExt` usage with `#[cfg(unix)]` and provide a non-unix fallback (the project targets macOS + Linux, both unix, but keep `cargo build` valid by gating). Tests for permissions are `#[cfg(unix)]`.

---

### Task 1: `fileops` — file info + permissions

**Files:** `src/fileops.rs`; Test `tests/fileops_tests.rs` (append).

- [ ] **Step 1: Append the failing test:**

```rust
#[cfg(unix)]
#[test]
fn info_reports_size_and_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("info");
    let f = d.join("data.bin");
    fs::write(&f, b"abcd").unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(0o640)).unwrap();

    let info = tuiui::fileops::info(&f).unwrap();
    assert_eq!(info.size, 4);
    assert!(!info.is_dir);
    assert!(!info.is_symlink);
    assert_eq!(info.mode & 0o777, 0o640);
    assert_eq!(tuiui::fileops::mode_rwx(info.mode), "rw-r-----");

    let _ = fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn info_follows_symlink_reports_target() {
    let d = tmp("link");
    let target = d.join("real.txt");
    fs::write(&target, b"x").unwrap();
    let link = d.join("alias.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let info = tuiui::fileops::info(&link).unwrap();
    assert!(info.is_symlink);
    assert_eq!(info.link_target.as_deref(), Some(target.as_path()));

    let _ = fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn set_permissions_changes_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("chmod");
    let f = d.join("s.sh");
    fs::write(&f, b"#!/bin/sh\n").unwrap();
    tuiui::fileops::StdFs.set_mode(&f, 0o755).unwrap();
    let m = fs::metadata(&f).unwrap().permissions().mode();
    assert_eq!(m & 0o777, 0o755);
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --test fileops_tests info`).

- [ ] **Step 3: Implement.** Append to `src/fileops.rs`:

```rust
/// Detailed metadata for the Get-Info panel.
#[derive(Clone, Debug)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub mode: u32,
    pub link_target: Option<PathBuf>,
}

/// Gather metadata for `path` (following the link for size/mode, but recording
/// whether the path itself is a symlink and where it points).
pub fn info(path: &Path) -> io::Result<FileInfo> {
    let lmeta = std::fs::symlink_metadata(path)?;
    let is_symlink = lmeta.file_type().is_symlink();
    let link_target = if is_symlink { std::fs::read_link(path).ok() } else { None };
    // Follow for the real size/mode where possible; fall back to the link's own.
    let meta = std::fs::metadata(path).unwrap_or(lmeta);
    let mode = mode_of(&meta);
    Ok(FileInfo {
        path: path.to_path_buf(),
        size: meta.len(),
        modified: meta.modified().ok(),
        is_dir: meta.is_dir(),
        is_symlink,
        mode,
        link_target,
    })
}

#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}
#[cfg(not(unix))]
fn mode_of(_meta: &std::fs::Metadata) -> u32 { 0 }

/// Render the low 9 mode bits as `rwxr-xr-x`.
pub fn mode_rwx(mode: u32) -> String {
    let bit = |shift: u32, ch: char| if mode & (1 << shift) != 0 { ch } else { '-' };
    let mut s = String::with_capacity(9);
    for (r, w, x) in [(8, 7, 6), (5, 4, 3), (2, 1, 0)] {
        s.push(bit(r, 'r'));
        s.push(bit(w, 'w'));
        s.push(bit(x, 'x'));
    }
    s
}
```

Add to the `FsOps` trait and `StdFs` impl a `set_mode`:

```rust
    // in trait FsOps:
    /// Set Unix permission bits on `path` (no-op on non-unix).
    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()>;
```

```rust
    // in impl FsOps for StdFs:
    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        {
            let _ = (path, mode);
        }
        Ok(())
    }
```

> Any other `FsOps` impls (none today besides `StdFs`) would need `set_mode`; there are none, so just `StdFs`.

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/fileops.rs tests/fileops_tests.rs
git commit -m "fileops: FileInfo + mode_rwx + set_mode (Get-Info backend)"
```

---

### Task 2: Get-Info overlay

**Files:** `src/filemanager.rs`; Test `tests/filemanager_tests.rs` (append).

Adds `Overlay::GetInfo { idx }`, `begin_get_info()`, and renders a read-only info box (size, modified, type, permissions `rwxr-xr-x`, and for symlinks the target). Interactive `chmod` is deferred (the `set_mode` backend exists for a later toggle).

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn get_info_overlay_opens_for_focused_entry() {
    use tuiui::filemanager::Overlay;
    let d = tmp("getinfo");
    fs::write(d.join("a.txt"), b"hello").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.select_at(0, false, false);
    fm.begin_get_info();
    assert!(matches!(fm.overlay(), Some(Overlay::GetInfo { .. })));
    // render must not panic and must include the permission triad somewhere
    let _ = fm.render(80, 24);
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --test filemanager_tests get_info`).

- [ ] **Step 3: Implement.**

(a) Add the variant to `Overlay`:

```rust
    GetInfo { idx: usize },
```

(b) Add the opener to `impl<F: FsOps>`:

```rust
    pub fn begin_get_info(&mut self) {
        self.overlay = Some(Overlay::GetInfo { idx: self.cursor });
    }
```

(c) Extend `render_overlay` to handle `GetInfo` — match it and draw a multi-line box. Build the body from `crate::fileops::info(&entry.path)`:

```rust
            Overlay::GetInfo { idx } => {
                let Some(e) = self.entries.get(*idx) else { return; };
                let mut lines = vec![format!("Name: {}", e.name)];
                if let Ok(info) = crate::fileops::info(&e.path) {
                    lines.push(format!("Path: {}", info.path.display()));
                    lines.push(format!("Size: {} bytes", info.size));
                    let kind = if info.is_dir { "Folder" } else { e.role_label() };
                    lines.push(format!("Kind: {kind}"));
                    lines.push(format!("Permissions: {} ({:o})", crate::fileops::mode_rwx(info.mode), info.mode & 0o777));
                    if let Some(t) = &info.link_target {
                        lines.push(format!("Symlink \u{2192} {}", t.display()));
                    }
                }
                lines.push("[Esc] close".into());
                self.draw_box(buf, w, h, "Get Info", &lines);
                return;
            }
```

> The existing `render_overlay` draws a fixed 4-row box; `GetInfo` needs N rows. Refactor the box-drawing into a `draw_box(&self, buf, w, h, title, lines: &[String])` helper (sizes the box to `lines.len()+2` tall, centered) and have the other overlay arms call it too (keep their existing one/two-line bodies). Add `Entry::role_label()` — a small method on `Entry` in `fileops.rs` mapping `Role` to a label ("Text", "Image", "PDF", …); or inline a match in filemanager. Prefer a helper `fn role_label(role: Role) -> &'static str` in `openwith.rs` (reuse it for the preview/columns tasks).

(d) Add `role_label` to `src/openwith.rs`:

```rust
impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Image => "Image", Role::Video => "Video", Role::Audio => "Audio",
            Role::Text => "Text", Role::Code => "Code", Role::Archive => "Archive",
            Role::Pdf => "PDF", Role::Directory => "Folder",
            Role::Executable => "Executable", Role::Other => "Document",
        }
    }
}
```

Then in the overlay use `e.role.label()` (Entry exposes `role`). Drop the `e.role_label()` reference above and use `e.role.label()` directly.

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/filemanager.rs src/openwith.rs tests/filemanager_tests.rs
git commit -m "filemanager: Get-Info overlay (size/kind/permissions/symlink)"
```

---

### Task 3: Image thumbnails via the A1 layer

**Files:** `src/filemanager.rs`, `src/session.rs`; Test `tests/filemanager_tests.rs` (unit: requests/placements) + `tests/session_tests.rs` (integration: a placement is emitted).

The widget loads nothing itself; it (a) reports image entries that need a thumbnail and (b) computes per-tile cell placements for thumbnails the session has loaded.

- [ ] **Step 1: Append the failing unit test** (`tests/filemanager_tests.rs`):

```rust
#[test]
fn thumbnail_requests_lists_image_entries() {
    let d = tmp("thumbreq");
    fs::write(d.join("pic.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    fs::write(d.join("note.txt"), b"hi").unwrap();
    let fm = FileManager::new(d.clone(), BTreeMap::new());
    let reqs = fm.thumbnail_requests();
    assert_eq!(reqs.len(), 1);
    assert!(reqs[0].1.ends_with("pic.png"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn set_thumb_then_placement_is_reported() {
    use tuiui::geometry::Rect;
    let d = tmp("thumbplace");
    fs::write(d.join("pic.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    let idx = fm.thumbnail_requests()[0].0;
    fm.set_thumb(idx, 12345);
    // content rect at origin (0,0) sized 80x24
    let places = fm.thumbnail_placements(Rect::new(10, 2, 80, 24), true);
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].id, 12345);
    assert!(places[0].visible);
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement the widget side.**

(a) Add a field to `FileManager`: `thumbs: std::collections::HashMap<usize, u64>,` (entry idx → ImageId). Init empty in `with_fs`. **Clear it in `reload()`** (entries changed → indices invalid): add `self.thumbs.clear();` in `reload`.

(b) Methods on `impl<F: FsOps>`:

```rust
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
    pub fn thumbnail_placements(&self, content: crate::geometry::Rect, visible: bool) -> Vec<crate::protocol::ImagePlacement> {
        // Thumbnails only render in Icon view (List/Columns show glyphs/preview).
        if self.view != ViewMode::Icon {
            return Vec::new();
        }
        let area_x = SIDEBAR_W;
        let area_w = (content.w - SIDEBAR_W).max(1);
        let cols = (area_w / TILE_W).max(1);
        let mut out = Vec::new();
        for (&idx, &id) in &self.thumbs {
            if idx >= self.entries.len() { continue; }
            let col = idx as i32 % cols;
            let row = idx as i32 / cols;
            let cx = content.x + area_x + col * TILE_W;
            let cy = content.y + LIST_TOP + row * TILE_H;
            if cy + 1 >= content.y + content.h { continue; } // below the viewport
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
```

> The thumbnail occupies the glyph row of the tile (`TILE_H` row 0), one cell tall, `TILE_W-1` wide — a small inline image above the name. The FM still draws the glyph underneath as the fallback for terminals without Kitty graphics (no render change needed).

- [ ] **Step 4: Implement the session side + integration test.**

(a) `tests/session_tests.rs` — append:

```rust
#[test]
fn file_manager_emits_thumbnail_placement_for_image() {
    let dir = std::env::temp_dir().join(format!("tuiui-fmthumb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
    image::DynamicImage::ImageRgba8(img).save(dir.join("p.png")).unwrap();

    let mut core = SessionCore::new(100, 30, Config::default());
    core.apply(ClientMsg::OpenFileManager);
    // navigate the FM into our temp dir by faking it: open at temp via a second open
    core.open_filemanager_at(dir.clone()); // test helper below
    let frame = core.build_frame();
    assert!(frame.images.iter().any(|p| p.cols >= 1), "expected a thumbnail placement");
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
```

> Add a small `#[cfg(test)] pub fn open_filemanager_at(&mut self, dir: PathBuf)` test helper on `SessionCore` that opens an FM rooted at `dir` and runs the thumbnail refresh — or, simpler, make `open_filemanager` take the root from a param internally and expose a pub test-only opener. Keep it minimal; the goal is to prove `build_frame` emits an FM thumbnail placement.

(b) Session plumbing in `src/session.rs`:

- Add `fn refresh_fm_thumbnails(&mut self)`:

```rust
    /// Load thumbnails for the focused file manager's image entries into the
    /// shared ImageStore and hand the ids back to the widget.
    fn refresh_fm_thumbnails(&mut self) {
        let reqs = match self.focused_filemanager_mut() {
            Some(f) => f.thumbnail_requests(),
            None => return,
        };
        for (idx, path) in reqs {
            // Bound thumbnail pixels: a tile is ~13 cells wide; cells are ~8x16px.
            if let Some(id) = self.images.load(&path, 13 * 8, 1 * 16) {
                if let Some(f) = self.focused_filemanager_mut() {
                    f.set_thumb(idx, id);
                }
            }
        }
    }
```

- Call `self.refresh_fm_thumbnails();` after FM apply branches that change entries: at the end of `open_filemanager` and after `FileManagerActivate` (navigate), `FileManagerBack`, `FileManagerParent`, `FileManagerToggleHidden`, and after paste/trash/new-folder/rename commits (anything that reloads). Simplest: call it once at the bottom of the whole `apply()` match when the focused window is an FM (guard cheaply with `if self.focused_is_filemanager() { self.refresh_fm_thumbnails(); }`), so every FM interaction refreshes. Loads are content-hash-cached in `ImageStore`, so repeated calls are cheap.

- In `build_frame`, after the existing ImageView placement loop, add an FM loop:

```rust
        for w in self.wm.z_ordered() {
            if w.minimized { continue; }
            if let Some(WinContent::FileManager(f)) = self.contents.get(&w.id) {
                let cr = w.content_rect();
                let vis = self.fully_unobstructed(w);
                images.extend(f.thumbnail_placements(cr, vis));
            }
        }
```

- [ ] **Step 5: Run → PASS** (`cargo test --offline --test filemanager_tests thumbnail` and `--test session_tests file_manager_emits`).

- [ ] **Step 6: Commit:**

```bash
git add src/filemanager.rs src/session.rs tests/filemanager_tests.rs tests/session_tests.rs
git commit -m "filemanager: image thumbnails via A1 placements (Icon view)"
```

---

### Task 4: Preview pane

**Files:** `src/filemanager.rs`, `src/session.rs` (a `ClientMsg::FileManagerTogglePreview` + apply), `src/client.rs` (Space key); Test `tests/filemanager_tests.rs`.

A toggleable right-hand pane showing the focused entry: text/code → first ~40 lines; pdf → page text via `pdftotext`/`mutool` if present else metadata; image → its name/size (the thumbnail itself renders via Task 3 in Icon view); other → metadata.

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn preview_toggle_and_text_head() {
    let d = tmp("preview");
    fs::write(d.join("a.txt"), b"line1\nline2\nline3\n").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.set_view(ViewMode::List);
    fm.select_at(0, false, false);
    assert!(!fm.preview_open());
    fm.toggle_preview();
    assert!(fm.preview_open());
    let lines = fm.preview_lines(20);
    assert!(lines.iter().any(|l| l.contains("line1")));
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.**

(a) Field: `preview: bool,` (init false). Methods:

```rust
    pub fn preview_open(&self) -> bool { self.preview }
    pub fn toggle_preview(&mut self) { self.preview = !self.preview; }

    /// The preview body for the focused entry (≤ `max` lines).
    pub fn preview_lines(&self, max: usize) -> Vec<String> {
        let Some(e) = self.entries.get(self.cursor) else { return vec![]; };
        use crate::openwith::Role::*;
        match e.role {
            Text | Code => read_head(&e.path, max),
            Pdf => pdf_preview(&e.path, max),
            Directory => vec![format!("{} \u{2014} folder", e.name)],
            _ => {
                let mut v = vec![format!("Name: {}", e.name), format!("Kind: {}", e.role.label()), format!("Size: {} bytes", e.size)];
                v.truncate(max);
                v
            }
        }
    }
```

Free functions in `filemanager.rs`:

```rust
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
                let lines: Vec<String> = text.lines().take(max).map(|l| l.chars().take(200).collect()).collect();
                if !lines.is_empty() { return lines; }
            }
        }
    }
    vec!["PDF (install pdftotext or mutool for a text preview)".into()]
}
```

(b) Render: when `self.preview`, reserve the right ~⅓ of the content for the pane. In `render`, compute `let preview_w = if self.preview { (w / 3).clamp(20, 48) } else { 0 };` and shrink the entry area to `w - preview_w`. Draw a vertical separator at `w - preview_w - 1` and write `preview_lines(h - LIST_TOP - 1)` down the pane from `LIST_TOP`. Keep Icon/List layout math using the reduced width (replace bare `w` with `w - preview_w` where the entry area width is computed). **Important:** Icon thumbnail placements (Task 3) compute their own width from `content.w`; when the preview is open, thumbnails should use the reduced area too — pass the reduced width by having `thumbnail_placements` subtract the preview width. Add a `preview_cols(content_w) -> i32` helper used by both `render` and `thumbnail_placements` so they agree.

> To keep Task-3 placements correct with the preview open, add `fn preview_reserve(&self, w: i32) -> i32 { if self.preview { (w/3).clamp(20,48) } else { 0 } }` and subtract it from `area_w` in both `render` (Icon/List) and `thumbnail_placements`.

(c) Wire the toggle: add `ClientMsg::FileManagerTogglePreview`; in session `apply`, `if let Some(f) = self.focused_filemanager_mut() { f.toggle_preview(); }`; in `client.rs` FM-navigating branch, map `KeyCode::Char(' ')` (Space, no ctrl) → `FileManagerTogglePreview`.

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/filemanager.rs src/session.rs src/client.rs tests/filemanager_tests.rs
git commit -m "filemanager: preview pane (text head / pdf / metadata) + Space toggle"
```

---

### Task 5: Miller-columns view

**Files:** `src/filemanager.rs`, `src/client.rs` (`3` key); Test `tests/filemanager_tests.rs`.

A third `ViewMode::Columns` showing parent | current | preview-of-focused. Cursor navigation stays list-like within the current column.

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn columns_view_cycles_and_renders() {
    let d = tmp("cols");
    fs::create_dir(d.join("sub")).unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.set_view(ViewMode::Columns);
    assert_eq!(fm.view(), ViewMode::Columns);
    let buf = fm.render(100, 24);
    assert_eq!(buf.width(), 100);
    // cycle_view goes Icon -> List -> Columns -> Icon
    let mut f2 = FileManager::new(d.clone(), BTreeMap::new());
    f2.cycle_view();
    assert_eq!(f2.view(), ViewMode::List);
    f2.cycle_view();
    assert_eq!(f2.view(), ViewMode::Columns);
    f2.cycle_view();
    assert_eq!(f2.view(), ViewMode::Icon);
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.**

(a) Add `Columns` to `ViewMode`. Replace the two-way toggle with a three-way cycle method and update callers:

```rust
    pub fn cycle_view(&mut self) {
        self.view = match self.view {
            ViewMode::Icon => ViewMode::List,
            ViewMode::List => ViewMode::Columns,
            ViewMode::Columns => ViewMode::Icon,
        };
    }
```

> The session's `FileManagerToggleView` apply branch currently flips Icon↔List; change it to call `f.cycle_view()`. The `Target::ToggleView` click in `handle_click` likewise calls `cycle_view`.

(b) Render the `Columns` arm in `render`: split the entry area into three columns. Left = parent directory listing (read `self.cwd.parent()` via `self.fs.list`, highlight the current dir), middle = current entries (cursor highlighted, like List), right = preview of the focused entry (reuse `preview_lines(h)` for files, or a mini-listing for a focused subdir). Each column ~⅓ of `area_w`. Keep it read-only navigation; clicking a middle-column row selects (extend `hit_test` to map the middle column's rows to `Target::Entry`, the left column to a `Target::Up`-like parent nav — for v1 it's acceptable for hit-testing in Columns to only handle the middle column; left/right are visual).

> Implementation note: `move_cursor` already treats any delta as ±1 in List; make `Columns` behave like `List` for cursor movement (add `ViewMode::Columns => cur + dx.signum() + dy.signum()` in the `move_cursor` match, same as List).

(c) `client.rs`: in the FM-navigating branch, map `KeyCode::Char('3')` → a message that sets Columns. Simplest: replace the separate `1`/`2`/`3` handling with a single `FileManagerCycleView` on a key like `Tab`? No — keep explicit: send `FileManagerToggleView` (now = cycle) for `1`/`2`/`3` is wrong. Instead: keep `FileManagerToggleView` mapped to a key (e.g. `v`) that cycles, and drop the numeric mapping, OR add three messages. **Chosen:** map `KeyCode::Char('1')`→Icon, `'2'`→List, `'3'`→Columns via three messages `FileManagerViewIcon/List/Columns`. Add those three `ClientMsg` variants + apply branches calling `f.set_view(...)`. Update the toolbar/`Target::ToggleView` click to `cycle_view`.

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test filemanager_tests columns` and the existing `view_toggle_switches_modes` inline test — update it if it asserted the old 2-way toggle; switch it to use `cycle_view` or `set_view`).

- [ ] **Step 5: Commit:**

```bash
git add src/filemanager.rs src/session.rs src/client.rs tests/filemanager_tests.rs
git commit -m "filemanager: Miller-columns view + 1/2/3 view selection"
```

---

### Task 6: Tabs

**Files:** `src/filemanager.rs` (refactor), `src/session.rs` + `src/client.rs` (new-tab/close-tab/next-tab messages); Test `tests/filemanager_tests.rs`.

The final refactor: per-folder state moves onto a `Tab`; `FileManager` becomes a tab container. Widget-global state (clipboard, overlay, status, action, handlers, fs, cols_per_row) stays on `FileManager`.

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn tabs_open_switch_and_close() {
    let d = tmp("tabs");
    fs::create_dir(d.join("sub")).unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    assert_eq!(fm.tab_count(), 1);
    fm.new_tab();
    assert_eq!(fm.tab_count(), 2);
    assert_eq!(fm.active_tab(), 1);
    // navigate only the active tab
    fm.activate(); // into "sub" (dirs first)
    assert_eq!(fm.cwd(), d.join("sub"));
    fm.next_tab();
    assert_eq!(fm.active_tab(), 0);
    assert_eq!(fm.cwd(), d.as_path()); // tab 0 unchanged
    fm.close_tab();
    assert_eq!(fm.tab_count(), 1);
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement the refactor.**

(a) Define `Tab` and move per-folder fields onto it:

```rust
struct Tab {
    cwd: PathBuf,
    entries: Vec<Entry>,
    cursor: usize,
    selection: BTreeSet<usize>,
    view: ViewMode,
    show_hidden: bool,
    history: Vec<PathBuf>,
    hpos: usize,
    scroll: i32,
    thumbs: std::collections::HashMap<usize, u64>,
    preview: bool,
}
```

`FileManager` becomes:

```rust
pub struct FileManager<F: FsOps = StdFs> {
    fs: F,
    tabs: Vec<Tab>,
    active: usize,
    clipboard: Option<Clipboard>,
    overlay: Option<Overlay>,
    handlers: BTreeMap<String, String>,
    status: String,
    action: Option<FileManagerAction>,
    cols_per_row: std::cell::Cell<i32>,
}
```

(b) Add `fn tab(&self) -> &Tab { &self.tabs[self.active] }` and `fn tab_mut(&mut self) -> &mut Tab`. Rewrite the existing methods to delegate: `cwd()` → `&self.tab().cwd`, `entries()` → `&self.tab().entries`, `cursor()`/`view()`/etc. read `self.tab()`; `reload`, `move_cursor`, `select_at`, `navigate_to`, `go_*`, `toggle_hidden`, `set_view`/`cycle_view`, `set_thumb`/`thumbnail_*`, `preview_*` all operate on `self.tab_mut()` (note: `reload` needs `self.fs` + `self.tab_mut()` — borrow `self.fs` and the tab separately, e.g. pull `let (fs, tab)` via split borrows or list into a temp `let es = self.fs.list(&self.tabs[self.active].cwd, ...)` then assign). Overlay/clipboard/status/action stay on `self` and are unchanged.

> This is a mechanical but wide refactor. Keep every existing public method's signature identical so stage-1 tests and the session wiring keep compiling. The public API does not change except for the new tab methods below.

(c) New public tab methods:

```rust
    pub fn tab_count(&self) -> usize { self.tabs.len() }
    pub fn active_tab(&self) -> usize { self.active }

    pub fn new_tab(&mut self) {
        let cwd = self.tab().cwd.clone();
        let mut t = Tab::new(cwd);
        // list it
        t.entries = self.fs.list(&t.cwd, t.show_hidden).unwrap_or_default();
        self.tabs.push(t);
        self.active = self.tabs.len() - 1;
    }

    pub fn close_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.tabs.remove(self.active);
            self.active = self.active.min(self.tabs.len() - 1);
        }
    }

    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }
```

With `Tab::new(cwd)` building a default tab (Icon view, history=[cwd], etc.). Have `with_fs` build the first tab via `Tab::new` then `reload`.

(d) Render a tab strip when `tabs.len() > 1`: one row (e.g. at `TOOLBAR_Y + 1`, shifting `LIST_TOP` to `3` when the strip shows) listing each tab's folder name, the active one highlighted. To avoid shifting all the layout consts, gate it: add `fn content_top(&self) -> i32 { if self.tabs.len() > 1 { LIST_TOP + 1 } else { LIST_TOP } }` and use `content_top()` instead of `LIST_TOP` in `render`/`hit_test`/`thumbnail_placements`. (Update those three to call `self.content_top()`.)

(e) Wire messages: add `ClientMsg::FileManagerNewTab`, `FileManagerCloseTab`, `FileManagerNextTab`; session `apply` branches call `f.new_tab()/close_tab()/next_tab()` then `refresh_fm_thumbnails()`; `client.rs` FM-navigating branch maps `Ctrl+T`→NewTab, `Ctrl+W`→CloseTab, `Tab`→NextTab. (Note `Ctrl+W` currently isn't bound in the FM branch; add it. Keep `Esc`=close window.)

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test filemanager_tests tabs`), then the FULL gate:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --offline && cargo test --offline && cargo clippy --offline --all-targets
```
All green, 0 warnings. Fix anything broken by the refactor (the stage-1 tests must still pass).

- [ ] **Step 5: Commit:**

```bash
git add src/filemanager.rs src/session.rs src/client.rs tests/filemanager_tests.rs
git commit -m "filemanager: tabs (per-tab folder state + strip + Ctrl+T/W/Tab)"
```

---

### Task 7: Final verification + docs

- [ ] **Step 1:** Full gate green (build + test + clippy 0 warnings).
- [ ] **Step 2:** `cargo install --path . --root ~/.local --force`; on the host `tuiui kill ; tuiui`. In **Files**: confirm thumbnails show for a folder of images (Icon view, Ghostty/Kitty), `Get Info` shows permissions + symlink target, the preview pane toggles with Space, `1`/`2`/`3` switch Icon/List/Columns, and `Ctrl+T`/`Ctrl+W`/`Tab` manage tabs.
- [ ] **Step 3:** Update `README.md` — note thumbnails, preview, columns, tabs, Get-Info under the file manager bullet and shortcuts (Space preview, `3` columns, `Ctrl+T`/`Ctrl+W`/`Tab` tabs, `Get Info`). Mark stage-2 done in the roadmap line. Commit:

```bash
git add README.md
git commit -m "docs: file manager rich features (thumbnails, preview, columns, tabs, Get-Info)"
```

---

## Notes for the implementer
- Thumbnails live in the **session's** `ImageStore` (loaded by `refresh_fm_thumbnails`), so `image_png`/blob-once bookkeeping keep working unchanged. The widget only stores ids + computes cell rects.
- Keep every stage-1 public method signature stable through the Tabs refactor so the session wiring and stage-1 tests compile untouched. Only ADD methods.
- `Get Info` is read-only in this stage; the `set_mode` backend exists for a future interactive chmod toggle (not in scope here).
- Columns hit-testing only needs the middle column for v1; left/right columns are visual.
- After the Tabs refactor, double-check `render`, `hit_test`, and `thumbnail_placements` all use `self.content_top()` (not bare `LIST_TOP`) and the reduced width when the preview pane is open.
