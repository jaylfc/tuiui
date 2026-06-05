# Graphics Passthrough (A2) — Spike Design

**Status:** Approved design (2026-06-05). Subsystem **A2** of the desktop-OS
roadmap — the research spike that lets **hosted PTY apps** (yazi previews, `timg`,
later a terminal browser) display real images inside a tuiui window. Scoped as a
**minimal spike**: prove the end-to-end path with yazi, defer full fidelity.

**Goal:** When a hosted app emits Kitty graphics escapes, capture them before our
embedded emulator swallows them, decode the image, and render it inside the app's
window — reusing the A1 image layer (`ImageStore` + `ImagePlacement` + the client's
Kitty renderer) for output. Validation: **yazi's image preview shows inside a tuiui
window** (on a Ghostty/Kitty/WezTerm outer terminal).

**Architecture:** A stateful **graphics-splitting tap** sits in the PTY reader
thread (`ptyhost`), between the raw PTY bytes and alacritty's `Processor`. It pulls
Kitty-graphics APC sequences (`ESC _ G … ESC \`) out of the stream (alacritty never
sees them), answers the support query so the app emits graphics, decodes transmitted
images, and records placements at the current cursor cell into a thread-safe
`AppGraphics` on the `AppInstance`. `session::build_frame` reads `AppGraphics` for
each `App` window, loads images into the session `ImageStore`, and emits
`ImagePlacement`s offset to the window — which the existing A1 client renderer draws.

**Tech stack:** Rust; existing `alacritty_terminal` pipeline, `image` crate (decode),
`flate2` (zlib `o=z` — already transitively present via `png`/`image`; add if
needed), `base64`, the A1 `imagestore`/`protocol`/`client`. No new heavy deps.

**Why a tap, not an alacritty change:** alacritty_terminal 0.26's public `Handler`
trait exposes no graphics/APC hook — it ignores APC. Intercepting upstream is fully
decoupled, needs no fork, and fails safe (a parse miss just passes bytes through).

---

## Module layout

- **Create `src/kittygfx.rs`** — the protocol-facing core, all pure and unit-testable:
  - A streaming **splitter** (`GraphicsTap`) that consumes PTY byte chunks and yields
    `(passthrough_bytes, Vec<GraphicsCmd>)`, buffering partial APC across reads.
  - A **command parser**: control key/value pairs + payload → `GraphicsCmd`.
  - A **transmission assembler**: reassemble chunked (`m=1`) payloads; resolve
    direct (`t=d` base64), temp-file (`t=t`), and file (`t=f`) sources into raw image
    bytes; decode (`f=24` RGB, `f=32` RGBA, `f=100` PNG; optional `o=z` zlib) → PNG.
  - **State**: image store (kitty image id → decoded PNG) and current placements.
- **Modify `src/ptyhost.rs`** — run the tap in the reader thread: feed passthrough
  bytes to `parser.advance`, apply graphics commands (after advancing so the cursor
  is current), answer `a=q` queries via the PTY writer, and publish results into
  `AppInstance.graphics: Arc<Mutex<AppGraphics>>`. Set a TERM hint so apps emit.
- **Modify `src/session.rs`** — for `App` windows, `refresh_app_graphics` loads
  `AppGraphics` PNGs into `self.images` and `build_frame` emits offset/clipped/visible
  `ImagePlacement`s (mirrors the FM/desktop thumbnail loops).
- **Reuse unchanged:** `imagestore`, `protocol::ImagePlacement`, `client::reconcile_images`.

## The Kitty graphics protocol (what the spike covers)

A command is an APC string: `ESC _ G <control> ; <base64-payload> ESC \`, where
`<control>` is comma-separated `key=value` pairs. Keys the spike handles:

- **`a`** action: `t` transmit, `T` transmit-and-display, `p` put/place, `d` delete,
  `q` query. (Spike: `t`, `T`, `p`, `d`, `q`.)
- **`i`** image id (app-assigned); **`p`** placement id (optional).
- **`f`** format: `24` (RGB), `32` (RGBA), `100` (PNG). (Spike: all three.)
- **`t`** transmission medium: `d` direct (payload is the image bytes), `f` file,
  `t` temp file (payload is a path, possibly base64). (Spike: `d`, `f`, `t`.)
- **`s`,`v`** pixel width/height (needed to decode raw RGB/RGBA).
- **`m`** more-chunks flag: `1` = more follow, `0`/absent = last. (Spike: reassemble.)
- **`o`** compression: `z` = zlib. (Spike: inflate.)
- **`c`,`r`** placement size in cols/rows; **`X`,`Y`** pixel offset within the cell
  (ignored in the spike — we place on cell boundaries).
- **`d`** (on delete) delete target: `a`/`A` all, `i` by id. (Spike: all + by-id.)

**Deferred (not in the spike):** shared-memory medium (`t=s`); animation (`a=a`,
frames); unicode-placeholder placement (`U=1`); z-index (`z=`); precise pixel offsets
(`X`/`Y`); cursor-movement semantics after display (`C=`); scroll-aware repositioning
(we rely on the app deleting+replacing on redraw, which yazi does).

## The splitter (`GraphicsTap`)

```rust
/// Streaming separator: feed PTY bytes, get back the non-graphics bytes (for the
/// emulator) and any completed graphics commands. Holds partial-APC state across calls.
pub struct GraphicsTap { /* scan state, partial APC buffer */ }

pub struct Split { pub passthrough: Vec<u8>, pub commands: Vec<GraphicsCmd> }

impl GraphicsTap {
    pub fn new() -> Self;
    /// Consume `bytes`; emit passthrough + completed commands. A graphics APC in
    /// progress is buffered (no passthrough for its bytes) until `ESC \` closes it.
    pub fn feed(&mut self, bytes: &[u8]) -> Split;
}
```

State machine: scan for `ESC _ G` (APC introducer + the `G` graphics marker). Bytes
outside an APC pass through verbatim. On `ESC _`, peek the next byte: if `G`, enter
graphics-APC capture (buffer until the `ESC \` string terminator, then parse);
otherwise it's a non-graphics APC — pass it through untouched (alacritty handles/ignores
it). Handle the introducer/terminator straddling a chunk boundary (the partial buffer
persists across `feed` calls, like alacritty's `Processor`).

```rust
pub struct GraphicsCmd {
    pub control: Vec<(char, String)>, // raw key=val pairs, e.g. ('a',"T"),('i',"31")
    pub payload: Vec<u8>,             // raw bytes between ';' and ESC\ (still base64 if direct)
}
```

A thin accessor layer reads typed values (`action()`, `id()`, `format()`, `medium()`,
`more()`, `width()`, `height()`, `compression()`, `cols()`, `rows()`, `delete_target()`).

## Transmission assembler + image state (`kittygfx.rs`)

```rust
pub struct GraphicsState {
    images: HashMap<u32, Vec<u8>>,        // kitty image id → decoded PNG bytes
    pending: HashMap<u32, PendingXmit>,   // chunked transmits in progress
    pub placements: Vec<Placement>,       // current on-screen placements
    pub queries: Vec<String>,             // pending a=q replies to write back to the PTY
}

pub struct Placement {
    pub image_id: u32,
    pub col: u16, pub row: u16,           // cursor cell at place time (app-grid coords)
    pub cols: u16, pub rows: u16,         // size in cells (from c/r, or derived from pixels/cell-size)
}

impl GraphicsState {
    /// Apply a command at the given cursor cell. Returns nothing; updates state.
    /// `transmit`/`T` reassemble + decode; `p`/`T` push a Placement; `d` removes;
    /// `q` pushes an OK reply onto `queries`.
    pub fn apply(&mut self, cmd: &GraphicsCmd, cursor_col: u16, cursor_row: u16);
    pub fn png(&self, image_id: u32) -> Option<&[u8]>;
}
```

- **Reassembly:** `m=1` chunks accumulate in `pending[id]` until `m=0`/absent, then
  the full payload is resolved+decoded and moved into `images[id]`.
- **Resolve source:** `t=d` → the payload IS the (base64) image bytes; `t=f`/`t=t` →
  the payload (base64-decoded) is a filesystem path to read. (`t=t` temp files may be
  deleted by the app after we read — we read promptly on `m=0`.)
- **Decode:** `f=100` PNG → store as-is (validate via `image::load_from_memory`);
  `f=24`/`f=32` raw → build an `RgbImage`/`RgbaImage` from `s×v` and re-encode PNG;
  `o=z` → inflate before decoding. Cap dimensions (downscale very large images) to
  stay SSH-friendly, reusing `ImageStore`'s thumbnailing on the session side.
- **Cell size:** placements prefer explicit `c`/`r`; otherwise derive cells from the
  image pixel size and an assumed cell size (e.g. 8×16 px) — a spike approximation,
  noted as imprecise.

## PTY reader-thread integration (`ptyhost.rs`)

```rust
// reader thread (replaces the current advance loop):
let mut tap = GraphicsTap::new();
loop {
    let n = match reader.read(&mut buf) { Ok(0)|Err(_) => break, Ok(n) => n };
    let split = tap.feed(&buf[..n]);
    if let Ok(mut t) = tclone.lock() {
        parser.advance(&mut *t, &split.passthrough);     // emulator sees only text
        let (cc, cr) = cursor_cell(&t);                  // current cursor after advancing
        if let Ok(mut g) = gclone.lock() {
            for cmd in &split.commands { g.apply(cmd, cc, cr); }
            for reply in g.queries.drain(..) {           // answer a=q so apps emit graphics
                let _ = writer.lock().map(|mut w| { let _ = w.write_all(reply.as_bytes()); });
            }
        }
    }
}
```

- `AppInstance` gains `graphics: Arc<Mutex<GraphicsState>>` (cloned into the thread).
- `cursor_cell(term)` reads `term.grid().cursor.point` → `(col, line)`.
- **TERM hint:** set `TERM=xterm-kitty` for the child (strongest signal that Kitty
  graphics are supported). Keep a fallback: if that proves to break terminfo-sensitive
  apps on the host, revert to `xterm-256color` and rely solely on the `a=q` reply.
  (The spike sets `xterm-kitty`; this is a one-line knob.)

## Session rendering (`session.rs`)

- `fn refresh_app_graphics(&mut self)` — for each `App` window, lock its
  `GraphicsState`, and for every distinct `image_id` referenced by a placement, load
  its PNG into `self.images` (hash-cached), mapping kitty id → `ImageId`.
- `build_frame` — after the FM/desktop image loops, for each non-minimized `App`
  window with placements: emit one `ImagePlacement { id, rect, cols, rows, visible }`
  per placement, where `rect` = the window content origin + `(col,row)` clamped to the
  content rect, clipped to the window, `visible = fully_unobstructed(win)`. The client
  draws it via the existing A1 path.
- Call `refresh_app_graphics` each frame (cheap: hash-cached loads), or when a window's
  graphics generation counter changes (add a `dirty`/counter to `GraphicsState` to
  avoid re-scanning every frame).

## Testing (deterministic, no yazi in CI)

`kittygfx.rs` unit tests feed hand-built Kitty escape byte strings and assert state:
- **Splitter:** a graphics APC is removed from passthrough and yielded as a command;
  surrounding text passes through; an APC split across two `feed` calls reassembles;
  a non-graphics APC (`ESC _ X … ESC \`) passes through untouched.
- **Direct PNG transmit+display** (`a=T,f=100,t=d` with a tiny real PNG, base64):
  `apply` decodes and creates a placement at the given cursor cell; `png(id)` returns
  valid PNG bytes (`image::load_from_memory` succeeds).
- **Raw RGBA** (`f=32,s=2,v=2` + 16 bytes): decodes to a 2×2 PNG.
- **Chunked** (`m=1` then `m=0`): reassembles before decoding.
- **Temp-file** (`t=t`, payload = base64 path to a temp PNG we wrote): reads + decodes.
- **Delete** (`a=d,d=A`): clears placements; (`a=d,i=ID`): removes only that id.
- **Query** (`a=q`): pushes an `ESC _ G i=<id>;OK ESC \` reply onto `queries`.

`session` integration: construct an `AppInstance`-like state with a placement and a
loaded image; assert `build_frame` emits an `ImagePlacement` offset into the window.
(End-to-end yazi rendering is verified manually on the mini.)

## Build sequence (informs the plan)

1. `GraphicsCmd` + control parsing + the `GraphicsTap` splitter (+ tests).
2. Transmission assembler: chunk reassembly + `t=d` base64 + PNG/raw/zlib decode (+ tests).
3. `t=f`/`t=t` file sources + `a=d` delete + `a=q` query reply (+ tests).
4. `GraphicsState::apply` wiring placements at the cursor (+ tests).
5. `ptyhost` reader-thread integration: tap, cursor read, `AppGraphics`, query write-back,
   TERM hint.
6. `session` `refresh_app_graphics` + `build_frame` placements (+ a session test).
7. Manual yazi verification on the mini + README note + a `docs` findings update.

## Risks & open questions (spike will answer)

- **Does yazi emit at all** under `TERM=xterm-kitty` + `a=q` reply? (First thing to
  confirm on the mini; a debug log of captured commands de-risks this.)
- **Transmission medium yazi actually uses** (`t=d` vs `t=f`/`t=t`) — covered both.
- **Placement positioning accuracy** (cursor cell vs where yazi expects the image).
- **Redraw churn**: yazi deletes+retransmits per navigation; ensure no leak/stale image
  (honor `a=d`; consider a generation counter).
- **Cell-size approximation** for raw images without `c`/`r`.

## Out of scope (post-spike)

Shared-memory transmission, animation, unicode placeholders, z-index, sub-cell pixel
offsets, scroll-tracking placements with the grid, and a generic Sixel path (separate
effort). These come only if the spike proves the core path works.
