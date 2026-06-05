# A1 Native Image Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Display real raster images inside tuiui windows via the Kitty graphics protocol, with a native image-viewer window as the first consumer.

**Architecture:** The daemon attaches image **placements** (id + screen rect + visibility) to each frame and ships PNG bytes once per attach; the thin client emits Kitty graphics transmit/place/delete escapes, gated on terminal capability, with a cell **placeholder** as the universal fallback. Images are decoded/downscaled/hashed by an `ImageStore`.

**Tech Stack:** Rust, the `image` crate (decode), Kitty graphics protocol, existing compositor/session/protocol/client.

**Reference spec:** `docs/superpowers/specs/2026-06-05-image-layer-design.md`

---

## File Structure

- **Create `src/imagestore.rs`** — decode → downscale → PNG → hash; cache by id.
- **Create `src/kitty.rs`** — pure Kitty escape builders (transmit/place/delete) + base64.
- **Create `src/imageview.rs`** — `ImageView` window content + placeholder rendering.
- **Modify `src/protocol.rs`** — `ImagePlacement`, `ImageBlob`; `FrameMsg` fields.
- **Modify `src/terminal.rs`** — `Caps.kitty_graphics`.
- **Modify `src/session.rs`** — `WinContent::ImageView`, `ImageStore`, placements, `@image`.
- **Modify `src/daemon.rs`** — per-client `sent_image_ids` + blob fill.
- **Modify `src/client.rs`** — image reconcile in the reader thread.
- **Modify `src/lib.rs`** — register modules.

---

### Task 1: `image` dep + `ImageStore`

**Files:** `Cargo.toml`, Create `src/imagestore.rs`, `src/lib.rs`; Test `tests/imagestore_tests.rs`.

- [ ] **Step 1: Add the dependency**

In `Cargo.toml` `[dependencies]`: `image = { version = "0.25", default-features = false, features = ["png", "jpeg", "gif", "webp"] }`

- [ ] **Step 2: Write the failing test** (`tests/imagestore_tests.rs`):

```rust
use tuiui::imagestore::ImageStore;

/// A 20×20 solid-red PNG, encoded at test time via the `image` crate.
fn red_png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

#[test]
fn load_is_deterministic_and_downscales() {
    let mut s = ImageStore::new();
    let png = red_png(100, 100);
    let id1 = s.load_bytes(&png, 40, 40).unwrap();
    let id2 = s.load_bytes(&png, 40, 40).unwrap();
    assert_eq!(id1, id2, "same input + target → same id");
    let (w, h) = s.dimensions(id1).unwrap();
    assert!(w <= 40 && h <= 40, "downscaled to fit the target box");
    assert!(!s.png_bytes(id1).unwrap().is_empty());
}

#[test]
fn corrupt_bytes_return_none() {
    let mut s = ImageStore::new();
    assert!(s.load_bytes(&[1, 2, 3, 4], 40, 40).is_none());
}
```

- [ ] **Step 3: Run → FAIL** (`cargo test --offline --test imagestore_tests`; needs network once to fetch `image`, then offline).

- [ ] **Step 4: Implement `src/imagestore.rs`**

```rust
//! Decodes, downscales, and caches images for the native image layer. Each image
//! is keyed by a content hash of its downscaled PNG (`ImageId`).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Content hash of a downscaled PNG.
pub type ImageId = u64;

struct Entry {
    png: Vec<u8>,
    w: u32,
    h: u32,
}

/// Caches decoded+downscaled images by id.
#[derive(Default)]
pub struct ImageStore {
    by_id: HashMap<ImageId, Entry>,
}

impl ImageStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode `bytes`, downscale to fit `max_w × max_h` pixels, re-encode PNG, and
    /// cache it. Returns the content id, or `None` if the bytes aren't a decodable image.
    pub fn load_bytes(&mut self, bytes: &[u8], max_w: u32, max_h: u32) -> Option<ImageId> {
        let img = image::load_from_memory(bytes).ok()?;
        let scaled = img.thumbnail(max_w.max(1), max_h.max(1)); // preserves aspect, never upsizes past box
        let (w, h) = (scaled.width(), scaled.height());
        let mut png = std::io::Cursor::new(Vec::new());
        scaled.write_to(&mut png, image::ImageFormat::Png).ok()?;
        let png = png.into_inner();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        png.hash(&mut hasher);
        let id = hasher.finish();
        self.by_id.entry(id).or_insert(Entry { png, w, h });
        Some(id)
    }

    /// Decode an image file at `path`.
    pub fn load(&mut self, path: &std::path::Path, max_w: u32, max_h: u32) -> Option<ImageId> {
        let bytes = std::fs::read(path).ok()?;
        self.load_bytes(&bytes, max_w, max_h)
    }

    pub fn png_bytes(&self, id: ImageId) -> Option<&[u8]> {
        self.by_id.get(&id).map(|e| e.png.as_slice())
    }

    pub fn dimensions(&self, id: ImageId) -> Option<(u32, u32)> {
        self.by_id.get(&id).map(|e| (e.w, e.h))
    }
}
```

Register in `src/lib.rs`: `pub mod imagestore;`

- [ ] **Step 5: Run → PASS. Commit:**

```bash
git add Cargo.toml Cargo.lock src/imagestore.rs src/lib.rs tests/imagestore_tests.rs
git commit -m "image: ImageStore — decode/downscale/hash/cache (image crate)"
```

---

### Task 2: Kitty escape builders + base64

**Files:** Create `src/kitty.rs`, `src/lib.rs`; Test `tests/kitty_tests.rs`.

- [ ] **Step 1: Write the failing test** (`tests/kitty_tests.rs`):

```rust
use tuiui::kitty::{b64, delete, place, transmit};

#[test]
fn base64_matches_known_vectors() {
    assert_eq!(b64(b""), "");
    assert_eq!(b64(b"M"), "TQ==");
    assert_eq!(b64(b"Ma"), "TWE=");
    assert_eq!(b64(b"Man"), "TWFu");
}

#[test]
fn place_and_delete_strings() {
    assert_eq!(place(7, 10, 4), "\x1b_Ga=p,i=7,c=10,r=4,q=2\x1b\\");
    assert_eq!(delete(7), "\x1b_Ga=d,d=i,i=7,q=2\x1b\\");
}

#[test]
fn transmit_one_chunk_for_small_payload() {
    // Small payloads fit a single chunk: m=0.
    let s = transmit(3, b"Man");
    assert_eq!(s, "\x1b_Gf=100,a=t,t=d,i=3,q=2,m=0;TWFu\x1b\\");
}

#[test]
fn transmit_chunks_large_payload() {
    // > 4096 base64 chars forces multiple m=1 chunks then a final m=0.
    let big = vec![0u8; 4096]; // 4096 bytes → ~5462 base64 chars → 2 chunks
    let s = transmit(1, &big);
    assert!(s.matches("\x1b_G").count() >= 2);
    assert!(s.contains("m=1"));
    assert!(s.trim_end().ends_with("\x1b\\"));
    // The last chunk must be m=0.
    assert!(s.contains("m=0;") || s.contains(",m=0;"));
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement `src/kitty.rs`**

```rust
//! Pure builders for the Kitty graphics protocol escape sequences used by the
//! client. No I/O — every function returns the exact bytes to write.

/// Standard base64 (RFC 4648) of `data`.
pub fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Transmit a PNG image with id `id` (chunked into ≤4096-char base64 pieces).
pub fn transmit(id: u64, png: &[u8]) -> String {
    let data = b64(png);
    let chunks: Vec<&str> = if data.is_empty() {
        vec![""]
    } else {
        // Split on char boundaries; base64 is ASCII so byte slicing is safe.
        data.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap()).collect()
    };
    let mut out = String::new();
    let n = chunks.len();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = if i + 1 < n { 1 } else { 0 };
        if i == 0 {
            out.push_str(&format!("\x1b_Gf=100,a=t,t=d,i={id},q=2,m={more};{chunk}\x1b\\"));
        } else {
            out.push_str(&format!("\x1b_Gm={more};{chunk}\x1b\\"));
        }
    }
    out
}

/// Place image `id` at the current cursor cell, scaled to `cols × rows` cells.
pub fn place(id: u64, cols: u16, rows: u16) -> String {
    format!("\x1b_Ga=p,i={id},c={cols},r={rows},q=2\x1b\\")
}

/// Delete all placements of image `id` (keeps the transmitted data for re-placing).
pub fn delete(id: u64) -> String {
    format!("\x1b_Ga=d,d=i,i={id},q=2\x1b\\")
}
```

Register in `src/lib.rs`: `pub mod kitty;`

- [ ] **Step 4: Run → PASS. Commit:**

```bash
git add src/kitty.rs src/lib.rs tests/kitty_tests.rs
git commit -m "image: Kitty graphics escape builders (transmit/place/delete) + base64"
```

---

### Task 3: Protocol types + `Caps.kitty_graphics`

**Files:** `src/protocol.rs`, `src/terminal.rs`; Test `tests/protocol_image_tests.rs`.

- [ ] **Step 1: Write the failing test** (`tests/protocol_image_tests.rs`):

```rust
use tuiui::geometry::Rect;
use tuiui::protocol::{ImageBlob, ImagePlacement};

#[test]
fn placement_round_trips() {
    let p = ImagePlacement { id: 9, rect: Rect::new(2, 1, 10, 4), cols: 10, rows: 4, visible: true };
    let s = serde_json::to_string(&p).unwrap();
    let back: ImagePlacement = serde_json::from_str(&s).unwrap();
    assert_eq!(back.id, 9);
    assert!(back.visible);
}

#[test]
fn blob_round_trips() {
    let b = ImageBlob { id: 9, png_base64: "TWFu".into() };
    let s = serde_json::to_string(&b).unwrap();
    let back: ImageBlob = serde_json::from_str(&s).unwrap();
    assert_eq!(back.png_base64, "TWFu");
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement** — in `src/protocol.rs` add (near `Flags`):

```rust
/// A request to place image `id` at `rect` (screen cells). `visible=false` tells
/// the client to remove the placement (occluded or closed).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ImagePlacement {
    pub id: u64,
    pub rect: crate::geometry::Rect,
    pub cols: u16,
    pub rows: u16,
    pub visible: bool,
}

/// PNG bytes for image `id`, base64-encoded, sent once per attach.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ImageBlob {
    pub id: u64,
    pub png_base64: String,
}
```

In the `FrameMsg` struct add (with skew tolerance):

```rust
    #[serde(default)]
    pub images: Vec<ImagePlacement>,
    #[serde(default)]
    pub image_data: Vec<ImageBlob>,
```

In `src/terminal.rs`, add to `Caps`:

```rust
    /// Whether the terminal supports the Kitty graphics protocol.
    pub kitty_graphics: bool,
```

and in `Caps::detect()`, after computing `pixel_mouse`:

```rust
        let kitty_graphics = matches!(term.as_str(), "kitty" | "WezTerm" | "ghostty");
        Caps { truecolor, pixel_mouse, kitty_graphics }
```

- [ ] **Step 4: Build the crate** (every `FrameMsg { … }` literal in `daemon.rs` needs the two new fields — set them in Task 5; for now add `images: Vec::new(), image_data: Vec::new()` to keep it compiling). Run `cargo test --offline --test protocol_image_tests` → PASS.

- [ ] **Step 5: Commit:**

```bash
git add src/protocol.rs src/terminal.rs tests/protocol_image_tests.rs
git commit -m "image: ImagePlacement/ImageBlob protocol types + Caps.kitty_graphics"
```

---

### Task 4: `ImageView` content + placeholder + `@image` launch

**Files:** Create `src/imageview.rs`, `src/lib.rs`, `src/session.rs`; Test `tests/imageview_tests.rs`.

- [ ] **Step 1: Write the failing test** (`tests/imageview_tests.rs`):

```rust
use tuiui::imageview::ImageView;

#[test]
fn placeholder_shows_filename() {
    let v = ImageView::new("/x/cat.png".into(), Some(7), (64, 48));
    let buf = v.render(30, 8);
    // The filename appears somewhere in the placeholder cells.
    let text: String = (0..buf.height())
        .flat_map(|y| (0..buf.width()).map(move |x| (x, y)))
        .filter_map(|(x, y)| buf.get(x, y).map(|c| c.ch))
        .collect();
    assert!(text.contains("cat.png"));
    assert_eq!(v.image_id(), Some(7));
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement `src/imageview.rs`**

```rust
//! A window that displays an image. It renders a cell **placeholder** (border +
//! filename + dimensions) — the universal fallback — and reports its `ImageId`
//! so the session can attach a Kitty graphics placement over the same rect.

use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::imagestore::ImageId;

pub struct ImageView {
    path: String,
    id: Option<ImageId>,
    dims: (u32, u32),
}

impl ImageView {
    pub fn new(path: String, id: Option<ImageId>, dims: (u32, u32)) -> Self {
        Self { path, id, dims }
    }

    pub fn image_id(&self) -> Option<ImageId> {
        self.id
    }

    pub fn render(&self, w: i32, h: i32) -> CellBuffer {
        let t = crate::theme::current();
        let mut buf = CellBuffer::new(w, h);
        let bg = t.window_bg;
        buf.fill(Cell { ch: ' ', fg: t.dim, bg, attrs: Default::default() });
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        let label = if self.id.is_some() {
            format!("🖼  {}  ({}×{})", name, self.dims.0, self.dims.1)
        } else {
            format!("cannot display  {name}")
        };
        let x = ((w - label.chars().count() as i32) / 2).max(0);
        let y = (h / 2).max(0);
        let fg = Rgba { r: 200, g: 208, b: 220, a: 255 };
        buf.write_str(x, y, &label, fg, bg);
        buf
    }
}
```

Register in `src/lib.rs`: `pub mod imageview;`

- [ ] **Step 4: Wire `WinContent::ImageView` + `@image` in `src/session.rs`**

Add the variant to `WinContent` and handle it in `render`/`resize`/`is_alive`/`kill` (an ImageView is always alive, never resizes a PTY):

```rust
    ImageView(crate::imageview::ImageView),
```

In `WinContent::render` add: `WinContent::ImageView(v) => v.render(w, h),`. In `is_alive`: `WinContent::ImageView(_) => true,`. `resize`/`kill`/`write_input`: no-op for `ImageView`.

Add an `ImageStore` field to `SessionCore` (`images: crate::imagestore::ImageStore`), init `ImageStore::new()` in `new`. Add an opener:

```rust
    /// Open an image file in a new ImageView window.
    fn open_image(&mut self, path: String) {
        let expanded = expand_tilde(&path);
        // Assume an 8×16 px cell to bound the decode to a screen-sized image.
        let id = self.images.load(&expanded, (self.w.max(1) as u32) * 8, (self.h.max(1) as u32) * 16);
        let dims = id.and_then(|i| self.images.dimensions(i)).unwrap_or((0, 0));
        let w = 60.min((self.w - 4).max(20));
        let h = 24.min((self.h - 4).max(8));
        let rect = Rect::new((self.w - w) / 2, 2, w, h);
        let win = self.wm.add_window(format!("image: {}", path.rsplit('/').next().unwrap_or(&path)), rect);
        self.contents.insert(win, WinContent::ImageView(crate::imageview::ImageView::new(path.clone(), id, dims)));
        self.titles.push((win, format!("image: {}", path.rsplit('/').next().unwrap_or(&path))));
    }
```

Route `@image` in `launch_entry`:

```rust
            "@image" => { if let Some(p) = e.args.first().cloned() { self.open_image(p); } }
```

- [ ] **Step 5: Run + build** → `cargo test --offline --test imageview_tests` PASS; `cargo build --offline` OK. **Commit:**

```bash
git add src/imageview.rs src/lib.rs src/session.rs tests/imageview_tests.rs
git commit -m "image: ImageView window + placeholder + @image launch hook"
```

---

### Task 5: Frame placements + occlusion + daemon blob fill

**Files:** `src/session.rs`, `src/daemon.rs`; Test `tests/session_tests.rs` (append).

- [ ] **Step 1: Write the failing test** (append to `tests/session_tests.rs`; uses the public `apply`/`build_frame`):

```rust
#[test]
fn image_window_emits_a_visible_placement() {
    let mut core = SessionCore::new(80, 24, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "@noop".into(), args: vec![] });
    // Open an image via the launcher entry path used by the store/launcher.
    core.apply(ClientMsg::OpenImage("/nonexistent/cat.png".into()));
    let frame = core.build_frame();
    // Even when decode fails (no file), a placement is attached with visible=false
    // OR none; a real image would be visible. Assert the field exists and is sane.
    assert!(frame.images.iter().all(|p| p.cols >= 1 && p.rows >= 1));
}
```

(Adjust: add a `ClientMsg::OpenImage(String)` variant routed to `open_image` so the test and the store can open images directly.)

- [ ] **Step 2: Run → FAIL** (`Frame.images`, `ClientMsg::OpenImage` missing).

- [ ] **Step 3: Implement**

Add `ClientMsg::OpenImage(String)` and route it: `ClientMsg::OpenImage(p) => self.open_image(p),`.

Add `images: Vec<ImagePlacement>` to the `Frame` struct. In `build_frame`, after the window loop, attach placements:

```rust
        let mut images = Vec::new();
        for w in self.wm.z_ordered() {
            if w.minimized { continue; }
            let id = match self.contents.get(&w.id) {
                Some(WinContent::ImageView(v)) => v.image_id(),
                _ => None,
            };
            if let Some(id) = id {
                let cr = w.content_rect();
                let visible = self.fully_unobstructed(&w);
                images.push(crate::protocol::ImagePlacement {
                    id, rect: cr, cols: cr.w.max(1) as u16, rows: cr.h.max(1) as u16, visible,
                });
            }
        }
        // ... existing chrome/launcher/tray/dirpicker/help layers ...
        Frame { layers, cursor: Some(self.cursor), images }
```

Add the occlusion helper + an image-bytes accessor:

```rust
    /// Whether `win`'s content rect is fully unobstructed by any higher window.
    fn fully_unobstructed(&self, win: &crate::window::Window) -> bool {
        let cr = win.content_rect();
        !self.wm.z_ordered().iter().any(|o| {
            !o.minimized && o.z > win.z && o.rect.intersect(cr).is_some()
        })
    }

    /// PNG bytes for an image id currently held in the store.
    pub fn image_png(&self, id: u64) -> Option<Vec<u8>> {
        self.images.png_bytes(id).map(|b| b.to_vec())
    }

    /// All current image placements (for the daemon's blob bookkeeping).
    pub fn image_placements(&self) -> Vec<crate::protocol::ImagePlacement> {
        self.build_frame().images
    }
```

In `src/daemon.rs::serve_client`, add `let mut sent_image_ids: std::collections::HashSet<u64> = HashSet::new();` (reset per client) and, when building `FrameMsg`, fill `image_data` for new visible ids:

```rust
        let images = frame.images.clone();
        let mut image_data = Vec::new();
        for p in &images {
            if p.visible && sent_image_ids.insert(p.id) {
                if let Some(png) = core.image_png(p.id) {
                    image_data.push(crate::protocol::ImageBlob { id: p.id, png_base64: crate::kitty::b64(&png) });
                }
            }
        }
        let msg = FrameMsg { changes, cursor: frame.cursor, flags, images, image_data };
```

(Use `frame.images` from the already-built `frame`; do not call `build_frame` twice.)

- [ ] **Step 4: Build + test** → `cargo test --offline` green.

- [ ] **Step 5: Commit:**

```bash
git add src/session.rs src/daemon.rs tests/session_tests.rs
git commit -m "image: frame placements + occlusion + daemon blob-once bookkeeping"
```

---

### Task 6: Client image reconcile

**Files:** `src/client.rs`.

- [ ] **Step 1: Implement** — in the reader thread (where `frame_to_ansi` is called), after writing the cell ANSI, reconcile images. Hold per-thread state:

```rust
            let mut transmitted: std::collections::HashSet<u64> = std::collections::HashSet::new();
            let mut active: std::collections::HashSet<u64> = std::collections::HashSet::new();
```

After `out.write_all(ansi.as_bytes())`:

```rust
                            if caps.kitty_graphics {
                                let mut g = String::new();
                                // Transmit any new blobs.
                                for blob in &msg.image_data {
                                    if transmitted.insert(blob.id) {
                                        // The daemon base64'd already; decode is unnecessary —
                                        // re-wrap as a transmit escape using the raw base64.
                                        g.push_str(&crate::kitty::transmit_b64(blob.id, &blob.png_base64));
                                    }
                                }
                                // Place visible, delete the rest.
                                let mut now = std::collections::HashSet::new();
                                g.push_str("\x1b[s"); // save cursor
                                for p in &msg.images {
                                    if p.visible {
                                        now.insert(p.id);
                                        g.push_str(&format!("\x1b[{};{}H", p.rect.y + 1, p.rect.x + 1));
                                        g.push_str(&crate::kitty::place(p.id, p.cols, p.rows));
                                    }
                                }
                                for id in active.difference(&now) {
                                    g.push_str(&crate::kitty::delete(*id));
                                }
                                g.push_str("\x1b[u"); // restore cursor
                                active = now;
                                let _ = out.write_all(g.as_bytes());
                            }
```

Add to `src/kitty.rs` a transmit variant that takes already-base64'd data (the daemon encodes it, so the client must not double-encode):

```rust
/// Transmit a PNG whose bytes are already base64-encoded (chunked).
pub fn transmit_b64(id: u64, data_b64: &str) -> String {
    let chunks: Vec<&str> = if data_b64.is_empty() {
        vec![""]
    } else {
        data_b64.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap()).collect()
    };
    let mut out = String::new();
    let n = chunks.len();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = if i + 1 < n { 1 } else { 0 };
        if i == 0 {
            out.push_str(&format!("\x1b_Gf=100,a=t,t=d,i={id},q=2,m={more};{chunk}\x1b\\"));
        } else {
            out.push_str(&format!("\x1b_Gm={more};{chunk}\x1b\\"));
        }
    }
    out
}
```

Refactor `transmit` to call `transmit_b64(id, &b64(png))` to keep one chunking path, and add a `kitty_tests` case asserting `transmit(3, b"Man") == transmit_b64(3, "TWFu")`.

- [ ] **Step 2: Build + test** → green; `cargo clippy --offline --all-targets` → 0 warnings.

- [ ] **Step 3: Commit:**

```bash
git add src/client.rs src/kitty.rs tests/kitty_tests.rs
git commit -m "image: client reconcile (transmit-once / place / delete) gated on caps"
```

---

### Task 7: Final verification + manual smoke

- [ ] `cargo build --offline && cargo clippy --offline --all-targets && cargo test --offline` → builds, 0 warnings, all pass.
- [ ] Reinstall: `cargo install --path . --root ~/.local --force`; on the mini `tuiui kill ; tuiui`.
- [ ] In Ghostty: add a launcher entry `[[launcher]] name="Photo" command="@image" args=["~/Pictures/some.png"]` (or trigger `OpenImage`), open it → the photo renders in the window; move the window → it follows; cover it with another window → it hides (placeholder shows); close it → it disappears.
- [ ] Commit any fixups.

## Notes for the implementer
- The daemon base64-encodes blobs; the client must use `transmit_b64` (no double-encode).
- `q=2` on every escape suppresses terminal replies that would otherwise corrupt stdin.
- Save/restore the cursor (`\x1b[s` / `\x1b[u`) around image ops so cell rendering is unaffected.
- Occlusion is all-or-nothing in v1 (`fully_unobstructed`); partial clipping is a later enhancement.
