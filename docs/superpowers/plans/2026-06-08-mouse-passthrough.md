# Mouse Passthrough Into Apps — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Forward full-fidelity mouse (all buttons, drag, scroll, modifiers) into the focused
PTY app when that app has requested mouse reporting — identically in desktop and simple modes —
using the app's own encoding (SGR or legacy), without disturbing tuiui's existing chrome/WM
mouse handling.

**Architecture:** Shared mouse types + a pure encoder in `src/mouse.rs`. The apphost reports
each app's terminal mouse mode (`AppMouse`, from `alacritty Term.mode()`) on its per-app
frames. The daemon publishes the focused app's content rect as `Flags.app_area` **only when
that app wants mouse**; the thin client routes events inside `app_area` as the new rich
`ClientMsg::MouseInput` (everything else stays on today's chrome variants). The daemon
encodes a `MouseInput` and writes it to the app's PTY via `apphost.input`.

**Tech Stack:** Rust 2021, `alacritty_terminal::term::TermMode`, `crossterm` mouse events,
the apphost (`AppHost`/`LocalAppHost`/`RemoteAppHost`/`proto`), `serde`.

**Reference:** Spec `docs/superpowers/specs/2026-06-08-mouse-passthrough-design.md`.

---

## Background (verified)

- `alacritty_terminal` 0.26 `TermMode` flags: `MOUSE_REPORT_CLICK` (1<<3), `SGR_MOUSE` (1<<5),
  `MOUSE_MOTION` (1<<6), `ALT_SCREEN` (1<<12), `MOUSE_DRAG` (1<<13), `UTF8_MOUSE` (1<<14),
  `ALTERNATE_SCROLL` (1<<15), and `MOUSE_MODE = REPORT_CLICK|MOTION|DRAG`. `Term::mode() -> TermMode`,
  `.contains(flag)`. The `Term` lives in `AppInstance` (`src/ptyhost.rs`, `term: Arc<Mutex<Term<..>>>`).
- apphost `AppHost` trait (`src/apphost/api.rs`) + `LocalAppHost`/`RemoteAppHost`. `HostEvt::Frame`
  (`src/apphost/proto.rs`) is pushed per app on change; `RemoteAppHost` caches frames.
- `protocol::Flags` is `#[serde(default)]`. `ClientMsg` (in `src/session.rs`) is serde.
- Client mouse capture (`src/client.rs`): currently Left Down/Drag/Up, Right Down, Moved→Drag,
  Scroll only for Store/FM. `crossterm::event::MouseEvent { kind, column, row, modifiers }`.
- `daemon.rs::serve_client` builds `Flags { ... }` each frame and forwards `ClientMsg` to
  `core.apply`. `SessionCore::apply` handles each `ClientMsg`.

---

## Task 1: `src/mouse.rs` — shared types + pure encoder

**Files:**
- Create: `src/mouse.rs`
- Modify: `src/lib.rs` (`pub mod mouse;`)

- [ ] **Step 1: Types + encoder**

```rust
//! Shared mouse types and the encoder that turns a frontend mouse event into the
//! byte sequence a PTY app expects, honouring the app's reported mouse mode.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton { Left, Middle, Right, None }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction { Down, Up, Drag, Move, ScrollUp, ScrollDown, ScrollLeft, ScrollRight }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MouseMods { pub shift: bool, pub ctrl: bool, pub alt: bool }

/// A mouse event from the client. `col`/`row` are screen cells (0-based) on the
/// wire; the daemon localises them (subtracts the app area origin) before encoding.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseInput {
    pub col: i32,
    pub row: i32,
    pub button: MouseButton,
    pub action: MouseAction,
    pub mods: MouseMods,
}

/// The focused app's terminal mouse mode, derived from `alacritty Term.mode()`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AppMouse {
    pub report_click: bool,
    pub report_drag: bool,
    pub report_motion: bool,
    pub sgr: bool,
    pub utf8: bool,
    pub alternate_scroll: bool,
    pub alt_screen: bool,
}

impl AppMouse {
    /// Whether the app wants click/drag/motion mouse reporting.
    pub fn wants_mouse(&self) -> bool {
        self.report_click || self.report_drag || self.report_motion
    }
    /// Whether the frontend should treat the pointer over this app as "in-app"
    /// (forwardable): true if it wants mouse, or wheel-to-arrows applies.
    pub fn captures_pointer(&self) -> bool {
        self.wants_mouse() || (self.alternate_scroll && self.alt_screen)
    }
}

const ESC: u8 = 0x1b;

/// Encode `ev` (with **0-based, app-local** col/row) for an app in mouse mode
/// `m`. Returns the PTY bytes, or `None` if this event should not be forwarded.
pub fn encode(ev: &MouseInput, m: &AppMouse) -> Option<Vec<u8>> {
    use MouseAction::*;

    // Alternate-scroll: wheel becomes arrow keys when the app is on the alt
    // screen with alternate-scroll set and is NOT in mouse-reporting mode.
    let scroll = matches!(ev.action, ScrollUp | ScrollDown | ScrollLeft | ScrollRight);
    if scroll && !m.wants_mouse() {
        if m.alternate_scroll && m.alt_screen {
            let arrow: &[u8] = match ev.action {
                ScrollUp => b"\x1b[A",
                ScrollDown => b"\x1b[B",
                ScrollLeft => b"\x1b[D",
                ScrollRight => b"\x1b[C",
                _ => unreachable!(),
            };
            return Some(arrow.to_vec());
        }
        return None;
    }
    if !m.wants_mouse() {
        return None;
    }

    // Gate by reporting level.
    match ev.action {
        Move if !m.report_motion => return None,
        Drag if !(m.report_drag || m.report_motion) => return None,
        _ => {}
    }

    // Base button code.
    let mut cb: u32 = match ev.action {
        ScrollUp => 64,
        ScrollDown => 65,
        ScrollLeft => 66,
        ScrollRight => 67,
        _ => match ev.button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::None => 3,
        },
    };
    // Motion bit for drag/move.
    if matches!(ev.action, Drag | Move) {
        cb += 32;
    }
    // Modifiers.
    if ev.mods.shift { cb += 4; }
    if ev.mods.alt { cb += 8; }
    if ev.mods.ctrl { cb += 16; }

    // 1-based coordinates.
    let col = ev.col.max(0) as u32 + 1;
    let row = ev.row.max(0) as u32 + 1;

    if m.sgr {
        // ESC [ < Cb ; col ; row (M=press/motion/scroll, m=release)
        let final_ch = if matches!(ev.action, Up) { b'm' } else { b'M' };
        let mut out = Vec::new();
        out.push(ESC);
        out.extend_from_slice(b"[<");
        out.extend_from_slice(cb.to_string().as_bytes());
        out.push(b';');
        out.extend_from_slice(col.to_string().as_bytes());
        out.push(b';');
        out.extend_from_slice(row.to_string().as_bytes());
        out.push(final_ch);
        Some(out)
    } else {
        // Legacy X10: ESC [ M (32+Cb) (32+col) (32+row), release => button 3.
        let cb_legacy = if matches!(ev.action, Up) {
            // keep motion/mod bits but force button bits to release (3)
            (cb & !0b11) + 3
        } else {
            cb
        };
        let enc = |v: u32| -> u8 { (32 + v.min(223 - 32 + 32 - 1).min(223)) as u8 };
        // Clamp coords to the 223 ceiling (1..=223), then +32.
        let cx = (col.min(223) + 32) as u8;
        let cy = (row.min(223) + 32) as u8;
        let cbb = (cb_legacy.min(223) + 32) as u8;
        let _ = enc; // (helper kept for clarity; cx/cy/cbb computed inline)
        Some(vec![ESC, b'[', b'M', cbb, cx, cy])
    }
}
```

NOTE: simplify the legacy `enc` closure away — the inline `cx/cy/cbb` are what's used; remove
the unused closure to keep clippy clean. The legacy clamp keeps each byte ≤ `32+223`.

- [ ] **Step 2: Register module**

`src/lib.rs`: add `pub mod mouse;`.

- [ ] **Step 3: Tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ev(col: i32, row: i32, button: MouseButton, action: MouseAction) -> MouseInput {
        MouseInput { col, row, button, action, mods: MouseMods::default() }
    }
    fn sgr() -> AppMouse { AppMouse { report_click: true, report_drag: true, report_motion: false, sgr: true, utf8: false, alternate_scroll: false, alt_screen: false } }
    fn legacy() -> AppMouse { AppMouse { report_click: true, report_drag: false, report_motion: false, sgr: false, utf8: false, alternate_scroll: false, alt_screen: false } }

    #[test]
    fn sgr_left_press_release() {
        // app-local (0,0) -> 1;1
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Down), &sgr()).unwrap(), b"\x1b[<0;1;1M");
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Up), &sgr()).unwrap(), b"\x1b[<0;1;1m");
    }

    #[test]
    fn sgr_right_and_middle_and_mods() {
        assert_eq!(encode(&ev(2, 0, MouseButton::Right, MouseAction::Down), &sgr()).unwrap(), b"\x1b[<2;3;1M");
        let mut e = ev(0, 0, MouseButton::Middle, MouseAction::Down);
        e.mods = MouseMods { shift: false, ctrl: true, alt: false };
        assert_eq!(encode(&e, &sgr()).unwrap(), b"\x1b[<17;1;1M"); // 1 + ctrl(16)
    }

    #[test]
    fn sgr_scroll() {
        assert_eq!(encode(&ev(0, 0, MouseButton::None, MouseAction::ScrollUp), &sgr()).unwrap(), b"\x1b[<64;1;1M");
        assert_eq!(encode(&ev(0, 0, MouseButton::None, MouseAction::ScrollDown), &sgr()).unwrap(), b"\x1b[<65;1;1M");
    }

    #[test]
    fn drag_gated_by_report_drag() {
        let mut m = sgr(); m.report_drag = false;
        assert!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Drag), &m).is_none());
        let m2 = sgr(); // report_drag true
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Drag), &m2).unwrap(), b"\x1b[<32;1;1M");
    }

    #[test]
    fn legacy_encoding_and_clamp() {
        // (0,0): ESC [ M  32+0  32+1  32+1
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Down), &legacy()).unwrap(), vec![0x1b, b'[', b'M', 32, 33, 33]);
        // release => button 3
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Up), &legacy()).unwrap(), vec![0x1b, b'[', b'M', 35, 33, 33]);
    }

    #[test]
    fn alternate_scroll_to_arrows() {
        let m = AppMouse { report_click: false, report_drag: false, report_motion: false, sgr: false, utf8: false, alternate_scroll: true, alt_screen: true };
        assert_eq!(encode(&ev(5, 5, MouseButton::None, MouseAction::ScrollUp), &m).unwrap(), b"\x1b[A");
        assert_eq!(encode(&ev(5, 5, MouseButton::None, MouseAction::ScrollDown), &m).unwrap(), b"\x1b[B");
        // a click in such an app is not forwarded
        assert!(encode(&ev(5, 5, MouseButton::Left, MouseAction::Down), &m).is_none());
    }

    #[test]
    fn no_mouse_means_no_bytes() {
        let m = AppMouse::default();
        assert!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Down), &m).is_none());
    }
}
```

- [ ] **Step 4: Test + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test mouse:: 2>&1 | tail -20         # all encoder tests pass
cargo clippy --all-targets 2>&1 | tail -10 # zero warnings
git add src/mouse.rs src/lib.rs
git commit --no-verify -m "mouse: shared mouse types + pure PTY mouse-event encoder (SGR/legacy)"
```

---

## Task 2: apphost reports each app's mouse mode

**Files:**
- Modify: `src/ptyhost.rs` (`AppInstance::mouse_mode`)
- Modify: `src/apphost/api.rs` (`AppHost::mouse_mode`)
- Modify: `src/apphost/host.rs` (`LocalAppHost`)
- Modify: `src/apphost/proto.rs` (`HostEvt::Frame.mouse`)
- Modify: `src/apphost/server.rs` (send `mouse`; re-push frame when it changes)
- Modify: `src/apphost/remote.rs` (cache + `mouse_mode`)

- [ ] **Step 1: `AppInstance::mouse_mode`**

In `src/ptyhost.rs`, add (reads the emulator mode under the lock):

```rust
    /// The app's current terminal mouse mode (what it asked the terminal for).
    pub fn mouse_mode(&self) -> crate::mouse::AppMouse {
        use alacritty_terminal::term::TermMode;
        let mode = self.term.lock().unwrap().mode();
        crate::mouse::AppMouse {
            report_click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
            report_drag: mode.contains(TermMode::MOUSE_DRAG),
            report_motion: mode.contains(TermMode::MOUSE_MOTION),
            sgr: mode.contains(TermMode::SGR_MOUSE),
            utf8: mode.contains(TermMode::UTF8_MOUSE),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
        }
    }
```

Confirm the field is `self.term` and the lock type matches existing usage (the file already
locks `term`). `Term::mode()` returns `TermMode`.

- [ ] **Step 2: `AppHost::mouse_mode` + `LocalAppHost`**

`src/apphost/api.rs` — add to the trait:
```rust
    /// The app's current terminal mouse mode (default = no mouse).
    fn mouse_mode(&self, id: AppId) -> crate::mouse::AppMouse { let _ = id; crate::mouse::AppMouse::default() }
```
(Default keeps it non-breaking; Local/Remote override.)

`src/apphost/host.rs` — in `impl AppHost for LocalAppHost`:
```rust
    fn mouse_mode(&self, id: AppId) -> crate::mouse::AppMouse {
        self.apps.get(&id).map(|a| a.mouse_mode()).unwrap_or_default()
    }
```

- [ ] **Step 3: proto carries it**

`src/apphost/proto.rs` — add `mouse: crate::mouse::AppMouse` to `HostEvt::Frame`:
```rust
    Frame {
        app: u64,
        grid: CellBuffer,
        placements: Vec<Placement>,
        images: Vec<ImgBlob>,
        alive: bool,
        mouse: crate::mouse::AppMouse,
    },
```
Update the `frame_round_trips_with_grid` test to set `mouse: Default::default()`.

- [ ] **Step 4: server sends mouse + re-pushes on change**

`src/apphost/server.rs` `serve_frontend`: track `last_mouse: HashMap<AppId, crate::mouse::AppMouse>`.
When building a frame, read `let mouse = local.mouse_mode(id);`, include it in the `HostEvt::Frame`,
and add `|| last_mouse.get(&id) != Some(&mouse)` to the change condition so a mouse-mode change
(e.g. vim enabling mouse) forces a frame even if the grid is unchanged; update `last_mouse` when
sent and clear it on `Gone`.

- [ ] **Step 5: RemoteAppHost caches + serves it**

`src/apphost/remote.rs`: add `mouse: crate::mouse::AppMouse` to `Cached`; in `apply_evt`'s
`HostEvt::Frame` arm set `entry.mouse = mouse;` (destructure the new field). Add:
```rust
    fn mouse_mode(&self, id: AppId) -> crate::mouse::AppMouse {
        self.cache.lock().unwrap().apps.get(&id).map(|c| c.mouse).unwrap_or_default()
    }
```
to `impl AppHost for RemoteAppHost`. Update the `HostEvt::Frame { .. }` match in `apply_evt`
to bind `mouse`.

- [ ] **Step 6: Build + tests + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test apphost:: 2>&1 | tail -20     # proto + loopback still pass
cargo build 2>&1 | tail -10
cargo clippy --all-targets 2>&1 | tail -10
git add src/ptyhost.rs src/apphost/
git commit --no-verify -m "apphost: report each app's terminal mouse mode (AppMouse) on frames"
```

---

## Task 3: protocol — `ClientMsg::MouseInput` + `Flags.app_area`

**Files:**
- Modify: `src/protocol.rs` (`Flags.app_area`)
- Modify: `src/session.rs` (`ClientMsg::MouseInput`)

- [ ] **Step 1: `Flags.app_area`**

In `src/protocol.rs` `struct Flags` (it is `#[serde(default)]`), add:
```rust
    /// The focused app's content rect, set only when that app wants mouse. The
    /// client routes events inside it as `ClientMsg::MouseInput` (passthrough);
    /// `None` keeps all mouse on the normal chrome/WM path.
    pub app_area: Option<Rect>,
```
(`Rect` is already imported and `serde`.)

- [ ] **Step 2: `ClientMsg::MouseInput`**

In `src/session.rs` add to `ClientMsg`:
```rust
    /// A raw mouse event destined for the focused app's PTY (passthrough).
    MouseInput(crate::mouse::MouseInput),
```
Add the arm in `apply` (full handling comes in Task 4 — for now a stub so it compiles):
```rust
            ClientMsg::MouseInput(m) => self.forward_mouse_to_app(m),
```
and a temporary empty `fn forward_mouse_to_app(&mut self, _m: crate::mouse::MouseInput) {}`
(replaced in Task 4). Ensure the top-of-`apply` `matches!` mouse-logging filter includes
`ClientMsg::MouseInput(_)` so it isn't spammed to the debug log (add it to that filter list).

- [ ] **Step 3: Build + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -10
cargo test 2>&1 | grep -E "test result|FAILED" | tail -5
cargo clippy --all-targets 2>&1 | tail -10
git add src/protocol.rs src/session.rs
git commit --no-verify -m "protocol: ClientMsg::MouseInput + Flags.app_area (passthrough plumbing)"
```

---

## Task 4: session — publish `app_area` + forward mouse to the PTY

**Files:**
- Modify: `src/session.rs`
- Modify: `src/daemon.rs` (set `Flags.app_area`)

- [ ] **Step 1: Compute the focused app's mouse area**

Add to `impl SessionCore`:
```rust
    /// The focused app's content rect IFF that app currently captures the
    /// pointer (wants mouse, or alt-scroll). Used by the client to route
    /// in-app mouse events. `None` → all mouse stays on the chrome/WM path.
    pub fn app_mouse_area(&self) -> Option<crate::geometry::Rect> {
        let fid = self.wm.focused()?;
        let aid = match self.contents.get(&fid)? {
            WinContent::App(aid) => *aid,
            _ => return None,
        };
        if !self.apphost.mouse_mode(aid).captures_pointer() {
            return None;
        }
        if self.simple {
            Some(self.simple_content_rect())
        } else {
            // Skip if the window is minimized or fully obstructed.
            let w = self.wm.get(fid)?;
            if w.minimized { return None; }
            Some(w.content_rect())
        }
    }
```

- [ ] **Step 2: Real `forward_mouse_to_app`**

Replace the Task-3 stub:
```rust
    fn forward_mouse_to_app(&mut self, m: crate::mouse::MouseInput) {
        let Some(area) = self.app_mouse_area() else { return };
        if m.col < area.x || m.col >= area.x + area.w || m.row < area.y || m.row >= area.y + area.h {
            return;
        }
        let Some(fid) = self.wm.focused() else { return };
        let aid = match self.contents.get(&fid) { Some(WinContent::App(aid)) => *aid, _ => return };
        let mode = self.apphost.mouse_mode(aid);
        let local = crate::mouse::MouseInput { col: m.col - area.x, row: m.row - area.y, ..m };
        if let Some(bytes) = crate::mouse::encode(&local, &mode) {
            self.apphost.input(aid, &bytes);
        }
    }
```

- [ ] **Step 3: Publish `app_area` from the daemon**

In `src/daemon.rs::serve_client`, add to the `Flags { ... }` construction:
```rust
            app_area: core.app_mouse_area(),
```

- [ ] **Step 4: Test**

Add a session test (uses `LocalAppHost`; spawns an app that turns on SGR mouse, then asserts
`app_mouse_area()` becomes `Some` and a forwarded click reaches the app). Because asserting PTY
bytes is racy, assert the simpler observable: `app_mouse_area()` is `None` for a plain `sh`
(no mouse) and the `MouseInput` arm is a no-op then. Add a focused deterministic test of the
area gating using a fake/echo if practical; otherwise rely on the encoder's own tests plus:

```rust
#[test]
fn app_mouse_area_none_without_mouse_mode() {
    use tuiui::session::{SessionCore, ClientMsg};
    use tuiui::config::Config;
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    // A bare shell hasn't enabled mouse reporting → no app area, passthrough off.
    assert!(core.app_mouse_area().is_none());
    core.shutdown();
}
```

- [ ] **Step 5: Build + suite + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -10
cargo test 2>&1 | grep -E "test result|FAILED" | tail -20
cargo clippy --all-targets 2>&1 | tail -10
git add src/session.rs src/daemon.rs tests/session_tests.rs
git commit --no-verify -m "session: forward in-app mouse to the PTY + publish app_area flag"
```

---

## Task 5: client — full mouse capture + `app_area` gating

**Files:**
- Modify: `src/client.rs`

- [ ] **Step 1: Capture the full mouse set and gate on `app_area`**

In `src/client.rs`, replace the `Event::Mouse` handling. Build a helper that maps a crossterm
`MouseEvent` to our `MouseInput`, and route by `flags.app_area`:

```rust
                Event::Mouse(me) => {
                    let p = Point::new(me.column as i32, me.row as i32);
                    let mods = crate::mouse::MouseMods {
                        shift: me.modifiers.contains(KeyModifiers::SHIFT),
                        ctrl: me.modifiers.contains(KeyModifiers::CONTROL),
                        alt: me.modifiers.contains(KeyModifiers::ALT),
                    };
                    let in_app = f.app_area.map(|r| r.contains(p)).unwrap_or(false);
                    if in_app {
                        if let Some(input) = to_mouse_input(&me, p, mods) {
                            send(&mut out_stream, &ClientMsg::MouseInput(input))?;
                        }
                    } else {
                        // existing chrome/WM routing (unchanged) ...
                    }
                }
```

Move the CURRENT mouse-match body verbatim into the `else` branch (Left Down→double/MouseDown,
Right Down→MouseRightDown, Drag/Moved→MouseDrag, Up→MouseUp, Scroll for Store/FM). Add the
mapping helper near the bottom of `client.rs`:

```rust
fn to_mouse_input(me: &event::MouseEvent, p: Point, mods: crate::mouse::MouseMods) -> Option<crate::mouse::MouseInput> {
    use crate::mouse::{MouseAction, MouseButton, MouseInput};
    use crossterm::event::{MouseButton as XB, MouseEventKind as K};
    let (button, action) = match me.kind {
        K::Down(XB::Left) => (MouseButton::Left, MouseAction::Down),
        K::Down(XB::Middle) => (MouseButton::Middle, MouseAction::Down),
        K::Down(XB::Right) => (MouseButton::Right, MouseAction::Down),
        K::Up(XB::Left) => (MouseButton::Left, MouseAction::Up),
        K::Up(XB::Middle) => (MouseButton::Middle, MouseAction::Up),
        K::Up(XB::Right) => (MouseButton::Right, MouseAction::Up),
        K::Drag(XB::Left) => (MouseButton::Left, MouseAction::Drag),
        K::Drag(XB::Middle) => (MouseButton::Middle, MouseAction::Drag),
        K::Drag(XB::Right) => (MouseButton::Right, MouseAction::Drag),
        K::Moved => (MouseButton::None, MouseAction::Move),
        K::ScrollUp => (MouseButton::None, MouseAction::ScrollUp),
        K::ScrollDown => (MouseButton::None, MouseAction::ScrollDown),
        K::ScrollLeft => (MouseButton::None, MouseAction::ScrollLeft),
        K::ScrollRight => (MouseButton::None, MouseAction::ScrollRight),
    };
    Some(MouseInput { col: p.x, row: p.y, button, action, mods })
}
```

Ensure `Rect::contains` exists (it's used elsewhere) and `Flags` is in scope. Keep the
existing double-click detection in the `else` branch only (chrome).

- [ ] **Step 2: Build + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -10
cargo test 2>&1 | grep -E "test result|FAILED" | tail -10
cargo clippy --all-targets 2>&1 | tail -10
git add src/client.rs
git commit --no-verify -m "client: full mouse capture; route in-app events as MouseInput passthrough"
```

---

## Task 6: Gate + manual

- [ ] **Step 1: Full gate**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -3 && cargo test 2>&1 | grep -cE "test result: ok" && cargo clippy --all-targets 2>&1 | grep -cE "warning:|error:"
```

- [ ] **Step 2: Deploy + manual**

```bash
cargo install --root ~/.local --path . --force
tuiui kill; tuiui
```
- **btop**: click panels / sort columns, scroll the process list.
- **yazi**: click files, scroll.
- **vim**: `:set mouse=a` → click to move cursor, wheel to scroll, drag to visual-select.
- **less/man**: wheel scrolls (alternate-scroll → arrows).
- Confirm in BOTH modes (desktop: also that the window **title-bar drag still works** for a
  mouse-app — those cells are outside `app_area`; simple: full-screen app forwards everywhere).
- Confirm a non-mouse app (plain shell) is unaffected: clicks still do chrome/WM things.

- [ ] **Step 3: Update memory**

Add to `tuiui-roadmap-state`: mouse passthrough DONE (apphost reports `AppMouse`; `src/mouse.rs`
encoder; `Flags.app_area` gating; forwards SGR/legacy incl. scroll/drag/mods/alt-scroll; both
modes). Note native gpm is the next planned item.

---

## Self-Review Notes

- **No chrome regressions:** when `app_area` is `None` (app doesn't want mouse) or the pointer
  is outside it, the client sends the exact same messages as today → all existing routing and
  tests hold. Only in-app events change.
- **Focus interplay (desktop):** `app_area` is the *focused* app's content rect, so clicking an
  unfocused mouse-app window goes to chrome first (focuses it); the next click forwards. Title
  bar / borders are outside `app_area` → window drag/resize still work.
- **Encoder purity:** `src/mouse.rs` has no I/O; the daemon localises coords before calling it.
  The 223 legacy clamp prevents byte overflow.
- **Mouse-mode changes mid-session** (vim toggling `mouse=a`) propagate because the apphost
  re-pushes a frame when `AppMouse` changes (Task 2 Step 4), updating `Flags.app_area` within a
  frame.
