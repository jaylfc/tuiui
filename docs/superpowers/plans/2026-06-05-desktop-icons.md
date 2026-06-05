# Desktop Icons (D) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clickable desktop icons — merged from the live `~/Desktop` folder and user pins — on an invisible snap grid, drag-to-rearrange (persisted), double-click to open via Default Apps, with a right-click context menu (open / rename / Trash / new folder) and A1 thumbnails.

**Architecture:** A new `desktop` module owns a `DesktopIcons` model rendered as a compositor `Layer` at **z=0** (windows are `z≥1`, so icons sit above the wallpaper and beneath windows). `SessionCore` owns one `DesktopIcons`, renders it, and routes only fall-through clicks (those missing chrome and every window) to it. Opening reuses the FM's effect dispatch (`openwith::resolve`); thumbnails reuse the session `ImageStore`; deletes reuse `fileops::trash`.

**Tech Stack:** Rust; existing `fileops`, `openwith`, `imagestore`, compositor/session/protocol/client. No new crates.

**Reference spec:** `docs/superpowers/specs/2026-06-05-desktop-icons-design.md`.

---

## Stage surface this builds on (verified)

- `src/wm.rs`: windows get `z` from `next_z` starting at **1**, incrementing. So **z=0** is below all windows.
- `src/compositor.rs`: `pub struct Layer { pub z: i32, pub origin: Point, pub buf: CellBuffer, pub opacity: f32, pub scissor: Option<Rect> }`.
- `src/session.rs`: `build_frame(&self)` pushes window layers (from `wm.z_ordered()`), then chrome (`render_menubar`, `render_dock`), then launcher/dirpicker/help/images. `handle_mouse(&mut self, kind: MouseKind, p: Point)` checks help/dirpicker/launcher/tray/menubar/dock, then `route_mouse(kind, p, &windows, self.drag)` → `self.exec(action, p)`. `MouseKind::{Down,Drag,Up}`. The FM effect dispatch is `drain_fm_action` + `focused_fm_cwd`; thumbnails via `refresh_fm_thumbnails` (loads into `self.images: ImageStore`, hands ids back). `open_image(path: String)`, `launch_in(name, command, args, cwd: Option<PathBuf>)`, `launch_entry(AppEntry)` (handles `@files`/`@store`/`@settings`/`@image`), `open_filemanager_root(root: PathBuf)` (private; opens Files at a dir). `picker_root()` → the `~`/configured root. `fully_unobstructed(&self, win)` exists for windows.
- `src/openwith.rs`: `classify(&Path, is_dir) -> Role`, `resolve(&Path, is_dir, &BTreeMap<String,String>) -> OpenAction` (`Navigate/Builtin(&str)/RunApp{command,args}/OpenWithMenu`), `Role` + `Role::label()`.
- `src/fileops.rs`: `Entry { name, path, is_dir, size, modified, role }`, `StdFs`/`FsOps` (`list/mkdir/rename/copy/move_to/trash`), `unique_destination`, `trash_dir`.
- `src/config.rs`: `Config` derives `Clone, Debug, Deserialize, Serialize`, `#[serde(default)]` on the struct + explicit `impl Default`. `AppEntry { name, command, args, category, requires_cwd, cwd }`. `Config::save()` writes `~/.config/tuiui/config.toml`.
- `src/protocol.rs`: `ImagePlacement { id, rect, cols, rows, visible }`; `Flags` (`#[serde(default)]`) has the focus flags incl. `filemanager_focused/_editing`.
- `src/client.rs`: mouse arm sends left-only (`MouseEventKind::Down(MouseButton::Left) => ClientMsg::MouseDown`, etc.); imports `MouseButton`. `Point::new(col, row)`.
- `src/buffer.rs` / `src/cell.rs`: `CellBuffer::{new,width,height,fill,set,write_str}`, `Cell { ch, fg, bg, attrs }`, `Rgba { r,g,b,a }`. `src/geometry.rs`: `Rect::new`, `Rect::contains(Point)`, `Rect::intersect`.

## Conventions

- `export PATH="$HOME/.cargo/bin:$PATH"` before cargo. Build before commit. Per-task: build clean + task tests pass. Final task: full gate (`build && test && clippy --all-targets`) → 0 warnings.
- Commit per task with the exact message. No AI attribution. Branch `main`.
- Disk tests use a unique temp dir under `std::env::temp_dir()` keyed by `std::process::id()`, cleaned up after. The model takes the desktop dir + pins + positions as **inputs** (no real `~/Desktop` in tests).

---

### Task 1: Config `[desktop]` fields + default pins

**Files:** `src/config.rs`; Test `tests/config_tests.rs` (append).

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn desktop_defaults_have_files_and_store_pins() {
    let c = Config::default();
    assert!(c.desktop_enabled);
    let cmds: Vec<&str> = c.desktop_pins.iter().map(|p| p.command.as_str()).collect();
    assert!(cmds.contains(&"@files"));
    assert!(cmds.contains(&"@store"));
    assert!(c.desktop_positions.is_empty());
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --test config_tests desktop_defaults`).

- [ ] **Step 3: Implement.** Add to `struct Config`:

```rust
    /// Whether desktop icons are shown on the wallpaper.
    #[serde(default = "default_true")]
    pub desktop_enabled: bool,
    /// Pinned desktop shortcuts (reuses AppEntry); shown alongside ~/Desktop files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub desktop_pins: Vec<AppEntry>,
    /// Saved grid positions: key (abs path or pin command) → (col, row).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub desktop_positions: std::collections::BTreeMap<String, (u16, u16)>,
```

Add the helper if not present:

```rust
fn default_true() -> bool { true }
```

In `impl Default for Config`, set:

```rust
            desktop_enabled: true,
            desktop_pins: vec![
                AppEntry { name: "Files".into(), command: "@files".into(), args: vec![], category: None, requires_cwd: None, cwd: None },
                AppEntry { name: "Store".into(), command: "@store".into(), args: vec![], category: None, requires_cwd: None, cwd: None },
            ],
            desktop_positions: std::collections::BTreeMap::new(),
```

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/config.rs tests/config_tests.rs
git commit -m "config: [desktop] enabled + pins + positions (Files/Store default pins)"
```

---

### Task 2: `desktop` model — types, merge, grid, hit-test, selection, actions

**Files:** Create `src/desktop.rs`; Modify `src/lib.rs`; Test `tests/desktop_tests.rs`.

- [ ] **Step 1: Write the failing test** (`tests/desktop_tests.rs`):

```rust
use std::collections::BTreeMap;
use std::fs;
use tuiui::config::AppEntry;
use tuiui::desktop::{DesktopIcons, DesktopAction, IconSource};
use tuiui::geometry::Point;

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tuiui-dt-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn pins() -> Vec<AppEntry> {
    vec![AppEntry { name: "Files".into(), command: "@files".into(), args: vec![], category: None, requires_cwd: None, cwd: None }]
}

#[test]
fn merges_folder_entries_and_pins() {
    let d = tmp("merge");
    fs::write(d.join("notes.md"), b"x").unwrap();
    fs::create_dir(d.join("proj")).unwrap();
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&pins(), &BTreeMap::new());
    dt.layout(100, 30); // assign cells
    // 2 folder items + 1 pin = 3 icons
    assert_eq!(dt.icons().len(), 3);
    assert!(dt.icons().iter().any(|i| i.label == "Files" && matches!(i.source, IconSource::Pinned)));
    assert!(dt.icons().iter().any(|i| i.label == "proj" && matches!(i.source, IconSource::Folder)));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn saved_position_wins_else_first_free_cell() {
    let d = tmp("pos");
    fs::write(d.join("a"), b"").unwrap();
    fs::write(d.join("b"), b"").unwrap();
    let mut pos = BTreeMap::new();
    pos.insert(d.join("b").to_string_lossy().to_string(), (2u16, 1u16));
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&[], &pos);
    dt.layout(100, 30);
    let b = dt.icons().iter().find(|i| i.label == "b").unwrap();
    assert_eq!(b.cell, (2, 1));
    // a has no saved position → first free cell (0,0)
    let a = dt.icons().iter().find(|i| i.label == "a").unwrap();
    assert_eq!(a.cell, (0, 0));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn hit_test_and_select_then_double_click_opens() {
    let d = tmp("hit");
    fs::create_dir(d.join("proj")).unwrap();
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&[], &BTreeMap::new());
    dt.layout(100, 30);
    // proj is at cell (0,0): tile origin (0*14, 1+0*3) = (0,1); glyph row y=1
    let p = Point::new(2, 1);
    assert_eq!(dt.icon_at(p), Some(0));
    assert!(dt.icon_at(Point::new(60, 20)).is_none()); // empty desktop
    dt.click(p, false);
    assert_eq!(dt.selection(), vec![0]);
    dt.double_click(p);
    assert_eq!(dt.take_action(), Some(DesktopAction::Open(d.join("proj"))));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn double_click_pin_runs_command() {
    let d = tmp("pin");
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&pins(), &BTreeMap::new());
    dt.layout(100, 30);
    let idx = dt.icons().iter().position(|i| i.label == "Files").unwrap();
    // place a click on that icon's cell
    let (col, row) = dt.icons()[idx].cell;
    let p = Point::new((col as i32) * 14 + 1, 1 + (row as i32) * 3);
    dt.double_click(p);
    assert_eq!(dt.take_action(), Some(DesktopAction::Run { command: "@files".into(), args: vec![] }));
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --test desktop_tests`).

- [ ] **Step 3: Implement `src/desktop.rs`** (model only; render in Task 3):

```rust
//! Desktop icons: a wallpaper-level icon layer merged from the live `~/Desktop`
//! folder and user pins. Not a window — rendered at z=0 beneath all windows and
//! hit-tested only for clicks that fall through to the empty desktop.

use crate::config::AppEntry;
use crate::fileops::{FsOps, StdFs};
use crate::geometry::Point;
use crate::openwith::{classify, Role};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Layout: each icon occupies a tile this many cells wide/tall; the grid starts
/// one row below the menubar.
pub const ICON_W: i32 = 14;
pub const ICON_H: i32 = 3;
pub const GRID_TOP: i32 = 1; // below the menubar row

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSource { Folder, Pinned }

#[derive(Clone, Debug)]
pub struct DesktopIcon {
    pub path: PathBuf,
    pub label: String,
    pub role: Role,
    pub source: IconSource,
    pub command: Option<String>, // pins only
    pub cell: (u16, u16),        // (col, row)
    pub thumb: Option<u64>,
}

/// What the session must effect (the model never touches windows/PTYs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopAction {
    Open(PathBuf),
    Run { command: String, args: Vec<String> },
    Unpin(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopOverlay {
    Context { idx: usize, anchor: Point },
    DesktopMenu { anchor: Point },
    Rename { idx: usize, name: String },
    NewFolder { name: String },
}

pub struct DesktopIcons<F: FsOps = StdFs> {
    fs: F,
    desktop_dir: PathBuf,
    icons: Vec<DesktopIcon>,
    selection: BTreeSet<usize>,
    drag: Option<(usize, Point)>,
    overlay: Option<DesktopOverlay>,
    action: Option<DesktopAction>,
    cols: u16,
    rows: u16,
}

impl DesktopIcons<StdFs> {
    pub fn new(desktop_dir: PathBuf) -> Self {
        Self::with_fs(StdFs, desktop_dir)
    }
}

impl<F: FsOps> DesktopIcons<F> {
    pub fn with_fs(fs: F, desktop_dir: PathBuf) -> Self {
        Self {
            fs,
            desktop_dir,
            icons: Vec::new(),
            selection: BTreeSet::new(),
            drag: None,
            overlay: None,
            action: None,
            cols: 1,
            rows: 1,
        }
    }

    pub fn icons(&self) -> &[DesktopIcon] { &self.icons }
    pub fn selection(&self) -> Vec<usize> { self.selection.iter().copied().collect() }
    pub fn overlay(&self) -> Option<&DesktopOverlay> { self.overlay.as_ref() }
    pub fn is_editing(&self) -> bool {
        matches!(self.overlay, Some(DesktopOverlay::Rename { .. }) | Some(DesktopOverlay::NewFolder { .. }))
    }
    pub fn take_action(&mut self) -> Option<DesktopAction> { self.action.take() }

    /// Rebuild the icon list from the folder + pins, keeping `positions`.
    pub fn reload(&mut self, pins: &[AppEntry], positions: &BTreeMap<String, (u16, u16)>) {
        let prev_keys: BTreeSet<String> =
            self.selection.iter().filter_map(|&i| self.icons.get(i)).map(Self::key_of).collect();
        let mut icons = Vec::new();
        if let Ok(entries) = self.fs.list(&self.desktop_dir, false) {
            for e in entries {
                icons.push(DesktopIcon {
                    label: e.name.clone(),
                    role: e.role,
                    path: e.path,
                    source: IconSource::Folder,
                    command: None,
                    cell: (0, 0),
                    thumb: None,
                });
            }
        }
        for p in pins {
            let target = p.cwd.clone().or_else(|| p.args.first().cloned()).unwrap_or_default();
            let path = PathBuf::from(&target);
            let role = if target.is_empty() { Role::Other } else { classify(&path, path.is_dir()) };
            icons.push(DesktopIcon {
                label: p.name.clone(),
                role,
                path,
                source: IconSource::Pinned,
                command: Some(p.command.clone()),
                cell: (0, 0),
                thumb: None,
            });
        }
        self.icons = icons;
        self.assign_cells(positions);
        // restore selection by key
        self.selection = self
            .icons
            .iter()
            .enumerate()
            .filter(|(_, i)| prev_keys.contains(&Self::key_of(i)))
            .map(|(i, _)| i)
            .collect();
    }

    /// The persistence key for an icon (pin command, else abs path).
    fn key_of(icon: &DesktopIcon) -> String {
        match &icon.command {
            Some(cmd) => cmd.clone(),
            None => icon.path.to_string_lossy().to_string(),
        }
    }
    pub fn icon_key(&self, idx: usize) -> Option<String> { self.icons.get(idx).map(Self::key_of) }

    /// Recompute the grid dimensions for a `w×h` screen.
    pub fn layout(&mut self, w: i32, h: i32) {
        self.cols = ((w / ICON_W).max(1)) as u16;
        // leave the menubar (1) and dock (1) rows out
        self.rows = (((h - GRID_TOP - 1) / ICON_H).max(1)) as u16;
    }

    /// Assign each icon a cell: saved position if present, else first free cell.
    fn assign_cells(&mut self, positions: &BTreeMap<String, (u16, u16)>) {
        let mut taken: BTreeSet<(u16, u16)> = BTreeSet::new();
        // first pass: saved positions
        for icon in &mut self.icons {
            if let Some(&cell) = positions.get(&Self::key_of(icon)) {
                icon.cell = cell;
                taken.insert(cell);
            }
        }
        // second pass: unplaced icons → first free cell, column-major
        for icon in &mut self.icons {
            if positions.contains_key(&Self::key_of(icon)) {
                continue;
            }
            let cell = Self::first_free(&taken, self.cols.max(1), self.rows.max(1));
            icon.cell = cell;
            taken.insert(cell);
        }
    }

    fn first_free(taken: &BTreeSet<(u16, u16)>, cols: u16, rows: u16) -> (u16, u16) {
        for col in 0..cols.max(1) {
            for row in 0..rows.max(1) {
                if !taken.contains(&(col, row)) {
                    return (col, row);
                }
            }
        }
        (0, 0) // grid full: stack at origin
    }

    /// The screen rect of an icon's tile.
    pub fn tile_rect(cell: (u16, u16)) -> crate::geometry::Rect {
        crate::geometry::Rect::new(cell.0 as i32 * ICON_W, GRID_TOP + cell.1 as i32 * ICON_H, ICON_W, ICON_H)
    }

    /// The icon under `p`, if any.
    pub fn icon_at(&self, p: Point) -> Option<usize> {
        self.icons.iter().position(|i| Self::tile_rect(i.cell).contains(p))
    }

    /// Left click: select the icon under `p` (clear others unless `ctrl`); on empty
    /// desktop clear selection and dismiss any overlay.
    pub fn click(&mut self, p: Point, ctrl: bool) {
        self.overlay = None;
        match self.icon_at(p) {
            Some(i) => {
                if ctrl {
                    if !self.selection.remove(&i) { self.selection.insert(i); }
                } else {
                    self.selection.clear();
                    self.selection.insert(i);
                }
            }
            None => self.selection.clear(),
        }
    }

    /// Double click: produce an Open/Run action for the icon under `p`.
    pub fn double_click(&mut self, p: Point) {
        let Some(i) = self.icon_at(p) else { return; };
        let icon = &self.icons[i];
        self.action = Some(match &icon.command {
            Some(cmd) => DesktopAction::Run { command: cmd.clone(), args: vec![] },
            None => DesktopAction::Open(icon.path.clone()),
        });
    }
}
```

Register in `src/lib.rs`: `pub mod desktop;`.

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/desktop.rs src/lib.rs tests/desktop_tests.rs
git commit -m "desktop: model — merge sources, grid cells, hit-test, select, actions"
```

---

### Task 3: `desktop` render

**Files:** `src/desktop.rs`; inline `#[cfg(test)] mod tests`.

- [ ] **Step 1: Write the failing inline test** (append a `mod tests` to `src/desktop.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tuiui-dtr-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn render_returns_screen_sized_buffer_with_label() {
        let d = tmp("render");
        fs::create_dir(d.join("proj")).unwrap();
        let mut dt = DesktopIcons::new(d.clone());
        dt.reload(&[], &BTreeMap::new());
        dt.layout(100, 30);
        let buf = dt.render(100, 30);
        assert_eq!(buf.width(), 100);
        assert_eq!(buf.height(), 30);
        let _ = fs::remove_dir_all(&d);
    }
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --lib desktop`).

- [ ] **Step 3: Implement render.** Append to `impl<F: FsOps>`:

```rust
    /// Render the icon layer into a `w×h` buffer (transparent background so the
    /// wallpaper shows through; only icon tiles draw).
    pub fn render(&self, w: i32, h: i32) -> crate::buffer::CellBuffer {
        use crate::cell::{Cell, Rgba};
        const FG: Rgba = Rgba { r: 220, g: 226, b: 236, a: 255 };
        const SEL_BG: Rgba = Rgba { r: 45, g: 58, b: 85, a: 200 };
        let transparent = Rgba::TRANSPARENT;
        let mut buf = crate::buffer::CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: FG, bg: transparent, attrs: Default::default() });
        for (i, icon) in self.icons.iter().enumerate() {
            let r = Self::tile_rect(icon.cell);
            let selected = self.selection.contains(&i);
            let bg = if selected { SEL_BG } else { transparent };
            // glyph row
            let glyph = glyph_for(icon.role);
            buf.set(r.x + ICON_W / 2, r.y, Cell { ch: glyph, fg: FG, bg, attrs: Default::default() });
            // label row (centered-ish, truncated)
            let name: String = icon.label.chars().take((ICON_W - 1) as usize).collect();
            for (k, _) in name.chars().enumerate() {
                // paint selection bg across the label width
                buf.set(r.x + k as i32, r.y + 1, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() });
            }
            buf.write_str(r.x, r.y + 1, &name, FG, bg);
        }
        buf
    }
```

Free function:

```rust
fn glyph_for(role: Role) -> char {
    match role {
        Role::Directory => '\u{1F4C1}',
        Role::Image => '\u{1F5BC}',
        Role::Audio => '\u{1F3B5}',
        Role::Video => '\u{1F3AC}',
        Role::Archive => '\u{1F4E6}',
        Role::Pdf => '\u{1F4D5}',
        _ => '\u{1F4C4}',
    }
}
```

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/desktop.rs
git commit -m "desktop: render icon tiles (glyph + label + selection)"
```

---

### Task 4: Drag-to-snap + position persistence (model)

**Files:** `src/desktop.rs`; Test `tests/desktop_tests.rs` (append).

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn drag_snaps_to_target_cell_and_reports_position() {
    let d = tmp("drag");
    fs::create_dir(d.join("proj")).unwrap();
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&[], &BTreeMap::new());
    dt.layout(100, 30);
    // proj starts at (0,0); grab it and drop at a point inside cell (2,1)
    dt.begin_drag(Point::new(2, 1));
    let drop = Point::new(2 * 14 + 3, 1 + 1 * 3 + 1); // inside cell (2,1)
    let moved = dt.end_drag(drop);
    assert!(moved); // a move happened
    let key = dt.icon_key(0).unwrap();
    assert_eq!(dt.icons()[0].cell, (2, 1));
    // the model exposes the position to persist
    assert_eq!(dt.position_of(&key), Some((2, 1)));
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.** Append to `impl<F: FsOps>`:

```rust
    pub fn begin_drag(&mut self, p: Point) {
        if let Some(i) = self.icon_at(p) {
            let r = Self::tile_rect(self.icons[i].cell);
            self.drag = Some((i, Point::new(p.x - r.x, p.y - r.y)));
            self.selection.clear();
            self.selection.insert(i);
        }
    }

    pub fn drag_to(&mut self, _p: Point) { /* ghost position is render-only; no-op for state */ }

    /// Finish a drag: snap the dragged icon to the nearest free cell under `p`.
    /// Returns true if its cell changed.
    pub fn end_drag(&mut self, p: Point) -> bool {
        let Some((i, _grab)) = self.drag.take() else { return false; };
        let col = ((p.x / ICON_W).clamp(0, self.cols.max(1) as i32 - 1)) as u16;
        let row = (((p.y - GRID_TOP) / ICON_H).clamp(0, self.rows.max(1) as i32 - 1)) as u16;
        let target = self.nearest_free((col, row), i);
        let changed = self.icons[i].cell != target;
        self.icons[i].cell = target;
        changed
    }

    /// The cell `(col,row)` if free, else the nearest free cell (spiral-ish scan).
    fn nearest_free(&self, want: (u16, u16), ignore: usize) -> (u16, u16) {
        let occupied = |c: (u16, u16)| {
            self.icons.iter().enumerate().any(|(j, ic)| j != ignore && ic.cell == c)
        };
        if !occupied(want) { return want; }
        for radius in 1..(self.cols.max(self.rows) as i32 + 1) {
            for dc in -radius..=radius {
                for dr in -radius..=radius {
                    let c = (want.0 as i32 + dc, want.1 as i32 + dr);
                    if c.0 < 0 || c.1 < 0 || c.0 >= self.cols as i32 || c.1 >= self.rows as i32 { continue; }
                    let cell = (c.0 as u16, c.1 as u16);
                    if !occupied(cell) { return cell; }
                }
            }
        }
        want
    }

    /// The current cell of the icon with key `key` (for persistence).
    pub fn position_of(&self, key: &str) -> Option<(u16, u16)> {
        self.icons.iter().find(|i| Self::key_of(i) == key).map(|i| i.cell)
    }

    /// All current positions (for bulk persistence after Clean Up).
    pub fn positions(&self) -> BTreeMap<String, (u16, u16)> {
        self.icons.iter().map(|i| (Self::key_of(i), i.cell)).collect()
    }

    /// Re-flow every icon to column-major order (Clean Up).
    pub fn clean_up(&mut self) {
        let mut taken = BTreeSet::new();
        for icon in &mut self.icons {
            let cell = Self::first_free(&taken, self.cols.max(1), self.rows.max(1));
            icon.cell = cell;
            taken.insert(cell);
        }
    }

    pub fn dragging(&self) -> bool { self.drag.is_some() }
```

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/desktop.rs tests/desktop_tests.rs
git commit -m "desktop: drag-to-snap, nearest-free-cell, positions + clean up"
```

---

### Task 5: Protocol + client (double-click & right-click detection)

**Files:** `src/protocol.rs`, `src/session.rs` (ClientMsg), `src/client.rs`; Test `tests/protocol_tests.rs` or inline (a serde round-trip of the new messages).

- [ ] **Step 1: Add a failing test** (`tests/protocol_tests.rs`, append) verifying the new messages serialize:

```rust
#[test]
fn new_mouse_messages_roundtrip() {
    use tuiui::session::ClientMsg;
    use tuiui::geometry::Point;
    for msg in [ClientMsg::MouseDouble(Point::new(3, 4)), ClientMsg::MouseRightDown(Point::new(5, 6))] {
        let s = serde_json::to_string(&msg).unwrap();
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        // Debug-compare (ClientMsg may not be PartialEq); compare via re-serialization.
        assert_eq!(serde_json::to_string(&back).unwrap(), s);
    }
}
```

> If `serde_json` isn't a dev-dependency, check how existing protocol tests round-trip (they may use `bincode`/the project's wire codec) and match that; the project already serializes `ClientMsg`, so reuse the same codec.

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.**

(a) `src/session.rs` — add to `enum ClientMsg`:

```rust
    MouseDouble(Point),
    MouseRightDown(Point),
    DesktopChar(char),
    DesktopBackspace,
    DesktopCommit,
    DesktopCancel,
```

(b) `src/protocol.rs` — add to `Flags`:

```rust
    /// The desktop has a rename/new-folder overlay open; forward typed chars.
    pub desktop_editing: bool,
```

(c) `src/client.rs` — double-click + right-click detection. Near the mouse handling, keep a small bit of state in the event loop (the client is a live process). Before the `match event::read()?`, add (in the enclosing scope) `let mut last_click: Option<(Point, std::time::Instant)> = None;`. In the `Event::Mouse` arm:

```rust
        MouseEventKind::Down(MouseButton::Left) => {
            let now = std::time::Instant::now();
            let dbl = last_click
                .map(|(lp, lt)| lp == p && now.duration_since(lt) < std::time::Duration::from_millis(400))
                .unwrap_or(false);
            if dbl {
                send(&mut out_stream, &ClientMsg::MouseDouble(p))?;
                last_click = None;
            } else {
                send(&mut out_stream, &ClientMsg::MouseDown(p))?;
                last_click = Some((p, now));
            }
        }
        MouseEventKind::Down(MouseButton::Right) => send(&mut out_stream, &ClientMsg::MouseRightDown(p))?,
```

> Keep the existing `Drag(Left)`/`Up(Left)`/`Moved`/scroll arms. `last_click` must be declared where it persists across loop iterations (outside the `loop`/`for`), so place it with the other pre-loop locals. The double-click still sends nothing redundant — the FIRST click already went as `MouseDown` (selecting); the second becomes `MouseDouble` (opening). That matches the desktop's click-then-double semantics.

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test protocol_tests`). Build the client.

- [ ] **Step 5: Commit:**

```bash
git add src/protocol.rs src/session.rs src/client.rs tests/protocol_tests.rs
git commit -m "protocol: MouseDouble + MouseRightDown + desktop_editing; client detection"
```

---

### Task 6: Session wiring — own desktop, render layer, fall-through routing, dispatch

**Files:** `src/session.rs`; Test `tests/session_tests.rs` (append).

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn desktop_click_selects_and_double_click_opens_files() {
    let dir = std::env::temp_dir().join(format!("tuiui-deskwire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir(dir.join("proj")).unwrap();

    // Point the desktop at our temp dir via a test helper.
    let mut core = SessionCore::new(100, 30, Config { desktop_pins: vec![], ..Config::default() });
    core.set_desktop_dir_for_test(dir.clone()); // reloads desktop at `dir`
    // "proj" is at cell (0,0): tile glyph at (7,1); click then double-click.
    let p = tuiui::geometry::Point::new(2, 1);
    core.apply(ClientMsg::MouseDown(p));
    assert_eq!(core.desktop_selection_len_for_test(), 1);
    let before = core.window_count();
    core.apply(ClientMsg::MouseDouble(p));
    assert_eq!(core.window_count(), before + 1); // a Files window opened on the folder
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
```

> Add two `#[doc(hidden)] pub` test helpers on `SessionCore`: `set_desktop_dir_for_test(&mut self, dir: PathBuf)` (sets the desktop dir, calls `reload_desktop`) and `desktop_selection_len_for_test(&self) -> usize`. Use `#[doc(hidden)] pub` (not `#[cfg(test)]`) so integration tests can call them.

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.**

(a) Field on `SessionCore`: `desktop: crate::desktop::DesktopIcons,`. Initialize in `SessionCore::new` with the desktop dir (`dirs::home_dir().map(|h| h.join("Desktop")).unwrap_or_default()`), then `reload_desktop()`.

(b) Helpers:

```rust
fn reload_desktop(&mut self) {
    self.desktop.reload(&self.cfg.desktop_pins, &self.cfg.desktop_positions);
    self.desktop.layout(self.w, self.h);
    self.refresh_desktop_thumbnails(); // Task 8 adds the body; stub returns early until then
}

#[doc(hidden)]
pub fn set_desktop_dir_for_test(&mut self, dir: std::path::PathBuf) {
    self.desktop = crate::desktop::DesktopIcons::new(dir);
    self.reload_desktop();
}
#[doc(hidden)]
pub fn desktop_selection_len_for_test(&self) -> usize { self.desktop.selection().len() }
```

> For Task 6, add a no-op `fn refresh_desktop_thumbnails(&mut self) {}` (Task 8 fills it in).

(c) `build_frame` — render the desktop layer FIRST (before the window loop) so windows overlay it, only when `self.cfg.desktop_enabled`:

```rust
        if self.cfg.desktop_enabled {
            let buf = self.desktop.render(self.w, self.h);
            layers.push(Layer { z: 0, origin: Point::new(0, 0), buf, opacity: 1.0, scissor: None });
        }
```

(d) `handle_mouse` — fall-through routing. The desktop only gets a click when no window is hit. The cleanest seam: compute the window route first; if it resolves to a no-op on the empty desktop, hand the click to the desktop. Concretely, after the chrome checks and the `windows`/`route_mouse` computation:

```rust
        let action = route_mouse(kind, p, &windows, self.drag);
        // A left/right/double click that hits no window falls through to the desktop.
        if self.cfg.desktop_enabled && matches!(action, Action::None) && self.drag.is_none() {
            match kind {
                MouseKind::Down => { self.desktop.click(p, false); return; }
                _ => {}
            }
        }
        self.exec(action, p);
```

> Check the real name of the "nothing happened" `Action` variant (likely `Action::None` or similar); if `route_mouse` returns something else for an empty-desktop click, match that. The desktop click must NOT fire when a window was hit.

(e) Route the new `ClientMsg` in `apply`:

```rust
ClientMsg::MouseRightDown(p) => self.handle_desktop_right(p),
ClientMsg::MouseDouble(p) => {
    if self.cfg.desktop_enabled && self.window_at_is_none(p) {
        self.desktop.double_click(p);
        self.drain_desktop_action();
    }
}
ClientMsg::DesktopChar(c) => { self.desktop.overlay_char(c); }
ClientMsg::DesktopBackspace => { self.desktop.overlay_backspace(); }
ClientMsg::DesktopCommit => self.desktop_commit(),
ClientMsg::DesktopCancel => { self.desktop.cancel_overlay(); }
```

> `window_at_is_none(p)` — a small helper: true if no non-minimized window's rect contains `p`. (Task 7 adds `handle_desktop_right`, `overlay_char/backspace`, `cancel_overlay`, `desktop_commit`; for Task 6 add minimal stubs: `handle_desktop_right` = no-op, the overlay char/backspace/cancel/commit can be stubs that compile. Or implement Task 7 fully before wiring these — your call, but keep Task 6 compiling. Simplest: in Task 6 only wire `MouseRightDown`→no-op and `MouseDouble`; defer the Desktop* text messages' apply arms to Task 7.)

(f) `drain_desktop_action` (mirror `drain_fm_action`):

```rust
fn drain_desktop_action(&mut self) {
    let action = self.desktop.take_action();
    match action {
        Some(crate::desktop::DesktopAction::Open(path)) => {
            let is_dir = path.is_dir();
            match crate::openwith::resolve(&path, is_dir, &self.cfg.default_apps) {
                crate::openwith::OpenAction::Navigate => self.open_filemanager_root(path),
                crate::openwith::OpenAction::Builtin("@image") => self.open_image(path.to_string_lossy().to_string()),
                crate::openwith::OpenAction::Builtin(_) => {}
                crate::openwith::OpenAction::RunApp { command, args } => {
                    let name = args.last().and_then(|a| a.rsplit('/').next()).unwrap_or(&command).to_string();
                    self.launch_in(name, command, args, path.parent().map(|p| p.to_path_buf()));
                }
                crate::openwith::OpenAction::OpenWithMenu => {}
            }
        }
        Some(crate::desktop::DesktopAction::Run { command, args }) => {
            self.launch_entry(crate::config::AppEntry { name: command.clone(), command, args, category: None, requires_cwd: None, cwd: None });
        }
        Some(crate::desktop::DesktopAction::Unpin(cmd)) => {
            self.cfg.desktop_pins.retain(|p| p.command != cmd);
            let _ = self.cfg.save();
            self.reload_desktop();
        }
        None => {}
    }
}
```

> `open_filemanager_root` is private and opens a Files window at a dir — reuse it. For a file whose role is `Navigate` only directories qualify; a folder opens Files there.

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test session_tests desktop_click`).

- [ ] **Step 5: Commit:**

```bash
git add src/session.rs tests/session_tests.rs
git commit -m "desktop: session wiring — render layer, fall-through clicks, open dispatch"
```

---

### Task 7: Context menus + rename/new-folder/trash + drag persistence

**Files:** `src/desktop.rs`, `src/session.rs`, `src/client.rs`; Test `tests/desktop_tests.rs` + `tests/session_tests.rs`.

- [ ] **Step 1: Append model tests** (`tests/desktop_tests.rs`):

```rust
#[test]
fn right_click_opens_context_and_menu_targets() {
    use tuiui::desktop::DesktopOverlay;
    let d = tmp("ctx");
    fs::write(d.join("a.txt"), b"x").unwrap();
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&[], &BTreeMap::new());
    dt.layout(100, 30);
    dt.right_click(Point::new(2, 1)); // on the icon
    assert!(matches!(dt.overlay(), Some(DesktopOverlay::Context { .. })));
    dt.right_click(Point::new(60, 20)); // empty desktop
    assert!(matches!(dt.overlay(), Some(DesktopOverlay::DesktopMenu { .. })));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn rename_overlay_edits_and_commit_requests_fs() {
    let d = tmp("ren");
    fs::write(d.join("old.txt"), b"x").unwrap();
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&[], &BTreeMap::new());
    dt.layout(100, 30);
    dt.begin_rename(0);
    for _ in 0.."old.txt".len() { dt.overlay_backspace(); }
    for c in "new.txt".chars() { dt.overlay_char(c); }
    // commit performs the rename via the model's fs (StdFs on temp dir)
    dt.overlay_commit();
    assert!(d.join("new.txt").exists());
    assert!(!d.join("old.txt").exists());
    let _ = fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement model side** (`src/desktop.rs`). Append:

```rust
    pub fn right_click(&mut self, p: Point) {
        match self.icon_at(p) {
            Some(i) => {
                self.selection.clear();
                self.selection.insert(i);
                self.overlay = Some(DesktopOverlay::Context { idx: i, anchor: p });
            }
            None => self.overlay = Some(DesktopOverlay::DesktopMenu { anchor: p }),
        }
    }

    pub fn begin_rename(&mut self, idx: usize) {
        if let Some(icon) = self.icons.get(idx) {
            if matches!(icon.source, IconSource::Folder) {
                self.overlay = Some(DesktopOverlay::Rename { idx, name: icon.label.clone() });
            }
        }
    }
    pub fn begin_new_folder(&mut self) { self.overlay = Some(DesktopOverlay::NewFolder { name: String::new() }); }
    pub fn cancel_overlay(&mut self) { self.overlay = None; }

    pub fn overlay_char(&mut self, c: char) {
        match &mut self.overlay {
            Some(DesktopOverlay::Rename { name, .. }) | Some(DesktopOverlay::NewFolder { name }) => name.push(c),
            _ => {}
        }
    }
    pub fn overlay_backspace(&mut self) {
        match &mut self.overlay {
            Some(DesktopOverlay::Rename { name, .. }) | Some(DesktopOverlay::NewFolder { name }) => { name.pop(); }
            _ => {}
        }
    }

    /// Commit rename/new-folder via the fs; returns true if the folder changed
    /// (so the session reloads). Trash is requested via `request_trash`.
    pub fn overlay_commit(&mut self) -> bool {
        match self.overlay.take() {
            Some(DesktopOverlay::Rename { idx, name }) if !name.trim().is_empty() => {
                if let Some(icon) = self.icons.get(idx) {
                    let _ = self.fs.rename(&icon.path.clone(), name.trim());
                }
                true
            }
            Some(DesktopOverlay::NewFolder { name }) if !name.trim().is_empty() => {
                let _ = self.fs.mkdir(&self.desktop_dir.clone(), name.trim());
                true
            }
            _ => false,
        }
    }

    /// Move the selected folder icons to Trash; returns true if anything was trashed.
    pub fn trash_selection(&mut self) -> bool {
        let paths: Vec<PathBuf> = self
            .selection
            .iter()
            .filter_map(|&i| self.icons.get(i))
            .filter(|i| matches!(i.source, IconSource::Folder))
            .map(|i| i.path.clone())
            .collect();
        let mut any = false;
        for p in paths {
            if self.fs.trash(&p).is_ok() { any = true; }
        }
        self.overlay = None;
        any
    }

    /// Request the action behind a context-menu choice. The session calls these on
    /// the menu item the user clicked (open/rename/trash/unpin/open-with handled
    /// session-side; here we set up overlays / emit actions where pure).
    pub fn context_idx(&self) -> Option<usize> {
        match self.overlay {
            Some(DesktopOverlay::Context { idx, .. }) => Some(idx),
            _ => None,
        }
    }
    pub fn icon_is_pinned(&self, idx: usize) -> bool {
        self.icons.get(idx).map(|i| matches!(i.source, IconSource::Pinned)).unwrap_or(false)
    }
    pub fn icon_command(&self, idx: usize) -> Option<String> {
        self.icons.get(idx).and_then(|i| i.command.clone())
    }
    pub fn icon_path(&self, idx: usize) -> Option<PathBuf> { self.icons.get(idx).map(|i| i.path.clone()) }
```

Also render the overlay (context menu / text box) on top — add a small `render_overlay(&self, buf, w, h)` called at the end of `render`, drawing a menu box at the anchor with the appropriate items (Folder: Open/Open with…/Rename/Move to Trash; Pinned: Open/Unpin; DesktopMenu: New Folder/Clean Up; Rename/NewFolder: a text field). Keep it simple (a boxed list); hit-testing the menu items is done session-side via `menu_item_at(p) -> Option<DesktopMenuItem>` — add that enum + method so the session can act on a click in an open menu.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopMenuItem { Open, OpenWith, Rename, Trash, Unpin, NewFolder, CleanUp }
```

`menu_item_at(&self, p: Point) -> Option<DesktopMenuItem>` maps a click within the open menu's rows to an item (compute the same row rects `render_overlay` uses).

- [ ] **Step 4: Wire the session + client.** In `session.rs`:
- `handle_desktop_right(p)` → `self.desktop.right_click(p); ` (renders the menu).
- When a menu is open, a left `MouseDown` should be routed to the menu first (before fall-through): in `handle_mouse`, if `self.desktop.overlay().is_some()`, try `self.desktop.menu_item_at(p)` and act: `Open`→`double_click`+drain; `Rename`→`begin_rename(idx)`; `Trash`→`if self.desktop.trash_selection() { self.reload_desktop(); }`; `Unpin`→push `DesktopAction::Unpin` (or directly remove + save + reload); `NewFolder`→`begin_new_folder`; `CleanUp`→`self.desktop.clean_up(); self.persist_desktop_positions();`; a click outside the menu cancels. Then `return`.
- `desktop_commit()` → `if self.desktop.overlay_commit() { self.reload_desktop(); }`.
- `persist_desktop_positions()` → `self.cfg.desktop_positions = self.desktop.positions(); let _ = self.cfg.save();`.
- After a successful `end_drag` (drag handled in `exec`/mouse-up path), call `persist_desktop_positions()`. **Drag wiring:** in `handle_mouse`, when a left `Down` falls through to the desktop AND hits an icon, call `self.desktop.begin_drag(p)` instead of `click` (then on `MouseKind::Up` with `self.desktop.dragging()`, call `if self.desktop.end_drag(p) { self.persist_desktop_positions(); }`). A down on an icon that isn't followed by movement still selects (begin_drag selects the icon); a plain click thus selects, a drag moves+persists.
- `Flags.desktop_editing` populated in `daemon.rs`: `desktop_editing: core.desktop_editing()` (add `pub fn desktop_editing(&self) -> bool { self.desktop.is_editing() }`).
- `client.rs`: add a branch `} else if f.desktop_editing { match k.code { Esc=>DesktopCancel, Enter=>DesktopCommit, Backspace=>DesktopBackspace, Char(c)=>DesktopChar(c), _=>{} } }` before the default key forwarding.

- [ ] **Step 5: Append a session test:**

```rust
#[test]
fn desktop_new_folder_via_menu_creates_dir() {
    let dir = std::env::temp_dir().join(format!("tuiui-deskmk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut core = SessionCore::new(100, 30, Config { desktop_pins: vec![], ..Config::default() });
    core.set_desktop_dir_for_test(dir.clone());
    core.apply(ClientMsg::MouseRightDown(tuiui::geometry::Point::new(60, 20))); // empty desktop menu
    // Drive new-folder directly via the editing messages (menu click tested in unit tests):
    core.begin_desktop_new_folder_for_test();
    for c in "Stuff".chars() { core.apply(ClientMsg::DesktopChar(c)); }
    core.apply(ClientMsg::DesktopCommit);
    assert!(dir.join("Stuff").is_dir());
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
```

> Add `#[doc(hidden)] pub fn begin_desktop_new_folder_for_test(&mut self) { self.desktop.begin_new_folder(); }`.

- [ ] **Step 6: Run → PASS.** Commit:

```bash
git add src/desktop.rs src/session.rs src/client.rs src/daemon.rs tests/desktop_tests.rs tests/session_tests.rs
git commit -m "desktop: context menus, rename/new-folder/trash, drag persistence"
```

---

### Task 8: Thumbnails

**Files:** `src/desktop.rs`, `src/session.rs`; Test `tests/session_tests.rs`.

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn desktop_image_icon_emits_thumbnail_placement() {
    let dir = std::env::temp_dir().join(format!("tuiui-deskthumb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([9, 9, 9, 255]));
    image::DynamicImage::ImageRgba8(img).save(dir.join("p.png")).unwrap();

    let mut core = SessionCore::new(100, 30, Config { desktop_pins: vec![], ..Config::default() });
    core.set_desktop_dir_for_test(dir.clone());
    let frame = core.build_frame();
    assert!(frame.images.iter().any(|pl| pl.cols >= 1), "expected a desktop thumbnail placement");
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.**

(a) `desktop.rs` — add `set_thumb` + `thumbnail_requests` + `thumbnail_placements` (mirror the FM):

```rust
    pub fn thumbnail_requests(&self) -> Vec<(usize, PathBuf)> {
        self.icons.iter().enumerate()
            .filter(|(_, i)| i.role == Role::Image && !i.path.as_os_str().is_empty())
            .map(|(i, ic)| (i, ic.path.clone()))
            .collect()
    }
    pub fn set_thumb(&mut self, idx: usize, id: u64) {
        if let Some(i) = self.icons.get_mut(idx) { i.thumb = Some(id); }
    }
    /// Placements for loaded thumbnails; `is_visible(rect)` decides occlusion.
    pub fn thumbnail_placements(&self, visible: impl Fn(crate::geometry::Rect) -> bool) -> Vec<crate::protocol::ImagePlacement> {
        let mut out = Vec::new();
        for icon in &self.icons {
            if let Some(id) = icon.thumb {
                let r = Self::tile_rect(icon.cell);
                let cell = crate::geometry::Rect::new(r.x + ICON_W / 2 - 1, r.y, 2, 1);
                out.push(crate::protocol::ImagePlacement {
                    id, rect: cell, cols: cell.w.max(1) as u16, rows: cell.h.max(1) as u16,
                    visible: visible(r),
                });
            }
        }
        out.sort_by_key(|p| p.id);
        out
    }
```

(b) `session.rs` — fill in `refresh_desktop_thumbnails` (was a stub from Task 6):

```rust
fn refresh_desktop_thumbnails(&mut self) {
    let reqs = self.desktop.thumbnail_requests();
    for (idx, path) in reqs {
        if let Some(id) = self.images.load(&path, 13 * 8, 16) {
            self.desktop.set_thumb(idx, id);
        }
    }
}
```

(c) `build_frame` — after the FM thumbnail loop, emit desktop placements (only when enabled). A desktop icon tile is "visible" when no non-minimized window covers it:

```rust
        if self.cfg.desktop_enabled {
            let occluded = |r: crate::geometry::Rect| {
                self.wm.z_ordered().iter().any(|w| !w.minimized && w.rect.intersect(r).is_some())
            };
            images.extend(self.desktop.thumbnail_placements(|r| !occluded(r)));
        }
```

- [ ] **Step 4: Run → PASS.** Then the FULL gate (`build && test && clippy --all-targets`, 0 warnings).

- [ ] **Step 5: Commit:**

```bash
git add src/desktop.rs src/session.rs tests/session_tests.rs
git commit -m "desktop: image thumbnails via A1 placements"
```

---

### Task 9: Manual verification + docs

- [ ] **Step 1:** Full gate green.
- [ ] **Step 2:** `cargo install --path . --root ~/.local --force`; on the host `tuiui kill ; tuiui`. Verify: `~/Desktop` files + the Files/Store pins show as icons; double-click a folder opens **Files** there, a file opens its default app, a pin launches; drag an icon → it snaps and the position survives a restart; right-click an icon → Open/Rename/Trash, right-click empty → New Folder/Clean Up; an image on the Desktop shows a thumbnail (Ghostty/Kitty).
- [ ] **Step 3:** Update `README.md` — add desktop icons to the features list + the roadmap line (`✅ Desktop icons`), and a one-line controls note (double-click opens, drag to arrange, right-click for the menu). Commit:

```bash
git add README.md
git commit -m "docs: desktop icons feature + controls in README"
```

---

## Notes for the implementer
- The desktop only ever sees **fall-through** clicks (no window hit), so existing window/chrome mouse behavior is untouched — verify a click on a window does NOT reach the desktop (the session test `desktop_click_...` implicitly relies on this; add an explicit check if cheap).
- Icons render at **z=0** (under windows); the context/rename **overlay** should render above windows — render the overlay into a separate high-z layer in `build_frame` (e.g. just below the launcher), not into the z=0 icon buffer.
- Effect dispatch, Trash, and `openwith::resolve` are reused from B/C — do not reimplement them.
- Thumbnails live in the session `ImageStore` (loaded by `refresh_desktop_thumbnails`); the widget only stores ids + computes cell rects, exactly like the file manager.
- Persisted positions are written on drag-drop and Clean Up via `Config::save()`; new `~/Desktop` files auto-place in the first free cell.
