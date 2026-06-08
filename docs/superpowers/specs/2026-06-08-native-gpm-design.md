# Native gpm Support (bare Linux console mouse) — Design

**Status:** Approved (2026-06-08). Linux-console-only; MIT-clean (no libgpm linkage).

## Problem / goal

On a bare Linux virtual console (no X/Wayland, no GUI terminal), the kernel VT does not emit
xterm mouse escape sequences, so `crossterm` (and thus tuiui) gets no mouse. `gpm` provides
mouse events via its own socket, not as stdin escapes. Goal: make tuiui talk to gpm's socket
**directly** (no `libgpm` link → no GPL contamination of our MIT license), so a bare console +
the `gpm` daemon gives mouse that flows through tuiui's existing mouse pipeline (incl. the new
in-app passthrough). The bare-console experience then matches GUI-SSH **mouse-wise** (images
still can't render on a raw VT — that's unchanged and unrelated).

## Constraints / honesty

- **Linux-console-only.** On macOS / inside any GUI terminal / over SSH-from-GUI, gpm is
  irrelevant and the normal escape-sequence mouse path is used; the gpm code must be a no-op
  there and must not affect that path.
- **Cannot be live-tested on the dev Mac.** Pure parsing/mapping is unit-tested cross-platform;
  the actual gpm connection is verified by the user on the Debian console. So the implementation
  is defensive (never panics, fails silent-but-logged) and logs connection/VC/events under
  `TUIUI_DEBUG` for on-device debugging.
- **ABI reproduction.** We reproduce gpm's C structs by byte offset (no `libgpm`). Target
  x86_64/aarch64 Linux. Parse defensively (fixed struct size, parse by offset, ignore trailing).

## gpm wire facts (reproduced, not linked)

- Socket: `/dev/gpmctl` (Unix stream).
- On connect, write a `Gpm_Connect` (16 bytes): `eventMask:u16, defaultMask:u16, minMod:u16,
  maxMod:u16, pid:i32, vc:i32`. We send `eventMask = MOVE|DRAG|DOWN|UP (0x0F)`,
  `defaultMask = 0`, `minMod = 0`, `maxMod = 0xFFFF`, `pid = getpid()`, `vc = <our VC>`.
- Then read `Gpm_Event` records (28 bytes, 4-byte aligned):
  `buttons:u8@0, modifiers:u8@1, vc:u16@2, dx:i16@4, dy:i16@6, x:i16@8, y:i16@10,
   type:i32@12, clicks:i32@16, margin:i32@20, wdx:i16@24, wdy:i16@26`.
- `type` bits (`Gpm_Etype`): MOVE=1, DRAG=2, DOWN=4, UP=8 (others ignored).
- `buttons` bits: RIGHT=1, MIDDLE=2, LEFT=4.
- `modifiers` bits (Linux KG_*): shift=1<<0, ctrl=1<<2, alt=1<<3.
- `x`,`y` are 1-based character cells. Wheel: `wdy>0` → up, `wdy<0` → down (preferred);
  if `wdy==0` ignore wheel.

## VC (virtual console) detection

gpm only forwards events for the connecting client's VC. Determine it from stdin (fd 0):
- Confirm it's a Linux console: `ioctl(0, KDGKBTYPE)` succeeds (returns KB_101/KB_84) — else
  we're not on a VT → **don't start gpm** (no-op).
- VC number: `fstat(0)`; for char-major 4 with minor `N>0`, VC = `N` (`/dev/ttyN`); for minor 0
  (`/dev/tty0`/current), `ioctl(0, VT_GETSTATE, &vt_stat)` → `v_active`.
- Override: `TUIUI_GPM=0` forces gpm off; `TUIUI_GPM=1` forces an attempt even if detection is
  unsure (debug aid). Default = auto (start only when on a VT and `/dev/gpmctl` connects).

## Architecture

### `src/gpm.rs`

- **Pure (compiles + tested everywhere):**
  - `GpmEvent { buttons, modifiers, x, y, etype, wdy }` + `parse_event(&[u8]) -> Option<GpmEvent>`
    (validates length == 28, reads by offset, little-endian).
  - `to_mouse_input(prev_buttons, ev) -> Option<crate::mouse::MouseInput>`: maps a `GpmEvent`
    (with the previous button mask for press/release/drag disambiguation) to our `MouseInput`
    (`col=x-1, row=y-1`, button from the changed bit, action from `type`, scroll from `wdy`,
    mods from `modifiers`). Returns `None` for events we don't forward.
  - `encode_connect(pid, vc) -> [u8; 16]`.
- **Linux glue (`#[cfg(target_os = "linux")]`):**
  - `detect_vc() -> Option<i32>` (the ioctl/fstat logic above).
  - `start(flags: Arc<Mutex<Flags>>, out: UnixStream)`: if `TUIUI_GPM != "0"` and (`detect_vc()`
    is `Some` or `TUIUI_GPM == "1"`) and `/dev/gpmctl` connects, spawn a thread that writes the
    connect record then loops reading 28-byte events, maps each via `to_mouse_input` (tracking
    the previous button mask), and calls the shared `route_mouse` (below) on its own `out`
    clone + its own `last_click`. Logs status via `crate::dbg_log`.
  - Non-Linux: `start(..)` is a no-op stub.

### `src/client.rs` refactor

Factor the per-event mouse routing into a shared function so crossterm and gpm share it:

```rust
fn route_mouse(out: &mut UnixStream, f: &Flags, ev: crate::mouse::MouseInput,
               last_click: &mut Option<(Point, std::time::Instant)>) -> std::io::Result<()>
```
- If `f.app_area` contains `(ev.col, ev.row)` → `send(out, ClientMsg::MouseInput(ev))`.
- Else map `ev` to today's chrome variants (Left Down → double-click→`MouseDouble`/else
  `MouseDown`; Right Down → `MouseRightDown`; Left Drag → `MouseDrag`; Left Up → `MouseUp`;
  Move → `MouseDrag`; ScrollUp/Down → Store/FM as today). Behaviour byte-for-byte identical to
  the current inline code.

The existing `Event::Mouse(me)` arm becomes: `if let Some(ev) = to_mouse_input(&me, p, mods)
{ route_mouse(&mut out_stream, &f, ev, &mut last_click)?; }`. The gpm thread calls the same
`route_mouse`. The shared `Flags` is already an `Arc<Mutex<Flags>>` in `run`; clone it for gpm.
Call `gpm::start(flags.clone(), out_stream.try_clone()?)` once near the start of `run`.

## Behaviour

On a GUI terminal / SSH: gpm never starts; mouse works exactly as today (incl. passthrough).
On a bare Linux VT with `gpm` running: the gpm thread feeds the *same* `route_mouse`, so chrome,
window management, and in-app passthrough all work with the console mouse — matching the GUI
experience (minus images, which a raw VT can't display anyway).

## Testing

- **Pure unit tests (any OS):** `parse_event` (length guard, offset/endianness, a hand-built
  28-byte record → expected fields); `to_mouse_input` (left/middle/right down/up via button-mask
  deltas, drag via `type`, wheel via `wdy`, modifier bits, `x/y → col/row` 1→0 based, `None`
  cases); `encode_connect` (16 bytes, correct fields).
- **Build:** must compile on macOS (gpm glue cfg'd out → `start` no-op) and Linux.
- **Manual (user, Debian console):** `apt install gpm` (+ ensure `gpm` service running on the
  VT), run `tuiui` on the bare console; verify the mouse moves/clicks drive tuiui chrome + apps.
  `TUIUI_DEBUG=1` logs gpm connect/VC and a sample of events to `~/tuiui-debug.log` if it misbehaves.

## Out of scope

Wayland/libinput direct input (cage path covers GUI-on-bare-metal); gpm copy/paste selection;
non-Linux consoles; the DRM/KMS image backend (separate, declined).
