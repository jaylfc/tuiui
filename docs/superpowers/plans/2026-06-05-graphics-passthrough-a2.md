# Graphics Passthrough (A2) Spike Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Capture Kitty-graphics escapes emitted by hosted PTY apps before our embedded emulator swallows them, decode the images, and render them inside the app's window via the A1 image layer — proving the path with yazi's preview.

**Architecture:** A stateful **graphics-splitting tap** (`src/kittygfx.rs`) runs in the PTY reader thread, separating Kitty APC sequences from text, decoding transmitted images, and recording placements at the cursor into a thread-safe `GraphicsState` on the `AppInstance`. `session::build_frame` turns those into `ImagePlacement`s (offset to the window), which the existing A1 client renderer draws.

**Tech Stack:** Rust; `alacritty_terminal` pipeline, `image` (decode), new deps `base64` (decode) + `flate2` (zlib `o=z`), A1 `imagestore`/`protocol`/`client`.

**Reference spec:** `docs/superpowers/specs/2026-06-05-graphics-passthrough-a2-design.md`.

---

## Current surface (verified)

- `src/ptyhost.rs`: reader thread does `let mut parser = Processor::<StdSyncHandler>::new(); … parser.advance(&mut *t, &buf[..n]);`. `AppInstance { term: Arc<Mutex<Term<PtyResponder>>>, master, writer: Arc<Mutex<Box<dyn Write+Send>>>, child, cols, rows }`. `PtyResponder { writer }`. `spawn(...)` builds `term`, the reader thread, returns `AppInstance`. `snapshot()` reads `t.grid()` and indexes `grid[Line(y)][Column(x)]`. The child env sets `TERM=xterm-256color` (line ~91).
- `alacritty_terminal` cursor: `term.grid().cursor.point` is a `Point { line: Line(i32), column: Column(usize) }` (verify field names; `Line` derefs to i32, `Column` to usize). Use `let p = t.grid().cursor.point; (p.column.0 as u16, p.line.0 as u16)` — confirm and adjust.
- `src/imagestore.rs`: `ImageStore::load_bytes(&mut self, bytes, max_w, max_h) -> Option<u64>` (decodes+thumbnails+hashes+stores PNG), `png_bytes(id)`, `dimensions(id)`. `ImageId = u64`.
- `src/protocol.rs`: `ImagePlacement { id: u64, rect: Rect, cols: u16, rows: u16, visible: bool }`.
- `src/session.rs`: `build_frame` emits ImageView/FM/desktop placements into `images` before `Frame {…}`. `fully_unobstructed(&self, win)->bool`. `self.images: ImageStore`. `WinContent::App(AppInstance)`. `self.wm.z_ordered()` → windows with `.minimized`, `.rect`, `.content_rect()`, `.id`.
- `src/kitty.rs`: `b64(data)->String` (encode only). No decode anywhere.
- `Cargo.toml`: `image = { version = "0.25.10", default-features = false, features = ["png","jpeg","gif","webp"] }`. No `base64`/`flate2`.

## Conventions

- `export PATH="$HOME/.cargo/bin:$PATH"` before cargo. Build before commit. Per-task: build clean + task tests pass. Final task: full gate, 0 warnings.
- Commit per task with the exact message. No AI attribution. Branch `main`.
- Task 1 fetches the new crates (one network call) — run `cargo test --test kittygfx_tests` without `--offline` the first time, then `--offline`.

---

### Task 1: Deps + `GraphicsCmd` control parsing + the `GraphicsTap` splitter

**Files:** `Cargo.toml`, Create `src/kittygfx.rs`, `src/lib.rs`; Test `tests/kittygfx_tests.rs`.

- [ ] **Step 1: Add deps** to `Cargo.toml` `[dependencies]`: `base64 = "0.22"` and `flate2 = "1"`.

- [ ] **Step 2: Write the failing test** (`tests/kittygfx_tests.rs`):

```rust
use tuiui::kittygfx::{GraphicsTap, GraphicsCmd};

/// Build a Kitty graphics APC: ESC _ G <control> ; <payload> ESC \
fn apc(control: &str, payload: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b_G");
    v.extend_from_slice(control.as_bytes());
    v.push(b';');
    v.extend_from_slice(payload.as_bytes());
    v.extend_from_slice(b"\x1b\\");
    v
}

#[test]
fn splits_graphics_from_text() {
    let mut tap = GraphicsTap::new();
    let mut input = b"hello".to_vec();
    input.extend(apc("a=T,f=100,i=1", "AAAA"));
    input.extend_from_slice(b"world");
    let out = tap.feed(&input);
    assert_eq!(out.passthrough, b"helloworld");
    assert_eq!(out.commands.len(), 1);
    let c = &out.commands[0];
    assert_eq!(c.get('a').as_deref(), Some("T"));
    assert_eq!(c.get('i').as_deref(), Some("1"));
    assert_eq!(c.payload, b"AAAA");
}

#[test]
fn reassembles_apc_split_across_feeds() {
    let mut tap = GraphicsTap::new();
    let full = apc("a=t,i=9", "XYZ");
    let (a, b) = full.split_at(5); // split mid-APC
    let o1 = tap.feed(a);
    assert!(o1.commands.is_empty());
    assert!(o1.passthrough.is_empty()); // APC bytes are withheld, not passed through
    let o2 = tap.feed(b);
    assert_eq!(o2.commands.len(), 1);
    assert_eq!(o2.commands[0].get('i').as_deref(), Some("9"));
}

#[test]
fn non_graphics_apc_passes_through() {
    let mut tap = GraphicsTap::new();
    // ESC _ X ... ESC \  (not a 'G' graphics APC)
    let input = b"\x1b_Xsomething\x1b\\rest".to_vec();
    let out = tap.feed(&input);
    assert!(out.commands.is_empty());
    assert_eq!(out.passthrough, input); // passed through untouched
}
```

- [ ] **Step 3: Run → FAIL** (`cargo test --test kittygfx_tests`).

- [ ] **Step 4: Implement `src/kittygfx.rs`** (splitter + command; assembler/state in later tasks):

```rust
//! Kitty graphics protocol capture for hosted PTY apps (A2). A streaming tap
//! separates graphics APC sequences (`ESC _ G … ESC \`) from text so the embedded
//! emulator only sees text, and decodes transmitted images for our compositor.

/// One captured graphics command: control key/value pairs + the raw payload.
#[derive(Clone, Debug, Default)]
pub struct GraphicsCmd {
    pub control: Vec<(char, String)>,
    pub payload: Vec<u8>,
}

impl GraphicsCmd {
    pub fn get(&self, key: char) -> Option<String> {
        self.control.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
    }
    fn parse(control: &[u8], payload: Vec<u8>) -> Self {
        let s = String::from_utf8_lossy(control);
        let control = s
            .split(',')
            .filter_map(|kv| {
                let mut it = kv.splitn(2, '=');
                let k = it.next()?.trim();
                let v = it.next().unwrap_or("").trim();
                let kc = k.chars().next()?;
                Some((kc, v.to_string()))
            })
            .collect();
        GraphicsCmd { control, payload }
    }
}

/// Result of feeding a chunk: the non-graphics bytes (for the emulator) and any
/// completed graphics commands.
#[derive(Debug, Default)]
pub struct Split {
    pub passthrough: Vec<u8>,
    pub commands: Vec<GraphicsCmd>,
}

#[derive(Clone, Copy, PartialEq)]
enum St {
    Text,        // normal bytes
    Esc,         // saw ESC, deciding
    MaybeApc,    // saw ESC _, peeking the marker
    GfxControl,  // inside ESC _ G, before ';' — accumulating control
    GfxPayload,  // after ';' — accumulating payload
    GfxEsc,      // inside graphics APC, saw ESC, expecting '\'
    OtherApc,    // a non-graphics APC, pass through, scan for ESC \
    OtherApcEsc, // in OtherApc, saw ESC
}

/// Streaming graphics separator. Holds partial-APC state across `feed` calls.
pub struct GraphicsTap {
    st: St,
    control: Vec<u8>,
    payload: Vec<u8>,
    other: Vec<u8>, // buffered non-graphics APC bytes (incl. introducer) to pass through
}

impl Default for GraphicsTap {
    fn default() -> Self { Self::new() }
}

impl GraphicsTap {
    pub fn new() -> Self {
        Self { st: St::Text, control: Vec::new(), payload: Vec::new(), other: Vec::new() }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Split {
        let mut out = Split::default();
        for &b in bytes {
            match self.st {
                St::Text => {
                    if b == 0x1b { self.st = St::Esc; } else { out.passthrough.push(b); }
                }
                St::Esc => {
                    if b == b'_' { self.st = St::MaybeApc; }
                    else {
                        // not an APC: emit the ESC and this byte as text
                        out.passthrough.push(0x1b);
                        if b == 0x1b { self.st = St::Esc; } else { out.passthrough.push(b); self.st = St::Text; }
                    }
                }
                St::MaybeApc => {
                    if b == b'G' {
                        self.control.clear();
                        self.payload.clear();
                        self.st = St::GfxControl;
                    } else {
                        // non-graphics APC: pass it through verbatim (ESC _ already consumed)
                        self.other.clear();
                        self.other.extend_from_slice(b"\x1b_");
                        self.other.push(b);
                        self.st = if b == 0x1b { St::OtherApcEsc } else { St::OtherApc };
                    }
                }
                St::GfxControl => {
                    if b == b';' { self.st = St::GfxPayload; }
                    else if b == 0x1b { self.st = St::GfxEsc; } // payload-less command (e.g. a=q)
                    else { self.control.push(b); }
                }
                St::GfxPayload => {
                    if b == 0x1b { self.st = St::GfxEsc; } else { self.payload.push(b); }
                }
                St::GfxEsc => {
                    if b == b'\\' {
                        out.commands.push(GraphicsCmd::parse(&self.control, std::mem::take(&mut self.payload)));
                        self.control.clear();
                        self.st = St::Text;
                    } else {
                        // ESC inside payload that wasn't ST — keep it as payload
                        self.payload.push(0x1b);
                        if b == 0x1b { self.st = St::GfxEsc; } else { self.payload.push(b); self.st = St::GfxPayload; }
                    }
                }
                St::OtherApc => {
                    self.other.push(b);
                    if b == 0x1b { self.st = St::OtherApcEsc; }
                }
                St::OtherApcEsc => {
                    self.other.push(b);
                    if b == b'\\' {
                        out.passthrough.append(&mut self.other);
                        self.st = St::Text;
                    } else if b != 0x1b {
                        self.st = St::OtherApc;
                    }
                }
            }
        }
        out
    }
}
```

Register in `src/lib.rs`: `pub mod kittygfx;`.

- [ ] **Step 5: Run → PASS** (`cargo test --offline --test kittygfx_tests`). Commit:

```bash
git add Cargo.toml Cargo.lock src/kittygfx.rs src/lib.rs tests/kittygfx_tests.rs
git commit -m "kittygfx: graphics APC splitter + command control parsing"
```

---

### Task 2: Transmission assembler — chunk reassembly + decode (direct/PNG/raw/zlib)

**Files:** `src/kittygfx.rs`; Test `tests/kittygfx_tests.rs` (append).

- [ ] **Step 1: Append tests:**

```rust
use tuiui::kittygfx::GraphicsState;

fn tiny_png() -> Vec<u8> {
    // 1x1 red PNG, generated via the image crate.
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

#[test]
fn direct_png_transmit_decodes() {
    use base64::Engine;
    let png = tiny_png();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let cmd = tuiui::kittygfx::parse_one(&apc("a=t,f=100,t=d,i=7", &b64));
    let mut st = GraphicsState::new();
    st.apply(&cmd, 0, 0);
    assert!(st.png(7).is_some());
    assert!(image::load_from_memory(st.png(7).unwrap()).is_ok());
}

#[test]
fn raw_rgba_transmit_decodes() {
    use base64::Engine;
    // 2x2 RGBA = 16 bytes
    let raw: Vec<u8> = (0..16).map(|i| i as u8).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    let cmd = tuiui::kittygfx::parse_one(&apc("a=t,f=32,t=d,s=2,v=2,i=3", &b64));
    let mut st = GraphicsState::new();
    st.apply(&cmd, 0, 0);
    assert!(st.png(3).is_some());
}

#[test]
fn chunked_transmit_reassembles() {
    use base64::Engine;
    let png = tiny_png();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let (a, b) = b64.split_at(b64.len() / 2);
    let mut st = GraphicsState::new();
    st.apply(&tuiui::kittygfx::parse_one(&apc("a=t,f=100,t=d,i=5,m=1", a)), 0, 0);
    assert!(st.png(5).is_none()); // not complete yet
    st.apply(&tuiui::kittygfx::parse_one(&apc("i=5,m=0", b)), 0, 0);
    assert!(st.png(5).is_some());
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.** Append to `src/kittygfx.rs`:

```rust
use std::collections::HashMap;

/// Parse exactly one APC command from `bytes` (test/helper convenience).
pub fn parse_one(bytes: &[u8]) -> GraphicsCmd {
    let mut tap = GraphicsTap::new();
    tap.feed(bytes).commands.into_iter().next().unwrap_or_default()
}

struct Pending {
    format: u32,         // 24/32/100
    medium: char,        // d/f/t
    width: u32, height: u32,
    zlib: bool,
    data: Vec<u8>,       // accumulated base64 (direct) or path bytes
}

/// A placed image on the app's grid (cell coordinates relative to the app PTY).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub image_id: u32,
    pub col: u16, pub row: u16,
    pub cols: u16, pub rows: u16,
}

/// Captured graphics state for one hosted app.
pub struct GraphicsState {
    images: HashMap<u32, Vec<u8>>,    // id → PNG
    pending: HashMap<u32, Pending>,
    pub placements: Vec<Placement>,
    pub queries: Vec<Vec<u8>>,        // a=q replies to write back to the PTY
    pub generation: u64,              // bumped on any change (session dirty check)
}

impl Default for GraphicsState { fn default() -> Self { Self::new() } }

impl GraphicsState {
    pub fn new() -> Self {
        Self { images: HashMap::new(), pending: HashMap::new(), placements: Vec::new(), queries: Vec::new(), generation: 0 }
    }
    pub fn png(&self, id: u32) -> Option<&[u8]> { self.images.get(&id).map(|v| v.as_slice()) }

    fn num(cmd: &GraphicsCmd, k: char, default: u32) -> u32 {
        cmd.get(k).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    /// Apply a command at cursor cell `(col,row)`.
    pub fn apply(&mut self, cmd: &GraphicsCmd, col: u16, row: u16) {
        let action = cmd.get('a').unwrap_or_else(|| "t".into());
        match action.as_str() {
            "t" | "T" => {
                self.accumulate(cmd);
                if cmd.get('m').as_deref() != Some("1") {
                    let id = Self::num(cmd, 'i', 0);
                    self.finish_transmit(id);
                    if action == "T" { self.place(id, col, row, cmd); }
                }
                self.generation += 1;
            }
            "p" => { let id = Self::num(cmd, 'i', 0); self.place(id, col, row, cmd); self.generation += 1; }
            "d" => { self.delete(cmd); self.generation += 1; }
            "q" => { self.queries.push(reply_ok(Self::num(cmd, 'i', 0))); }
            _ => {}
        }
    }

    fn accumulate(&mut self, cmd: &GraphicsCmd) {
        let id = Self::num(cmd, 'i', 0);
        let e = self.pending.entry(id).or_insert_with(|| Pending {
            format: Self::num(cmd, 'f', 32),
            medium: cmd.get('t').and_then(|s| s.chars().next()).unwrap_or('d'),
            width: Self::num(cmd, 's', 0),
            height: Self::num(cmd, 'v', 0),
            zlib: cmd.get('o').as_deref() == Some("z"),
            data: Vec::new(),
        });
        e.data.extend_from_slice(&cmd.payload);
    }

    fn finish_transmit(&mut self, id: u32) {
        let Some(p) = self.pending.remove(&id) else { return; };
        if let Some(png) = decode(&p) { self.images.insert(id, png); }
    }

    fn place(&mut self, id: u32, col: u16, row: u16, cmd: &GraphicsCmd) {
        if !self.images.contains_key(&id) { return; }
        let cols = Self::num(cmd, 'c', 0) as u16;
        let rows = Self::num(cmd, 'r', 0) as u16;
        let (cols, rows) = if cols > 0 && rows > 0 { (cols, rows) } else { derive_cells(self.images.get(&id)) };
        self.placements.retain(|pl| pl.image_id != id); // one placement per id (spike)
        self.placements.push(Placement { image_id: id, col, row, cols, rows });
    }

    fn delete(&mut self, cmd: &GraphicsCmd) {
        match cmd.get('d').as_deref() {
            Some("i") | Some("I") => { let id = Self::num(cmd, 'i', 0); self.placements.retain(|p| p.image_id != id); }
            _ => self.placements.clear(), // a/A or unspecified → all
        }
    }
}

/// `ESC _ G i=<id>;OK ESC \`
fn reply_ok(id: u32) -> Vec<u8> {
    format!("\x1b_Gi={id};OK\x1b\\").into_bytes()
}

/// Decode a completed transmission into PNG bytes.
fn decode(p: &Pending) -> Option<Vec<u8>> {
    use base64::Engine;
    // direct payload is base64; file/temp payload is a base64 path.
    let decoded = base64::engine::general_purpose::STANDARD.decode(&p.data).ok()?;
    let raw: Vec<u8> = match p.medium {
        'f' | 't' => {
            let path = String::from_utf8_lossy(&decoded).to_string();
            std::fs::read(path).ok()?
        }
        _ => decoded, // 'd'
    };
    let raw = if p.zlib { inflate(&raw)? } else { raw };
    match p.format {
        100 => { image::load_from_memory(&raw).ok()?; Some(raw) } // already PNG; validate
        24 => reencode_raw(&raw, p.width, p.height, false),
        32 => reencode_raw(&raw, p.width, p.height, true),
        _ => image::load_from_memory(&raw).ok().map(|_| raw),
    }
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

fn reencode_raw(raw: &[u8], w: u32, h: u32, alpha: bool) -> Option<Vec<u8>> {
    if w == 0 || h == 0 { return None; }
    let img = if alpha {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, raw.to_vec())?)
    } else {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, raw.to_vec())?)
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

/// Approximate a cell footprint from the image pixel size (8x16 px cells).
fn derive_cells(png: Option<&Vec<u8>>) -> (u16, u16) {
    if let Some(bytes) = png {
        if let Ok(img) = image::load_from_memory(bytes) {
            let (w, h) = (image::GenericImageView::dimensions(&img));
            return (((w / 8).max(1)) as u16, ((h / 16).max(1)) as u16);
        }
    }
    (10, 5)
}
```

- [ ] **Step 4: Run → PASS.** Commit:

```bash
git add src/kittygfx.rs tests/kittygfx_tests.rs
git commit -m "kittygfx: transmission assembler — chunk reassembly + PNG/raw/zlib decode"
```

---

### Task 3: Place/delete/query behavior + temp-file source

**Files:** `src/kittygfx.rs` (implemented in Task 2); Test `tests/kittygfx_tests.rs` (append).

Task 2 already implemented `place`/`delete`/`query`/temp-file; this task locks them with tests. (Fix Task 2 code if a test fails.)

- [ ] **Step 1: Append tests:**

```rust
#[test]
fn transmit_and_display_places_at_cursor() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&tiny_png());
    let cmd = tuiui::kittygfx::parse_one(&apc("a=T,f=100,t=d,i=2,c=4,r=2", &b64));
    let mut st = GraphicsState::new();
    st.apply(&cmd, 6, 3);
    assert_eq!(st.placements.len(), 1);
    let p = &st.placements[0];
    assert_eq!((p.col, p.row, p.cols, p.rows), (6, 3, 4, 2));
}

#[test]
fn delete_all_and_by_id() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&tiny_png());
    let mut st = GraphicsState::new();
    st.apply(&tuiui::kittygfx::parse_one(&apc("a=T,f=100,t=d,i=1,c=1,r=1", &b64)), 0, 0);
    st.apply(&tuiui::kittygfx::parse_one(&apc("a=T,f=100,t=d,i=2,c=1,r=1", &b64)), 1, 0);
    assert_eq!(st.placements.len(), 2);
    st.apply(&tuiui::kittygfx::parse_one(&apc("a=d,d=i,i=1", "")), 0, 0);
    assert_eq!(st.placements.len(), 1);
    st.apply(&tuiui::kittygfx::parse_one(&apc("a=d,d=A", "")), 0, 0);
    assert!(st.placements.is_empty());
}

#[test]
fn query_pushes_ok_reply() {
    let mut st = GraphicsState::new();
    st.apply(&tuiui::kittygfx::parse_one(&apc("a=q,i=99", "")), 0, 0);
    assert_eq!(st.queries.len(), 1);
    assert!(st.queries[0].windows(2).any(|w| w == b"OK"));
}

#[test]
fn temp_file_source_is_read() {
    use base64::Engine;
    let dir = std::env::temp_dir().join(format!("tuiui-a2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("img.png");
    std::fs::write(&path, tiny_png()).unwrap();
    let path_b64 = base64::engine::general_purpose::STANDARD.encode(path.to_string_lossy().as_bytes());
    let mut st = GraphicsState::new();
    st.apply(&tuiui::kittygfx::parse_one(&apc("a=t,f=100,t=t,i=8", &path_b64)), 0, 0);
    assert!(st.png(8).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run → PASS** (fix Task 2 impl if any fail).

- [ ] **Step 3: Commit:**

```bash
git add tests/kittygfx_tests.rs
git commit -m "kittygfx: tests for place/delete/query + temp-file transmission"
```

---

### Task 4: PTY reader-thread integration

**Files:** `src/ptyhost.rs`; inline test for `cursor_cell` if practical (else covered manually).

- [ ] **Step 1: Implement.** In `src/ptyhost.rs`:

(a) Add `graphics: Arc<Mutex<crate::kittygfx::GraphicsState>>` to `AppInstance`. Construct it before the reader thread and clone into the thread + into the struct.

(b) Replace the reader-thread body with the tap pipeline:

```rust
        let tclone = term.clone();
        let gclone = graphics.clone();
        let wclone = writer.clone();
        std::thread::spawn(move || {
            let mut parser = Processor::<StdSyncHandler>::new();
            let mut tap = crate::kittygfx::GraphicsTap::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = match reader.read(&mut buf) { Ok(0) | Err(_) => break, Ok(n) => n };
                let split = tap.feed(&buf[..n]);
                if let Ok(mut t) = tclone.lock() {
                    parser.advance(&mut *t, &split.passthrough);
                    if !split.commands.is_empty() {
                        let (col, row) = cursor_cell(&t);
                        if let Ok(mut g) = gclone.lock() {
                            for cmd in &split.commands { g.apply(cmd, col, row); }
                            if !g.queries.is_empty() {
                                if let Ok(mut w) = wclone.lock() {
                                    for q in g.queries.drain(..) { let _ = w.write_all(&q); }
                                    let _ = w.flush();
                                }
                            }
                        }
                    }
                }
            }
        });
```

(c) Add the cursor reader (verify the alacritty API — `term.grid().cursor.point`):

```rust
fn cursor_cell(term: &Term<PtyResponder>) -> (u16, u16) {
    let p = term.grid().cursor.point;
    (p.column.0 as u16, p.line.0.max(0) as u16)
}
```

(d) Add an accessor + the TERM hint. Change the child env from `TERM=xterm-256color` to `TERM=xterm-kitty` (the strongest Kitty-graphics-supported signal). Keep `COLORTERM=truecolor`. Add:

```rust
    /// A snapshot of this app's captured graphics placements + a generation counter.
    pub fn graphics(&self) -> std::sync::MutexGuard<'_, crate::kittygfx::GraphicsState> {
        self.graphics.lock().unwrap()
    }
```

> If `term.grid().cursor.point` field names differ in alacritty_terminal 0.26, adjust (the cursor is `term.grid().cursor`; its position field may be `.point` with `.line`/`.column`). Confirm by reading the `Term`/`Grid`/`Cursor` types; the existing `snapshot()` already uses `Line`/`Column` indexing so the index types are imported.

- [ ] **Step 2: Build + run existing tests** (`cargo test --offline`) — nothing should regress; the tap is transparent for text-only apps (passthrough == input when no graphics).

- [ ] **Step 3: Commit:**

```bash
git add src/ptyhost.rs
git commit -m "ptyhost: graphics tap in reader thread + cursor capture + TERM=xterm-kitty"
```

---

### Task 5: Session rendering — emit App graphics placements

**Files:** `src/session.rs`; Test `tests/session_tests.rs` (append).

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn app_graphics_placement_is_emitted_in_frame() {
    // Drive an AppInstance's GraphicsState directly, then assert build_frame emits it.
    use base64::Engine;
    let mut core = SessionCore::new(100, 30, Config::default());
    core.apply(ClientMsg::Launch { name: "sh".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    // Inject a placement+image into the launched app's graphics state via a test helper.
    let png = { let i = image::RgbaImage::from_pixel(2, 2, image::Rgba([1,2,3,255])); let mut b = std::io::Cursor::new(Vec::new()); image::DynamicImage::ImageRgba8(i).write_to(&mut b, image::ImageFormat::Png).unwrap(); b.into_inner() };
    core.inject_app_graphics_for_test(&png);
    let frame = core.build_frame();
    assert!(frame.images.iter().any(|p| p.cols >= 1), "expected an app graphics placement");
    core.shutdown();
}
```

> Add `#[doc(hidden)] pub fn inject_app_graphics_for_test(&mut self, png: &[u8])`: find the most-recently-launched `WinContent::App`, lock its `graphics()`, insert the PNG as image id 1 (add a small `#[doc(hidden)] pub fn insert_image_for_test(&mut self, id: u32, png: Vec<u8>)` on `GraphicsState`), and push a `Placement { image_id: 1, col: 0, row: 0, cols: 2, rows: 1 }` (add `#[doc(hidden)] pub fn push_placement_for_test`). Keep these helpers minimal.

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.** In `src/session.rs`:

(a) `fn refresh_app_graphics(&mut self)` — for each `App` window, lock `graphics()`, and for each placement's `image_id`, if not already mapped, `self.images.load_bytes(png, MAX, MAX)` and remember the kitty-id→ImageId map. Simplest spike approach: store the resulting `ImageId` alongside, or recompute each frame (hash-cached `load_bytes` makes it cheap). Because `build_frame` is `&self`, do the loading in a `&mut self` pass: call `refresh_app_graphics` from `apply` (after handling a message) and from a periodic tick, OR add it to the existing reap/refresh path. For the spike, call it at the top of any `apply` that could change app output is overkill — instead expose it and call it once per frame from the daemon before `build_frame` (the daemon already calls `build_frame`; add a `core.refresh_app_graphics()` call right before). Add the kitty-id→ImageId map onto a per-window `HashMap` kept on `SessionCore` (`app_image_ids: HashMap<(WindowId,u32), u64>`).

(b) `build_frame` — after the desktop image loop, for each non-minimized `App` window:

```rust
        for w in self.wm.z_ordered() {
            if w.minimized { continue; }
            if let Some(WinContent::App(app)) = self.contents.get(&w.id) {
                let g = app.graphics();
                if g.placements.is_empty() { continue; }
                let cr = w.content_rect();
                let vis = self.fully_unobstructed(w);
                for pl in &g.placements {
                    if let Some(&img) = self.app_image_ids.get(&(w.id, pl.image_id)) {
                        let x = cr.x + pl.col as i32;
                        let y = cr.y + pl.row as i32;
                        if x >= cr.x + cr.w || y >= cr.y + cr.h { continue; }
                        let cols = pl.cols.min((cr.x + cr.w - x).max(1) as u16);
                        let rows = pl.rows.min((cr.y + cr.h - y).max(1) as u16);
                        images.push(crate::protocol::ImagePlacement { id: img, rect: crate::geometry::Rect::new(x, y, cols as i32, rows as i32), cols, rows, visible: vis });
                    }
                }
            }
        }
```

(c) `refresh_app_graphics` populates `self.app_image_ids` by loading each referenced PNG into `self.images` (mutable), keyed by `(window_id, kitty_image_id)`. Call it from the daemon right before `build_frame` (add the call in `src/daemon.rs` where `core.build_frame()` is invoked: `core.refresh_app_graphics();` immediately before). Make `refresh_app_graphics` `pub`.

- [ ] **Step 4: Run → PASS.** Then the FULL gate (`build && test && clippy --all-targets`, 0 warnings).

- [ ] **Step 5: Commit:**

```bash
git add src/session.rs src/daemon.rs src/kittygfx.rs tests/session_tests.rs
git commit -m "session: emit hosted-app graphics placements (A2) in build_frame"
```

---

### Task 6: Manual yazi verification + docs

- [ ] **Step 1:** Full gate green. `cargo install --path . --root ~/.local --force`; on a **Ghostty/Kitty/WezTerm** terminal, `tuiui kill ; tuiui`.
- [ ] **Step 2:** Install yazi (`Store → yazi`, or `brew install yazi`). Launch it in a tuiui window, navigate to a folder with images. **Expected:** the preview image renders inside the window.
  - If nothing renders, add a temporary debug log in the reader thread (`eprintln!` the captured `cmd.control` to a file) to see whether yazi emits graphics at all (TERM/query detection), what medium it uses, and the cursor cell — then iterate. This is the spike's learning loop.
- [ ] **Step 3:** Record findings in `docs/superpowers/specs/2026-06-05-graphics-passthrough-a2-design.md` (a "Spike results" section: did yazi emit? medium? positioning accuracy? what's needed for full fidelity). Update `README.md` only if it works (note "experimental: images in hosted apps like yazi"). Commit:

```bash
git add docs/ README.md
git commit -m "docs: A2 graphics passthrough spike results"
```

---

## Notes for the implementer
- The tap MUST be transparent for text-only apps: when no graphics APC appears, `feed(bytes).passthrough == bytes` and `commands` is empty. The existing test suite passing after Task 4 confirms this.
- Keep `kittygfx.rs` pure (no PTY/session deps) so it stays unit-testable; the PTY/session glue lives in `ptyhost.rs`/`session.rs`.
- `decode` caps nothing itself; the session's `ImageStore::load_bytes(png, MAX_W, MAX_H)` downscales — pass a sane bound (e.g. window pixel size) so big previews stay SSH-friendly.
- The biggest unknown is whether yazi emits graphics at all under our emulator; Task 6's debug loop exists to answer it. If `TERM=xterm-kitty` breaks other apps' terminfo on the host, revert to `xterm-256color` and rely on the `a=q` reply (Task 4 (d) is the knob).
- This is a SPIKE: scroll-tracking, animation, shmem, unicode placeholders, and z-index are explicitly out (see spec). Don't add them; record what full fidelity needs in the findings.
