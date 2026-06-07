# tuiui

**A desktop environment for the terminal.** tuiui is a windowing shell that runs *inside* a terminal: floating, overlapping windows — each hosting a real terminal application — with a mouse cursor, a top menubar + status tray, a bottom dock, configurable grid tiling, an app launcher, and an app store backed by the [awesome-tuis](https://github.com/rothgar/awesome-tuis) catalog.

It's a multiplexer at heart (apps run as real child processes in PTYs and are composited into windows), built from scratch in Rust.

> **Status: active development.** The shell, window management, persistent daemon, app launcher, store, settings, theming, a macOS-style status tray, and configurable grid tiling all work today. GUI/Wayland streaming is on the roadmap below.

## What works today

- **Floating, overlapping windows** with drop shadows, each running a real TUI (btop, a shell, vim, …) in its own pseudo-terminal.
- **Faithful rendering** via a full terminal emulator ([`alacritty_terminal`](https://docs.rs/alacritty_terminal)) — even demanding apps like btop render correctly.
- **Mouse-driven**: drag titlebars to move, drag edges to resize, click the dock to focus, click titlebar buttons to minimize/maximize/close.
- **Configurable grid tiling** — set a rows×columns grid (e.g. 2×3 for an ultra-wide) and use drag-to-cell snapping (with a live preview), a one-key *tile-all*, an *auto-tile* mode, or send a window straight to a numbered cell.
- **Menubar status tray** — clock, CPU/memory, volume, WiFi, Bluetooth, and battery, with click-through popovers that control the **host's** volume, switch to a known WiFi network, and connect a paired Bluetooth device.
- **Native image viewing** — real raster images inside windows via the Kitty graphics protocol (Ghostty/Kitty/WezTerm), with a cell placeholder fallback elsewhere. Open one with a launcher entry `command = "@image"`, `args = ["~/pic.png"]`.
- **File manager** — a native, mouse-and-keyboard file browser (launcher entry **Files**, or `@files`): **icon-grid, list, and Miller-columns** views, **image thumbnails** (via the Kitty graphics layer), a **preview pane** (text head / PDF text / metadata), **tabs**, **Get Info** (size, kind, Unix permissions, symlink target), folder navigation with history, single/ctrl/shift selection, new folder, rename, copy/cut/paste, and **delete-to-Trash** (never a hard delete). Double-click/Enter opens each file with its default app.
- **Default Apps** — a configurable file-type → app map (**Settings → Default Apps**): images open in the built-in viewer, text/code in your `$EDITOR`, and you can cycle the handler for each role. The file manager uses it to open files "just like a real OS."
- **Desktop icons** — clickable icons on the wallpaper, merged from your live `~/Desktop` folder and pinned shortcuts. Double-click to open (via Default Apps), drag to rearrange (snaps to a grid, positions persisted), and right-click for a context menu (open / rename / move to Trash / new folder). Image files show thumbnails on Kitty-graphics terminals.
- **App launcher** — a Windows-95-style **cascading menu** (categories fly out submenus of apps on hover/arrow) *and* a Spotlight search overlay. Navigate the cascade by mouse (hover to open, click to launch) or keyboard (`↑/↓`, `→` into a submenu, `←` back, `Enter` to launch).
- **App store** — browse/search/install from a curated, **100%-verified** catalog of ~590 TUIs (incl. a dedicated **AI** category: Claude Code, Gemini CLI, Aider, opencode, Codex, Crush, Goose, Plandex, and more), OS-aware so Linux-only tools never show on macOS and vice-versa.
- **Custom apps** — add your own launcher entries (name + command) from **Settings → Apps**.
- **Working-directory picker** — launching a coding agent (Claude Code, Aider, …) opens a browsable file-tree so it starts in the project you choose; remembers recent directories.
- **Theming** — four built-in palettes (midnight, nord, gruvbox, dracula), switchable live from **Settings → Appearance**.
- **Persistent daemon + thin client** (tmux-style): windows and processes survive detach and SSH disconnects.
- **In-app updater** — check for and install updates from **Settings → Updates**.

## Controls

tuiui uses a **leader key** (`Ctrl+Space`) so its shortcuts never collide with macOS, your terminal, or the focused app. Press the leader, release, then a key:

| Shortcut | Action |
|---|---|
| `Ctrl+Space` then `Space` | Spotlight launcher (type to filter, ↑/↓, Enter) |
| `Ctrl+Space` then `a` | App menu (dropdown) |
| `Ctrl+Space` then `m` / `n` | Maximize / minimize focused window |
| `Ctrl+Space` then `[` / `]` | Snap focused window left / right half |
| `Ctrl+Space` then `t` | Tile all windows into the grid |
| `Ctrl+Space` then `T` | Toggle auto-tile mode |
| `Ctrl+Space` then `1`–`9` | Send focused window to grid cell N |
| `Ctrl+Space` then `s` / `,` | Open the Store / Settings |
| `Ctrl+Space` then `?` | **Help** — show this shortcut cheatsheet in-app (any key dismisses) |
| `Ctrl+Space` then `q` / `Q` | Detach (keep running) / shut down the daemon |

Forget a shortcut? Press **`Ctrl+Space` then `?`** for the in-app cheatsheet.

Set the grid (rows × columns), gap, and auto-tile from **Settings → Windows**.

In the **working-directory picker** (opens when launching a coding agent): `↑`/`↓` to move, `→`/`←` to expand/collapse, `n` to make a new folder, `.` to toggle hidden dirs, `Enter` to open there, `Esc` to cancel.

In the **file manager** (launcher → **Files**): `↑`/`↓`/`←`/`→` move the cursor, `Enter` opens (folder → navigate, file → default app), `Backspace` goes up, `Ctrl+C`/`Ctrl+X`/`Ctrl+V` copy/cut/paste, `Delete` moves to Trash (with confirm), `F2` renames, `Ctrl+N` makes a new folder, `1`/`2`/`3` switch icon/list/columns views, `Space` toggles the preview pane, `.` toggles hidden files, `Esc` closes. **Tabs:** `Ctrl+T` new, `Ctrl+W` close, `Tab` switch. **Get Info** (size, kind, Unix permissions, symlink target) is on the context menu. Click an entry to select, the toolbar `◂ ▸ ▲` to navigate, and the scroll wheel to move through long folders. Image folders show **thumbnails** in icon view on terminals with the Kitty graphics protocol.

On the **desktop** (the empty wallpaper): click an icon to select, **double-click to open** (folders → the file manager, files → their default app, pins → the app), **drag** an icon to rearrange it (snaps to a grid, position saved), and **right-click** an icon (open / rename / move to Trash) or the empty desktop (new folder / clean up). Icons come from your `~/Desktop` folder plus pinned shortcuts.

Mouse: click **✦ tuiui** (top-left) for the app menu, the **✕ Quit** button (top-right) to exit, titlebar buttons (`– ▢ ✕`), drag titlebars/edges to move/resize, drag a window to a screen edge to snap it into a grid cell, click a tray indicator (clock/volume/WiFi/…) for its popover, and click dock pills to focus.

## Build & run

Requires a [Rust toolchain](https://rustup.rs).

### Install

**Prebuilt binary** (macOS arm64/x86_64, Linux x86_64 — no Rust needed):

```bash
curl -fsSL https://raw.githubusercontent.com/jaylfc/tuiui/main/install.sh | sh
```

**Or build from source** with a [Rust toolchain](https://rustup.rs):

```bash
cargo install --git https://github.com/jaylfc/tuiui
```

Either way the `tuiui` binary lands on your `PATH`, so you can just run:

```bash
tuiui            # start the daemon (if needed) and attach
```

Update later from inside the app (**Settings → Updates → Check / Update**), or manually:

```bash
cargo install --git https://github.com/jaylfc/tuiui --force
tuiui kill && tuiui     # restart the daemon onto the new build
```

### Run from a clone (for development)

```bash
cargo run --release        # starts the daemon (if needed) and attaches a client
```

tuiui runs as a **persistent daemon + thin client** (like tmux): the daemon owns
your windows and processes and keeps them alive, while the client renders to your
terminal. Detaching — or an SSH disconnect — leaves everything running; reattach
and it's all still there.

```bash
tuiui            # ensure the daemon is running, then attach
tuiui attach     # attach to an already-running daemon
tuiui kill       # shut the daemon down (closes all windows)
```

Detach with **`Ctrl+Space` then `q`** (or `Ctrl+Alt+Q`, or the ✕ Quit button);
fully shut down from inside with **`Ctrl+Space` then `Q`**. The socket lives in a
per-user `0700` directory (`$XDG_RUNTIME_DIR` or the temp dir).

Configuration lives at `~/.config/tuiui/config.toml` (see [example below](#configuration)). On first run with no config, tuiui opens your `$SHELL` and auto-detects installed TUIs.

### Recommended terminal

tuiui wants a **truecolor, mouse-capable terminal**: **Ghostty**, Kitty, WezTerm, or iTerm2. Avoid macOS Terminal.app (weak truecolor + flaky mouse).

### Current development setup

This project is currently developed and tested on **macOS** using **[Ghostty](https://ghostty.org)**, frequently driving a tuiui instance **running on a remote machine over SSH** (the Mac is the thin client; tuiui and the apps run on the host). Two things matter in that setup:

- **Truecolor over SSH:** SSH doesn't forward `COLORTERM`, so export it on the host before launching for full 24-bit color (otherwise tuiui falls back to a 256-color approximation):
  ```bash
  export COLORTERM=truecolor
  cargo run --release
  ```
- **Terminfo over SSH:** if `clear`/apps complain about an unknown `xterm-ghostty` terminal, install Ghostty's terminfo on the host once:
  ```bash
  infocmp -x xterm-ghostty | ssh user@host -- tic -x -
  ```

A native daemon + thin-client attach (proper persistent remote sessions, instead of plain `cargo run` over SSH) is on the roadmap.

## Configuration

```toml
snapping_enabled = true   # drag-to-cell snapping
snap_threshold = 3        # edge band (cells) that engages snapping
window_shadows = true
theme = "midnight"        # midnight | nord | gruvbox | dracula

# Tiling grid (also editable in Settings → Windows)
grid_rows = 2
grid_cols = 3
tile_gap = 0
auto_tile = false

# Working-directory picker (for coding agents flagged requires_cwd)
default_project_dir = "~/Development"   # picker opens here (default: ~)

# Auto-started at launch (and shown in the dock)
[[apps]]
name = "btop"
command = "btop"
[[apps]]
name = "shell"
command = "zsh"

# Extra apps offered in the launcher (installed TUIs are auto-added).
# Also addable from Settings → Apps.
[[launcher]]
name = "lazygit"
command = "lazygit"
category = "Git"
```

Most of these are editable live from the in-app **Settings** panel, which writes this file back.

## Architecture

A pure-logic core (geometry, cell compositor, window manager, input routing) wrapped by I/O adapters (a `crossterm` terminal backend, a `portable-pty` + `alacritty_terminal` process host). A `SessionCore` owns the windows and apps and talks to the front-end through a `ClientMsg`/`Frame` boundary designed to later cross a socket for remote attach.

Design docs and the slice-by-slice plan live in [`docs/superpowers/`](docs/superpowers/).

## Roadmap

- **✅ Slice 1 — Shell:** compositor, window manager, PTY host, chrome, launcher.
- **✅ Slice 2 — Daemon:** persistent daemon + thin client; detach/reattach keeps windows and processes alive.
- **✅ Slice 3 — Store:** browse/search/install a 100%-verified, OS-aware catalog (incl. an AI tools category).
- **✅ Slice 4 — Settings:** sidebar settings panel writing `config.toml` (Windows, Appearance, Updates, Apps).
- **✅ Slice 5 — Theming:** four live-switchable palettes from Settings → Appearance.
- **✅ Menubar tray:** clock/CPU/mem/volume/WiFi/Bluetooth/battery with host-control popovers (macOS + Linux backends).
- **✅ Grid tiling:** configurable R×C grid — drag-to-cell, auto-tile, send-to-cell, tile-all.
- **✅ Working-directory picker:** a browsable file-tree on launch for apps flagged `requires_cwd` (the AI CLIs); remembers recent dirs.
- **✅ Native image layer:** Kitty-graphics image rendering inside windows (image viewer; thumbnail engine for the file manager).
- **✅ Default Apps:** configurable file-type → app map (Settings → Default Apps).
- **✅ File manager:** native mouse+keyboard browser — icon/list/**columns** views, navigation, copy/cut/paste, rename, new folder, delete-to-Trash, open-with-default, **image thumbnails**, **tabs**, **preview pane**, and **Get Info** (permissions/symlink).
- **✅ Desktop icons:** clickable wallpaper icons from `~/Desktop` + pins; double-click to open, drag-to-grid (persisted), right-click menu (rename/Trash/new folder), thumbnails.
- **✅ Cascading launcher:** Windows-95-style flyout menu — categories cascade into app submenus on hover/arrow; mouse + keyboard navigable.
- **Slice 6 — GUI/Wayland mode** (host real GUI apps; audio/video streaming to the client) — plus parked ideas: a fullscreen **browser PWA** of tuiui and a **Win95-style cascading launcher**.
- **Slice 7 — Standalone "TUI-OS" app** (bundle a GPU terminal + tuiui into a fullscreen app).

## Credits

The app catalog is generated from [rothgar/awesome-tuis](https://github.com/rothgar/awesome-tuis) via `scripts/gen_catalog.py`. tuiui stands on the shoulders of `alacritty_terminal`, `crossterm`, and `portable-pty`.

## License

**[PolyForm Noncommercial 1.0.0](LICENSE)** — source-available. You may use, modify,
and share tuiui for any **noncommercial** purpose (personal, hobby, research,
education, nonprofits). **Commercial use is reserved to the copyright holder**
(jaylfc); contact for a commercial license.

tuiui's dependencies are permissive (MIT / Apache-2.0 / BSD / Zlib / Unlicense, plus
one MPL-2.0 crate used unmodified); their notices are collected in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
