# Tuiui — Slice 1: "The Shell proves itself"

**Status:** Approved design, ready for implementation planning.
**Date:** 2026-06-03
**Parent:** `2026-06-03-tuiui-vision-design.md`

## Goal

Prove the riskiest, most-defining capabilities of Tuiui in a single runnable binary: **composite overlapping floating windows, each hosting a real terminal app, driven by a mouse cursor, with a dock to switch between them.** If this works, the whole concept works.

## In scope

- A single-process binary (`tuiui`) that takes over the terminal (raw mode, alternate screen, mouse capture) and renders a desktop.
- **Compositor** rendering z-ordered layers to the terminal with double-buffered diffing; a rendered **mouse cursor**; alpha-blended window shadows.
- **Process/PTY host**: run 2–3 **bundled** apps (configurable list, e.g. `btop`, `lazygit`, a shell) each in its own PTY, parse with `vt100`, expose as a window's content.
- **Window manager**: floating overlapping windows with titlebars; focus + z-order (click to raise); **drag titlebar to move**; **drag edge/corner to resize**; close button; **drag-to-edge snapping** to half-screen (the one snapping assist proven in this slice).
- **Minimal chrome**: top menubar (static: `✦ Tuiui` + focused app name + clock) and a **bottom dock** listing running apps; clicking a dock entry focuses/raises that window.
- **Input routing**: events over chrome/titlebars/borders go to the window manager; events inside a focused window's content area are forwarded to that app's PTY (translated to the app's local coordinates).
- **Capability detection**: detect truecolor + mouse mode (SGR-1016 pixel if available, else SGR-1006 cell); render accordingly. Cell-accurate mouse is the baseline that must work everywhere.
- **Read-only config**: read bundled-app list, snap-threshold, and toggles from `~/.config/tuiui/config.toml` if present, else sane defaults. (Writing/Settings UI is Slice 4.)
- Clean teardown: quit hotkey kills child PTYs, restores the terminal.
- **Core/client boundary present in-process**: the window/app state lives behind a `SessionCore` interface that communicates with the renderer/input front-end via a message type, so Slice 2 can move it across a socket without restructuring.

## Out of scope (deferred to later slices)

Daemon/socket/SSH attach & persistence (Slice 2); the Store, catalog sync, installer (Slice 3); Settings panel & config writing, themes (Slice 4–5); per-app menubar menus (static menubar only here); multiple workspaces/virtual desktops; floating *non-app* widgets (calculator etc.); Super+Arrow tiling and snap-to-grid (only drag-to-edge half-snap is in this slice).

## Proposed crate stack

- `crossterm` — terminal setup (raw mode, alt screen, mouse capture, resize/focus events), reading input events.
- `portable-pty` — spawn child apps in PTYs (cross-platform incl. ConPTY).
- `vt100` — parse child app output into a screen grid (cells + styles + cursor).
- **Compositor primitive — spike first:** evaluate `opentui_rust` for alpha-blended, scissor-clipped, double-buffered cell composition + hit-testing. **Fallback:** a small custom double-buffered cell compositor (known-quantity; ~a few hundred lines) if the port isn't ready. The compositor choice is settled by the Task 1 spike, not assumed.
- `tokio` *(or threads)* — concurrent PTY read loops feeding the render loop. Decision deferred to the plan; threads + channels are acceptable for Slice 1.

## Components & boundaries

| Component | Responsibility | Depends on | Testable via |
|---|---|---|---|
| `compositor` | Layers → terminal cells; alpha; diff render; cursor; hit-test | terminal backend | golden-frame buffer tests |
| `pty_host` | Spawn app, pump PTY, `vt100` parse, expose grid + dirty regions | portable-pty, vt100 | headless scripted-app tests |
| `window` / `wm` | Window structs; focus; z-order; move/resize/snap geometry | (pure) | geometry unit tests |
| `chrome` | Menubar + dock layers; dock hit regions | compositor, wm state | golden-frame tests |
| `input` | Map raw events → WM action OR forward-to-app | wm, pty_host | routing unit tests |
| `session_core` | Owns windows+apps; the message boundary | wm, pty_host | protocol round-trip tests |
| `app` / main loop | Wire it all; render loop; teardown | all | manual + smoke test |

## Key data model (sketch)

- `Cell { ch, fg: Rgba, bg: Rgba, attrs }`
- `Rect { x, y, w, h }` (cell coordinates)
- `Layer { z, origin, buffer, opacity, scissor }`
- `Window { id, title, rect, z, state: Floating|SnappedLeft|SnappedRight, app: AppId }`
- `AppInstance { id, pty, parser: vt100::Parser, title }`
- `Focus { window: Option<WindowId> }`
- `CoreMsg` / `ClientMsg` — the in-process protocol (e.g. `ClientMsg::Input(Event)`, `ClientMsg::Resize`, `CoreMsg::Frame(Vec<Layer>)`) — designed so it can later cross a socket.

## Interaction model

- **Move:** mouse-down on titlebar → drag → window follows; release near a screen edge → snap to that half (if snapping enabled).
- **Resize:** mouse-down on border/corner → drag → resize; the app's PTY is resized (`SIGWINCH`) to match the new content area.
- **Focus/raise:** mouse-down anywhere on a window raises it and focuses it; dock click does the same.
- **Forward to app:** when a focused window's content area has the mouse/keyboard, translate coordinates to the app's local space and write to its PTY.
- **Quit:** a reserved chord (e.g. `Ctrl+Alt+Q`) tears down all PTYs and restores the terminal. Reserved chords are intercepted before app-forwarding.

## Risks & open questions (resolve during planning/spike)

1. **`opentui_rust` readiness** — Task 1 spike decides compositor; custom fallback bounded.
2. **Reserved-key collisions** — a global modifier (e.g. `Super`/`Ctrl+Alt`) must be carved out from app input; pick one that rarely clashes.
3. **PTY resize correctness** — apps must repaint cleanly on resize; verify with `btop`/`lazygit`.
4. **Performance** — diffed double-buffering must keep full-desktop redraws cheap; budget a frame cap.
5. **Pixel vs cell mouse** — drag feels best with pixel mouse, but must remain usable at cell granularity on plain terminals.

## Done when

- `tuiui` launches, shows menubar + dock + desktop.
- 2–3 bundled apps open in floating windows; each runs and renders correctly (`btop` animates, `lazygit` is interactive).
- Windows can be moved, resized, raised, focused, and closed with the mouse; drag-to-edge snaps to halves.
- Keyboard/mouse reach the focused app; the quit chord restores the terminal with no leaked child processes.
- Golden-frame, headless-PTY, and geometry test suites pass.
