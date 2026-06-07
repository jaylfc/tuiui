# Mouse Passthrough Into Apps — Design

**Status:** Approved (2026-06-08). Full-fidelity mouse forwarding into PTY apps, in both
desktop and simple view modes.

## Problem / goal

The mouse currently only drives tuiui's own chrome (menubar, dock, launcher hover) and, in
desktop mode, window management. It is never delivered to the running app, so btop/yazi/vim/
lazygit/tmux can't be used with the mouse. Goal: when the pointer is over a **focused app
that has requested mouse reporting**, forward the event (all buttons, drag, scroll, modifiers)
into that app's PTY using the app's own encoding — identically in desktop and simple modes.

## Key facts

- The embedded emulator (`alacritty_terminal::Term`, owned by the apphost) tracks exactly
  what the app asked for, in `Term.mode()` → `TermMode`: `MOUSE_REPORT_CLICK`, `MOUSE_DRAG`,
  `MOUSE_MOTION`, `SGR_MOUSE`, `UTF8_MOUSE`, `ALTERNATE_SCROLL`. This is the source of truth
  for *whether* to forward and *how* to encode.
- Forwarding = writing the encoded mouse bytes to the app's PTY (`apphost.input`); the app
  reacts. We never feed the emulator — it just renders the app's output.

## Architecture (low-risk gating)

The **daemon computes the focused app's content rect, but only when that app wants mouse**,
and ships it to the client as `Flags.app_area: Option<Rect>`. The thin client then routes:
- event **inside `app_area`** → send the new rich `ClientMsg::MouseInput` (for passthrough),
- event **anywhere else** → send today's chrome/WM variants (`MouseDown/Drag/Up/RightDown/
  MouseDouble`), unchanged.

This keeps every existing chrome/WM path and its tests intact, and isolates all new logic.
`app_area` is the focused app's **content** rect (not its decorations), so title-bar drag and
window resize still work for mouse-apps in desktop mode; the menubar (row 0) and dock
(row h-1) are always outside it. In simple mode `app_area` is the full work area. Because it's
the *focused* app only, clicking an unfocused mouse-app window first focuses it (chrome path),
and subsequent clicks forward (in-app) — the expected behaviour.

## Components

### 1. App mouse mode (apphost → frontend)

- A serializable `AppMouse { report_click, report_drag, report_motion, sgr, utf8, alternate_scroll }`
  (all `bool`), derived from `Term.mode()`. `wants_mouse() = report_click || report_drag || report_motion`.
- `AppInstance::mouse_mode() -> AppMouse` reads `term.lock().mode()`.
- `AppHost::mouse_mode(&self, AppId) -> AppMouse`. `LocalAppHost` reads the instance;
  `RemoteAppHost` returns the cached value.
- `proto::HostEvt::Frame` gains `mouse: AppMouse` (tiny; rides the per-frame push it already
  sends on change — also send a frame when `mouse` changes). `RemoteAppHost` caches it.

### 2. Mouse encoder (`src/mouseenc.rs`, pure)

`encode(ev: &MouseInput, m: &AppMouse, alt_screen: bool) -> Option<Vec<u8>>` returning the
bytes to write to the PTY, or `None` if this event shouldn't be forwarded for the app's mode
(e.g. a pure move when only `report_click` is set, or scroll handled as arrow keys).

- **Button base codes:** left 0, middle 1, right 2; release 3 (legacy). Scroll: up 64, down
  65, left 66, right 67. Drag/motion adds 32. Modifiers add: shift 4, alt 8, ctrl 16.
- **SGR (when `sgr`):** `ESC [ < Cb ; col ; row M` for press/scroll/motion, `... m` for
  release; 1-based coords; no 223 limit.
- **Legacy/X10 (else):** `ESC [ M (32+Cb) (32+col) (32+row)`; clamp coords to 223; release is
  button code 3.
- **Reporting gates:** only emit drag if `report_drag`; only emit pure motion if
  `report_motion`; always emit clicks/scroll if `wants_mouse()`.
- **Alternate scroll:** if `alternate_scroll && alt_screen && !wants_mouse()`, a wheel event
  encodes as arrow keys (`ESC [ A` / `ESC [ B`) instead — this is how `less`/`man` scroll.
  (If the app reports mouse, wheel is sent as a mouse event instead.)
- Fully unit-tested (button/modifier/scroll matrices, SGR vs legacy, the 223 clamp).

### 3. Protocol (`src/protocol.rs`, `src/session.rs`)

- `Flags.app_area: Option<Rect>` (`#[serde(default)]` → older clients/daemons stay compatible).
- New `ClientMsg::MouseInput(MouseInput)` with
  `MouseInput { col: i32, row: i32, button: MouseButton, action: MouseAction, mods: MouseMods }`
  where `MouseButton { Left, Middle, Right, None }`,
  `MouseAction { Down, Up, Drag, Move, ScrollUp, ScrollDown, ScrollLeft, ScrollRight }`,
  `MouseMods { shift, ctrl, alt }` (bools). All `serde`.

### 4. Session (`src/session.rs`)

- Compute `app_area`: the focused window's content rect **iff** it is `WinContent::App(aid)`
  and `self.apphost.mouse_mode(aid).wants_mouse()`; in simple mode use `simple_content_rect()`.
  Set it on `Flags` in `daemon.rs::serve_client`.
- `apply(ClientMsg::MouseInput(m))`: re-validate the point is in the focused app's area;
  translate screen → app-local (`col - area.x`, `row - area.y`, both 0-based → encoder makes
  1-based); fetch `mouse_mode(aid)` + alt-screen; `mouseenc::encode(...)`; if `Some(bytes)`
  → `apphost.input(aid, &bytes)`. (Alt-screen state: add `AppMouse`-adjacent flag or read
  from `Term.mode().contains(ALT_SCREEN)` — include `alt_screen: bool` in `AppMouse`.)

### 5. Client (`src/client.rs`)

- Capture the full crossterm mouse set: Down/Up/Drag for Left/Middle/Right, Moved,
  ScrollUp/Down/Left/Right, with `event.modifiers`.
- For each event, if `flags.app_area` contains the cell → send `ClientMsg::MouseInput{...}`;
  else fall back to the existing sends (left → MouseDown/Drag/Up + the client-side
  double-click→MouseDouble; right → MouseRightDown; scroll → the current Store/FM routing;
  move → MouseDrag for launcher hover). Chrome behaviour is byte-for-byte unchanged when
  `app_area` is `None` or the pointer is outside it.

## Behaviour across modes

Identical: pointer over a focused mouse-app's content → forwarded to the app; everything else
→ tuiui chrome. Desktop mode additionally keeps title-bar drag / border resize (those cells
are outside `app_area`). Simple mode forwards across the whole work area.

## Testing

- **mouseenc unit tests** (pure): SGR + legacy for left/middle/right down/up/drag, scroll
  up/down/left/right, each modifier and combinations, the 223 legacy clamp, motion/drag gating,
  alternate-scroll→arrows.
- **AppMouse**: `Term.mode()` → flags mapping (spawn `sh`, no mouse → `wants_mouse()==false`;
  a unit test feeding a `?1000h`/`?1006h` sequence through the emulator → flags set).
- **proto**: `Frame` round-trips with `mouse`; `Flags`/`MouseInput` round-trip.
- **session**: `app_area` is `Some(content_rect)` only when the focused app wants mouse, else
  `None`; `apply(MouseInput)` over a mouse-app writes encoded bytes to the PTY (assert via a
  fake/echo or that `apphost.input` was called — use `LocalAppHost` + a app that echoes).
- **Manual:** btop (click panels, scroll), yazi (click files, scroll), vim `:set mouse=a`
  (click to move cursor, wheel scroll, drag select), in both desktop and simple modes; confirm
  window title-bar drag still works in desktop mode for a mouse-app.

## Out of scope (future)

Mouse for non-app native widgets is unchanged (they already have bespoke handling). Pixel
(SGR-1016) mouse. Bracketed-paste interactions. Focus-follows-mouse.
