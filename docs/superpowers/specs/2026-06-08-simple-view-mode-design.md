# Simple View Mode — Design

**Status:** Approved (2026-06-08). A frontend-only view toggle; no apphost/protocol/client changes.

## Problem / goal

tuiui today is a windowed desktop. The user wants a **tmux-like "simple" mode**: one app
full-screen at a time, switch between them, no window decorations or desktop — but reusing
tuiui's existing top menubar and bottom dock rather than a separate program. It is a **view
toggle inside tuiui**, flipped from the top bar. Apps live in the apphost (Phase 2), so the
same running instances appear in both modes; toggling never restarts or moves apps.

## Behaviour

Two view modes on the running frontend:

- **Desktop mode** (today): floating windows + decorations + desktop icons.
- **Simple mode**: the **focused** window is drawn full-screen — no border/title/shadow —
  filling the work area between the top menubar (row 0) and bottom dock (row h-1). The
  desktop-icon layer is hidden. All other windows are hidden (only the focused one shows).

Both bars stay in both modes:
- **Top menubar**: `Go` (launcher), the **mode toggle** button, focused-app name, tray, and
  the `tuiui ▾` power menu (Exit/Restart/Shutdown). Everything keeps working.
- **Bottom dock**: lists every open window — in simple mode it is the **app switcher**:
  clicking a dock pill focuses that app and makes it the full-screen one.

**The toggle button** sits just right of `Go` and shows the **current** mode's glyph:
- `⊞` (U+229E) when in **desktop** mode,
- `▦` (U+25A6) when in **simple** mode.

Clicking it switches modes. Rendered as `" ⊞ "` / `" ▦ "` (3 cells) at x=4 (Go is x=0..3,
focused-app name starts at x=10, so it fits with a gap and never collides).

**Empty simple mode** (no windows open): blank work area with a centered hint
`Press Go to launch an app`.

**App sizing:** in simple mode the focused app's PTY is resized to the full work area
(`Rect{x:0, y:1, w, h-2}`). On switching focus (dock click) the newly-focused app is resized
to fill. Toggling back to desktop restores every window's original rect/size (we never mutate
stored window geometry, so the desktop layout is preserved).

## Input

- **Keyboard**: unchanged — keystrokes go to the focused app exactly as today.
- **Mouse**: the menubar (Go / toggle / tray / power) and dock hit-tests run first, exactly
  as today, so all chrome works in both modes. In simple mode, clicks in the app area are not
  forwarded into the app (mouse-into-PTY passthrough is a separate future feature — out of
  scope here); they are ignored. Keyboard fully drives the app.

## Implementation (frontend-only)

- `SessionCore` gains `simple: bool` (default `false`) + `pub fn toggle_simple()` /
  `pub fn simple_mode() -> bool`.
- **chrome.rs**: `render_menubar` takes the current mode and draws the toggle glyph;
  add `menubar_mode_region() -> Rect` (x=4, w=3) for hit-testing.
- **session.rs handle_mouse**: a click in `menubar_mode_region()` calls `toggle_simple()`
  (and resizes the focused app appropriately). Add it alongside the existing brand/power
  region checks.
- **session.rs build_frame**: a simple-mode branch — skip the desktop layer and the
  per-window decorated render; render only the focused window's content buffer at the work
  area (no `render_window` chrome); still push menubar/dock and the launcher/tray/power/help
  overlays; emit the focused app's image placements offset to the full-screen rect; set the
  cursor within the full-screen rect. If no focused window, draw the hint.
- **App resize**: a `simple_content_rect()` helper (`Rect{0,1,w,h-2}`). When entering simple
  mode, on focus change in simple mode, and on terminal resize while simple, resize the
  focused app to it; when leaving simple mode, re-sync all app windows to their window
  content rects. Reuse the existing `sync_app_size`/apphost `resize` path; guard so we don't
  resize every tick (only on transitions).
- **Dock click in simple mode**: focuses + raises the window (existing dock behaviour) and
  then resizes the now-focused app to the full-screen rect.

No changes to `apphost`, `proto`, `RemoteAppHost`, the client, or the wire protocol.

## Testing

- Unit (powermenu-style, pure): `menubar_mode_region` width/position; `render_menubar` shows
  `⊞` in desktop mode and `▦` in simple mode.
- Session: clicking `menubar_mode_region()` toggles `simple_mode()`; in simple mode
  `build_frame` emits the focused window full-screen (a layer at origin (0,1) sized to the
  work area) and no desktop-icon layer; empty simple mode emits the hint; clicking a dock
  pill in simple mode changes the focused window.
- Manual: launch 2+ apps, toggle to simple (focused app fills screen, decorations gone),
  click dock pills to switch (each fills screen at the right size), toggle back (windows
  restored in place), Go/launcher and tuiui menu still work in simple mode.

## Out of scope (future)

Mouse event passthrough into PTY apps; a keyboard shortcut to cycle apps in simple mode
(dock click + Go suffice for now); remembering the mode across reload (defaults to desktop).
