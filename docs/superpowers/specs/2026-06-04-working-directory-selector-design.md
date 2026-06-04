# Working-Directory Selector — Design

**Status:** Approved direction (2026-06-04) — a "pretty" browsable file-tree
picker, triggered by a manifest flag.

**Goal:** Apps flagged in their manifest as needing a working directory (the AI
coding CLIs: `claude`, `aider`, `codex`, …) open a browsable **file-tree picker**
on launch; the directory the user chooses becomes that app's PTY working
directory. Apps without the flag launch exactly as today.

**Architecture:** A new daemon-side overlay widget (`dirpicker.rs`) mirroring the
launcher/store: it renders an expandable directory tree to compositor layers,
hit-tests clicks, and handles keys. The PTY host gains a `cwd`. The session
intercepts flagged launches, opens the picker, and launches on confirm. Manifest
flag lives in the store recipe and in `AppEntry`.

**Tech stack:** Existing Rust plumbing; standard-library filesystem reads behind a
small injectable lister (so the tree logic is unit-testable without disk).

---

## Manifest flag

- **Store apps:** `recipes.json` entries gain an optional `requires_cwd: bool`.
  The 16 AI-category tools get `requires_cwd: true`.
- **Config / custom apps:** `AppEntry` gains `requires_cwd: Option<bool>` and an
  optional fixed `cwd: Option<String>`. A fixed `cwd` launches there directly and
  skips the picker; `requires_cwd: true` opens the picker.
- **Resolution on launch:** store launches read the recipe flag; launcher/config
  launches read the `AppEntry` flag. No flag → launch in the daemon's cwd (today's
  behaviour, unchanged).

## The picker (`dirpicker.rs`)

A centred, bordered overlay (rounded box, launcher styling, theme colours) titled
**"Working directory"**, rendered above all chrome.

### Model
```rust
struct DirNode { name: String, path: PathBuf, expanded: bool, children: Option<Vec<DirNode>> }
pub struct DirPicker {
    root: PathBuf,            // from config default_project_dir (default ~)
    tree: Vec<DirNode>,       // roots' children
    selected: usize,          // index into the flattened visible list
    filter: String,           // type-to-filter within the current view
    show_hidden: bool,
    pending: PendingLaunch,   // name/command/args to spawn on confirm
}
```

Children are **loaded lazily** the first time a node expands (cached thereafter)
via a `DirLister` trait — the real impl reads the filesystem; tests inject a fake.
Only directories are shown (we are picking a directory). Unreadable directories
render as an empty/locked node — never a panic.

### Rendering ("pretty")
- A **breadcrumb** line of the currently-selected node's full path.
- The **tree body**: each visible row is `<indent>(▸|▾|·) 📁 name`, the selected
  row highlighted; expanded nodes show children indented one level.
- A **filter** line at the bottom while typing; a footer hint:
  `Enter open here · → expand · ← up · . hidden · Esc cancel`.
- An ASCII fallback (`>`/`v`/`-` and `[d]`) when box-drawing/emoji would overflow.

### Interaction
- **Up/Down** — move selection through the flattened visible tree (clamped).
- **Right / `l`** — expand the selected directory (lazy-load children) or, if
  already expanded, move into the first child.
- **Left / `h`** — collapse, or move to the parent.
- **Enter** — confirm: launch the pending app with the **selected directory** as
  cwd; record it in `recent_dirs` (MRU).
- **Type to filter** — incremental, case-insensitive match within the current
  node's children; Backspace edits the filter.
- **`.`** — toggle hidden directories.
- **Esc** — cancel the launch (drop `pending`).
- Mouse: click a row to select; click its ▸/▾ to expand/collapse; double-click (or
  click an already-selected row) confirms.

## PTY host

`AppInstance::spawn(command, args, w, h, cwd: Option<&Path>)` sets
`Command::current_dir(cwd)` when present. All existing call sites pass `None`.

## Session flow

- `launch_entry` / `store_activate`: if the resolved app `requires_cwd` and has no
  fixed `cwd`, build a `PendingLaunch` and open the `DirPicker` instead of
  spawning. On **confirm**, call `launch(name, command, args, cwd)`. On **cancel**,
  drop the pending launch.
- The picker is a daemon-side overlay: `MouseDown` is hit-tested against it before
  window routing; `build_frame` adds its layers; a `dirpicker_open` flag tells the
  client to route keys to it.

## Protocol / client

- `Flags` gains `dirpicker_open: bool`.
- New `ClientMsg`: `DirPickerUp`, `DirPickerDown`, `DirPickerExpand`,
  `DirPickerCollapse`, `DirPickerConfirm`, `DirPickerCancel`, `DirPickerChar(char)`,
  `DirPickerBackspace`, `DirPickerToggleHidden`.
- `client.rs`: when `dirpicker_open`, route arrows/Enter/Esc/Backspace and typed
  characters to these (same shape as the Spotlight and Settings-edit routing).

## Configuration

| Field | Default | Meaning |
|---|---|---|
| `default_project_dir: Option<String>` | `~` | Picker root (e.g. `~/Development`) |
| `recent_dirs: Vec<String>` | `[]` | MRU of chosen dirs, capped at 10 |
| `show_hidden_dirs: bool` | `false` | Show dot-directories by default |

`recent_dirs` surfaces as a small "Recent" group pinned at the top of the tree for
one-click reuse.

## Error handling & safety

- Directory listing errors (permission denied, vanished dir) render an empty node
  with a hint; never panic.
- Tilde / relative `default_project_dir` is expanded; a missing root falls back to
  `~` then `/`.
- Confirming a directory that no longer exists re-validates and shows an inline
  error instead of spawning into a bad cwd.
- Listing is synchronous but local-FS-cheap; a timeout/async pass is deferred
  (noted under YAGNI) — acceptable for local project trees.

## Testing (pure, deterministic; FS mocked via `DirLister`)

- Tree flatten: expanded-set → visible row order and indents.
- Expand/collapse and lazy child-load (fake lister), selection movement + clamping.
- Filter matching (case-insensitive, current-node scope) and hidden toggle.
- Path resolution / tilde expansion; `recent_dirs` MRU update + cap.
- `requires_cwd` resolution: store recipe flag and `AppEntry` flag/fixed-cwd paths
  each pick the right launch route.
- Spawn-with-cwd is integration-only (not run in CI).

## Build sequence

1. `requires_cwd` in `recipes.json` (flag the 16 AI tools) + `AppEntry` fields.
2. `AppInstance::spawn` cwd parameter (call sites pass `None`).
3. `dirpicker.rs` tree model + `DirLister` trait + rendering (+ tests).
4. Picker input handling (keys + mouse) producing confirm/cancel (+ tests).
5. Session `PendingLaunch` flow; `Flags.dirpicker_open`; `ClientMsg` + client
   routing.
6. Config fields + `recent_dirs` MRU + the "Recent" group.

## Out of scope (YAGNI)

- Selecting files (directories only).
- Creating new directories from the picker.
- Async/timeout directory listing (local FS assumed for v1).
- Per-app remembered last-dir (global MRU only for now).
