# Desktop Icons (D) — Design

**Status:** Approved design (2026-06-05). Subsystem **D** of the desktop-OS roadmap
(`docs/superpowers/specs/2026-06-05-desktop-os-roadmap.md`). Builds on B (Default
Apps / `openwith`), C (File Manager / `fileops`), and A1 (image layer).

**Goal:** Clickable icons on the desktop wallpaper — merged from the live
`~/Desktop` folder **and** user-pinned shortcuts — that you select, drag onto an
invisible grid (snapping, positions persisted), double-click to open via the
Default Apps engine, and right-click for a context menu (open / rename / move to
Trash / new folder). Image icons show A1 thumbnails.

**Architecture:** Desktop icons are **not a window**. A new `desktop` module owns a
`DesktopIcons` model that renders to a single compositor `Layer` at **z = 0**
(windows get `z ≥ 1`, so icons sit above the wallpaper and beneath every window).
`SessionCore` owns one `DesktopIcons`, renders its layer in `build_frame`, and
routes mouse events to it **only when a click falls through the chrome and misses
every window**. Opening an icon reuses the file manager's effect dispatch
(`openwith::resolve` → navigate / `@image` / `RunApp`); thumbnails reuse the
session's `ImageStore`; deletes reuse `fileops::trash`.

**Tech stack:** Rust; `std::fs` via the existing `fileops`; `openwith`;
`imagestore` (A1); existing compositor/session/protocol/client. No new crates.

---

## Module layout

- **Create `src/desktop.rs`** — the `DesktopIcons` model and its `DesktopIcon`
  items: source merge, grid layout + snap, selection, drag, context/rename
  overlays, `render`, and hit-testing. All logic is pure-ish and unit-testable
  against a temp `~/Desktop` + an injected positions map.
- **Modify `src/config.rs`** — `[desktop]` fields: `desktop_enabled: bool` (default
  `true`), `desktop_pins: Vec<AppEntry>` (pinned shortcuts; reuses `AppEntry`), and
  `desktop_positions: BTreeMap<String, (u16, u16)>` (key → grid `(col, row)`).
- **Modify `src/session.rs`** — own `desktop: DesktopIcons`; render its z=0 layer in
  `build_frame`; route fall-through clicks in `handle_mouse`; double-click → effect
  dispatch; right-click → context overlay; drag → snap + persist; thumbnails via a
  `refresh_desktop_thumbnails` mirror of `refresh_fm_thumbnails`; reload `~/Desktop`
  when the desktop is interacted with (and on startup).
- **Modify `src/protocol.rs`** — add `Flags.desktop_editing: bool` (rename/new-folder
  overlay wants typed chars).
- **Modify `src/session.rs` (ClientMsg) + `src/client.rs`** — add
  `ClientMsg::MouseDouble(Point)` and `ClientMsg::MouseRightDown(Point)`; the client
  detects a double-click (two left-downs within ~400 ms at the same cell) and sends
  right-button downs.

## The model (`desktop.rs`)

```rust
pub enum IconSource { Folder, Pinned }

pub struct DesktopIcon {
    pub path: PathBuf,        // the ~/Desktop entry, or the pin's target (cwd/path)
    pub label: String,
    pub role: crate::openwith::Role,
    pub source: IconSource,
    pub command: Option<String>, // pins only: the AppEntry command (e.g. "@files")
    pub cell: (u16, u16),     // grid column, row
    pub thumb: Option<u64>,   // A1 ImageId for image entries
}

pub enum DesktopOverlay {
    Context { idx: usize, anchor: Point },   // right-click on an icon
    DesktopMenu { anchor: Point },           // right-click on empty desktop
    Rename { idx: usize, name: String },
    NewFolder { name: String },
}

pub enum DesktopAction {                     // what the session must effect
    Open(PathBuf),                           // resolve + open (folder/file/image)
    Run { command: String, args: Vec<String> },
    Unpin(String),                           // remove a pin (command key)
}

pub struct DesktopIcons {
    icons: Vec<DesktopIcon>,
    selection: BTreeSet<usize>,
    drag: Option<(usize, Point)>,            // dragging icon idx, grab offset
    overlay: Option<DesktopOverlay>,
    desktop_dir: PathBuf,
    action: Option<DesktopAction>,
    grid_cols: u16,
    grid_rows: u16,
}
```

### Sources & merge
On `reload(fs, pins, positions)`:
1. List `~/Desktop` via `fileops::list(desktop_dir, show_hidden=false)` → one
   `DesktopIcon { source: Folder }` per entry (label = file name, role from the
   entry).
2. Append one `DesktopIcon { source: Pinned }` per `desktop_pins` `AppEntry` (label =
   `entry.name`, `command = Some(entry.command)`, `path` = the pin's `cwd`/first arg
   if any, role = `classify` of that path or `Other`).
3. Assign each icon a `cell`: use `positions[key]` if present (key = abs path for
   folder items, `command` for pins); else the **first free cell** in column-major
   order (so new items flow into open slots without overlapping).

`reload` is called on startup, when the desktop layer is clicked, and after any
op that changes `~/Desktop` (rename/trash/new-folder). It preserves selection by
key where possible.

### Grid, snap, persistence
- The grid covers the **work area** (below the menubar `y=1`, above the dock row),
  cell size `ICON_W=14 × ICON_H=3` (matching the FM tiles). `grid_cols =
  work_w / ICON_W`, `grid_rows = work_h / ICON_H`, recomputed on resize.
- `free_cell()` scans column-major for the first cell not occupied by any icon.
- Dragging: `MouseDown` on an icon starts a drag (records grab offset);
  `MouseDrag` moves a ghost; `MouseUp` snaps to the **nearest free cell** under the
  cursor (or stays put if none). The new `(col,row)` is written to
  `desktop_positions[key]` and `config.save()` is called by the session.
- *Clean Up* (desktop context menu) re-flows every icon to column-major order and
  rewrites all positions.

## Mouse routing (`session.rs::handle_mouse`)

Desktop hit-testing is inserted **after** the chrome checks (menubar/dock/tray/
launcher/dirpicker) and **after** window routing reports no window was hit — i.e.
the click landed on the empty desktop. Concretely: try `route_mouse` first; if it
yields a window action, run it as today; only if it does **not** hit a window do we
offer the click to `desktop.handle_click/handle_right/handle_double`.

- **Left down** on an icon → select it (clear others unless Ctrl), begin a possible
  drag. On empty desktop → clear selection (and dismiss any desktop overlay).
- **Drag / up** → move + snap as above (only when a desktop drag is active).
- **Double-click** (`ClientMsg::MouseDouble`) on an icon → set
  `DesktopAction::Open`/`Run`; the session drains it (same dispatch as the FM:
  folder → open **Files** at that dir, `@image` → image viewer, `RunApp`/pin
  command → launch). A double-click that hits a window is ignored by the desktop.
- **Right down** (`ClientMsg::MouseRightDown`) on an icon → `Context` overlay at the
  cursor; on empty desktop → `DesktopMenu` overlay.

Because the desktop only ever sees fall-through clicks, **existing window/chrome
mouse behavior is unchanged**.

## Context menus & overlays

- **Folder-item context:** *Open*, *Open with…*, *Rename*, *Move to Trash*.
- **Pinned-item context:** *Open*, *Unpin*.
- **Empty-desktop context:** *New Folder*, *Clean Up*.
- *Rename* / *New Folder* open a small text overlay; while open the session reports
  `Flags.desktop_editing = true` so the client forwards typed characters
  (`DesktopChar` / `DesktopBackspace` / `DesktopCommit` / `DesktopCancel` messages,
  mirroring the FM overlay text path). Commit calls `fileops::rename` / `mkdir` on
  `~/Desktop`, then `reload`. *Move to Trash* calls `fileops::trash` then `reload`.
- Overlays render as a small box at the anchor (reuse the FM's `draw_box` style;
  duplicate a tiny local helper rather than coupling the modules).

## Rendering (`build_frame`)

- Build one `CellBuffer` the size of the screen; for each icon draw its tile at
  `origin = (col*ICON_W, 1 + row*ICON_H)` (the `+1` clears the menubar): a glyph or
  (fallback under) thumbnail on row 0 and the truncated label on row 1, selection
  highlighted. Push it as `Layer { z: 0, origin: (0,0), buf, opacity: 1.0, scissor:
  None }` **before** the window loop so windows overlay it.
- Image icons: the session loads a thumbnail into its `ImageStore`
  (`refresh_desktop_thumbnails`, hash-cached like the FM) and `build_frame` appends
  an `ImagePlacement` per **visible** icon tile (one not covered by any window — a
  desktop icon is visible iff no non-minimized window’s rect covers its tile;
  reuse/adapt `fully_unobstructed` against the icon rect). The glyph is the
  non-Kitty fallback.
- The overlay (context/rename) renders on top of the icon layer but still beneath
  windows? **No** — a right-click context menu should sit above windows so it’s
  usable; render the desktop **overlay** as a high-z layer (e.g. z just below the
  launcher) while the **icons** stay at z=0. (Icons live under windows; their menu
  pops above.)

## Config & persistence (`config.rs`)

```toml
[desktop]
desktop_enabled = true

[[desktop.desktop_pins]]            # reuses AppEntry
name = "Files"
command = "@files"

[desktop.desktop_positions]
"/Users/jay/Desktop/notes.md" = [0, 0]
"@files" = [0, 1]
```

All three fields use `#[serde(default)]`; defaults: enabled `true`, no pins, empty
positions. The session seeds a couple of sensible default pins (**Files**, **Store**)
on first run only if `desktop_pins` is empty *and* the key is absent (don’t fight a
user who removed them — gate on a `desktop_seeded` bool or simply seed only when the
config file had no `[desktop]` table). Simpler: seed pins in `Config::default()` so a
fresh config gets them; an existing config without them stays as the user left it.

## Effect dispatch & thumbnails (`session.rs`)

- `drain_desktop_action()` mirrors `drain_fm_action`: `Open(path)` →
  `openwith::resolve(path, is_dir, &cfg.default_apps)` → `Navigate` opens **Files** at
  that dir (or its parent for a file-less folder), `Builtin("@image")` →
  `open_image`, `RunApp` → `launch_in`. `Run { command, args }` (a pin) → the
  existing `launch_entry`-style dispatch (so `@files`/`@store`/`@settings` pins work).
  `Unpin(cmd)` → remove from `cfg.desktop_pins`, `save`, `reload`.
- `refresh_desktop_thumbnails()` loads image-role icons’ thumbnails into
  `self.images` and hands the ids back to `desktop.set_thumb(idx, id)`; called after
  desktop reloads. `build_frame` emits the placements.

## Testing

- `desktop.rs` (pure, deterministic, temp `~/Desktop` + injected positions/pins):
  source merge & labeling; `free_cell` flow; snap-to-nearest-free-cell; `hit_test`
  (icon vs empty); selection (single/ctrl); context-target selection (folder vs pin
  menus); rename/new-folder overlay text + commit (via fake/`StdFs` on a temp dir);
  positions round-trip. `DesktopAction` is produced (not executed) by the model so
  open/run/unpin are assertable without the session.
- `session` integration: a fall-through left-click selects a desktop icon; a
  double-click on a folder icon opens a **Files** window; an image icon emits a
  thumbnail `ImagePlacement`; a right-click opens the context overlay; a click on a
  window does **not** reach the desktop.
- `client`: double-click detection emits `MouseDouble`; right-button emits
  `MouseRightDown`.

## Build sequence (informs the plan)

1. `config.rs` `[desktop]` fields + default pins.
2. `desktop.rs` model: types, `reload`/merge, grid + `free_cell` + `snap`, hit-test,
   selection + tests.
3. `desktop.rs` render (icon tiles + selection) + tests.
4. Drag-to-snap + position persistence (model side) + tests.
5. Protocol: `MouseDouble`, `MouseRightDown`, `Flags.desktop_editing` + the
   `Desktop*` ClientMsg variants; client double-click + right-click detection.
6. Session wiring: own `DesktopIcons`, render the z=0 layer, fall-through mouse
   routing, `drain_desktop_action`, config save on drag + tests.
7. Context menus + rename/new-folder/trash overlays (model + session + client text
   routing) + tests.
8. Thumbnails: `refresh_desktop_thumbnails` + `build_frame` placements + tests.
9. Manual verification + README.

## Out of scope (v2)

- Rubber-band marquee multi-select; multi-monitor; animated drag ghosts.
- Custom wallpaper images; per-icon custom labels/emblems.
- Auto-watching `~/Desktop` with inotify/FSEvents (we reload on interaction, not in
  real time).
- Symlink retargeting; copy/paste *onto* the desktop (use the file manager).
