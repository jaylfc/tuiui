# tuiui

**A desktop environment for the terminal.** tuiui is a windowing shell that runs *inside* a terminal: floating, overlapping windows — each hosting a real terminal application — with a mouse cursor, a top menubar, a bottom dock, window snapping, an app launcher, and (in progress) an app store backed by the [awesome-tuis](https://github.com/rothgar/awesome-tuis) catalog.

It's a multiplexer at heart (apps run as real child processes in PTYs and are composited into windows), built from scratch in Rust.

> **Status: early, active development.** The core shell, window management, app launcher, and app catalog work today. The store UI, settings panel, theming, and a daemon/remote layer are on the roadmap below.

## What works today

- **Floating, overlapping windows** with drop shadows, each running a real TUI (btop, a shell, vim, …) in its own pseudo-terminal.
- **Faithful rendering** via a full terminal emulator ([`alacritty_terminal`](https://docs.rs/alacritty_terminal)) — even demanding apps like btop render correctly.
- **Mouse-driven**: drag titlebars to move, drag edges to resize, click the dock to focus, click titlebar buttons to minimize/maximize/close.
- **Window snapping** (drag to a screen edge → half-screen) and keyboard window management.
- **Top menubar + bottom dock** chrome.
- **App launcher** — a menubar dropdown *and* a Spotlight-style search overlay, with apps grouped by category.
- **App catalog** — the full awesome-tuis list (582 apps, 12 categories), bundled and used to auto-detect which TUIs you already have installed on `$PATH`.

## Controls

tuiui uses a **leader key** (`Ctrl+Space`) so its shortcuts never collide with macOS, your terminal, or the focused app. Press the leader, release, then a key:

| Shortcut | Action |
|---|---|
| `Ctrl+Space` then `Space` | Spotlight launcher (type to filter, ↑/↓, Enter) |
| `Ctrl+Space` then `a` | App menu (dropdown) |
| `Ctrl+Space` then `m` / `n` | Maximize / minimize focused window |
| `Ctrl+Space` then `[` / `]` | Snap focused window left / right |
| `Ctrl+Space` then `s` / `,` | Open the Store / Settings |
| `Ctrl+Space` then `q` / `Q` | Detach (keep running) / shut down the daemon |

Mouse: click **✦ tuiui** (top-left) for the app menu, the **✕ Quit** button (top-right) to exit, titlebar buttons (`– ▢ ✕`), drag titlebars/edges to move/resize, click dock pills to focus.

## Build & run

Requires a [Rust toolchain](https://rustup.rs).

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
snapping_enabled = true
snap_threshold = 3

# Auto-started at launch (and shown in the dock)
[[apps]]
name = "btop"
command = "btop"
[[apps]]
name = "shell"
command = "zsh"

# Extra apps offered in the launcher (installed TUIs are auto-added)
[[launcher]]
name = "lazygit"
command = "lazygit"
category = "Git"
```

## Architecture

A pure-logic core (geometry, cell compositor, window manager, input routing) wrapped by I/O adapters (a `crossterm` terminal backend, a `portable-pty` + `alacritty_terminal` process host). A `SessionCore` owns the windows and apps and talks to the front-end through a `ClientMsg`/`Frame` boundary designed to later cross a socket for remote attach.

Design docs and the slice-by-slice plan live in [`docs/superpowers/`](docs/superpowers/).

## Roadmap

- **✅ Slice 1 — Shell:** compositor, window manager, PTY host, chrome, launcher.
- **✅ Slice 3 — Store:** browse/search/install the full awesome-tuis catalog (mouse + keyboard; brew/cargo/go install).
- **✅ Slice 4 — Settings:** sidebar settings panel writing `config.toml`.
- **✅ Slice 2 — Daemon:** persistent daemon + thin client; detach/reattach keeps windows and processes alive.
- **Slice 5 — Theming** (make the palette/shadows fully configurable from Settings → Appearance).
- **Slice 6 — GUI/Wayland mode** (host real GUI apps via the Kitty graphics protocol).
- **Slice 7 — Standalone "TUI-OS" app** (bundle a GPU terminal + tuiui into a fullscreen app).

## Credits

The app catalog is generated from [rothgar/awesome-tuis](https://github.com/rothgar/awesome-tuis) via `scripts/gen_catalog.py`. tuiui stands on the shoulders of `alacritty_terminal`, `crossterm`, and `portable-pty`.

## License

MIT — see [LICENSE](LICENSE).
