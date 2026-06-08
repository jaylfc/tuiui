# Native gpm Support — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give tuiui a mouse on a bare Linux console by speaking the `gpm` socket protocol
directly (no `libgpm` link → MIT-clean), feeding events through the same client mouse pipeline.
Linux-console-only; a no-op everywhere else; cross-platform-compilable.

**Architecture:** Extract the client's per-event mouse routing into a shared `route_mouse`.
Add `src/gpm.rs`: pure event parsing/mapping (unit-tested on any OS) + a Linux-only reader
thread that connects to `/dev/gpmctl`, parses `Gpm_Event` records, and calls `route_mouse`.

**Tech Stack:** Rust 2021, `libc` (already a dep), `std::os::unix::net::UnixStream`,
`crate::mouse` types, `crossterm`.

**Reference:** Spec `docs/superpowers/specs/2026-06-08-native-gpm-design.md`. Verified gpm ABI:
`Gpm_Connect` 16 bytes (`eventMask:u16, defaultMask:u16, minMod:u16, maxMod:u16, pid:i32, vc:i32`);
`Gpm_Event` 28 bytes (`buttons:u8@0, modifiers:u8@1, vc:u16@2, dx:i16@4, dy:i16@6, x:i16@8,
y:i16@10, type:i32@12, clicks:i32@16, margin:i32@20, wdx:i16@24, wdy:i16@26`). type bits
MOVE=1/DRAG=2/DOWN=4/UP=8; buttons RIGHT=1/MIDDLE=2/LEFT=4; modifiers shift=1<<0/ctrl=1<<2/alt=1<<3.

---

## Task 1: Extract `route_mouse` in the client (behavior-preserving)

**Files:**
- Modify: `src/client.rs`

- [ ] **Step 1: Add the shared router**

The current `Event::Mouse(me)` arm does: build `MouseInput` (in-app) or chrome variants. Move
that decision into a free function. Add near the bottom of `src/client.rs` (next to
`to_mouse_input`/`send`):

```rust
/// Route one mouse event: into the focused app (passthrough) when the pointer is
/// in `f.app_area`, otherwise via the existing chrome/WM variants. Shared by the
/// crossterm path and the gpm reader so both behave identically.
pub(crate) fn route_mouse(
    out: &mut UnixStream,
    f: &Flags,
    ev: crate::mouse::MouseInput,
    last_click: &mut Option<(Point, std::time::Instant)>,
) -> std::io::Result<()> {
    use crate::mouse::{MouseAction as A, MouseButton as B};
    let p = Point::new(ev.col, ev.row);
    if f.app_area.map(|r| r.contains(p)).unwrap_or(false) {
        return send(out, &ClientMsg::MouseInput(ev));
    }
    match (ev.button, ev.action) {
        (B::Left, A::Down) => {
            let now = std::time::Instant::now();
            let dbl = last_click
                .map(|(lp, lt)| lp == p && now.duration_since(lt) < std::time::Duration::from_millis(400))
                .unwrap_or(false);
            if dbl {
                send(out, &ClientMsg::MouseDouble(p))?;
                *last_click = None;
            } else {
                send(out, &ClientMsg::MouseDown(p))?;
                *last_click = Some((p, now));
            }
        }
        (B::Right, A::Down) => send(out, &ClientMsg::MouseRightDown(p))?,
        (B::Left, A::Drag) => send(out, &ClientMsg::MouseDrag(p))?,
        (B::Left, A::Up) => send(out, &ClientMsg::MouseUp(p))?,
        (_, A::Move) => send(out, &ClientMsg::MouseDrag(p))?,
        (_, A::ScrollUp) if f.store_focused => send(out, &ClientMsg::StoreUp)?,
        (_, A::ScrollDown) if f.store_focused => send(out, &ClientMsg::StoreDown)?,
        (_, A::ScrollUp) if f.filemanager_focused => send(out, &ClientMsg::FileManagerUp)?,
        (_, A::ScrollDown) if f.filemanager_focused => send(out, &ClientMsg::FileManagerDown)?,
        _ => {}
    }
    Ok(())
}
```

Ensure `Flags` and `Point` are imported in `client.rs` (they are — `use crate::protocol::{Flags, FrameMsg};` and `use crate::geometry::{Point, SnapZone};`).

- [ ] **Step 2: Use it from the crossterm arm**

Replace the body of the `Event::Mouse(me)` arm with:

```rust
                Event::Mouse(me) => {
                    let p = Point::new(me.column as i32, me.row as i32);
                    let mods = crate::mouse::MouseMods {
                        shift: me.modifiers.contains(KeyModifiers::SHIFT),
                        ctrl: me.modifiers.contains(KeyModifiers::CONTROL),
                        alt: me.modifiers.contains(KeyModifiers::ALT),
                    };
                    if let Some(ev) = to_mouse_input(&me, p, mods) {
                        route_mouse(&mut out_stream, &f, ev, &mut last_click)?;
                    }
                }
```

This is behaviour-identical: `route_mouse` reproduces the old in-app/chrome branching exactly.

- [ ] **Step 3: Build + suite + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -10
cargo test 2>&1 | grep -E "test result|FAILED" | tail -10   # all pass
cargo clippy --all-targets 2>&1 | tail -10                  # zero warnings
git add src/client.rs
git commit --no-verify -m "client: extract shared route_mouse (prep for gpm reader)"
```

---

## Task 2: `src/gpm.rs` — pure parse/map + Linux reader

**Files:**
- Create: `src/gpm.rs`
- Modify: `src/lib.rs` (`pub mod gpm;`)
- Modify: `src/client.rs` (call `gpm::start` in `run`)

- [ ] **Step 1: Pure types + parse + map + connect-encode**

Create `src/gpm.rs`:

```rust
//! Native `gpm` (General Purpose Mouse) client for the bare Linux console.
//!
//! Speaks the `/dev/gpmctl` socket protocol directly — no `libgpm` linkage, so it
//! does not affect tuiui's MIT licensing. Linux-console-only; a no-op elsewhere.

use crate::mouse::{MouseAction, MouseButton, MouseInput, MouseMods};

// gpm event-type bits.
const GPM_MOVE: i32 = 1;
const GPM_DRAG: i32 = 2;
const GPM_DOWN: i32 = 4;
const GPM_UP: i32 = 8;
// gpm button bits.
const GPM_B_RIGHT: u8 = 1;
const GPM_B_MIDDLE: u8 = 2;
const GPM_B_LEFT: u8 = 4;

/// On-wire size of `Gpm_Event` (4-byte aligned).
pub const GPM_EVENT_SIZE: usize = 28;
/// On-wire size of `Gpm_Connect`.
pub const GPM_CONNECT_SIZE: usize = 16;

/// The subset of a `Gpm_Event` we use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpmEvent {
    pub buttons: u8,
    pub modifiers: u8,
    pub x: i16,
    pub y: i16,
    pub etype: i32,
    pub wdy: i16,
}

/// Parse a raw 28-byte `Gpm_Event` (little-endian). `None` if too short.
pub fn parse_event(b: &[u8]) -> Option<GpmEvent> {
    if b.len() < GPM_EVENT_SIZE {
        return None;
    }
    let i16le = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
    let i32le = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    Some(GpmEvent {
        buttons: b[0],
        modifiers: b[1],
        x: i16le(8),
        y: i16le(10),
        etype: i32le(12),
        wdy: i16le(26),
    })
}

/// Build the 16-byte `Gpm_Connect` record requesting all click/move events.
pub fn encode_connect(pid: i32, vc: i32) -> [u8; GPM_CONNECT_SIZE] {
    let event_mask: u16 = (GPM_MOVE | GPM_DRAG | GPM_DOWN | GPM_UP) as u16; // 0x0F
    let default_mask: u16 = 0;
    let min_mod: u16 = 0;
    let max_mod: u16 = 0xFFFF;
    let mut out = [0u8; GPM_CONNECT_SIZE];
    out[0..2].copy_from_slice(&event_mask.to_le_bytes());
    out[2..4].copy_from_slice(&default_mask.to_le_bytes());
    out[4..6].copy_from_slice(&min_mod.to_le_bytes());
    out[6..8].copy_from_slice(&max_mod.to_le_bytes());
    out[8..12].copy_from_slice(&pid.to_le_bytes());
    out[12..16].copy_from_slice(&vc.to_le_bytes());
    out
}

fn mods_of(m: u8) -> MouseMods {
    MouseMods { shift: m & (1 << 0) != 0, ctrl: m & (1 << 2) != 0, alt: m & (1 << 3) != 0 }
}

fn button_of(bits: u8) -> MouseButton {
    if bits & GPM_B_LEFT != 0 {
        MouseButton::Left
    } else if bits & GPM_B_MIDDLE != 0 {
        MouseButton::Middle
    } else if bits & GPM_B_RIGHT != 0 {
        MouseButton::Right
    } else {
        MouseButton::None
    }
}

/// Map a `GpmEvent` (with the previous button mask) to a `MouseInput`, or `None`
/// if it isn't a forwardable event. Coordinates convert 1-based → 0-based cells.
pub fn to_mouse_input(prev: u8, ev: &GpmEvent) -> Option<MouseInput> {
    let col = (ev.x as i32 - 1).max(0);
    let row = (ev.y as i32 - 1).max(0);
    let mods = mods_of(ev.modifiers);

    // Wheel takes priority (some gpm builds set type=DOWN for wheel too).
    if ev.wdy != 0 {
        let action = if ev.wdy > 0 { MouseAction::ScrollUp } else { MouseAction::ScrollDown };
        return Some(MouseInput { col, row, button: MouseButton::None, action, mods });
    }

    let (button, action) = if ev.etype & GPM_DOWN != 0 {
        (button_of(ev.buttons & !prev).max_or(ev.buttons), MouseAction::Down)
    } else if ev.etype & GPM_UP != 0 {
        (button_of(prev & !ev.buttons).max_or(prev), MouseAction::Up)
    } else if ev.etype & GPM_DRAG != 0 {
        (button_of(ev.buttons), MouseAction::Drag)
    } else if ev.etype & GPM_MOVE != 0 {
        (MouseButton::None, MouseAction::Move)
    } else {
        return None;
    };
    Some(MouseInput { col, row, button, action, mods })
}

// Helper: prefer the changed-bit button, else fall back to the raw mask.
trait MaxOr { fn max_or(self, raw: u8) -> MouseButton; }
impl MaxOr for MouseButton {
    fn max_or(self, raw: u8) -> MouseButton {
        if matches!(self, MouseButton::None) { button_of(raw) } else { self }
    }
}
```

NOTE: the `MaxOr` trait is a tiny readability helper; if clippy dislikes it, inline the
fallback (`let b = button_of(ev.buttons & !prev); let b = if matches!(b, MouseButton::None)
{ button_of(ev.buttons) } else { b };`). Keep behaviour: on DOWN use the newly-set bit (else
any set bit); on UP use the cleared bit (else any previously-set bit).

- [ ] **Step 2: Linux reader + non-Linux stub**

Append to `src/gpm.rs`:

```rust
/// Start the gpm reader if we're on a Linux console and gpm is reachable.
/// No-op on other platforms or when not on a VT (unless `TUIUI_GPM=1`).
#[cfg(target_os = "linux")]
pub fn start(flags: std::sync::Arc<std::sync::Mutex<crate::protocol::Flags>>, out: std::os::unix::net::UnixStream) {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let force = std::env::var("TUIUI_GPM").ok();
    if force.as_deref() == Some("0") {
        return;
    }
    let vc = match detect_vc() {
        Some(vc) => vc,
        None => {
            if force.as_deref() != Some("1") {
                return; // not on a VT
            }
            0
        }
    };
    let mut sock = match UnixStream::connect("/dev/gpmctl") {
        Ok(s) => s,
        Err(e) => {
            crate::dbg_log(&format!("gpm: /dev/gpmctl connect failed: {e}"));
            return;
        }
    };
    let pid = unsafe { libc::getpid() };
    if sock.write_all(&encode_connect(pid, vc)).is_err() {
        crate::dbg_log("gpm: connect write failed");
        return;
    }
    crate::dbg_log(&format!("gpm: connected (vc={vc}, pid={pid})"));

    std::thread::spawn(move || {
        let mut out = out;
        let mut last_click: Option<(crate::geometry::Point, std::time::Instant)> = None;
        let mut prev_buttons: u8 = 0;
        let mut buf = [0u8; GPM_EVENT_SIZE];
        loop {
            if sock.read_exact(&mut buf).is_err() {
                crate::dbg_log("gpm: reader ended");
                break;
            }
            let Some(ev) = parse_event(&buf) else { continue };
            if let Some(input) = to_mouse_input(prev_buttons, &ev) {
                let f = *flags.lock().unwrap();
                if crate::client::route_mouse(&mut out, &f, input, &mut last_click).is_err() {
                    break; // daemon socket gone
                }
            }
            prev_buttons = ev.buttons;
        }
    });
}

#[cfg(not(target_os = "linux"))]
pub fn start(_flags: std::sync::Arc<std::sync::Mutex<crate::protocol::Flags>>, _out: std::os::unix::net::UnixStream) {}

/// Determine our virtual-console number, or `None` if stdin isn't a Linux VT.
#[cfg(target_os = "linux")]
fn detect_vc() -> Option<i32> {
    const KDGKBTYPE: libc::c_ulong = 0x4B33;
    const VT_GETSTATE: libc::c_ulong = 0x5603;
    #[repr(C)]
    struct VtStat { v_active: libc::c_ushort, v_signal: libc::c_ushort, v_state: libc::c_ushort }

    // Must be a console keyboard (KDGKBTYPE succeeds on a VT).
    let mut kbtype: u8 = 0;
    if unsafe { libc::ioctl(0, KDGKBTYPE, &mut kbtype as *mut u8) } != 0 {
        return None;
    }
    // VC number from the device minor (ttyN -> N); minor 0 (tty0/current) -> VT_GETSTATE.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(0, &mut st) } != 0 {
        return None;
    }
    let rdev = st.st_rdev as u64;
    let major = (rdev >> 8) & 0xff;
    let minor = (rdev & 0xff) as i32;
    if major != 4 {
        return None; // not a console TTY
    }
    if minor > 0 {
        return Some(minor);
    }
    let mut vs: VtStat = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(0, VT_GETSTATE, &mut vs as *mut VtStat) } == 0 {
        Some(vs.v_active as i32)
    } else {
        None
    }
}
```

- [ ] **Step 3: Register + wire into the client**

`src/lib.rs`: add `pub mod gpm;`.

In `src/client.rs::run`, after the `flags`/`out_stream` are set up (and after the socket reader
thread is spawned), start gpm with a clone of the shared flags + a stream clone:

```rust
    // On a bare Linux console, gpm provides the mouse (the VT emits no xterm
    // mouse sequences). No-op on GUI terminals / non-Linux.
    crate::gpm::start(flags.clone(), out_stream.try_clone()?);
```

Place it after `let flags = Arc::new(Mutex::new(Flags::default()));` and the reader-thread block
(so `flags` exists and is the same `Arc` the reader updates). `out_stream` already exists.

- [ ] **Step 4: Pure unit tests**

Add to `src/gpm.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mouse::{MouseAction, MouseButton};

    fn raw(buttons: u8, modifiers: u8, x: i16, y: i16, etype: i32, wdy: i16) -> [u8; GPM_EVENT_SIZE] {
        let mut b = [0u8; GPM_EVENT_SIZE];
        b[0] = buttons;
        b[1] = modifiers;
        b[8..10].copy_from_slice(&x.to_le_bytes());
        b[10..12].copy_from_slice(&y.to_le_bytes());
        b[12..16].copy_from_slice(&etype.to_le_bytes());
        b[26..28].copy_from_slice(&wdy.to_le_bytes());
        b
    }

    #[test]
    fn parse_reads_fields_by_offset() {
        let ev = parse_event(&raw(GPM_B_LEFT, 0, 10, 5, GPM_DOWN, 0)).unwrap();
        assert_eq!(ev.buttons, GPM_B_LEFT);
        assert_eq!((ev.x, ev.y), (10, 5));
        assert_eq!(ev.etype, GPM_DOWN);
    }

    #[test]
    fn parse_rejects_short() {
        assert!(parse_event(&[0u8; 10]).is_none());
    }

    #[test]
    fn left_down_then_up_maps_buttons_and_1based_coords() {
        let down = parse_event(&raw(GPM_B_LEFT, 0, 3, 2, GPM_DOWN, 0)).unwrap();
        let mi = to_mouse_input(0, &down).unwrap();
        assert_eq!((mi.col, mi.row), (2, 1)); // 1-based 3,2 -> 0-based 2,1
        assert_eq!(mi.button, MouseButton::Left);
        assert_eq!(mi.action, MouseAction::Down);
        // release: prev had LEFT, now 0 -> released LEFT
        let up = parse_event(&raw(0, 0, 3, 2, GPM_UP, 0)).unwrap();
        let mu = to_mouse_input(GPM_B_LEFT, &up).unwrap();
        assert_eq!(mu.button, MouseButton::Left);
        assert_eq!(mu.action, MouseAction::Up);
    }

    #[test]
    fn right_and_middle_and_drag() {
        let r = to_mouse_input(0, &parse_event(&raw(GPM_B_RIGHT, 0, 1, 1, GPM_DOWN, 0)).unwrap()).unwrap();
        assert_eq!(r.button, MouseButton::Right);
        let m = to_mouse_input(0, &parse_event(&raw(GPM_B_MIDDLE, 0, 1, 1, GPM_DOWN, 0)).unwrap()).unwrap();
        assert_eq!(m.button, MouseButton::Middle);
        let d = to_mouse_input(GPM_B_LEFT, &parse_event(&raw(GPM_B_LEFT, 0, 1, 1, GPM_DRAG, 0)).unwrap()).unwrap();
        assert_eq!((d.button, d.action), (MouseButton::Left, MouseAction::Drag));
    }

    #[test]
    fn wheel_maps_to_scroll() {
        let up = to_mouse_input(0, &parse_event(&raw(0, 0, 1, 1, GPM_MOVE, 1)).unwrap()).unwrap();
        assert_eq!(up.action, MouseAction::ScrollUp);
        let dn = to_mouse_input(0, &parse_event(&raw(0, 0, 1, 1, GPM_MOVE, -1)).unwrap()).unwrap();
        assert_eq!(dn.action, MouseAction::ScrollDown);
    }

    #[test]
    fn modifiers_decode() {
        let e = to_mouse_input(0, &parse_event(&raw(GPM_B_LEFT, (1 << 0) | (1 << 2), 1, 1, GPM_DOWN, 0)).unwrap()).unwrap();
        assert!(e.mods.shift && e.mods.ctrl && !e.mods.alt);
    }

    #[test]
    fn connect_record_is_16_bytes_with_fields() {
        let c = encode_connect(1234, 2);
        assert_eq!(c.len(), 16);
        assert_eq!(u16::from_le_bytes([c[0], c[1]]), 0x0F); // event mask
        assert_eq!(i32::from_le_bytes([c[8], c[9], c[10], c[11]]), 1234);
        assert_eq!(i32::from_le_bytes([c[12], c[13], c[14], c[15]]), 2);
    }
}
```

- [ ] **Step 5: Build (both targets) + test + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test gpm:: 2>&1 | tail -20          # pure tests pass on macOS
cargo build 2>&1 | tail -10
cargo test 2>&1 | grep -E "test result|FAILED" | tail -10
cargo clippy --all-targets 2>&1 | tail -15  # zero warnings (incl. the cfg'd Linux code path)
git add src/gpm.rs src/lib.rs src/client.rs
git commit --no-verify -m "gpm: native /dev/gpmctl mouse reader for the bare Linux console (MIT-clean)"
```

NOTE: on macOS the Linux `start`/`detect_vc` are cfg'd out, so clippy won't lint them there; the
implementer should `cargo clippy` as normal (the Linux block is checked when built on Linux). Keep
the Linux code warning-free by construction (no unused imports under either cfg).

---

## Task 3: Gate + manual (user, on Debian)

- [ ] **Step 1: Full gate (macOS dev box)**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -3 && cargo test 2>&1 | grep -cE "test result: ok" && cargo clippy --all-targets 2>&1 | grep -cE "warning:|error:"
```
Build OK, tests pass, clippy 0. (This proves cross-platform compile + the pure logic; the live
gpm path is Linux-only.)

- [ ] **Step 2: Deploy note + manual (Debian console)**

This must be verified by the user on the bare-metal Debian box:
```bash
sudo apt install gpm           # and ensure the gpm service is running on the VT
# build/install tuiui on Debian (cargo install --path .), then on a bare console (Ctrl-Alt-F3):
tuiui
```
Move/click the mouse: it should drive tuiui chrome (menubar/dock), windows, and in-app
passthrough. If it doesn't, run with `TUIUI_DEBUG=1 tuiui` and check `~/tuiui-debug.log` for
`gpm: connected (vc=…)` and reader status; `TUIUI_GPM=1` forces an attempt if VC detection
misfires. Report the log lines back for iteration.

- [ ] **Step 3: Update memory**

Add to `tuiui-roadmap-state`: native gpm DONE (Linux-console mouse via `/dev/gpmctl` socket
protocol, no libgpm; pure parse/map tested; live-verified on Debian = pending user). Note macOS
can't test the live path.

---

## Self-Review Notes

- **No effect off-console:** `start` returns early on non-Linux (stub) and when `detect_vc()`
  is `None` (not a VT) — so GUI/SSH mouse is untouched. `route_mouse` is the *same* code the
  crossterm path uses, so console mouse behaves identically to GUI mouse.
- **Defensive:** every gpm syscall/read is error-checked; failures log and stop the thread
  rather than panicking. ABI structs are parsed by fixed offset with a length guard.
- **License:** no `libgpm` linkage — we only open a Unix socket and exchange byte records, so
  gpm's GPL does not reach our MIT binary.
- **Untestable on mac:** the pure parse/map/encode are fully unit-tested; the socket/ioctl path
  is Linux-only and verified on the Debian console by the user. Keep it well-logged for that.
```
