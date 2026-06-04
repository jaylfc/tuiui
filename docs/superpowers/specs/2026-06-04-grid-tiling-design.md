# Configurable Grid Tiling — Design

**Status:** Approved direction (2026-06-04) — "all of the above" interaction set.

**Goal:** Generalise tuiui's window tiling from left/right half-snap to a
**user-configurable R×C grid**, with four interaction styles sharing one grid
core: drag-to-cell snapping, an auto-tile mode, keyboard send-to-cell, and a
one-shot tile-all command. Motivating case: an ultra-wide monitor laid out as
2 rows × 3 columns.

**Architecture:** A pure grid model in `geometry.rs` (cell → rect math) drives all
four interactions. The window manager gains tiling operations; the session wires
input (drag previews, keys) to them; settings expose the grid. No protocol change
for display; a few new `ClientMsg` variants for the keyboard commands.

**Tech stack:** Existing Rust WM/compositor/session; no new dependencies.

---

## Current state

`geometry.rs` has `SnapZone { Left, Right }`, `snapped_rect(zone, work)`, and
`snap_zone(p, work, threshold)`. `WindowManager` snaps on drag-end; `Config` has
`snapping_enabled` and `snap_threshold`. `WindowState` is
`Floating | SnappedLeft | SnappedRight | Maximized`.

## Configuration (defaults chosen — please confirm on review)

Added to `Config` (Windows section of Settings):

| Field | Default | Meaning |
|---|---|---|
| `grid_rows: u8` | `2` | Preferred grid rows (1–6) |
| `grid_cols: u8` | `2` | Preferred grid columns (1–6) |
| `tile_gap: i32` | `0` | Cells of gutter between tiled windows |
| `auto_tile: bool` | `false` | Auto-arrange all windows into the grid |

`snapping_enabled` / `snap_threshold` are kept; `snapping_enabled` now also gates
drag-to-cell snapping. (Default grid is 2×2 = quadrants, a gentle upgrade from
halves; the user sets 2×3 for the ultra-wide.)

## Grid core (`geometry.rs`)

```rust
pub struct Grid { pub rows: u8, pub cols: u8 }

impl Grid {
    pub fn cells(&self) -> u8;                 // rows * cols
    /// Rect for cell (row, col) within `work`, accounting for `gap`. Cells are
    /// 1×1 in v1; the signature carries spans for forward-compatibility.
    pub fn cell_rect(&self, work: Rect, row: u8, col: u8, gap: i32) -> Rect;
    /// The (row, col) cell whose region contains `p` (clamped to the grid).
    pub fn cell_at(&self, work: Rect, p: Point) -> (u8, u8);
    /// Row-major cell index <-> (row, col).
    pub fn index_of(&self, row: u8, col: u8) -> u8;
    pub fn row_col(&self, index: u8) -> (u8, u8);
}
```

Even division distributes any remainder pixels to the leftmost/topmost cells so
the grid always covers the full work area with no gaps (besides `tile_gap`).

`WindowState` gains `Tiled { row: u8, col: u8 }` (replacing the two `Snapped*`
variants; left/right half become `Tiled` cells in a 1×2 interpretation — see
Keybindings). `restore_rect` still holds the pre-tile floating rect.

## Interactions

### 1. Drag-to-cell snapping
While dragging with `snapping_enabled`, if the pointer is within `snap_threshold`
of any work-area edge, compute the target cell via `cell_at` and render a
**translucent preview** of that cell (a low-opacity highlight layer). On drop,
snap the window to that cell (`Tiled{row,col}`). Interior drops (away from edges)
keep the window floating, so free placement still works. This generalises
Windows-style edge/corner snapping: corners → corner cells, edges → edge cells.

### 2. Auto-tile mode
When `auto_tile` is on (config toggle or `leader → g`), all non-minimized windows
are arranged into the grid in z-order (oldest→newest), row-major. Triggered on
open/close/resize. **Overflow:** windows beyond `rows*cols` stay floating above
the grid (with the dock as the switcher); documented, not silently hidden.
**Swap on drop:** dragging a tiled window onto another tiled window's cell swaps
the two windows' cells.

### 3. Keyboard send-to-cell
`leader → 1..9` sends the focused window to cell N (row-major, 1-based). No-op if
N exceeds `rows*cols`. Sets `Tiled` state and resizes its PTY.

### 4. Tile-all command
`leader → t` arranges the current windows into the grid once (same layout as
auto-tile) without turning the mode on.

Existing `leader → [ / ]` (snap left/right half) are preserved and reinterpreted
as "tile to the left / right column spanning all rows" — i.e. send to column 0 /
last column. With a 1×2 grid this is identical to today's behaviour.

## Window manager additions

```rust
fn tile_all(&mut self, grid: Grid, gap: i32);             // arrange all visible windows
fn send_to_cell(&mut self, id, grid: Grid, row, col, gap);// place one window
fn swap_cells(&mut self, a: WindowId, b: WindowId);       // auto-tile drop swap
```

Each placement sets `Tiled{row,col}` + the cell rect; the session then calls
`sync_app_size` so the hosted PTY matches.

## Session / input wiring

- New `ClientMsg`: `TileAll`, `ToggleAutoTile`, `SendToCell(u8)`.
- `client.rs` leader map gains `t` → `TileAll`, `g` → `ToggleAutoTile`,
  `1..9` → `SendToCell(n)`.
- Drag handling: during `MouseDrag`, when the edge-trigger condition holds,
  the session records the candidate cell and `build_frame` adds the preview
  layer; `EndDrag` snaps to that cell instead of the old left/right zones.
- `auto_tile` re-applies in `apply` after any window open/close and on `Resize`.
- Settings "Windows" section gains rows: grid rows, grid cols, gap, auto-tile.

## Error handling & edge cases

- `rows`/`cols` clamped to 1–6; `grid.cells()` never zero.
- A cell smaller than `MIN_W`×`MIN_H` (tiny terminal / huge grid) still places the
  window at the minimum size, clamped on-screen (never panics).
- Overflow windows float (documented above).
- Toggling `auto_tile` off restores nothing automatically — windows keep their
  current tiled rects and become freely movable again (predictable).

## Testing (pure, deterministic)

- `cell_rect`: exact rects for a 2×3 grid over a known work area, with gap 0 and
  gap 1; remainder distribution; tiny work areas clamp to `MIN_*`.
- `cell_at`: pointer→cell mapping across all 6 regions and out-of-bounds clamps.
- `index_of` / `row_col` round-trip for every cell.
- `tile_all`: N windows → expected cells in z-order; overflow stays floating.
- `send_to_cell`: geometry + out-of-range no-op.
- `swap_cells`: two windows exchange rects/states.
- Left/right half preserved: `leader [` on a 1×2 grid equals the old snap rect.

## Build sequence

1. `Grid` + cell math in `geometry.rs` (+ tests).
2. `WindowState::Tiled` + WM `send_to_cell` / `tile_all` / `swap_cells` (+ tests).
3. Config fields + Settings "Windows" rows.
4. Keyboard commands (`ClientMsg` + client leader map + session handlers).
5. Drag-to-cell preview + snap in the session.
6. Auto-tile mode (re-apply hooks + swap-on-drop).

## Out of scope (YAGNI)

- Cell spanning / merged cells (signature is span-ready; behaviour is 1×1 in v1).
- Saved named layouts / multiple workspaces.
- Per-monitor grids (single work area for now).
