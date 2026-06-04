# Grid Tiling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A user-configurable R×C grid with drag-to-cell snapping, an auto-tile mode, keyboard send-to-cell, and a tile-all command.

**Architecture:** A pure `Grid` model in `geometry.rs` (cell↔rect math) drives every interaction. `WindowState` gains a `Tiled { row, col }` variant (existing `Snapped*`/`Maximized` kept). The window manager gains `send_to_cell`/`tile_all`/`swap_cells`. The session wires keyboard commands, drag-to-cell preview/snap, and auto-tile re-application; settings expose the grid. Existing left/right half-snap is untouched.

**Tech Stack:** Existing Rust WM/compositor/session; no new dependencies.

**Reference spec:** `docs/superpowers/specs/2026-06-04-grid-tiling-design.md`

---

## File Structure

- **Modify `src/geometry.rs`** — add `Grid` + `cell_rect`/`cell_at`/`index_of`/`row_col`.
- **Modify `src/window.rs`** — add `WindowState::Tiled { row, col }`.
- **Modify `src/wm.rs`** — add `send_to_cell`, `tile_all`, `swap_cells`.
- **Modify `src/config.rs`** — add `grid_rows`/`grid_cols`/`tile_gap`/`auto_tile`.
- **Modify `src/session.rs`** — `ClientMsg` variants, handlers, drag-to-cell preview/snap, auto-tile re-apply, swap-on-drop.
- **Modify `src/client.rs`** — leader map `t`/`T`/`1..9`.
- **Modify `src/settings.rs`** — Windows section: grid rows/cols/gap/auto-tile.
- **Tests:** `tests/geometry_tests.rs`, `tests/wm_tests.rs` (append).

---

### Task 1: `Grid` cell math

**Files:** Modify `src/geometry.rs`; Test `tests/geometry_tests.rs`.

- [ ] **Step 1: Write the failing test** (append to `tests/geometry_tests.rs`):

```rust
use tuiui::geometry::Grid;

#[test]
fn grid_cell_rects_tile_the_work_area() {
    let work = Rect::new(0, 1, 12, 6); // 12 wide, 6 tall
    let g = Grid { rows: 2, cols: 3 };
    assert_eq!(g.cells(), 6);
    // top-left cell
    assert_eq!(g.cell_rect(work, 0, 0, 0), Rect::new(0, 1, 4, 3));
    // bottom-right cell
    assert_eq!(g.cell_rect(work, 1, 2, 0), Rect::new(8, 4, 4, 3));
}

#[test]
fn grid_cell_at_maps_pointer_to_cell() {
    let work = Rect::new(0, 1, 12, 6);
    let g = Grid { rows: 2, cols: 3 };
    assert_eq!(g.cell_at(work, Point::new(1, 2)), (0, 0));
    assert_eq!(g.cell_at(work, Point::new(9, 5)), (1, 2));
    // out of bounds clamps
    assert_eq!(g.cell_at(work, Point::new(999, 999)), (1, 2));
}

#[test]
fn grid_index_round_trip() {
    let g = Grid { rows: 2, cols: 3 };
    assert_eq!(g.index_of(1, 2), 5);
    assert_eq!(g.row_col(5), (1, 2));
    assert_eq!(g.row_col(0), (0, 0));
}

#[test]
fn grid_gap_shrinks_cells() {
    let work = Rect::new(0, 0, 12, 4);
    let g = Grid { rows: 1, cols: 2 };
    // gap of 1 inserts a 1-col gutter between the two cells
    let a = g.cell_rect(work, 0, 0, 1);
    let b = g.cell_rect(work, 0, 1, 1);
    assert!(a.x + a.w <= b.x); // no overlap, gutter present
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --offline --test geometry_tests`
Expected: FAIL — `Grid` not found.

- [ ] **Step 3: Implement in `src/geometry.rs`**

```rust
/// A rows×cols tiling grid over a work area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid { pub rows: u8, pub cols: u8 }

impl Grid {
    /// Total cell count (never zero — rows/cols are treated as at least 1).
    pub fn cells(&self) -> u8 { self.rows.max(1) * self.cols.max(1) }

    /// Rect for cell (row, col) within `work`, leaving `gap` cells of gutter
    /// between adjacent cells. Remainder columns/rows widen the first cells so
    /// the grid covers the whole work area.
    pub fn cell_rect(&self, work: Rect, row: u8, col: u8, gap: i32) -> Rect {
        let (rows, cols) = (self.rows.max(1) as i32, self.cols.max(1) as i32);
        let (row, col) = (row.min(self.rows.max(1) - 1) as i32, col.min(self.cols.max(1) - 1) as i32);
        let total_gx = gap * (cols - 1);
        let total_gy = gap * (rows - 1);
        let cw = (work.w - total_gx).max(cols) / cols;
        let ch = (work.h - total_gy).max(rows) / rows;
        let x = work.x + col * (cw + gap);
        let y = work.y + row * (ch + gap);
        // Last cell absorbs the remainder so the grid reaches the work edge.
        let w = if col == cols - 1 { work.x + work.w - x } else { cw };
        let h = if row == rows - 1 { work.y + work.h - y } else { ch };
        Rect::new(x, y, w.max(1), h.max(1))
    }

    /// The (row, col) cell whose region contains `p` (clamped to the grid).
    pub fn cell_at(&self, work: Rect, p: Point) -> (u8, u8) {
        let (rows, cols) = (self.rows.max(1) as i32, self.cols.max(1) as i32);
        let dx = (p.x - work.x).clamp(0, work.w - 1);
        let dy = (p.y - work.y).clamp(0, work.h - 1);
        let col = (dx * cols / work.w.max(1)).clamp(0, cols - 1);
        let row = (dy * rows / work.h.max(1)).clamp(0, rows - 1);
        (row as u8, col as u8)
    }

    /// Row-major cell index for (row, col).
    pub fn index_of(&self, row: u8, col: u8) -> u8 { row * self.cols.max(1) + col }
    /// (row, col) for a row-major index.
    pub fn row_col(&self, index: u8) -> (u8, u8) {
        let cols = self.cols.max(1);
        (index / cols, index % cols)
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --offline --test geometry_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/geometry.rs tests/geometry_tests.rs
git commit -m "tiling: Grid cell math (cell_rect/cell_at/index round-trip)"
```

---

### Task 2: `WindowState::Tiled` + WM placement ops

**Files:** Modify `src/window.rs`, `src/wm.rs`; Test `tests/wm_tests.rs`.

- [ ] **Step 1: Write the failing test** (append to `tests/wm_tests.rs`):

```rust
use tuiui::geometry::Grid;

#[test]
fn send_to_cell_places_window_in_grid() {
    let work = Rect::new(0, 1, 12, 6);
    let mut m = WindowManager::new(work);
    let a = m.add_window("a".into(), Rect::new(0, 1, 3, 3));
    m.send_to_cell(a, Grid { rows: 2, cols: 3 }, 1, 2, 0);
    let w = m.get(a).unwrap();
    assert_eq!(w.rect, Rect::new(8, 4, 4, 3));
    assert_eq!(w.state, WindowState::Tiled { row: 1, col: 2 });
}

#[test]
fn tile_all_assigns_cells_in_z_order() {
    let work = Rect::new(0, 1, 12, 6);
    let mut m = WindowManager::new(work);
    let a = m.add_window("a".into(), Rect::new(0, 1, 3, 3));
    let b = m.add_window("b".into(), Rect::new(0, 1, 3, 3));
    m.tile_all(Grid { rows: 1, cols: 2 }, 0);
    assert_eq!(m.get(a).unwrap().rect, Rect::new(0, 1, 6, 6));
    assert_eq!(m.get(b).unwrap().rect, Rect::new(6, 1, 6, 6));
}

#[test]
fn swap_cells_exchanges_two_windows() {
    let work = Rect::new(0, 1, 12, 6);
    let mut m = WindowManager::new(work);
    let a = m.add_window("a".into(), Rect::new(0, 1, 3, 3));
    let b = m.add_window("b".into(), Rect::new(0, 1, 3, 3));
    m.tile_all(Grid { rows: 1, cols: 2 }, 0);
    let (ra, rb) = (m.get(a).unwrap().rect, m.get(b).unwrap().rect);
    m.swap_cells(a, b);
    assert_eq!(m.get(a).unwrap().rect, rb);
    assert_eq!(m.get(b).unwrap().rect, ra);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --offline --test wm_tests`
Expected: FAIL — `Tiled`, `send_to_cell`, `tile_all`, `swap_cells` not found.

- [ ] **Step 3: Implement**

In `src/window.rs`, add a variant to `WindowState`:

```rust
    /// Tiled into grid cell (row, col).
    Tiled { row: u8, col: u8 },
```

In `src/wm.rs`, add to `impl WindowManager` (use `crate::geometry::Grid`):

```rust
    /// Place window `id` into grid cell (row, col), saving its floating rect for
    /// restore. Resizes/positions it to the cell and records the `Tiled` state.
    pub fn send_to_cell(&mut self, id: WindowId, grid: crate::geometry::Grid, row: u8, col: u8, gap: i32) {
        let work = self.work;
        if let Some(w) = self.get_mut(id) {
            if w.state == WindowState::Floating { w.restore_rect = w.rect; }
            w.rect = grid.cell_rect(work, row, col, gap);
            w.state = WindowState::Tiled { row, col };
        }
    }

    /// Arrange all non-minimized windows into the grid in z-order (row-major).
    /// Windows beyond `grid.cells()` are left untouched (they float on top).
    pub fn tile_all(&mut self, grid: crate::geometry::Grid, gap: i32) {
        let ids: Vec<WindowId> = {
            let mut v: Vec<&Window> = self.windows.iter().filter(|w| !w.minimized).collect();
            v.sort_by_key(|w| w.z);
            v.into_iter().map(|w| w.id).collect()
        };
        for (i, id) in ids.iter().enumerate() {
            if i as u8 >= grid.cells() { break; }
            let (row, col) = grid.row_col(i as u8);
            self.send_to_cell(*id, grid, row, col, gap);
        }
    }

    /// Swap the rects and tiled-states of two windows (auto-tile drag swap).
    pub fn swap_cells(&mut self, a: WindowId, b: WindowId) {
        let (ra, sa) = match self.get(a) { Some(w) => (w.rect, w.state), None => return };
        let (rb, sb) = match self.get(b) { Some(w) => (w.rect, w.state), None => return };
        if let Some(w) = self.get_mut(a) { w.rect = rb; w.state = sb; }
        if let Some(w) = self.get_mut(b) { w.rect = ra; w.state = sa; }
    }
```

(`WindowState` must be `Copy` for the swap; it already derives `Clone, Copy` — verify and add `Copy` if missing.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --offline --test wm_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/window.rs src/wm.rs tests/wm_tests.rs
git commit -m "tiling: WindowState::Tiled + send_to_cell/tile_all/swap_cells"
```

---

### Task 3: Config fields

**Files:** Modify `src/config.rs`; Test `tests/config_tests.rs`.

- [ ] **Step 1: Write the failing test** (append):

```rust
#[test]
fn config_defaults_grid_2x2() {
    let c = Config::default();
    assert_eq!(c.grid_rows, 2);
    assert_eq!(c.grid_cols, 2);
    assert_eq!(c.tile_gap, 0);
    assert!(!c.auto_tile);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --offline --test config_tests`
Expected: FAIL — fields not found.

- [ ] **Step 3: Implement** — add to `Config` struct and `Default`:

```rust
    /// Tiling grid rows (1..=6).
    pub grid_rows: u8,
    /// Tiling grid columns (1..=6).
    pub grid_cols: u8,
    /// Cells of gutter between tiled windows.
    pub tile_gap: i32,
    /// Auto-arrange all windows into the grid.
    pub auto_tile: bool,
```

In `Default`: `grid_rows: 2, grid_cols: 2, tile_gap: 0, auto_tile: false,`. (The `#[serde(default)]` on `Config` makes these optional in existing TOML files.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --offline --test config_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs tests/config_tests.rs
git commit -m "tiling: config grid_rows/grid_cols/tile_gap/auto_tile"
```

---

### Task 4: Keyboard commands (ClientMsg + client leader map + handlers)

**Files:** Modify `src/session.rs`, `src/client.rs`.

- [ ] **Step 1: Add `ClientMsg` variants** (in `src/session.rs`, near `SnapFocused`):

```rust
    /// Tile all windows into the configured grid (one-shot).
    TileAll,
    /// Toggle auto-tile mode.
    ToggleAutoTile,
    /// Send the focused window to grid cell N (1-based, row-major).
    SendToCell(u8),
```

- [ ] **Step 2: Handle them in `apply`** (add arms):

```rust
            ClientMsg::TileAll => {
                let grid = self.grid();
                self.wm.tile_all(grid, self.cfg.tile_gap);
                self.sync_all_app_sizes();
            }
            ClientMsg::ToggleAutoTile => {
                self.cfg.auto_tile = !self.cfg.auto_tile;
                let _ = self.cfg.save();
                if self.cfg.auto_tile { let g = self.grid(); self.wm.tile_all(g, self.cfg.tile_gap); self.sync_all_app_sizes(); }
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
```

Add helpers to `impl SessionCore`:

```rust
    fn grid(&self) -> crate::geometry::Grid {
        crate::geometry::Grid { rows: self.cfg.grid_rows.clamp(1, 6), cols: self.cfg.grid_cols.clamp(1, 6) }
    }
    fn sync_all_app_sizes(&mut self) {
        let ids: Vec<_> = self.wm.z_ordered().iter().map(|w| w.id).collect();
        for id in ids { self.sync_app_size(id); }
    }
```

- [ ] **Step 3: Route leader keys in `src/client.rs`** (in the `if leader { ... }` match, add arms):

```rust
                            KeyCode::Char('t') => send(&mut out_stream, &ClientMsg::TileAll)?,
                            KeyCode::Char('T') => send(&mut out_stream, &ClientMsg::ToggleAutoTile)?,
                            KeyCode::Char(c @ '1'..='9') => send(&mut out_stream, &ClientMsg::SendToCell(c as u8 - b'0'))?,
```

- [ ] **Step 4: Build + test**

Run: `cargo build --offline && cargo test --offline`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/session.rs src/client.rs
git commit -m "tiling: leader t/T/1-9 → tile-all / auto-tile / send-to-cell"
```

---

### Task 5: Drag-to-cell preview + snap, auto-tile re-apply, swap-on-drop

**Files:** Modify `src/session.rs`.

- [ ] **Step 1: Replace the drag-end edge-snap with grid cell snap**

In `exec`'s `Action::EndDrag` arm, replace the `snap_zone`-based block so that, when `snapping_enabled` and the pointer is within `snap_threshold` of any work-area edge, the window snaps to the grid cell under the pointer. In auto-tile mode, if the drop cell is occupied by another window, swap instead:

```rust
            Action::EndDrag => {
                if let Some(Hit::Moving { id, .. }) = self.drag {
                    let work = Rect::new(0, 1, self.w, self.h - 2);
                    if self.cfg.snapping_enabled && near_edge(p, work, self.cfg.snap_threshold) {
                        let grid = self.grid();
                        let (row, col) = grid.cell_at(work, p);
                        if self.cfg.auto_tile {
                            if let Some(other) = self.window_in_cell(grid, row, col, id) {
                                self.wm.swap_cells(id, other);
                                self.sync_app_size(id); self.sync_app_size(other);
                            } else {
                                self.wm.send_to_cell(id, grid, row, col, self.cfg.tile_gap);
                                self.sync_app_size(id);
                            }
                        } else {
                            self.wm.send_to_cell(id, grid, row, col, self.cfg.tile_gap);
                            self.sync_app_size(id);
                        }
                    }
                }
                self.drag = None;
            }
```

Add free helpers / methods:

```rust
    fn window_in_cell(&self, grid: crate::geometry::Grid, row: u8, col: u8, except: WindowId) -> Option<WindowId> {
        self.wm.z_ordered().into_iter().find(|w| w.id != except && w.state == crate::window::WindowState::Tiled { row, col }).map(|w| w.id)
    }
```

```rust
/// True when `p` is within `threshold` cells of any edge of `work`.
fn near_edge(p: Point, work: Rect, threshold: i32) -> bool {
    p.x - work.x < threshold || work.right() - p.x < threshold
        || p.y - work.y < threshold || work.bottom() - p.y < threshold
}
```

- [ ] **Step 2: Auto-tile re-apply on open/close/resize**

After a successful `launch` (window added) and in `close`, when `self.cfg.auto_tile`, call `self.wm.tile_all(self.grid(), self.cfg.tile_gap)` then `sync_all_app_sizes()`. In the `Resize` handler, if `auto_tile`, re-tile after updating the work area.

- [ ] **Step 3: Drag-to-cell preview layer**

Track `self.drag_preview: Option<Rect>`. During `MouseDrag` with a moving window, if `snapping_enabled && near_edge`, set it to `grid.cell_rect(cell_at(p))`; else `None`. In `build_frame`, if `Some(rect)`, push a translucent highlight layer (a `CellBuffer` filled with a low-alpha accent bg) just below the launcher layers.

- [ ] **Step 4: Build + test**

Run: `cargo build --offline && cargo test --offline`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/session.rs
git commit -m "tiling: drag-to-cell preview/snap, auto-tile re-apply, swap-on-drop"
```

---

### Task 6: Settings — Windows section grid controls

**Files:** Modify `src/settings.rs`; Test `tests/` (settings inline tests).

- [ ] **Step 1: Extend the Windows section**

In `src/settings.rs`, change the Windows section (`section == 0`) from 2 rows to 6: snapping, threshold, grid rows, grid cols, tile gap, auto-tile. Update `item_count` for section 0 to `6`, add render rows, and add `adjust` arms:

```rust
            (0, 2) => self.cfg.grid_rows = step_u8(self.cfg.grid_rows, dir, 1, 6),
            (0, 3) => self.cfg.grid_cols = step_u8(self.cfg.grid_cols, dir, 1, 6),
            (0, 4) => self.cfg.tile_gap = (self.cfg.tile_gap + dir.signum()).clamp(0, 4),
            (0, 5) => self.cfg.auto_tile = flip(self.cfg.auto_tile, dir),
```

with a helper:

```rust
fn step_u8(v: u8, dir: i32, lo: u8, hi: u8) -> u8 {
    match dir { -1 => v.saturating_sub(1).max(lo), 1 => (v + 1).min(hi), _ => if v >= hi { lo } else { v + 1 } }
}
```

Render the four new rows with `◂ N ▸` style values (rows 3..6, indices 2..5).

- [ ] **Step 2: Build + test**

Run: `cargo build --offline && cargo test --offline`
Expected: all green (existing settings tests still pass; the Apps section is now index-shifted only if it depended on Windows count — verify the `to_apps` test helper still finds "Apps").

- [ ] **Step 3: Commit**

```bash
git add src/settings.rs
git commit -m "tiling: Settings > Windows grid rows/cols/gap/auto-tile"
```

---

### Task 7: Final verification

- [ ] **Step 1: Build, clippy, tests**

Run: `cargo build --offline && cargo clippy --offline --all-targets && cargo test --offline`
Expected: builds, zero clippy warnings, all tests pass.

- [ ] **Step 2: Manual smoke (on the mini)**

Set grid to 2×3 in Settings → Windows. Open several windows; `leader t` tiles them into the grid; `leader 5` sends the focused window to cell 5; `leader T` toggles auto-tile (new windows fill cells); drag a window to an edge → it snaps to that cell with a preview highlight.

- [ ] **Step 3: Commit any fixups**

```bash
git commit -am "tiling: smoke-test fixups"
```

---

## Notes for the implementer

- `WindowState` must be `Copy` for `swap_cells`; confirm the derive.
- Keep the existing `SnapZone`/`snapped_rect` and leader `[`/`]` half-snap working — this plan only generalises drag-end and adds new commands.
- Overflow windows (beyond `grid.cells()`) are intentionally left floating; do not hide them.
