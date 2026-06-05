# File Manager + Default Apps — Design

**Status:** Approved design (2026-06-05). Subsystems **B (Default Apps)** and
**C (File Manager)** of the desktop-OS roadmap, built together.

**Goal:** A native, mouse-and-keyboard file manager that looks and behaves like
Finder/Nautilus — icon-grid default with switchable list/column views, full file
operations (incl. drag-move and delete-to-Trash), image thumbnails via the A1
image layer — plus a configurable **Default Apps** system so double-clicking a
file opens it with the right app, "just like a real OS."

**Architecture:** A daemon-side `FileManager` window widget (like the store/
settings) drives the UI and operations. A standalone `openwith` engine classifies
files by role and resolves the app to open them, configured from a new Settings →
Default Apps panel. Filesystem operations live in a `fileops` module behind a
trait so they're unit-testable. Image thumbnails reuse the A1 placement mechanism.

**Tech stack:** Rust; `std::fs`; the existing `imagestore` (A1) for thumbnails;
existing compositor/session/protocol/client; per-OS Trash via XDG/`~/.Trash`.

---

## Module layout

- **Create `src/openwith.rs`** — Default Apps engine: `Role` classification by
  extension, the role→handler map, `resolve(path) -> OpenAction`, config load/merge.
- **Create `src/fileops.rs`** — `FsOps` trait (list/copy/move/rename/mkdir/trash)
  with a real `StdFs` impl and a test fake; recursive copy/move; OS Trash.
- **Create `src/filemanager.rs`** — the `FileManager` widget: state, three view
  renderers, mouse + keyboard handling, operation dispatch, thumbnail placements.
- **Modify `src/settings.rs`** — a **Default Apps** section editing the role map.
- **Modify `src/config.rs`** — `[default_apps]` map + FM prefs (default view,
  show-hidden, sidebar favorites).
- **Modify `src/session.rs`** — `WinContent::FileManager`, `@files` launch + open,
  FM message handlers, thumbnail placements in `build_frame`.
- **Modify `src/protocol.rs` / `src/client.rs`** — `filemanager_focused` flag and
  the FM keyboard routing.

## B — Default Apps engine (`openwith.rs`)

```rust
pub enum Role { Image, Video, Audio, Text, Code, Archive, Pdf, Directory, Executable, Other }

/// What to do when opening a path.
pub enum OpenAction {
    Navigate,                 // a directory → the FM cd's into it
    Builtin(&'static str),    // a tuiui viewer, e.g. "@image"
    RunApp { command: String, args: Vec<String> }, // launch a TUI app with the file
    OpenWithMenu,             // unknown → let the user pick
}
```

- **Classification:** an extension→`Role` table first, with **real MIME detection
  as the fallback** — magic-byte sniffing via the `infer` crate (and `mime_guess`
  for extension→MIME) so an extension-less or mislabeled file is still classified.
  (`png/jpg/gif/webp → Image`, `md/txt/log/json/toml/… → Text`, `rs/py/js/go/c/… →
  Code`, `mp3/flac/… → Audio`, `mp4/mkv/… → Video`, `zip/tar/gz/… → Archive`,
  `pdf → Pdf`.) Directories → `Directory`; executable-bit, no known type →
  `Executable`; else `Other`.
- **Handler map** (config `[default_apps]`, each role → a string):
  - `@image` (builtin viewer), `@navigate` (builtin) for directories,
  - or an app command — `text/code → editor` (default `$EDITOR` else `vi`),
    `audio → ` (a configured player, empty by default → Open-with menu), etc.
  - **OS roles** also stored: `editor`, `browser`, `terminal`, `file_manager`
    (the menubar/dock/keyboard shortcuts launch these).
- **`resolve(path)`** → classify → look up the role's handler → an `OpenAction`.
  Unset/unknown handler → `OpenWithMenu`.
- **Defaults are OS-aware** (e.g. `browser`/`terminal` differ on macOS vs Linux)
  and never point at something guaranteed-absent.

### Settings → Default Apps
A new section listing each role and its current handler; selecting a row opens a
small chooser (cycle through known launcher/catalog apps, or `@image`/`@navigate`
builtins, or type a command). Writes back to `config.toml` `[default_apps]`.

## C — File manager UI (`filemanager.rs`)

`WinContent::FileManager(FileManager)`. State:

```rust
struct FileManager {
    cwd: PathBuf,
    entries: Vec<Entry>,         // name, path, is_dir, size, modified, role, thumb: Option<ImageId>
    cursor: usize,               // focused entry
    selection: BTreeSet<usize>,
    view: ViewMode,              // Icon | List | Columns
    history: Vec<PathBuf>, hpos: usize, // back/forward
    scroll: i32,
    show_hidden: bool,
    clipboard: Option<Clipboard>,// {paths, op: Copy|Cut}
    overlay: Option<Overlay>,    // Rename | NewFolder | ConfirmDelete | ContextMenu | OpenWith | Error
    status: String,
}
```

- **Chrome:** a top **path/breadcrumb bar** (clickable crumbs) + a toolbar
  (`◂ ▸` back/forward, `▲` up, view toggle), a left **sidebar** (Home, Desktop,
  Documents, Downloads, Pictures + config favorites + recent dirs).
- **Views:** **Icon grid (default)** — wrapped tiles of glyph/thumbnail + name;
  **List** — name/size/modified rows, sortable; **Columns** — Miller columns with
  a right-hand detail/preview. Toggle via toolbar or `1`/`2`/`3`.
- **Thumbnails:** for `Image` entries the FM loads a small thumbnail through the
  A1 `ImageStore` and reports an `ImagePlacement` per **visible** image tile
  (clipped to the pane, hidden when scrolled off or the window is occluded).
  Non-images show a file-type glyph (📁 / 📄 / 🎵 / 🎬 / 📦 / 🖼…).
- **Tabs:** a tab strip below the toolbar; each tab owns its own `cwd`, history,
  view, selection, and scroll. `Ctrl/Cmd+T` new tab, `Ctrl+W` close, `Ctrl+Tab` /
  click to switch. The `FileManager` holds `tabs: Vec<Tab>` + `active: usize`; all
  the per-folder state above moves onto `Tab`.
- **Preview pane:** a right-hand pane (always on in Columns view; toggleable in the
  others with `Space` / a toolbar button) showing the focused entry:
  - image → the A1 thumbnail at a larger size,
  - text/code → the first ~40 lines,
  - pdf → first-page text via `pdftotext`/`mutool` when present, else page count +
    metadata,
  - other → name, size, type, modified, permissions.
- **Get Info (permissions/symlink):** context-menu *Get Info* (and `Cmd/Ctrl+I`)
  opens an overlay with full path, size, modified, **Unix permissions** (`rwxr-xr-x`,
  togglable → `chmod`), owner, and — for symlinks — the link target.

## Interactions

**Mouse:** click = select (clears others); Ctrl/Cmd-click = toggle; Shift-click =
range; **double-click = open** (`resolve`); **right-click = context menu**
(*Open*, *Open with…*, *Rename*, *Copy*, *Cut*, *Paste*, *New folder*, *Move to
Trash*); **drag a selection onto a folder/sidebar item = move**; clicks on
crumbs/sidebar/toolbar navigate. *Open with…* shows a chooser to open the file
with any app for this one time (overrides the default).

**Keyboard:** arrows/`hjkl` move the cursor; **Enter** open; **Backspace** = parent;
`Ctrl/Cmd+C/X/V` = copy/cut/paste; **Delete** → Trash; **F2** = rename;
`Ctrl+Shift+N` = new folder; type-ahead jumps to the next matching name; `.`
toggles hidden; `1/2/3` switch views; `Esc` closes an overlay; `Tab` moves between
sidebar and pane.

## File operations & Trash (`fileops.rs`)

```rust
pub trait FsOps {
    fn list(&self, dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>>;
    fn mkdir(&self, parent: &Path, name: &str) -> io::Result<PathBuf>;
    fn rename(&self, path: &Path, new_name: &str) -> io::Result<PathBuf>;
    fn copy(&self, src: &Path, dst_dir: &Path) -> io::Result<()>; // recursive for dirs
    fn move_to(&self, src: &Path, dst_dir: &Path) -> io::Result<()>;
    fn trash(&self, path: &Path) -> io::Result<()>; // never hard-delete
}
```

- **Trash:** macOS → move into `~/.Trash` (deduping names); Linux → XDG
  `~/.local/share/Trash/files` (with a `.trashinfo`) or shell out to `trash`/`gio
  trash` when present. **No hard delete in v1.** Deleting >N items asks for
  confirmation.
- **Copy/move** recurse for directories; name collisions get a " copy"/numbered
  suffix. Every op returns `io::Result`; failures populate `status` (and an error
  overlay for destructive ops) — **never silent**.
- A real `StdFs` impl; a fake `FsOps` drives the unit tests.

## Open-with flow

`Enter`/double-click → `openwith::resolve(path)`:
- `Navigate` → `cwd = path`, reload entries, push history.
- `Builtin("@image")` → the session opens an `ImageView` window (A1).
- `RunApp { command, args }` → `launch_in` a new PTY window running the app with
  the file path appended.
- `OpenWithMenu` → an overlay listing apps (from the launcher/catalog); the choice
  becomes a one-off `RunApp`. Right-click *Open with…* always shows this menu.

## Session / protocol wiring

- `WinContent::FileManager`; opened via a launcher `@files` action + a dock/menubar
  entry; only one FM window (re-focused if already open), like the store.
- `Flags.filemanager_focused`; `client.rs` routes keys to FM `ClientMsg` variants
  when focused (navigation, selection, open, overlay text input for rename/new-
  folder/type-ahead, view switch). Mouse arrives via the existing `MouseDown`
  path, hit-tested by the FM (sidebar / crumbs / toolbar / entries / overlay).
- `build_frame` asks the FM for its thumbnail `ImagePlacement`s and appends them
  to `Frame.images` (reusing the daemon's blob-once bookkeeping).

## Error handling & safety

- Trash, not delete; confirm bulk/destructive ops; permission/IO errors surface in
  the status line / error overlay; no panics on unreadable dirs (shown empty).
- Directory listing is synchronous but local-FS-cheap; large dirs are paginated by
  the scroll viewport (we only thumbnail **visible** image tiles).
- Thumbnails are bounded in pixel size and cached by content hash (A1), so a
  folder of photos stays SSH-friendly (each sent once).

## Testing (pure, deterministic)

- `openwith`: extension→`Role` classification; `resolve()` routing for every role;
  unknown → `OpenWithMenu`; config override wins over default.
- `fileops` via the fake `FsOps`: copy/move/rename/mkdir path computation, collision
  suffixing, trash routing — no real disk in CI.
- `filemanager`: grid/list layout + hit-testing (entry/sidebar/crumb/toolbar rects),
  selection logic (single/ctrl/shift), clipboard paste target, breadcrumb parsing,
  type-ahead match, scroll clamping, which tiles are "visible" for thumbnails.
- Real FS operations and PTY launches are integration/manual only.

## Build sequence (informs the plan)

1. `openwith` engine + role table + **MIME fallback (`infer`/`mime_guess`)** +
   `resolve` + config + tests.
2. `fileops` trait + `StdFs` + Trash + tests (fake FsOps).
3. `FileManager` model (with `Tab`) + icon-grid render + hit-testing + tests.
4. Navigation (sidebar/crumbs/history) + keyboard + mouse selection.
5. Open-with flow wired to the session (Navigate/Builtin/RunApp/Menu).
6. Operations (new folder, rename, copy/cut/paste, move, drag, Trash) + overlays.
7. List + Columns views; view toggle.
8. Image thumbnails via A1 placements.
9. **Tabs** (tab strip + per-tab state + shortcuts).
10. **Preview pane** (image/text/pdf/metadata).
11. **Get Info overlay** (permissions/`chmod` + symlink target).
12. Settings → Default Apps panel.
13. Session/protocol/client wiring + `@files` launch + dock entry.

## Folded into v1 (from the original deferred list)

Text/PDF preview pane, tabs, real MIME detection, and Get-Info permissions/symlink
info are now **in v1** (above).

## Out of scope (deferred to v2)

- **Split panes** (dual-pane side-by-side).
- **File-content search** (recursive grep across a tree).
- **Network / cloud mounts** (SFTP/SMB/cloud) — its own subsystem.
- Custom per-folder view settings; symlink *retargeting* (editing the target).
