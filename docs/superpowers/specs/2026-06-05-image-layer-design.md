# A1 — Native Image Layer (Kitty Graphics) — Design

**Status:** Approved design (2026-06-05). First subsystem of the desktop-OS
roadmap (`2026-06-05-desktop-os-roadmap.md`).

**Goal:** Let tuiui's compositor display real raster images inside windows via the
Kitty graphics protocol, with a native **image-viewer** window as the first
consumer (file-manager thumbnails and desktop icons reuse the layer later).

**Architecture:** The daemon computes image **placements** (which image goes
where, and whether it's currently visible); the thin client emits the Kitty
graphics escape sequences. Image bytes are decoded + downscaled once, cached by
content hash, and transmitted to the client **once per attach**, then referenced
by id. Terminals without graphics support fall back to a cell placeholder.

**Tech stack:** Rust; new `image` crate (decode PNG/JPEG/GIF/WebP); existing
compositor / session / protocol / client; Kitty graphics protocol escapes.

---

## Why a separate channel (not cells)

The compositor renders a grid of text cells, diffed and re-emitted as ANSI. Images
cannot live in that grid. So images travel on a **parallel channel** in the frame:
placements (cheap, every frame) plus image data (heavy, once). This keeps the cell
pipeline untouched and lets non-graphics terminals keep working via placeholders.

**Out of scope (the A2 spike):** images emitted by *hosted* apps (yazi previews,
Carbonyl) are **not** handled here — our embedded `alacritty_terminal` swallows
those escapes. A1 only renders tuiui's *own* images.

## Module layout

- **Create `src/imagestore.rs`** — `ImageStore`: `load(path, max_w_px, max_h_px)
  → ImageId`, decoding via the `image` crate, downscaling to fit the target pixel
  box, re-encoding to PNG, hashing the PNG bytes for the id, and caching by
  `(path, target)`. Exposes `png_bytes(id) → &[u8]` and `dimensions(id)`.
- **Create `src/imageview.rs`** — `ImageView` window content: holds the path +
  `ImageId`, renders a **cell placeholder** (border, filename, `W×H`), and reports
  its `ImageId` for placement.
- **Modify `src/session.rs`** — `WinContent::ImageView(ImageView)`; own the
  `ImageStore`; in `build_frame`, attach an `ImagePlacement` for each visible
  ImageView window; resolve a `@image` launch action.
- **Modify `src/protocol.rs`** — `FrameMsg` gains `images: Vec<ImagePlacement>`
  and `image_data: Vec<ImageBlob>` (new-to-this-client ids only).
- **Modify `src/daemon.rs`** — per-client `sent_image_ids` set (reset on attach);
  fill `image_data` for unsent ids from the `ImageStore`; mark them sent.
- **Modify `src/terminal.rs`** — `Caps.kitty_graphics: bool`, detected from the
  terminal (Ghostty / Kitty / WezTerm).
- **Modify `src/client.rs`** — reconcile images each frame: transmit blobs,
  place/move visible placements, delete hidden/closed ones.

## Types

```rust
// protocol.rs
pub type ImageId = u64; // content hash of the downscaled PNG

pub struct ImagePlacement {
    pub id: ImageId,
    pub rect: Rect,     // screen cells the image occupies (window content rect)
    pub cols: u16,      // rect.w  — cells the image spans
    pub rows: u16,      // rect.h
    pub visible: bool,  // false → client deletes the placement (occluded/hidden)
}

pub struct ImageBlob {
    pub id: ImageId,
    pub png_base64: String, // sent once per attach
}
```

`FrameMsg` adds `images: Vec<ImagePlacement>` and `image_data: Vec<ImageBlob>`
(both `#[serde(default)]` for version-skew tolerance, matching `Flags`).

## Data flow

1. The user opens an image — for v1 via a launcher/config action `@image` with a
   path arg (the file manager opens images the same way later, through default
   apps). The session adds an `ImageView` window.
2. `ImageStore::load` decodes the file, downscales to the window's pixel size
   (cells × an assumed cell pixel size, e.g. 8×16), re-encodes PNG, hashes → `id`.
3. `build_frame`:
   - the `ImageView` draws its **placeholder cells** (so the area always has a
     base and non-graphics terminals show something), and
   - the session attaches an `ImagePlacement { id, rect=content_rect, cols, rows,
     visible }`, where `visible` = the window is non-minimized **and fully
     unobstructed** by any higher window (see Occlusion).
4. The daemon serve loop builds `FrameMsg`: for each placement whose `id` is not
   in this client's `sent_image_ids`, it appends an `ImageBlob` (base64 PNG from
   the `ImageStore`) and records the id as sent.
5. The client:
   - renders cell changes as today,
   - **transmits** each received `ImageBlob` via Kitty `a=t` (`f=100`, `i=id`,
     quiet), caching the id,
   - for each `visible` placement, moves the cursor to `rect` origin and **places**
     the image (`a=p`, `i=id`, `c=cols`, `r=rows`), tracking active placements,
   - **deletes** (`a=d`, `i=id`) any previously-active placement that is now absent
     or `visible=false`.

## Occlusion (v1)

A placement is `visible` only when its window's content rect is **fully
unobstructed** — i.e. no non-minimized window with a higher `z` overlaps it. If
covered, `visible=false` → the client deletes the placement and the placeholder
cells (already drawn beneath) show through. Per-pixel partial clipping under
overlap is a documented later enhancement, not v1.

## Capability detection & fallback

`Caps.kitty_graphics` is set when the terminal is known to support the protocol
(`$TERM`/`$TERM_PROGRAM` indicating Ghostty, Kitty, or WezTerm; conservative
default `false`). When `false`, the client skips all transmit/place/delete and the
**placeholder cells** (filename + dimensions in a bordered box) are the image. The
daemon still sends placements/blobs; an unsupporting client simply ignores them.

## SSH / bandwidth

Blobs are downscaled PNGs sent **once per attach per id**; every subsequent frame
carries only the small `ImagePlacement` list. Moving/resizing a window re-places
(no re-transmit). Re-attaching re-sends blobs (the new client starts empty), which
mirrors the existing full-cell-repaint on attach.

## Error handling & safety

- Decode failure (corrupt/unsupported file) → the `ImageView` shows a placeholder
  reading "cannot display" + the path; no panic, no placement attached.
- A missing/oversized file is clamped/skipped; the store never blocks the render
  thread for long (decode happens on `load`, off the per-frame path; cached after).
- Unknown image ids on the client (e.g. a delete for something never transmitted)
  are no-ops.

## Testing (pure, deterministic)

- `ImageStore`: a tiny embedded fixture (a few-pixel PNG) → deterministic `id`,
  downscale to a target box yields expected dimensions, same input → same id.
- Placement geometry: a window rect → `ImagePlacement` rect/cols/rows.
- Occlusion decision: given a window stack, the covered image is `visible=false`,
  the top one `true`.
- Kitty control-string builders: `transmit(id, png)`, `place(id, cols, rows)`,
  `delete(id)` produce the exact escape strings (assert the bytes; never write to
  a real terminal).
- Protocol round-trip: `ImagePlacement` / `ImageBlob` serialize/deserialize; a
  frame with `image_data` omitted (skew) still parses.

## Build sequence

1. `image` dep + `ImageStore` (decode/downscale/hash) + tests.
2. Kitty control-string builders (transmit/place/delete) + tests.
3. `ImagePlacement`/`ImageBlob` in the protocol (serde-default) + `Caps.kitty_graphics`.
4. `ImageView` content + placeholder rendering; `@image` launch hook.
5. `build_frame` placements + occlusion; daemon `sent_image_ids` + blob fill.
6. Client reconcile (transmit/place/delete) + capability gating.
7. Manual smoke on the mini in Ghostty (open a PNG; move/cover/close the window).

## Out of scope (YAGNI)

- Animation (GIF/APNG play), zoom/pan (v1 is fit-to-window).
- Partial-overlap clipping (v1 hides on any occlusion).
- Passthrough of images from hosted apps (the separate **A2** spike).
- Per-image color-profile / HDR handling.
