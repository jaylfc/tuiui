# Tuiui — Vision & Architecture

**Status:** Approved vision. Implemented as a sequence of vertical slices, each with its own spec → plan → build cycle.
**Date:** 2026-06-03

## What Tuiui is

A **desktop environment for the terminal**. A long-running core owns a set of overlapping windows; each window hosts a real terminal application (a TUI) running as a child process. Clients attach to the core — locally or over SSH — and render the desktop to whatever terminal they happen to be on. The "apps" are the terminal programs catalogued by lists like [awesome-tuis](https://github.com/rothgar/awesome-tuis), installed through a built-in store backed by Tuiui's own curated catalog.

The feel: a floating-window desktop with a top menubar, a bottom dock, a mouse cursor, snapping/tiling assists, a settings panel, and an app store — all in text cells.

## Core decisions (locked during brainstorming)

| Decision | Choice | Why |
|---|---|---|
| App execution model | **Multiplexer** — apps run as real child processes in windows, output composited | True simultaneous "desktop" feel; proven pattern (Ratty, RMUX, vt100 + portable-pty) |
| Language / foundation | **Rust, from scratch** | Full control over compositor, mouse cursor, taskbar; primitive crates are mature |
| Window model | **Floating / overlapping**, with **configurable** snapping + tiling assists | User wants desktop freedom first, tiling discipline optional |
| Desktop chrome | **Top menubar + bottom dock** (macOS-like) | Richest home for global + per-app menus |
| Remote access | **Daemon + thin-client split**, attach over SSH | Persistent sessions + per-client capability negotiation |
| Store catalog | **Git-repo-of-manifests** (`tuiui-catalog`), TOML manifests; ratings API optional later | Zero infra, PR-based curation, solo-ownable |
| Installer | Best recipe (prebuilt binary → cargo/go/npm → brew) into managed `~/.tuiui/apps/` | Clean install/uninstall, never touches system dirs |
| Store UI | Card **grid** to browse + **detail pane** on click | Discoverable + informative |
| Settings | **Sidebar + content pane** panel, persists to hand-editable `~/.config/tuiui/config.toml` | Scales to themes and beyond |
| Theming | Future slice | Needs compositor + config first |

## Architecture — layers

Each layer is a focused unit with a clean boundary, understandable and testable on its own.

1. **Compositor & input.** Cell buffer with true RGBA alpha-blending (Porter-Duff "over"), scissor clipping, double-buffered diff rendering, z-ordered layers. Owns the render loop, the rendered mouse cursor, and hit-testing. Knows nothing about windows or PTYs — composites layers, reports input events. Candidate primitive: [`opentui_rust`](https://github.com/Dicklesworthstone/opentui_rust); fallback is a custom double-buffered cell compositor.
2. **Process / PTY host.** One per running app: spawns the app in a [`portable-pty`](https://docs.rs/portable-pty) PTY, parses output with [`vt100`](https://docs.rs/vt100) into a screen grid, exposes that grid as a compositor layer, forwards keyboard/mouse into the app.
3. **Window manager.** Floating overlapping windows: focus, z-order, drag/move/resize, titlebars, and configurable snapping/tiling assists (drag-to-edge halves, Super+Arrow tiling, snap-to-grid, threshold). Pure geometry logic; testable without a terminal.
4. **Session core ↔ client protocol.** The boundary that makes remote work. Core (layers 1–3 + app state) talks to clients via a defined message protocol. v1: in-process. Later: core becomes a daemon, clients attach over a socket / SSH, with per-client capability negotiation (truecolor + alpha + pixel-mouse SGR-1016 where available; graceful degrade to SGR-1006 + 256-color). **Baked in from day one so "remote later" is incremental, not a rewrite.**
5. **Shell UI.** Desktop chrome as compositor layers above the app windows: top menubar (global + focused-app menus), bottom dock (favourites + running apps), launcher, window chrome.
6. **Store.** Syncs the git `tuiui-catalog` of per-app TOML manifests; resolves the best install recipe into `~/.tuiui/apps/`; install runs visibly in a window. Browse = grid, click = detail pane.
7. **Config & Settings.** Everything persists to hand-editable `~/.config/tuiui/config.toml`; the Settings panel edits that file.
8. **Theming** *(future).* Slots into Appearance once compositor + config exist.

## Trust model (v1)

Apps are arbitrary third-party programs. v1 trust = **curated catalog + the exact install command is shown + installs confined to `~/.tuiui/`**. OS-level sandboxing (namespaces/seccomp) is a future enhancement, explicitly out of scope for early slices.

## Build order

- **Slice 1 — "The Shell proves itself"** *(first detailed spec — see `2026-06-03-tuiui-slice1-shell-design.md`)*: compositor + window manager + process host + minimal chrome. Launch 2–3 bundled apps into floating, mouse-draggable, snappable windows; switch/focus/close via the dock; quit cleanly. Core/client boundary exists in-process. No store, no remote daemon, no settings UI.
- **Slice 2 — Daemon & remote:** split core into a daemon; attach/detach, session persistence, SSH attach, capability negotiation.
- **Slice 3 — The Store:** catalog sync, manifest format, installer, store UI.
- **Slice 4 — Settings & config:** full settings panel writing `config.toml`.
- **Slice 5 — Theming.**

Rationale: Slice 1 de-risks the defining, riskiest capabilities (compositing, mouse cursor, running real TUIs in windows). Everything else is valuable but lower-risk and builds on a proven shell.

## Testing strategy (whole project)

- **Golden-frame tests** — render to a buffer, assert cells — for compositor and chrome.
- **Headless PTY tests** — spawn a fake/scripted app, assert the window's parsed grid — for the process host.
- **Pure unit tests** — window-manager geometry, snapping math, install-recipe resolution.
- **Protocol round-trip tests** — for the core/client boundary (Slice 2+).
