# Tuiui Slice 1 — Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A single Rust binary `tuiui` that renders a floating-window terminal desktop — a top menubar + bottom dock, with 2–3 bundled terminal apps each running in its own PTY inside a mouse-draggable, resizable, snappable window.

**Architecture:** Pure logic core (geometry, cells, compositor, window manager, input routing) with zero I/O, wrapped by I/O adapters (a `crossterm` terminal backend and a `portable-pty` + `vt100` process host). A `SessionCore` owns the window-manager state and app instances and talks to the front-end render loop through an in-process `ClientMsg`/`CoreMsg` protocol designed to later cross a socket.

**Tech Stack:** Rust 2021, `crossterm` (terminal I/O + input), `portable-pty` (child PTYs), `vt100` (parse child output), `toml` + `serde` (read-only config), `dirs` (config path). Custom double-buffered cell compositor (no external compositor crate in Slice 1).

---

## File Structure

```
Cargo.toml
src/
  main.rs           # entry: parse args, build backend+core, run loop, teardown
  lib.rs            # `pub mod` declarations so integration tests can import
  geometry.rs       # Point, Rect, snapping math — PURE
  cell.rs           # Rgba (+ alpha `over`), CellAttrs, Cell — PURE
  buffer.rs         # CellBuffer: sized grid of cells, get/set, fill — PURE
  compositor.rs     # Layer, Compositor: composite layers, diff frames, cursor — PURE
  terminal.rs       # TerminalBackend trait, CrosstermBackend, Caps, frame->ANSI writer
  ptyhost.rs        # AppInstance: spawn PTY, vt100 parse, grid snapshot, resize, write
  window.rs         # WindowId, WindowState, Window — PURE
  wm.rs             # WindowManager: add/focus/raise/move/resize/snap — PURE
  chrome.rs         # render menubar + dock into layers; dock hit regions — PURE
  input.rs          # RawEvent -> Action (WM action | ForwardToApp) + coord translation — PURE
  session.rs        # SessionCore, ClientMsg, CoreMsg — owns wm + apps
  config.rs         # Config struct, read ~/.config/tuiui/config.toml, defaults
tests/
  geometry_tests.rs
  cell_tests.rs
  buffer_tests.rs
  compositor_tests.rs
  ptyhost_tests.rs
  wm_tests.rs
  input_tests.rs
  chrome_tests.rs
  config_tests.rs
```

Pure modules (`geometry`, `cell`, `buffer`, `compositor`, `window`, `wm`, `chrome`, `input`) are unit-tested in full. I/O modules (`terminal`, `ptyhost`) are tested behind trait seams / with scripted child processes. `main.rs` is wired last and smoke-tested manually.

**Branch:** do all work on a branch `slice-1-shell`. Commit after every task.

---

## Task 0: Project scaffold

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `src/main.rs`
- Test: `tests/smoke_tests.rs`

- [ ] **Step 1: Create branch**

Run: `git checkout -b slice-1-shell`

- [ ] **Step 2: Write `Cargo.toml`**

```toml
[package]
name = "tuiui"
version = "0.1.0"
edition = "2021"

[dependencies]
crossterm = "0.28"
portable-pty = "0.8"
vt100 = "0.15"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
dirs = "5"

[lib]
name = "tuiui"
path = "src/lib.rs"

[[bin]]
name = "tuiui"
path = "src/main.rs"
```

- [ ] **Step 3: Write `src/lib.rs`**

```rust
pub mod geometry;
pub mod cell;
pub mod buffer;
pub mod compositor;
pub mod terminal;
pub mod ptyhost;
pub mod window;
pub mod wm;
pub mod chrome;
pub mod input;
pub mod session;
pub mod config;
```

- [ ] **Step 4: Write placeholder modules so it compiles**

Create each `src/<mod>.rs` listed above as an empty file for now (tasks fill them). Write `src/main.rs`:

```rust
fn main() {
    println!("tuiui");
}
```

- [ ] **Step 5: Write `tests/smoke_tests.rs`**

```rust
#[test]
fn crate_builds() {
    assert_eq!(2 + 2, 4);
}
```

- [ ] **Step 6: Verify it builds and tests pass**

Run: `cargo test`
Expected: compiles, `crate_builds` passes. (Empty modules are fine.)

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "Scaffold tuiui crate: deps, module layout, smoke test"
```

---

## Task 1: Geometry (`geometry.rs`)

Pure `Point`/`Rect` and the snapping math. Cell coordinates; `i32` so windows can drag partly off-screen.

**Files:**
- Modify: `src/geometry.rs`
- Test: `tests/geometry_tests.rs`

- [ ] **Step 1: Write failing tests** (`tests/geometry_tests.rs`)

```rust
use tuiui::geometry::{Point, Rect, SnapZone, snap_zone, snapped_rect};

#[test]
fn contains_point() {
    let r = Rect::new(2, 3, 10, 5);
    assert!(r.contains(Point::new(2, 3)));
    assert!(r.contains(Point::new(11, 7)));
    assert!(!r.contains(Point::new(12, 7)));
    assert!(!r.contains(Point::new(1, 3)));
}

#[test]
fn right_bottom_edges() {
    let r = Rect::new(0, 0, 4, 3);
    assert_eq!(r.right(), 3);
    assert_eq!(r.bottom(), 2);
}

#[test]
fn snap_zone_detects_left_right_within_threshold() {
    let screen = Rect::new(0, 0, 80, 24);
    assert_eq!(snap_zone(Point::new(2, 10), screen, 3), Some(SnapZone::Left));
    assert_eq!(snap_zone(Point::new(78, 10), screen, 3), Some(SnapZone::Right));
    assert_eq!(snap_zone(Point::new(40, 10), screen, 3), None);
}

#[test]
fn snapped_rect_left_is_left_half_below_menubar_above_dock() {
    // work area excludes 1-row menubar (top) and 1-row dock (bottom)
    let work = Rect::new(0, 1, 80, 22);
    let left = snapped_rect(SnapZone::Left, work);
    assert_eq!(left, Rect::new(0, 1, 40, 22));
    let right = snapped_rect(SnapZone::Right, work);
    assert_eq!(right, Rect::new(40, 1, 40, 22));
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test geometry_tests`
Expected: FAIL (unresolved imports).

- [ ] **Step 3: Implement `src/geometry.rs`**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point { pub x: i32, pub y: i32 }

impl Point {
    pub fn new(x: i32, y: i32) -> Self { Self { x, y } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self { Self { x, y, w, h } }
    pub fn right(&self) -> i32 { self.x + self.w - 1 }
    pub fn bottom(&self) -> i32 { self.y + self.h - 1 }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    /// Intersection, or None if disjoint.
    pub fn intersect(&self, o: Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r < x || b < y { None } else { Some(Rect::new(x, y, r - x + 1, b - y + 1)) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapZone { Left, Right }

/// Returns a snap zone if `p` is within `threshold` cells of the left/right screen edge.
pub fn snap_zone(p: Point, screen: Rect, threshold: i32) -> Option<SnapZone> {
    if p.x <= screen.x + threshold - 1 { Some(SnapZone::Left) }
    else if p.x >= screen.right() - threshold + 1 { Some(SnapZone::Right) }
    else { None }
}

/// The rect a window takes when snapped, given the usable work area.
pub fn snapped_rect(zone: SnapZone, work: Rect) -> Rect {
    let half = work.w / 2;
    match zone {
        SnapZone::Left => Rect::new(work.x, work.y, half, work.h),
        SnapZone::Right => Rect::new(work.x + half, work.y, work.w - half, work.h),
    }
}
```

- [ ] **Step 4: Run, expect pass**

Run: `cargo test --test geometry_tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "Add geometry: Point, Rect, snapping math"
```

---

## Task 2: Cells & color (`cell.rs`)

**Files:**
- Modify: `src/cell.rs`
- Test: `tests/cell_tests.rs`

- [ ] **Step 1: Write failing tests** (`tests/cell_tests.rs`)

```rust
use tuiui::cell::{Rgba, Cell, CellAttrs};

#[test]
fn opaque_over_keeps_src() {
    let dst = Rgba::rgb(0, 0, 0);
    let src = Rgba::rgb(255, 0, 0);
    assert_eq!(src.over(dst), Rgba::rgb(255, 0, 0));
}

#[test]
fn half_alpha_blends_midway() {
    let dst = Rgba::rgb(0, 0, 0);
    let src = Rgba::new(255, 255, 255, 128);
    let out = src.over(dst);
    // 255 * 128/255 + 0 ≈ 128
    assert_eq!(out, Rgba::rgb(128, 128, 128));
}

#[test]
fn transparent_over_keeps_dst() {
    let dst = Rgba::rgb(10, 20, 30);
    let src = Rgba::new(255, 255, 255, 0);
    assert_eq!(src.over(dst), dst);
}

#[test]
fn default_cell_is_blank_space() {
    let c = Cell::default();
    assert_eq!(c.ch, ' ');
    assert_eq!(c.attrs, CellAttrs::default());
}
```

- [ ] **Step 2: Run, expect failure.** Run: `cargo test --test cell_tests`

- [ ] **Step 3: Implement `src/cell.rs`**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba { pub r: u8, pub g: u8, pub b: u8, pub a: u8 }

impl Rgba {
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self { Self { r, g, b, a } }
    pub fn rgb(r: u8, g: u8, b: u8) -> Self { Self { r, g, b, a: 255 } }
    pub const TRANSPARENT: Rgba = Rgba { r: 0, g: 0, b: 0, a: 0 };

    /// Porter-Duff "over": self composited on top of `dst`. Result is opaque.
    pub fn over(self, dst: Rgba) -> Rgba {
        if self.a == 255 { return Rgba::rgb(self.r, self.g, self.b); }
        if self.a == 0 { return Rgba::rgb(dst.r, dst.g, dst.b); }
        let a = self.a as u32;
        let inv = 255 - a;
        let mix = |s: u8, d: u8| -> u8 { ((s as u32 * a + d as u32 * inv) / 255) as u8 };
        Rgba::rgb(mix(self.r, dst.r), mix(self.g, dst.g), mix(self.b, dst.b))
    }
    /// Multiply this color's alpha by `opacity` (0.0–1.0).
    pub fn with_opacity(self, opacity: f32) -> Rgba {
        let a = (self.a as f32 * opacity).round().clamp(0.0, 255.0) as u8;
        Rgba::new(self.r, self.g, self.b, a)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CellAttrs { pub bold: bool, pub italic: bool, pub underline: bool, pub inverse: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell { pub ch: char, pub fg: Rgba, pub bg: Rgba, pub attrs: CellAttrs }

impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', fg: Rgba::rgb(200, 208, 220), bg: Rgba::TRANSPARENT, attrs: CellAttrs::default() }
    }
}
```

- [ ] **Step 4: Run, expect pass.** Run: `cargo test --test cell_tests`

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add cell + Rgba alpha blending"`

---

## Task 3: Cell buffer (`buffer.rs`)

**Files:**
- Modify: `src/buffer.rs`
- Test: `tests/buffer_tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
use tuiui::buffer::CellBuffer;
use tuiui::cell::{Cell, Rgba};

#[test]
fn new_buffer_is_default_filled() {
    let b = CellBuffer::new(4, 2);
    assert_eq!(b.width(), 4);
    assert_eq!(b.height(), 2);
    assert_eq!(b.get(0, 0).unwrap().ch, ' ');
}

#[test]
fn set_and_get_roundtrip() {
    let mut b = CellBuffer::new(3, 3);
    let mut c = Cell::default();
    c.ch = 'X';
    c.bg = Rgba::rgb(1, 2, 3);
    b.set(1, 2, c);
    assert_eq!(b.get(1, 2).unwrap().ch, 'X');
    assert_eq!(b.get(1, 2).unwrap().bg, Rgba::rgb(1, 2, 3));
}

#[test]
fn out_of_bounds_is_none() {
    let b = CellBuffer::new(2, 2);
    assert!(b.get(2, 0).is_none());
    assert!(b.get(0, 2).is_none());
}

#[test]
fn write_str_sets_consecutive_cells() {
    let mut b = CellBuffer::new(6, 1);
    b.write_str(1, 0, "hi", Rgba::rgb(255,255,255), Rgba::TRANSPARENT);
    assert_eq!(b.get(1, 0).unwrap().ch, 'h');
    assert_eq!(b.get(2, 0).unwrap().ch, 'i');
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/buffer.rs`**

```rust
use crate::cell::{Cell, Rgba};

#[derive(Clone, Debug)]
pub struct CellBuffer { w: i32, h: i32, cells: Vec<Cell> }

impl CellBuffer {
    pub fn new(w: i32, h: i32) -> Self {
        let (w, h) = (w.max(0), h.max(0));
        Self { w, h, cells: vec![Cell::default(); (w * h) as usize] }
    }
    pub fn width(&self) -> i32 { self.w }
    pub fn height(&self) -> i32 { self.h }
    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h { None }
        else { Some((y * self.w + x) as usize) }
    }
    pub fn get(&self, x: i32, y: i32) -> Option<&Cell> { self.idx(x, y).map(|i| &self.cells[i]) }
    pub fn set(&mut self, x: i32, y: i32, c: Cell) { if let Some(i) = self.idx(x, y) { self.cells[i] = c; } }
    pub fn fill(&mut self, c: Cell) { for cell in &mut self.cells { *cell = c; } }
    pub fn write_str(&mut self, x: i32, y: i32, s: &str, fg: Rgba, bg: Rgba) {
        for (i, ch) in s.chars().enumerate() {
            self.set(x + i as i32, y, Cell { ch, fg, bg, attrs: Default::default() });
        }
    }
}
```

- [ ] **Step 4: Run, expect pass.**

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add CellBuffer grid"`

---

## Task 4: Compositor (`compositor.rs`)

Composites z-ordered `Layer`s onto a base buffer (alpha-aware), overlays a cursor glyph, and diffs successive frames into a minimal change list.

**Files:**
- Modify: `src/compositor.rs`
- Test: `tests/compositor_tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
use tuiui::compositor::{Compositor, Layer, CellChange};
use tuiui::buffer::CellBuffer;
use tuiui::cell::{Cell, Rgba};
use tuiui::geometry::Point;

fn glyph(ch: char, bg: Rgba) -> Cell { Cell { ch, fg: Rgba::rgb(255,255,255), bg, attrs: Default::default() } }

#[test]
fn higher_z_layer_wins_glyph() {
    let mut comp = Compositor::new(4, 1);
    let mut low = CellBuffer::new(4, 1); low.set(0,0, glyph('A', Rgba::rgb(10,10,10)));
    let mut high = CellBuffer::new(4, 1); high.set(0,0, glyph('B', Rgba::rgb(20,20,20)));
    let frame = comp.composite(&[
        Layer { z: 0, origin: Point::new(0,0), buf: low, opacity: 1.0, scissor: None },
        Layer { z: 5, origin: Point::new(0,0), buf: high, opacity: 1.0, scissor: None },
    ], None);
    assert_eq!(frame.get(0,0).unwrap().ch, 'B');
}

#[test]
fn transparent_bg_shows_lower_layer_through() {
    let mut comp = Compositor::new(1, 1);
    let mut low = CellBuffer::new(1,1); low.set(0,0, glyph('A', Rgba::rgb(0,0,0)));
    // shadow: space cell, semi-transparent black bg
    let mut shadow = CellBuffer::new(1,1); shadow.set(0,0, Cell { ch:' ', fg: Rgba::TRANSPARENT, bg: Rgba::new(0,0,0,128), attrs: Default::default() });
    let frame = comp.composite(&[
        Layer { z:0, origin: Point::new(0,0), buf: low, opacity:1.0, scissor: None },
        Layer { z:1, origin: Point::new(0,0), buf: shadow, opacity:1.0, scissor: None },
    ], None);
    // glyph 'A' preserved (shadow has no glyph), bg darkened toward black
    assert_eq!(frame.get(0,0).unwrap().ch, 'A');
}

#[test]
fn cursor_overlays_inverse() {
    let mut comp = Compositor::new(2,1);
    let base = CellBuffer::new(2,1);
    let frame = comp.composite(&[Layer{z:0,origin:Point::new(0,0),buf:base,opacity:1.0,scissor:None}], Some(Point::new(1,0)));
    assert!(frame.get(1,0).unwrap().attrs.inverse);
    assert!(!frame.get(0,0).unwrap().attrs.inverse);
}

#[test]
fn diff_reports_only_changed_cells() {
    let mut comp = Compositor::new(2,1);
    let base = || CellBuffer::new(2,1);
    let l0 = Layer{z:0,origin:Point::new(0,0),buf:base(),opacity:1.0,scissor:None};
    let _ = comp.composite(&[l0], None); // first frame: everything "changed"
    comp.commit();
    let mut b2 = CellBuffer::new(2,1); b2.set(1,0, glyph('Z', Rgba::rgb(0,0,0)));
    let _ = comp.composite(&[Layer{z:0,origin:Point::new(0,0),buf:b2,opacity:1.0,scissor:None}], None);
    let changes: Vec<CellChange> = comp.diff();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].x, 1);
    assert_eq!(changes[0].cell.ch, 'Z');
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/compositor.rs`**

```rust
use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::geometry::{Point, Rect};

pub struct Layer {
    pub z: i32,
    pub origin: Point,
    pub buf: CellBuffer,
    pub opacity: f32,
    pub scissor: Option<Rect>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellChange { pub x: i32, pub y: i32, pub cell: Cell }

/// Desktop background color (composited under everything).
const DESKTOP_BG: Rgba = Rgba { r: 18, g: 20, b: 28, a: 255 };

pub struct Compositor {
    w: i32, h: i32,
    front: CellBuffer,   // last committed frame (what the terminal shows)
    back: CellBuffer,    // most recent composite
}

impl Compositor {
    pub fn new(w: i32, h: i32) -> Self {
        Self { w, h, front: CellBuffer::new(w, h), back: CellBuffer::new(w, h) }
    }
    pub fn resize(&mut self, w: i32, h: i32) {
        self.w = w; self.h = h;
        self.front = CellBuffer::new(w, h);
        self.back = CellBuffer::new(w, h);
    }
    pub fn width(&self) -> i32 { self.w }
    pub fn height(&self) -> i32 { self.h }

    /// Composite layers (any order; sorted by z here) plus optional cursor into the back buffer.
    pub fn composite(&mut self, layers: &[Layer], cursor: Option<Point>) -> &CellBuffer {
        // base
        let base = Cell { ch: ' ', fg: Rgba::rgb(90,100,120), bg: DESKTOP_BG, attrs: Default::default() };
        self.back.fill(base);

        let mut order: Vec<&Layer> = layers.iter().collect();
        order.sort_by_key(|l| l.z);

        for layer in order {
            for ly in 0..layer.buf.height() {
                for lx in 0..layer.buf.width() {
                    let gx = layer.origin.x + lx;
                    let gy = layer.origin.y + ly;
                    if gx < 0 || gy < 0 || gx >= self.w || gy >= self.h { continue; }
                    if let Some(s) = layer.scissor {
                        if !s.contains(Point::new(gx, gy)) { continue; }
                    }
                    let src = *layer.buf.get(lx, ly).unwrap();
                    let dst = *self.back.get(gx, gy).unwrap();
                    self.back.set(gx, gy, blend_cell(src, dst, layer.opacity));
                }
            }
        }

        if let Some(p) = cursor {
            if let Some(c) = self.back.get(p.x, p.y) {
                let mut c = *c; c.attrs.inverse = !c.attrs.inverse;
                self.back.set(p.x, p.y, c);
            }
        }
        &self.back
    }

    /// Cells that differ between front and back (the minimal terminal update).
    pub fn diff(&self) -> Vec<CellChange> {
        let mut out = Vec::new();
        for y in 0..self.h {
            for x in 0..self.w {
                let b = self.back.get(x, y).unwrap();
                let f = self.front.get(x, y).unwrap();
                if b != f { out.push(CellChange { x, y, cell: *b }); }
            }
        }
        out
    }

    /// Promote back -> front after the diff has been written to the terminal.
    pub fn commit(&mut self) { self.front = self.back.clone(); }
}

/// Composite `src` over `dst`, applying layer opacity to src's alpha.
fn blend_cell(src: Cell, dst: Cell, opacity: f32) -> Cell {
    let src_bg = src.bg.with_opacity(opacity);
    let out_bg = src_bg.over(dst.bg);
    let glyph_present = src.ch != ' ' && src_bg.a >= 8;
    if glyph_present {
        Cell { ch: src.ch, fg: src.fg.over(out_bg), bg: out_bg, attrs: src.attrs }
    } else {
        // no glyph: keep dst glyph/fg, only bg changes (shadows, tints)
        Cell { ch: dst.ch, fg: dst.fg, bg: out_bg, attrs: dst.attrs }
    }
}
```

- [ ] **Step 4: Run, expect pass.**

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add compositor: z-order, alpha blend, cursor, frame diff"`

---

## Task 5: Window model (`window.rs`)

**Files:**
- Modify: `src/window.rs`
- Test: covered via `wm_tests.rs` in Task 6 (no separate test file).

- [ ] **Step 1: Implement `src/window.rs`**

```rust
use crate::geometry::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState { Floating, SnappedLeft, SnappedRight }

#[derive(Clone, Debug)]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    pub rect: Rect,                 // outer rect incl. 1-row titlebar + 1-col borders
    pub z: i32,
    pub state: WindowState,
    pub restore_rect: Rect,         // rect to return to when un-snapping
}

impl Window {
    /// Inner content rect (excludes titlebar row + left/right/bottom border).
    pub fn content_rect(&self) -> Rect {
        Rect::new(self.rect.x + 1, self.rect.y + 1, (self.rect.w - 2).max(0), (self.rect.h - 2).max(0))
    }
    pub fn titlebar_rect(&self) -> Rect {
        Rect::new(self.rect.x, self.rect.y, self.rect.w, 1)
    }
}
```

- [ ] **Step 2: Verify it compiles.** Run: `cargo build`

- [ ] **Step 3: Commit.** `git add -A && git commit -m "Add window model"`

---

## Task 6: Window manager (`wm.rs`)

Owns windows, focus, z-order, and the move/resize/snap operations. Pure — no I/O.

**Files:**
- Modify: `src/wm.rs`
- Test: `tests/wm_tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
use tuiui::wm::WindowManager;
use tuiui::window::{WindowState};
use tuiui::geometry::{Rect, Point, SnapZone};

fn wm() -> WindowManager { WindowManager::new(Rect::new(0,1,80,22)) } // work area

#[test]
fn add_window_focuses_it_and_assigns_top_z() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2,2,20,8));
    let b = m.add_window("b".into(), Rect::new(5,5,20,8));
    assert_eq!(m.focused(), Some(b));
    assert!(m.get(b).unwrap().z > m.get(a).unwrap().z);
}

#[test]
fn raise_brings_to_front_and_focuses() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2,2,20,8));
    let _b = m.add_window("b".into(), Rect::new(5,5,20,8));
    m.raise(a);
    assert_eq!(m.focused(), Some(a));
    assert_eq!(m.topmost_at(Point::new(6,6)), Some(a)); // a now covers overlap
}

#[test]
fn topmost_at_returns_highest_z_window_under_point() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(0,1,20,8));
    assert_eq!(m.topmost_at(Point::new(1,2)), Some(a));
    assert_eq!(m.topmost_at(Point::new(79,20)), None);
}

#[test]
fn move_by_translates_rect() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2,2,20,8));
    m.move_by(a, 3, 1);
    assert_eq!(m.get(a).unwrap().rect, Rect::new(5,3,20,8));
}

#[test]
fn snap_left_sets_state_and_left_half_and_saves_restore() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(10,5,20,8));
    m.snap(a, SnapZone::Left);
    let w = m.get(a).unwrap();
    assert_eq!(w.state, WindowState::SnappedLeft);
    assert_eq!(w.rect, Rect::new(0,1,40,22));
    assert_eq!(w.restore_rect, Rect::new(10,5,20,8));
}

#[test]
fn resize_to_enforces_minimum() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2,2,20,8));
    m.resize_to(a, 1, 1); // below min
    let w = m.get(a).unwrap();
    assert!(w.rect.w >= 8 && w.rect.h >= 3);
}

#[test]
fn close_removes_and_refocuses_next_top() {
    let mut m = wm();
    let a = m.add_window("a".into(), Rect::new(2,2,20,8));
    let b = m.add_window("b".into(), Rect::new(5,5,20,8));
    m.close(b);
    assert!(m.get(b).is_none());
    assert_eq!(m.focused(), Some(a));
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/wm.rs`**

```rust
use crate::geometry::{Rect, Point, SnapZone, snapped_rect};
use crate::window::{Window, WindowId, WindowState};

pub const MIN_W: i32 = 8;
pub const MIN_H: i32 = 3;

pub struct WindowManager {
    work: Rect,
    windows: Vec<Window>,     // unordered; z is the truth for stacking
    focus: Option<WindowId>,
    next_id: u64,
    next_z: i32,
}

impl WindowManager {
    pub fn new(work: Rect) -> Self {
        Self { work, windows: Vec::new(), focus: None, next_id: 1, next_z: 1 }
    }
    pub fn work_area(&self) -> Rect { self.work }
    pub fn set_work_area(&mut self, r: Rect) { self.work = r; }

    pub fn add_window(&mut self, title: String, rect: Rect) -> WindowId {
        let id = WindowId(self.next_id); self.next_id += 1;
        let z = self.next_z; self.next_z += 1;
        self.windows.push(Window { id, title, rect, z, state: WindowState::Floating, restore_rect: rect });
        self.focus = Some(id);
        id
    }
    pub fn get(&self, id: WindowId) -> Option<&Window> { self.windows.iter().find(|w| w.id == id) }
    fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> { self.windows.iter_mut().find(|w| w.id == id) }
    pub fn focused(&self) -> Option<WindowId> { self.focus }
    /// Windows ordered bottom -> top (for rendering).
    pub fn z_ordered(&self) -> Vec<&Window> {
        let mut v: Vec<&Window> = self.windows.iter().collect();
        v.sort_by_key(|w| w.z);
        v
    }
    pub fn topmost_at(&self, p: Point) -> Option<WindowId> {
        self.windows.iter().filter(|w| w.rect.contains(p)).max_by_key(|w| w.z).map(|w| w.id)
    }
    pub fn raise(&mut self, id: WindowId) {
        let z = self.next_z; self.next_z += 1;
        if let Some(w) = self.get_mut(id) { w.z = z; }
        self.focus = Some(id);
    }
    pub fn move_by(&mut self, id: WindowId, dx: i32, dy: i32) {
        if let Some(w) = self.get_mut(id) {
            w.rect.x += dx; w.rect.y += dy;
            if w.state != WindowState::Floating { w.state = WindowState::Floating; }
        }
    }
    pub fn resize_to(&mut self, id: WindowId, w_new: i32, h_new: i32) {
        if let Some(win) = self.get_mut(id) {
            win.rect.w = w_new.max(MIN_W);
            win.rect.h = h_new.max(MIN_H);
            win.state = WindowState::Floating;
        }
    }
    pub fn snap(&mut self, id: WindowId, zone: SnapZone) {
        let work = self.work;
        if let Some(w) = self.get_mut(id) {
            if w.state == WindowState::Floating { w.restore_rect = w.rect; }
            w.rect = snapped_rect(zone, work);
            w.state = match zone { SnapZone::Left => WindowState::SnappedLeft, SnapZone::Right => WindowState::SnappedRight };
        }
    }
    pub fn close(&mut self, id: WindowId) {
        self.windows.retain(|w| w.id != id);
        if self.focus == Some(id) {
            self.focus = self.windows.iter().max_by_key(|w| w.z).map(|w| w.id);
        }
    }
}
```

- [ ] **Step 4: Run, expect pass.**

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add window manager: focus, z-order, move/resize/snap"`

---

## Task 7: Config (`config.rs`)

**Files:**
- Modify: `src/config.rs`
- Test: `tests/config_tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
use tuiui::config::Config;

#[test]
fn defaults_are_sane() {
    let c = Config::default();
    assert!(c.snapping_enabled);
    assert_eq!(c.snap_threshold, 3);
    assert!(!c.apps.is_empty());
}

#[test]
fn parses_toml_overrides() {
    let toml = r#"
snapping_enabled = false
snap_threshold = 5
[[apps]]
name = "shell"
command = "bash"
"#;
    let c = Config::from_toml_str(toml).unwrap();
    assert!(!c.snapping_enabled);
    assert_eq!(c.snap_threshold, 5);
    assert_eq!(c.apps.len(), 1);
    assert_eq!(c.apps[0].command, "bash");
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/config.rs`**

```rust
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct AppEntry {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub snapping_enabled: bool,
    pub snap_threshold: i32,
    pub apps: Vec<AppEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            snapping_enabled: true,
            snap_threshold: 3,
            apps: vec![
                AppEntry { name: "shell".into(), command: default_shell(), args: vec![] },
            ],
        }
    }
}

fn default_shell() -> String { std::env::var("SHELL").unwrap_or_else(|_| "bash".into()) }

impl Config {
    pub fn from_toml_str(s: &str) -> Result<Config, toml::de::Error> { toml::from_str(s) }

    /// Load from ~/.config/tuiui/config.toml, falling back to defaults on any error.
    pub fn load() -> Config {
        let path = dirs::config_dir().map(|d| d.join("tuiui").join("config.toml"));
        if let Some(p) = path {
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(cfg) = Config::from_toml_str(&text) { return cfg; }
            }
        }
        Config::default()
    }
}
```

- [ ] **Step 4: Run, expect pass.**

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add read-only config loader"`

---

## Task 8: Chrome (`chrome.rs`)

Renders the menubar (top row) and dock (bottom row) into compositor layers, and reports dock hit regions.

**Files:**
- Modify: `src/chrome.rs`
- Test: `tests/chrome_tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
use tuiui::chrome::{render_menubar, render_dock, dock_hit_regions, DockItem};
use tuiui::geometry::Point;
use tuiui::window::WindowId;

#[test]
fn menubar_layer_spans_top_row_and_shows_brand() {
    let layer = render_menubar(20, "btop");
    assert_eq!(layer.origin, Point::new(0,0));
    assert_eq!(layer.buf.height(), 1);
    let row: String = (0..20).map(|x| layer.buf.get(x,0).unwrap().ch).collect();
    assert!(row.contains("Tuiui"));
    assert!(row.contains("btop"));
}

#[test]
fn dock_layer_is_bottom_row() {
    let items = vec![DockItem { id: WindowId(1), label: "btop".into(), focused: true }];
    let layer = render_dock(40, 24, &items);
    assert_eq!(layer.origin, Point::new(0, 23));
}

#[test]
fn dock_hit_regions_map_clicks_to_windows() {
    let items = vec![
        DockItem { id: WindowId(1), label: "btop".into(), focused: true },
        DockItem { id: WindowId(2), label: "lazygit".into(), focused: false },
    ];
    let regions = dock_hit_regions(40, 24, &items);
    // a click inside the first region resolves to WindowId(1)
    let first = regions.iter().find(|(_, r)| r.contains(Point::new(r.x, 23))).map(|(id,_)| *id);
    assert_eq!(first, Some(WindowId(1)));
    // regions are on the bottom row
    assert!(regions.iter().all(|(_, r)| r.y == 23));
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/chrome.rs`**

```rust
use crate::buffer::CellBuffer;
use crate::cell::Rgba;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};
use crate::window::WindowId;

const MENUBAR_BG: Rgba = Rgba { r: 22, g: 27, b: 39, a: 255 };
const DOCK_BG: Rgba    = Rgba { r: 22, g: 27, b: 39, a: 255 };
const TEXT: Rgba       = Rgba { r: 200, g: 208, b: 220, a: 255 };
const BRAND: Rgba      = Rgba { r: 108, g: 182, b: 255, a: 255 };
const ACTIVE_BG: Rgba  = Rgba { r: 45, g: 58, b: 85, a: 255 };

pub struct DockItem { pub id: WindowId, pub label: String, pub focused: bool }

pub fn render_menubar(width: i32, focused_app: &str) -> Layer {
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: TEXT, bg: MENUBAR_BG, attrs: Default::default() });
    buf.write_str(1, 0, "✦ Tuiui", BRAND, MENUBAR_BG);
    buf.write_str(10, 0, focused_app, TEXT, MENUBAR_BG);
    Layer { z: 1000, origin: Point::new(0, 0), buf, opacity: 1.0, scissor: None }
}

/// Layout: each dock item is " label " padded, separated by one space, left-aligned from x=1.
fn dock_layout(items: &[DockItem]) -> Vec<(WindowId, Rect, String)> {
    let mut out = Vec::new();
    let mut x = 1;
    for it in items {
        let label = format!(" {} ", it.label);
        let w = label.chars().count() as i32;
        out.push((it.id, Rect::new(x, 0, w, 1), label));
        x += w + 1;
    }
    out
}

pub fn render_dock(width: i32, height: i32, items: &[DockItem]) -> Layer {
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: TEXT, bg: DOCK_BG, attrs: Default::default() });
    for (i, (_, r, label)) in dock_layout(items).into_iter().enumerate() {
        let bg = if items[i].focused { ACTIVE_BG } else { DOCK_BG };
        buf.write_str(r.x, 0, &label, TEXT, bg);
    }
    Layer { z: 1000, origin: Point::new(0, height - 1), buf, opacity: 1.0, scissor: None }
}

/// Hit regions in *screen* coordinates (bottom row).
pub fn dock_hit_regions(_width: i32, height: i32, items: &[DockItem]) -> Vec<(WindowId, Rect)> {
    dock_layout(items).into_iter()
        .map(|(id, r, _)| (id, Rect::new(r.x, height - 1, r.w, 1)))
        .collect()
}
```

- [ ] **Step 4: Run, expect pass.**

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add chrome: menubar + dock layers and hit regions"`

---

## Task 9: Window rendering into a layer (`wm.rs` addition)

Adds a function turning a `Window` + its app content buffer into a layer (titlebar, borders, shadow, content).

**Files:**
- Modify: `src/wm.rs` (append), `src/window.rs` if needed
- Test: `tests/wm_tests.rs` (append)

- [ ] **Step 1: Append failing tests to `tests/wm_tests.rs`**

```rust
use tuiui::wm::render_window;
use tuiui::buffer::CellBuffer;

#[test]
fn render_window_draws_title_and_content() {
    let mut m = wm();
    let id = m.add_window("btop".into(), Rect::new(0,1,12,5));
    let mut content = CellBuffer::new(10, 3);
    content.write_str(0,0,"hello", tuiui::cell::Rgba::rgb(255,255,255), tuiui::cell::Rgba::TRANSPARENT);
    let layers = render_window(m.get(id).unwrap(), &content, true);
    // shadow layer + window layer
    assert!(layers.len() >= 1);
    let win_layer = layers.last().unwrap();
    let titlerow: String = (0..12).map(|x| win_layer.buf.get(x,0).unwrap().ch).collect();
    assert!(titlerow.contains("btop"));
    // content 'h' should appear at inner (1,1)
    assert_eq!(win_layer.buf.get(1,1).unwrap().ch, 'h');
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Append implementation to `src/wm.rs`**

```rust
use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::compositor::Layer;
use crate::window::Window;

const TITLE_BG_FOCUS: Rgba = Rgba { r: 29, g: 36, b: 51, a: 255 };
const TITLE_BG_BLUR:  Rgba = Rgba { r: 20, g: 24, b: 34, a: 255 };
const TITLE_FG: Rgba = Rgba { r: 143, g: 183, b: 255, a: 255 };
const BORDER: Rgba = Rgba { r: 58, g: 68, b: 88, a: 255 };
const WIN_BG: Rgba = Rgba { r: 17, g: 20, b: 29, a: 255 };
const SHADOW: Rgba = Rgba { r: 0, g: 0, b: 0, a: 110 };

/// Returns [shadow_layer, window_layer]; window z derived from win.z.
pub fn render_window(win: &Window, content: &CellBuffer, focused: bool) -> Vec<Layer> {
    let r = win.rect;
    let base_z = 10 + win.z * 2;

    // shadow: solid translucent block offset by (1,1)
    let mut shadow = CellBuffer::new(r.w, r.h);
    shadow.fill(Cell { ch: ' ', fg: Rgba::TRANSPARENT, bg: SHADOW, attrs: Default::default() });
    let shadow_layer = Layer {
        z: base_z, origin: crate::geometry::Point::new(r.x + 1, r.y + 1),
        buf: shadow, opacity: 1.0, scissor: None,
    };

    // window body
    let mut buf = CellBuffer::new(r.w, r.h);
    buf.fill(Cell { ch: ' ', fg: Rgba::rgb(200,208,220), bg: WIN_BG, attrs: Default::default() });
    // titlebar
    let tbg = if focused { TITLE_BG_FOCUS } else { TITLE_BG_BLUR };
    for x in 0..r.w { buf.set(x, 0, Cell { ch: ' ', fg: TITLE_FG, bg: tbg, attrs: Default::default() }); }
    buf.write_str(2, 0, &win.title, TITLE_FG, tbg);
    if r.w >= 2 { buf.set(r.w - 2, 0, Cell { ch: '✕', fg: Rgba::rgb(255,107,107), bg: tbg, attrs: Default::default() }); }
    // borders (left/right/bottom)
    for y in 1..r.h {
        buf.set(0, y, Cell { ch: '│', fg: BORDER, bg: WIN_BG, attrs: Default::default() });
        buf.set(r.w-1, y, Cell { ch: '│', fg: BORDER, bg: WIN_BG, attrs: Default::default() });
    }
    for x in 0..r.w { buf.set(x, r.h-1, Cell { ch: '─', fg: BORDER, bg: WIN_BG, attrs: Default::default() }); }
    // content blit into inner rect (1,1)
    for cy in 0..content.height().min(r.h - 2) {
        for cx in 0..content.width().min(r.w - 2) {
            buf.set(1 + cx, 1 + cy, *content.get(cx, cy).unwrap());
        }
    }
    let win_layer = Layer { z: base_z + 1, origin: crate::geometry::Point::new(r.x, r.y), buf, opacity: 1.0, scissor: None };
    vec![shadow_layer, win_layer]
}
```

- [ ] **Step 4: Run, expect pass.** Run: `cargo test --test wm_tests`

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Render windows to layers: titlebar, borders, shadow, content"`

---

## Task 10: Input routing (`input.rs`)

Maps a raw event + current state into an `Action`. Pure decision function; the loop executes the action.

**Files:**
- Modify: `src/input.rs`
- Test: `tests/input_tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
use tuiui::input::{route_mouse, MouseKind, Hit, Action};
use tuiui::geometry::{Rect, Point};
use tuiui::window::{Window, WindowId, WindowState};

fn win(id: u64, rect: Rect, z: i32) -> Window {
    Window { id: WindowId(id), title: "t".into(), rect, z, state: WindowState::Floating, restore_rect: rect }
}

#[test]
fn click_on_titlebar_starts_move() {
    let w = win(1, Rect::new(0,1,20,8), 1);
    let act = route_mouse(MouseKind::Down, Point::new(3,1), &[w.clone()], None);
    assert_eq!(act, Action::BeginMove(WindowId(1)));
}

#[test]
fn click_on_close_glyph_closes() {
    let w = win(1, Rect::new(0,1,20,8), 1);
    // close glyph at rect.w-2 => x=18, titlebar row y=1
    let act = route_mouse(MouseKind::Down, Point::new(18,1), &[w], None);
    assert_eq!(act, Action::Close(WindowId(1)));
}

#[test]
fn click_on_bottom_right_corner_starts_resize() {
    let w = win(1, Rect::new(0,1,20,8), 1); // bottom row y=8, right col x=19
    let act = route_mouse(MouseKind::Down, Point::new(19,8), &[w], None);
    assert_eq!(act, Action::BeginResize(WindowId(1)));
}

#[test]
fn click_in_content_focuses_and_forwards() {
    let w = win(1, Rect::new(0,1,20,8), 1);
    let act = route_mouse(MouseKind::Down, Point::new(5,4), &[w], None);
    // content area: raises + forwards local coords (5-1, 4-2) = (4,2)
    assert_eq!(act, Action::FocusAndForward { id: WindowId(1), local: Point::new(4,2) });
}

#[test]
fn drag_while_moving_emits_move_to() {
    let w = win(1, Rect::new(0,1,20,8), 1);
    let act = route_mouse(MouseKind::Drag, Point::new(10,5), &[w], Some(Hit::Moving { id: WindowId(1), grab_dx: 3, grab_dy: 0 }));
    assert_eq!(act, Action::MoveTo { id: WindowId(1), x: 7, y: 5 });
}

#[test]
fn click_empty_desktop_is_noop() {
    let act = route_mouse(MouseKind::Down, Point::new(70,20), &[], None);
    assert_eq!(act, Action::None);
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/input.rs`**

```rust
use crate::geometry::Point;
use crate::window::{Window, WindowId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind { Down, Up, Drag, Move }

/// Drag-in-progress state the loop carries between events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Moving { id: WindowId, grab_dx: i32, grab_dy: i32 },
    Resizing { id: WindowId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    BeginMove(WindowId),
    BeginResize(WindowId),
    MoveTo { id: WindowId, x: i32, y: i32 },
    ResizeTo { id: WindowId, w: i32, h: i32 },
    Close(WindowId),
    FocusAndForward { id: WindowId, local: Point },
    EndDrag,
}

fn topmost_at(p: Point, windows: &[Window]) -> Option<&Window> {
    windows.iter().filter(|w| w.rect.contains(p)).max_by_key(|w| w.z)
}

pub fn route_mouse(kind: MouseKind, p: Point, windows: &[Window], drag: Option<Hit>) -> Action {
    // Continue an in-progress drag first.
    if let Some(h) = drag {
        match (kind, h) {
            (MouseKind::Drag, Hit::Moving { id, grab_dx, grab_dy }) =>
                return Action::MoveTo { id, x: p.x - grab_dx, y: p.y - grab_dy },
            (MouseKind::Drag, Hit::Resizing { id }) => {
                // resize so that bottom-right follows the cursor
                // loop converts using the window origin; here emit target corner
                return Action::ResizeTo { id, w: p.x, h: p.y }; // loop interprets relative to win origin
            }
            (MouseKind::Up, _) => return Action::EndDrag,
            _ => {}
        }
    }

    if kind != MouseKind::Down { return Action::None; }

    let w = match topmost_at(p, windows) { Some(w) => w, None => return Action::None };
    let r = w.rect;
    // close glyph
    if p.y == r.y && p.x == r.right() - 1 { return Action::Close(w.id); }
    // titlebar (top row, not the close glyph) -> move
    if p.y == r.y { return Action::BeginMove(w.id); }
    // bottom-right corner -> resize
    if p.x == r.right() && p.y == r.bottom() { return Action::BeginResize(w.id); }
    // content -> focus + forward (local coords relative to content origin)
    let local = Point::new(p.x - (r.x + 1), p.y - (r.y + 1));
    Action::FocusAndForward { id: w.id, local }
}
```

> Note for the loop (Task 12): on `BeginMove`, compute `grab_dx = p.x - win.rect.x`, `grab_dy = p.y - win.rect.y` and store `Hit::Moving`. On `ResizeTo { w, h }` interpret as new outer size `w - rect.x + 1` × `h - rect.y + 1` via `wm.resize_to`. On drag end near a screen edge with snapping enabled, call `wm.snap`.

- [ ] **Step 4: Run, expect pass.**

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add pure mouse input routing"`

---

## Task 11: Terminal backend (`terminal.rs`)

`crossterm` adapter behind a trait so the writer is testable with a fake. Detects capabilities, sets up/tears down the terminal, writes a frame diff as ANSI.

**Files:**
- Modify: `src/terminal.rs`
- Test: `tests/terminal_tests.rs`

- [ ] **Step 1: Write failing tests** (test the pure ANSI writer + caps via a capture buffer)

```rust
use tuiui::terminal::{Caps, frame_to_ansi};
use tuiui::compositor::CellChange;
use tuiui::cell::{Cell, Rgba};

#[test]
fn truecolor_change_emits_sgr_and_glyph() {
    let caps = Caps { truecolor: true, pixel_mouse: false };
    let changes = vec![CellChange { x: 2, y: 1, cell: Cell { ch: 'A', fg: Rgba::rgb(255,0,0), bg: Rgba::rgb(0,0,0), attrs: Default::default() } }];
    let out = frame_to_ansi(&changes, &caps);
    // cursor move to row 2 col 3 (1-based), set truecolor fg 255;0;0, print A
    assert!(out.contains("\x1b[2;3H"));
    assert!(out.contains("38;2;255;0;0"));
    assert!(out.contains('A'));
}

#[test]
fn no_changes_emits_nothing() {
    let caps = Caps { truecolor: true, pixel_mouse: false };
    assert_eq!(frame_to_ansi(&[], &caps), "");
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/terminal.rs`**

```rust
use crate::cell::Rgba;
use crate::compositor::CellChange;
use std::io::{Write, Stdout};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps { pub truecolor: bool, pub pixel_mouse: bool }

impl Caps {
    pub fn detect() -> Caps {
        let ct = std::env::var("COLORTERM").unwrap_or_default();
        let truecolor = ct.contains("truecolor") || ct.contains("24bit");
        // pixel mouse (SGR 1016) — conservatively off unless a known terminal
        let term = std::env::var("TERM_PROGRAM").unwrap_or_default();
        let pixel_mouse = matches!(term.as_str(), "kitty" | "WezTerm" | "ghostty");
        Caps { truecolor, pixel_mouse }
    }
}

fn fg_code(c: Rgba, caps: &Caps) -> String {
    if caps.truecolor { format!("38;2;{};{};{}", c.r, c.g, c.b) }
    else { format!("38;5;{}", ansi256(c)) }
}
fn bg_code(c: Rgba, caps: &Caps) -> String {
    if caps.truecolor { format!("48;2;{};{};{}", c.r, c.g, c.b) }
    else { format!("48;5;{}", ansi256(c)) }
}
fn ansi256(c: Rgba) -> u8 {
    // 6x6x6 cube
    let q = |v: u8| -> u32 { (v as u32 * 5 / 255) };
    (16 + 36 * q(c.r) + 6 * q(c.g) + q(c.b)) as u8
}

/// Build the ANSI byte string for a set of cell changes.
pub fn frame_to_ansi(changes: &[CellChange], caps: &Caps) -> String {
    let mut s = String::new();
    for ch in changes {
        s.push_str(&format!("\x1b[{};{}H", ch.y + 1, ch.x + 1)); // 1-based
        let a = &ch.cell.attrs;
        let mut sgr = vec![fg_code(ch.cell.fg, caps), bg_code(ch.cell.bg, caps)];
        if a.bold { sgr.push("1".into()); }
        if a.italic { sgr.push("3".into()); }
        if a.underline { sgr.push("4".into()); }
        if a.inverse { sgr.push("7".into()); }
        s.push_str(&format!("\x1b[0;{}m", sgr.join(";")));
        s.push(ch.cell.ch);
    }
    s.push_str("\x1b[0m");
    s
}

/// Terminal lifecycle (raw mode, alt screen, mouse capture). Not unit-tested; smoke-tested via main.
pub struct Terminal { out: Stdout, pub caps: Caps }

impl Terminal {
    pub fn enter() -> std::io::Result<Terminal> {
        use crossterm::{terminal, execute, event::{EnableMouseCapture}, cursor};
        terminal::enable_raw_mode()?;
        let mut out = std::io::stdout();
        execute!(out, terminal::EnterAlternateScreen, EnableMouseCapture, cursor::Hide)?;
        Ok(Terminal { out, caps: Caps::detect() })
    }
    pub fn size() -> std::io::Result<(i32, i32)> {
        let (c, r) = crossterm::terminal::size()?;
        Ok((c as i32, r as i32))
    }
    pub fn write_frame(&mut self, changes: &[CellChange]) -> std::io::Result<()> {
        let s = frame_to_ansi(changes, &self.caps);
        self.out.write_all(s.as_bytes())?;
        self.out.flush()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        use crossterm::{terminal, execute, event::DisableMouseCapture, cursor};
        let _ = execute!(self.out, DisableMouseCapture, terminal::LeaveAlternateScreen, cursor::Show);
        let _ = terminal::disable_raw_mode();
    }
}
```

- [ ] **Step 4: Run, expect pass.** Run: `cargo test --test terminal_tests`

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add terminal backend: caps detect, ANSI frame writer, lifecycle"`

---

## Task 12: PTY host (`ptyhost.rs`)

Spawns a child in a PTY, pumps its output through a `vt100::Parser` on a reader thread, exposes a snapshot of the screen as a `CellBuffer`, resizes, and writes input bytes.

**Files:**
- Modify: `src/ptyhost.rs`
- Test: `tests/ptyhost_tests.rs`

- [ ] **Step 1: Write failing tests** (scripted child via `printf`)

```rust
use tuiui::ptyhost::AppInstance;
use std::time::Duration;

#[test]
fn spawns_and_captures_output() {
    // child prints "READY" then sleeps; we read the parsed grid
    let mut app = AppInstance::spawn("sh", &["-c".into(), "printf READY; sleep 1".into()], 20, 5).unwrap();
    // give the reader thread a moment
    std::thread::sleep(Duration::from_millis(300));
    let grid = app.snapshot();
    let row0: String = (0..20).map(|x| grid.get(x,0).map(|c| c.ch).unwrap_or(' ')).collect();
    assert!(row0.starts_with("READY"), "got: {:?}", row0);
    app.kill();
}

#[test]
fn resize_changes_grid_dims() {
    let mut app = AppInstance::spawn("sh", &["-c".into(), "sleep 1".into()], 20, 5).unwrap();
    app.resize(30, 8);
    let grid = app.snapshot();
    assert_eq!(grid.width(), 30);
    assert_eq!(grid.height(), 8);
    app.kill();
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/ptyhost.rs`**

```rust
use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba, CellAttrs};
use portable_pty::{native_pty_system, PtySize, CommandBuilder, MasterPty, Child};
use std::sync::{Arc, Mutex};
use std::io::{Read, Write};

pub struct AppInstance {
    parser: Arc<Mutex<vt100::Parser>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    cols: u16, rows: u16,
}

impl AppInstance {
    pub fn spawn(cmd: &str, args: &[String], cols: i32, rows: i32) -> std::io::Result<AppInstance> {
        let pty = native_pty_system();
        let pair = pty.openpty(PtySize { rows: rows as u16, cols: cols as u16, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let mut builder = CommandBuilder::new(cmd);
        for a in args { builder.arg(a); }
        let child = pair.slave.spawn_command(builder)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows as u16, cols as u16, 0)));
        let mut reader = pair.master.try_clone_reader()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let writer = pair.master.take_writer()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        let pclone = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => { if let Ok(mut p) = pclone.lock() { p.process(&buf[..n]); } }
                }
            }
        });

        Ok(AppInstance { parser, master: pair.master, writer, child, cols: cols as u16, rows: rows as u16 })
    }

    pub fn snapshot(&self) -> CellBuffer {
        let p = self.parser.lock().unwrap();
        let screen = p.screen();
        let mut buf = CellBuffer::new(self.cols as i32, self.rows as i32);
        for y in 0..self.rows {
            for x in 0..self.cols {
                if let Some(c) = screen.cell(y, x) {
                    let ch = c.contents().chars().next().unwrap_or(' ');
                    buf.set(x as i32, y as i32, Cell {
                        ch: if ch == '\0' { ' ' } else { ch },
                        fg: vt_color(c.fgcolor(), Rgba::rgb(200,208,220)),
                        bg: vt_color(c.bgcolor(), Rgba::rgb(17,20,29)),
                        attrs: CellAttrs { bold: c.bold(), italic: c.italic(), underline: c.underline(), inverse: c.inverse() },
                    });
                }
            }
        }
        buf
    }

    pub fn resize(&mut self, cols: i32, rows: i32) {
        self.cols = cols as u16; self.rows = rows as u16;
        let _ = self.master.resize(PtySize { rows: rows as u16, cols: cols as u16, pixel_width: 0, pixel_height: 0 });
        if let Ok(mut p) = self.parser.lock() { p.set_size(rows as u16, cols as u16); }
    }

    pub fn write_input(&mut self, bytes: &[u8]) { let _ = self.writer.write_all(bytes); let _ = self.writer.flush(); }
    pub fn kill(&mut self) { let _ = self.child.kill(); }
    pub fn is_alive(&mut self) -> bool { matches!(self.child.try_wait(), Ok(None)) }
}

fn vt_color(c: vt100::Color, default: Rgba) -> Rgba {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r,g,b) => Rgba::rgb(r,g,b),
        vt100::Color::Idx(i) => idx_to_rgb(i),
    }
}

fn idx_to_rgb(i: u8) -> Rgba {
    // basic 16-color table; 16+ approximated by the 6x6x6 cube / grayscale ramp
    const BASE: [(u8,u8,u8);16] = [
        (0,0,0),(205,49,49),(13,188,121),(229,229,16),(36,114,200),(188,63,188),(17,168,205),(229,229,229),
        (102,102,102),(241,76,76),(35,209,139),(245,245,67),(59,142,234),(214,112,214),(41,184,219),(255,255,255)];
    if (i as usize) < 16 { let (r,g,b)=BASE[i as usize]; return Rgba::rgb(r,g,b); }
    if i >= 232 { let v = 8 + (i - 232) * 10; return Rgba::rgb(v,v,v); }
    let i = i - 16; let r = i/36; let g = (i%36)/6; let b = i%6;
    let s = |n:u8| if n==0 {0} else {55 + n*40};
    Rgba::rgb(s(r), s(g), s(b))
}
```

- [ ] **Step 4: Run, expect pass.** Run: `cargo test --test ptyhost_tests` (single-threaded if flaky: `-- --test-threads=1`)

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add PTY host: spawn, vt100 parse, snapshot, resize, input"`

---

## Task 13: Session core (`session.rs`)

Owns the `WindowManager` and the `AppInstance`s, maps `WindowId -> AppInstance`, applies `ClientMsg`, and produces a frame (a `Vec<Layer>`) via chrome + window rendering. This is the boundary that Slice 2 will put on a socket.

**Files:**
- Modify: `src/session.rs`
- Test: `tests/session_tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
use tuiui::session::{SessionCore, ClientMsg};
use tuiui::config::Config;
use tuiui::geometry::Point;

#[test]
fn launching_app_creates_window_and_dock_entry() {
    let mut core = SessionCore::new(80, 24, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 2".into()] });
    let frame = core.build_frame();
    // a window layer + menubar + dock present
    assert!(frame.layers.len() >= 3);
    assert_eq!(core.window_count(), 1);
    core.shutdown();
}

#[test]
fn click_dock_focuses_window() {
    let mut core = SessionCore::new(80, 24, Config::default());
    core.apply(ClientMsg::Launch { name: "a".into(), command: "sh".into(), args: vec!["-c".into(),"sleep 2".into()] });
    core.apply(ClientMsg::Launch { name: "b".into(), command: "sh".into(), args: vec!["-c".into(),"sleep 2".into()] });
    let regions = core.dock_regions();
    let (first_id, r) = regions[0];
    core.apply(ClientMsg::MouseDown(Point::new(r.x, r.y)));
    assert_eq!(core.focused(), Some(first_id));
    core.shutdown();
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `src/session.rs`**

```rust
use crate::chrome::{render_menubar, render_dock, dock_hit_regions, DockItem};
use crate::compositor::Layer;
use crate::config::Config;
use crate::geometry::{Point, Rect, SnapZone, snap_zone};
use crate::input::{route_mouse, MouseKind, Hit, Action};
use crate::ptyhost::AppInstance;
use crate::window::WindowId;
use crate::wm::{WindowManager, render_window};
use std::collections::HashMap;

pub enum ClientMsg {
    Launch { name: String, command: String, args: Vec<String> },
    MouseDown(Point),
    MouseDrag(Point),
    MouseUp(Point),
    Key(Vec<u8>),
    Resize { w: i32, h: i32 },
}

pub struct Frame { pub layers: Vec<Layer>, pub cursor: Option<Point> }

pub struct SessionCore {
    wm: WindowManager,
    apps: HashMap<WindowId, AppInstance>,
    titles: Vec<(WindowId, String)>,   // dock order
    cfg: Config,
    w: i32, h: i32,
    drag: Option<Hit>,
    cursor: Point,
}

impl SessionCore {
    pub fn new(w: i32, h: i32, cfg: Config) -> Self {
        let work = Rect::new(0, 1, w, h - 2); // exclude menubar + dock
        Self { wm: WindowManager::new(work), apps: HashMap::new(), titles: Vec::new(), cfg, w, h, drag: None, cursor: Point::new(w/2, h/2) }
    }
    pub fn window_count(&self) -> usize { self.apps.len() }
    pub fn focused(&self) -> Option<WindowId> { self.wm.focused() }
    pub fn dock_regions(&self) -> Vec<(WindowId, Rect)> {
        let items = self.dock_items();
        dock_hit_regions(self.w, self.h, &items)
    }

    fn dock_items(&self) -> Vec<DockItem> {
        let f = self.wm.focused();
        self.titles.iter().map(|(id, t)| DockItem { id: *id, label: t.clone(), focused: Some(*id) == f }).collect()
    }

    pub fn apply(&mut self, msg: ClientMsg) {
        match msg {
            ClientMsg::Launch { name, command, args } => self.launch(name, command, args),
            ClientMsg::MouseDown(p) => { self.cursor = p; self.handle_mouse(MouseKind::Down, p); }
            ClientMsg::MouseDrag(p) => { self.cursor = p; self.handle_mouse(MouseKind::Drag, p); }
            ClientMsg::MouseUp(p) => { self.cursor = p; self.handle_mouse(MouseKind::Up, p); }
            ClientMsg::Key(bytes) => { if let Some(id) = self.wm.focused() { if let Some(app) = self.apps.get_mut(&id) { app.write_input(&bytes); } } }
            ClientMsg::Resize { w, h } => { self.w = w; self.h = h; self.wm.set_work_area(Rect::new(0,1,w,h-2)); }
        }
    }

    fn launch(&mut self, name: String, command: String, args: Vec<String>) {
        let rect = Rect::new(4 + (self.titles.len() as i32 * 3), 3 + (self.titles.len() as i32 * 2), 48, 16);
        let id = self.wm.add_window(name.clone(), rect);
        let content = self.wm.get(id).unwrap().content_rect();
        if let Ok(app) = AppInstance::spawn(&command, &args, content.w.max(1), content.h.max(1)) {
            self.apps.insert(id, app);
            self.titles.push((id, name));
        } else {
            self.wm.close(id);
        }
    }

    fn handle_mouse(&mut self, kind: MouseKind, p: Point) {
        // dock clicks first
        if kind == MouseKind::Down {
            for (id, r) in self.dock_regions() {
                if r.contains(p) { self.wm.raise(id); return; }
            }
        }
        let windows: Vec<_> = self.wm.z_ordered().into_iter().cloned().collect();
        let action = route_mouse(kind, p, &windows, self.drag);
        self.exec(action, p);
    }

    fn exec(&mut self, action: Action, p: Point) {
        match action {
            Action::BeginMove(id) => {
                self.wm.raise(id);
                let r = self.wm.get(id).unwrap().rect;
                self.drag = Some(Hit::Moving { id, grab_dx: p.x - r.x, grab_dy: p.y - r.y });
            }
            Action::BeginResize(id) => { self.wm.raise(id); self.drag = Some(Hit::Resizing { id }); }
            Action::MoveTo { id, x, y } => self.wm.move_to(id, x, y),
            Action::ResizeTo { id, w, h } => {
                let r = self.wm.get(id).unwrap().rect;
                self.wm.resize_to(id, w - r.x + 1, h - r.y + 1);
                self.sync_app_size(id);
            }
            Action::Close(id) => self.close(id),
            Action::FocusAndForward { id, local } => {
                self.wm.raise(id);
                // (mouse-forwarding into apps is keyboard-first in Slice 1; raise is enough)
                let _ = local;
            }
            Action::EndDrag => {
                if let Some(Hit::Moving { id, .. }) = self.drag {
                    if self.cfg.snapping_enabled {
                        if let Some(z) = snap_zone(p, Rect::new(0,1,self.w,self.h-2), self.cfg.snap_threshold) {
                            self.wm.snap(id, z);
                            self.sync_app_size(id);
                        }
                    }
                }
                self.drag = None;
            }
            Action::None => {}
        }
    }

    fn sync_app_size(&mut self, id: WindowId) {
        if let Some(w) = self.wm.get(id) {
            let c = w.content_rect();
            if let Some(app) = self.apps.get_mut(&id) { app.resize(c.w.max(1), c.h.max(1)); }
        }
    }

    fn close(&mut self, id: WindowId) {
        if let Some(mut app) = self.apps.remove(&id) { app.kill(); }
        self.titles.retain(|(i,_)| *i != id);
        self.wm.close(id);
    }

    pub fn build_frame(&self) -> Frame {
        let mut layers: Vec<Layer> = Vec::new();
        let focused = self.wm.focused();
        for w in self.wm.z_ordered() {
            let content = self.apps.get(&w.id).map(|a| a.snapshot())
                .unwrap_or_else(|| crate::buffer::CellBuffer::new(w.content_rect().w, w.content_rect().h));
            layers.extend(render_window(w, &content, Some(w.id) == focused));
        }
        let app_name = focused.and_then(|id| self.titles.iter().find(|(i,_)| *i==id)).map(|(_,t)| t.clone()).unwrap_or_default();
        layers.push(render_menubar(self.w, &app_name));
        layers.push(render_dock(self.w, self.h, &self.dock_items()));
        Frame { layers, cursor: Some(self.cursor) }
    }

    pub fn reap_dead(&mut self) {
        let dead: Vec<WindowId> = self.apps.iter_mut().filter(|(_,a)| !a.is_alive()).map(|(id,_)| *id).collect();
        for id in dead { self.close(id); }
    }

    pub fn shutdown(&mut self) {
        for (_, app) in self.apps.iter_mut() { app.kill(); }
        self.apps.clear();
    }
}
```

> Requires a small addition to `wm.rs`: a `move_to(id, x, y)` setter (absolute). Add it next to `move_by`:
> ```rust
> pub fn move_to(&mut self, id: WindowId, x: i32, y: i32) {
>     if let Some(w) = self.get_mut(id) { w.rect.x = x; w.rect.y = y; w.state = WindowState::Floating; }
> }
> ```
> Add this in Task 13 Step 3 alongside the session code, then re-run wm tests.

- [ ] **Step 4: Run, expect pass.** Run: `cargo test --test session_tests` (and `cargo test --test wm_tests`)

- [ ] **Step 5: Commit.** `git add -A && git commit -m "Add SessionCore: owns wm+apps, applies ClientMsg, builds frames"`

---

## Task 14: Main loop & wiring (`main.rs`)

Ties the front-end (terminal backend + crossterm event reader) to the `SessionCore`. Not unit-tested; verified by the manual smoke test in Task 15.

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement `src/main.rs`**

```rust
use tuiui::compositor::Compositor;
use tuiui::config::Config;
use tuiui::session::{SessionCore, ClientMsg};
use tuiui::terminal::Terminal;
use tuiui::geometry::Point;
use crossterm::event::{self, Event, MouseEventKind, MouseButton, KeyCode, KeyModifiers, KeyEventKind};
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let cfg = Config::load();
    let (w, h) = Terminal::size()?;
    let mut term = Terminal::enter()?;
    let mut comp = Compositor::new(w, h);
    let mut core = SessionCore::new(w, h, cfg.clone());

    // launch bundled apps
    for app in &cfg.apps {
        core.apply(ClientMsg::Launch { name: app.name.clone(), command: app.command.clone(), args: app.args.clone() });
    }

    let mut dragging = false;
    'outer: loop {
        // input (poll so we can still animate apps like btop)
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    // reserved quit chord: Ctrl+Alt+Q
                    if k.modifiers.contains(KeyModifiers::CONTROL) && k.modifiers.contains(KeyModifiers::ALT) && k.code == KeyCode::Char('q') {
                        break 'outer;
                    }
                    core.apply(ClientMsg::Key(encode_key(k.code, k.modifiers)));
                }
                Event::Mouse(m) => {
                    let p = Point::new(m.column as i32, m.row as i32);
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => { dragging = true; core.apply(ClientMsg::MouseDown(p)); }
                        MouseEventKind::Drag(MouseButton::Left) => { core.apply(ClientMsg::MouseDrag(p)); }
                        MouseEventKind::Up(MouseButton::Left) => { dragging = false; core.apply(ClientMsg::MouseUp(p)); }
                        MouseEventKind::Moved => { core.apply(ClientMsg::MouseDrag(p)); } // updates cursor only when not dragging? keep simple
                        _ => {}
                    }
                    let _ = dragging;
                }
                Event::Resize(nc, nr) => { comp.resize(nc as i32, nr as i32); core.apply(ClientMsg::Resize { w: nc as i32, h: nr as i32 }); }
                _ => {}
            }
        }

        core.reap_dead();
        let frame = core.build_frame();
        let _ = comp.composite(&frame.layers, frame.cursor);
        let changes = comp.diff();
        term.write_frame(&changes)?;
        comp.commit();
    }

    core.shutdown();
    Ok(()) // Terminal::drop restores the screen
}

/// Minimal key encoding: printable chars + Enter/Backspace/Tab/Esc/arrows.
fn encode_key(code: KeyCode, mods: KeyModifiers) -> Vec<u8> {
    match code {
        KeyCode::Char(c) => {
            if mods.contains(KeyModifiers::CONTROL) {
                let b = (c.to_ascii_uppercase() as u8).wrapping_sub(0x40);
                vec![b]
            } else { c.to_string().into_bytes() }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        _ => vec![],
    }
}
```

> Note: the `MouseEventKind::Moved` arm forwarding to `MouseDrag` keeps the rendered cursor following the mouse even when not dragging. `SessionCore::handle_mouse` ignores non-Down drags unless a `drag` is active, so this is safe — but verify the cursor tracks correctly in the smoke test; if `Moved` events aren't delivered by the terminal, the cursor simply updates on clicks/drags.

- [ ] **Step 2: Build.** Run: `cargo build`. Fix any signature mismatches against earlier tasks.

- [ ] **Step 3: Commit.** `git add -A && git commit -m "Wire main loop: input, render, teardown, quit chord"`

---

## Task 15: Manual smoke test & polish

**Files:** none (verification) — fixes land in the relevant module.

- [ ] **Step 1: Create a test config** at `~/.config/tuiui/config.toml`:

```toml
snapping_enabled = true
snap_threshold = 3
[[apps]]
name = "shell"
command = "bash"
[[apps]]
name = "btop"
command = "btop"
```
(Use apps you have installed; `top` works if `btop` isn't present.)

- [ ] **Step 2: Run** `cargo run` in a truecolor terminal (Kitty/Ghostty/WezTerm/iTerm2). Verify:
  - menubar (top), dock (bottom), two floating windows with shadows
  - drag a titlebar to move; drag to the left edge → snaps to left half
  - drag bottom-right corner → resizes; the app reflows
  - click a dock entry → that window raises/focuses
  - type into the focused shell → input reaches it; `btop`/`top` animates
  - `Ctrl+Alt+Q` quits and the terminal is restored cleanly with no orphaned processes (`ps` check)

- [ ] **Step 3:** Fix any issues in the owning module with a follow-up test where feasible; commit each fix.

- [ ] **Step 4: Final commit.** `git add -A && git commit -m "Slice 1 smoke-tested: floating-window desktop runs bundled apps"`

---

## Self-review notes (author)

- **Spec coverage:** compositor+cursor+alpha (Tasks 2–4), PTY host running real apps (12), floating WM with move/resize/snap (6,9), menubar+dock chrome (8), input routing + coord translation (10), capability detection + graceful color downgrade (11), read-only config (7), in-process core/client boundary `ClientMsg`/`CoreMsg` (13), clean teardown + quit chord (14). All Slice-1 "Done when" items map to a task.
- **Deliberate Slice-1 simplifications (documented in spec as out-of-scope or here):** only drag-to-edge half-snap (no Super+Arrow/grid); mouse *clicks* raise/focus apps but per-cell mouse *forwarding* into apps is keyboard-first (full mouse forwarding is a fast-follow once cell/pixel mouse translation is validated); static menubar (no per-app menus). These are noted in the Slice 1 spec's "Out of scope".
- **Type consistency:** `Rgba`, `Cell`, `CellBuffer`, `Layer`, `Rect/Point`, `WindowId`, `Action`, `ClientMsg` names are used identically across tasks. `wm.move_to` is added in Task 13's note.
- **Known follow-ups for Slice 1 hardening (not blockers):** full mouse-into-app forwarding; SGR-1016 pixel-mouse drag smoothing; per-window content diffing for performance under many windows.
