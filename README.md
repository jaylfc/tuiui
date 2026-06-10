# tuiui

**A desktop environment for the terminal.** tuiui is a windowing shell that runs *inside* a terminal: floating, overlapping windows — each hosting a real terminal application — with a mouse cursor, a top menubar + status tray, a bottom dock, configurable grid tiling, an app launcher, and an app store backed by the [awesome-tuis](https://github.com/rothgar/awesome-tuis) catalog.

It's a multiplexer at heart — like tmux, but with windows and a mouse: apps run as real child processes in PTYs, composited into windows, and **kept alive by a background daemon**. Detach and reattach from any terminal — or over SSH from another machine — and your whole desktop is exactly where you left it. Built from scratch in Rust.

> **Status: active development.** The shell, window management, a persistent daemon that runs apps in a **separate process so they survive a UI reload/update**, mouse passthrough into apps, an app launcher + store, a file manager, desktop icons, settings, theming, a macOS-style status tray, and configurable grid tiling all work today. GUI/Wayland streaming is on the roadmap below.

## What works today

- **Floating, overlapping windows** with drop shadows, each running a real TUI (btop, a shell, vim, …) in its own pseudo-terminal.
- **Faithful rendering** via a full terminal emulator ([`alacritty_terminal`](https://docs.rs/alacritty_terminal)) — even demanding apps like btop render correctly.
- **Mouse-driven**: drag titlebars to move, drag edges to resize, click the dock to focus, click titlebar buttons to minimize/maximize/close, and **scroll the wheel** over any app window to page back through its scrollback (typing snaps back to the live bottom). The mouse also **passes through into apps** that request it (btop, yazi, lazygit, vim with `mouse=a`): clicks, drag, scroll, and modifiers, in both windowed and full-screen views.
- **Configurable grid tiling** — set a rows×columns grid (e.g. 2×3 for an ultra-wide) and use drag-to-cell snapping (with a live preview), a one-key *tile-all*, an *auto-tile* mode, or send a window straight to a numbered cell.
- **Menubar status tray** — clock + date, CPU/memory, volume, WiFi, Bluetooth, and battery (Linux sysfs / macOS `pmset`), with click-through popovers that control the **host's** volume, switch to a known WiFi network, and connect a paired Bluetooth device. Clicking the clock opens a **month calendar** (today highlighted, `◂ ▸` to browse months) that also shows your **upcoming events** and underlines their days when [`khal`](https://github.com/pimutils/khal) is installed.
- **Notifications** — when a background or minimized app rings the terminal bell, its dock pill gets an attention dot and a 🔔 counter appears in the tray; the popover lists who rang and when — click an entry to jump to that window. Apps that copy via **OSC 52** (vim, tmux, …) have their clipboard forwarded to your real terminal.
- **Native image viewing** — real raster images inside windows via the Kitty graphics protocol (Ghostty/Kitty/WezTerm), with a cell placeholder fallback elsewhere. Open one with a launcher entry `command = "@image"`, `args = ["~/pic.png"]`.
- **File manager** — a native, mouse-and-keyboard file browser (launcher entry **Files**, or `@files`): **icon-grid, list, and Miller-columns** views, **image thumbnails** (via the Kitty graphics layer), a **preview pane** (text head / PDF text / metadata), **tabs**, **Get Info** (size, kind, Unix permissions, symlink target), folder navigation with history, single/ctrl/shift selection, new folder, rename, copy/cut/paste, and **delete-to-Trash** (never a hard delete). Double-click/Enter opens each file with its default app.
- **Default Apps** — a configurable file-type → app map (**Settings → Default Apps**): images open in the built-in viewer, text/code in your `$EDITOR`, and you can cycle the handler for each role. The file manager uses it to open files "just like a real OS."
- **Desktop icons** — clickable icons on the wallpaper, merged from your live `~/Desktop` folder and pinned shortcuts. Double-click to open (via Default Apps), drag to rearrange (snaps to a grid, positions persisted), and right-click for a context menu (open / rename / move to Trash / new folder). Image files show thumbnails on Kitty-graphics terminals.
- **App launcher** — a Windows-95-style **cascading menu** (a one-click **Shell** quick-launch first, then categories that fly out submenus of apps on hover/arrow) *and* a Spotlight search overlay. Navigate the cascade by mouse (hover to open, click to launch) or keyboard (`↑/↓`, `→` into a submenu, `←` back, `Enter` to launch). A **`+` button** at the dock's bottom-left opens a new shell instantly.
- **App store** — browse/search/install from a curated, **100%-verified** catalog of **600+** TUIs — including a dedicated **AI** category (Claude Code, Gemini CLI, Aider, opencode, Codex, Crush, Goose, Plandex, …) and a full **terminal office suite** (word processor, spreadsheets, presentations, email, calendar, contacts, notes) — OS-aware so Linux-only tools never show on macOS and vice-versa. If an app needs a toolchain you don't have (Go, Rust, Node, Python), the installer **detects it and offers to install it first**.
- **Custom apps** — add your own launcher entries (name + command) from **Settings → Apps**.
- **Working-directory picker** — launching a coding agent (Claude Code, Aider, …) opens a browsable file-tree so it starts in the project you choose; remembers recent directories.
- **Theming** — four built-in palettes (midnight, nord, gruvbox, dracula), switchable live from **Settings → Appearance**.
- **Dock app-grouping + window rename** — windows of the same app collapse into one dock pill with a colored **letter badge** (per-app color, configurable in `[dock.badges]`); click a grouped pill to choose between its windows. **Rename** any window (double-click its titlebar or `Ctrl+Space r`) — the label changes but it stays grouped with its app.
- **Simple view mode** — a top-bar toggle (`⊞` desktop ⇄ `▦` simple) that flips to a tmux-style full-screen-single-app view (no window decorations), keeping the menubar + dock; the dock is your app switcher. Same running apps in both modes.
- **Persistent daemon + thin client, with live updates** (tmux-style): apps run in a separate **apphost** process and survive client detach, SSH disconnects, **and a frontend reload** — update the binary and **reload the UI without killing your apps** (menubar **Restart**, `tuiui reload`, or **Settings → Update & Reload**). Closing an app window (titlebar **✕**) asks for confirmation first since it ends the process; built-in panels (Store/Settings/Files) close without asking. `tuiui kill` stops everything (daemon + apphost), and works even while a client is attached.
- **Bare-console mouse (Linux)** — on a raw Linux VT with no GUI terminal, tuiui reads the mouse directly from the **gpm** daemon (`apt install gpm`); see [the gpm section](#mouse-on-a-bare-linux-console-gpm).
- **In-app updater** — check for and install updates from **Settings → Updates**. The default **main** channel installs the latest **prebuilt release** (a fast download, not a source recompile), reloads, and reopens Settings on the Updates screen — apps intact. A **dev** channel (toggle in the same section) tracks the dev branch from source for testing. Mouse-clickable.
- **Systems switcher** — hop between tuiui sessions on different machines from the power menu (**host name ▾ → Systems**). Saved machines show a live **●/○ reachability dot** and switch in one click over ssh; **+ Add Remote…** asks for an ssh target (+ optional password and per-system theme), then transfers your SSH key (`ssh-copy-id`, generating one if needed), installs tuiui on the remote (mac / Linux / WSL2) plus **gpm** for console mouse, and connects. Exiting the remote session drops you straight back into your local desktop — whose apps never stopped. Each system can carry its **own theme**, applied on attach via `TUIUI_THEME`. Passwords are used once for the key copy (via `sshpass`, which the installer sets up) and never saved.
- **Remote files** — every saved system gets a **Files on <name>** entry in the launcher (Systems category): browse the remote machine's disk over ssh, make/rename/trash folders, and **copy files between machines** — `Ctrl+C` in one window, `Ctrl+V` in another, and tuiui runs the `scp` in the background (local↔remote and remote↔remote via `scp -3`).
- **AI assistant** — the **✦** menubar button opens a persistent chat panel (or a floating window — pick in **Settings → Assistant**) running your coding agent. Six frameworks are supported and verified: **Claude Code**, **[opencode](https://opencode.ai)**, **[smallcode](https://github.com/Doorman11991/smallcode)** (fully-local small models), **[Kilo Code](https://kilo.ai/cli)**, **[Hermes](https://hermes-agent.nousresearch.com)** (`hermes --tui`), and **[OpenClaw](https://openclaw.ai)** (`openclaw tui`). The agent's instructions live in the repo's [`agent/`](agent/) pack — embedded in the binary and stamped into `~/.local/share/tuiui/assistant/` on every launch in every convention the CLIs read (`CLAUDE.md`, `AGENTS.md`, `HERMES.md`, `knowledge/*.md`, plus an OpenClaw workspace pointer and a smallcode `.env` template) — and that folder is forced as the agent's cwd whatever framework runs. The briefing covers its role, the desktop-control CLI (`tuiui launch/tile/theme/msg`), the logs, the repo (so it can fix bugs and open PRs), **and all your saved systems** — so the agent can ssh/scp across your machines ("get that file from my ubuntu box onto this desktop") with the keys Add Remote already installed; the systems list is synced to remotes during setup so the assistant knows the same machines everywhere. The chat survives detach and reloads; all six install from the Store with setup tips shown in the detail pane. Hermes and OpenClaw also get **launcher chat shortcuts** when installed.
- **Logs viewer** — launcher → **tuiui → Logs** opens a live view of `~/tuiui-debug.log` (logging is always on): scroll with `↑/↓/PgUp/PgDn` or the wheel, press **`c` to copy the log to your terminal's clipboard** (OSC 52 — works in Ghostty/Kitty/WezTerm, even over ssh), `r` to refresh. Perfect for pasting into a bug report.

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
| `Ctrl+Space` then `r` | **Rename** the focused window (type a new name, Enter) |
| `Ctrl+Space` then `?` | **Help** — show this shortcut cheatsheet in-app (any key dismisses) |
| `Ctrl+Space` then `q` | Detach (apps keep running in the background) |

Exit/Restart/Shutdown live in the top-right **host-name menu** (`▾`): **Exit** detaches, **Restart** reloads the UI keeping apps alive, **Shutdown** stops everything. **Systems** cascades the machine switcher: click a saved machine to ssh into its tuiui session (its `✕` forgets it), or **+ Add Remote…** to set up a new one (Tab/↑↓ between fields, `←`/`→` picks the theme, Enter connects). Setup also copies your terminal's **terminfo** to the remote (so Ghostty/Kitty `TERM`s like `xterm-ghostty` don't break curses apps there — with an automatic `xterm-256color` fallback on every connect) and syncs your saved-systems list. Troubleshooting a switch: every step logs to `~/tuiui-debug.log` (open it in-app via launcher → tuiui → **Logs**, press `c` to copy it); `TUIUI_DEBUG=1` additionally prints the exact ssh/setup script before it runs.

Forget a shortcut? Press **`Ctrl+Space` then `?`** for the in-app cheatsheet.

Set the grid (rows × columns), gap, and auto-tile from **Settings → Windows**.

In the **working-directory picker** (opens when launching a coding agent): `↑`/`↓` to move, `→`/`←` to expand/collapse, `n` to make a new folder, `.` to toggle hidden dirs, `Enter` to open there, `Esc` to cancel.

In the **file manager** (launcher → **Files**): `↑`/`↓`/`←`/`→` move the cursor, `Enter` opens (folder → navigate, file → default app), `Backspace` goes up, `Ctrl+C`/`Ctrl+X`/`Ctrl+V` copy/cut/paste, `Delete` moves to Trash (with confirm), `F2` renames, `Ctrl+N` makes a new folder, `1`/`2`/`3` switch icon/list/columns views, `Space` toggles the preview pane, `.` toggles hidden files, `Esc` closes. **Tabs:** `Ctrl+T` new, `Ctrl+W` close, `Tab` switch. **Get Info** (size, kind, Unix permissions, symlink target) is on the context menu. Click an entry to select, the toolbar `◂ ▸ ▲` to navigate, and the scroll wheel to move through long folders. Image folders show **thumbnails** in icon view on terminals with the Kitty graphics protocol.

On the **desktop** (the empty wallpaper): click an icon to select, **double-click to open** (folders → the file manager, files → their default app, pins → the app), **drag** an icon to rearrange it (snaps to a grid, position saved), and **right-click** an icon (open / rename / move to Trash) or the empty desktop (new folder / clean up). Icons come from your `~/Desktop` folder plus pinned shortcuts.

Mouse: click **tuiui** (top-left) for the app launcher, the **`⊞`/`▦`** toggle (next to it) to switch desktop/simple view, **✦** to open/hide the AI assistant panel, your **host name `▾`** (top-right) for the Exit/Restart/Shutdown menu, titlebar buttons (`– ▢ ✕`), and **double-click a titlebar to rename** the window. Drag titlebars/edges to move/resize, drag a window to a screen edge to snap it into a grid cell, click a tray indicator (clock/volume/WiFi/…) for its popover, and click dock pills to focus (a grouped pill opens a chooser). Scroll the wheel over a shell/app window to read its scrollback. The mouse passes through into apps that enable mouse reporting.

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
tuiui reload     # restart the UI only; apps keep running
tuiui kill       # shut the daemon down (closes all windows)

# Control a running desktop from any shell (also how the AI assistant drives it):
tuiui launch btop          # open a new window running btop
tuiui tile                 # tile all windows into the grid
tuiui theme nord           # switch theme
tuiui msg '"MaximizeFocused"'   # raw control message (ClientMsg JSON)
```

Detach with **`Ctrl+Space` then `q`** (apps keep running); to fully stop, **shut
down** from the top-right host-name menu (`▾`), `tuiui kill`, or `Ctrl+Space` then
`Q`. That same menu also has **Restart** (reload the UI, apps stay alive — see
*live updates* above). The socket lives in a per-user `0700` directory
(`$XDG_RUNTIME_DIR` or the temp dir).

Configuration lives at `~/.config/tuiui/config.toml` (see [example below](#configuration)). On first run with no config, tuiui opens your `$SHELL` and auto-detects installed TUIs.

### Recommended terminal

tuiui wants a **truecolor, mouse-capable terminal**: **Ghostty**, Kitty, WezTerm, or iTerm2. Avoid macOS Terminal.app (weak truecolor + flaky mouse). Inline images (image viewer, file-manager thumbnails, desktop icon tiles) need a terminal that speaks the **Kitty graphics protocol** (Ghostty/Kitty/WezTerm); without it those fall back to text glyphs.

This works over SSH too: your **local** terminal emulator does the mouse + graphics reporting, so a headless remote box needs nothing special — the emulator on the machine you're sitting at sends the events.

### Mouse on a bare Linux console (gpm)

If you run tuiui **directly on a bare Linux virtual console** — no X/Wayland, just a shell on a TTY (locally, or after SSHing into a headless box and dropping to its console) — the kernel console emits no terminal mouse sequences. Install **gpm** (the General Purpose Mouse daemon) and tuiui will talk to it directly for full mouse support:

```bash
sudo apt install gpm        # Debian/Ubuntu (use your distro's package elsewhere)
# make sure the gpm service is running on the console, then launch tuiui
```

tuiui auto-detects the console and connects to gpm's socket — no config needed (`TUIUI_GPM=0` disables it, `TUIUI_GPM=1` forces an attempt). It speaks gpm's socket protocol directly (no `libgpm` linkage), so it stays MIT-clean. Recommended for anyone running tuiui on a Linux shell without a desktop. Note: a bare console still can't display the inline image tiles (those need a Kitty-graphics terminal); windows, the dock, the launcher, and the mouse all work.

### Run the apphost as a service

The **apphost** (the process that owns your running apps) is normally spawned on
demand and kept alive across frontend reloads/detaches. To have it **auto-start on
login and restart if it crashes** — so your apps are always waiting for you — install
it as a per-user service:

```bash
tuiui service install      # set up + start the apphost service for this platform
tuiui service status       # show the backend, whether it's installed/running
tuiui service uninstall    # stop + remove it
```

Per platform (chosen automatically):

- **macOS** → a launchd **LaunchAgent** (`~/Library/LaunchAgents/co.uk.janlabs.tuiui-apphost.plist`).
  Because it runs inside your GUI login session, the apphost gets **Keychain access**,
  which also lets tools like the Claude Code CLI stay logged in when run inside tuiui.
- **Linux / WSL with systemd** → a `systemctl --user` unit (`tuiui-apphost.service`).
  Tip: `loginctl enable-linger $USER` keeps it running across logout.
- **Old WSL / minimal distros (no user systemd)** → a guarded block appended to
  `~/.profile` that starts the apphost on login (best-effort; no crash supervision).

The installer bakes your current `PATH`/`HOME`/`SHELL`/`LANG` into the service so it
can find and launch your apps — re-run `tuiui service install` to refresh them. The
apphost is idempotent: a redundant start (service + the daemon's on-demand spawn)
exits cleanly instead of fighting over the socket.

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

Persistent remote sessions work today: the daemon + apphost run on the host and keep your apps alive, so you can SSH in, `tuiui attach`, detach (or just drop the connection), and reattach later — from the same terminal, a different one, or another machine — and find everything exactly where you left it.

## Configuration

```toml
snapping_enabled = true   # drag-to-cell snapping
snap_threshold = 3        # edge band (cells) that engages snapping
window_shadows = true
theme = "midnight"        # midnight | nord | gruvbox | dracula

# AI assistant (the ✦ menubar button; also editable in Settings → Assistant)
# assistant_command = "opencode"   # pin a framework (default: auto-detect
#                                  # claude/opencode/smallcode/kilo/hermes/openclaw)
# assistant_args = ["--model", "anthropic/claude-sonnet-4-6"]  # extra CLI args
assistant_mode = "panel"  # "panel" (right-docked) | "window" (floating)

# In-app updater (Settings → Updates)
update_branch = "main"    # "main" = fast prebuilt releases; "dev" = build from source

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

# Dock app-badge colors: keyword (matched in the app name/command) → color
# (a named color or #rrggbb). Unlisted apps get a stable color hashed from
# their name. The badge is the app's initial; renamed windows keep it.
[dock.badges]
claude = "orange"
kilo = "yellow"
```

Most of these are editable live from the in-app **Settings** panel, which writes this file back.

## Architecture

A pure-logic core (geometry, cell compositor, window manager, input routing) wrapped by I/O adapters (a `crossterm` terminal backend, a `portable-pty` + `alacritty_terminal` process host). A `SessionCore` owns the windows and apps and talks to the front-end through a `ClientMsg`/`Frame` boundary that **crosses a Unix socket**: the daemon serves a thin client over it (the thing you attach), and the apps themselves live in a separate **apphost** process behind a second socket, so they survive a frontend reload/restart.

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
- **✅ Apphost/frontend split + live updates:** apps run in a separate long-lived process; the UI can **reload (or update) without killing apps** (menubar Restart, `tuiui reload`, Settings → Update & Reload).
- **✅ Mouse passthrough:** full-fidelity mouse (buttons/drag/scroll/modifiers) forwarded into apps that request it, in both views.
- **✅ Simple view mode:** `⊞`/`▦` top-bar toggle between the windowed desktop and a full-screen-single-app view.
- **✅ Dock grouping + window rename + app badges:** same-app windows group into one pill with a colored letter badge; rename windows (double-click titlebar / `Ctrl+Space r`).
- **✅ Bare-console mouse (Linux):** native `gpm` support for a mouse on a raw Linux VT (no GUI terminal needed).
- **✅ Apphost as a service:** `tuiui service install` runs the apphost as a per-user service (launchd / systemd `--user` / `~/.profile` fallback) that auto-starts on login and restarts on crash; the macOS LaunchAgent also restores Keychain access (e.g. Claude Code login) inside tuiui.
- **✅ Smart installs:** the store detects a missing toolchain (Go/Rust/Node/Python) before an install and offers to set it up first; a confirm dialog guards closing an app window (which ends its process).
- **✅ Terminal office suite:** word processor (`wordgrinder`), spreadsheets (`sheets`/`sc-im`/`visidata`), presentations (`slides`/`presenterm`), email (`himalaya`/`aerc`), calendar (`khal`/`calcure`), contacts (`khard`/`abook`), and notes (`nb`/`nap`) — all in the store.
- **✅ Systems switcher:** power menu → Systems — saved machines with live ●/○ dots; Add Remote transfers SSH keys, installs tuiui + gpm + your terminal's terminfo on the remote, syncs the systems list, and connects; per-system themes; drop back to the local desktop when the remote session ends.
- **✅ Clock + calendar:** date+time in the menubar; click for a month calendar with `◂ ▸` navigation and `khal` events.
- **✅ Notifications:** background bell → dock attention dot + 🔔 tray popover (click to focus); apps' OSC-52 copies forwarded to the host clipboard.
- **✅ Remote files:** browse saved systems over ssh in the file manager; copy between machines with Ctrl+C/Ctrl+V (background scp, `-3` for remote↔remote).
- **✅ Logs viewer:** launcher → tuiui → Logs; `c` copies the log to the host clipboard via OSC 52.
- **✅ Scrollback:** the mouse wheel pages back through any app window's history.
- **✅ Versioned releases + update channels:** semver in `Cargo.toml`, a `CHANGELOG`, and a main/dev channel switcher in Settings → Updates (main installs prebuilt releases fast; dev builds from source).
- **✅ AI assistant:** the ✦ chat panel/window — six agent frameworks, the repo `agent/` briefing pack, desktop control via the `tuiui` CLI, cross-machine awareness.
- **Slice 6 — GUI/Wayland mode** (host real GUI apps; audio/video streaming to the client) — plus a parked idea: a fullscreen **browser PWA** of tuiui (multiple simultaneous frontends on one apphost).
- **Slice 7 — Standalone "TUI-OS" app** (bundle a GPU terminal + tuiui into a fullscreen app).

## Credits

The app catalog is generated from [rothgar/awesome-tuis](https://github.com/rothgar/awesome-tuis) via `scripts/gen_catalog.py`. tuiui stands on the shoulders of `alacritty_terminal`, `crossterm`, and `portable-pty`.

## License

**[MIT](LICENSE)** © 2026 [JAN LABS LTD](https://janlabs.co.uk). Use, modify, and
distribute freely — including commercially — provided the copyright notice and
license text are retained.

tuiui's dependencies are permissive (MIT / Apache-2.0 / BSD / Zlib / Unlicense, plus
one MPL-2.0 crate used unmodified); their notices are collected in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
