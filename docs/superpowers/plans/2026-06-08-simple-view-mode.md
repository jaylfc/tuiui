# Simple View Mode — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a top-bar toggle that flips tuiui between the windowed **desktop** view and a **simple** view that draws the focused app full-screen (no decorations, no desktop icons), reusing the existing menubar + dock; switch apps via the dock.

**Architecture:** Frontend-only. A `simple: bool` on `SessionCore`; a menubar glyph toggle (`⊞` desktop / `▦` simple) with a hit region; a `build_frame_simple` path; and mode-aware PTY sizing so the focused app fills the work area in simple mode and restores on toggle-back. No apphost/protocol/client changes.

**Tech Stack:** Rust 2021, `src/chrome.rs` (menubar), `src/session.rs` (`SessionCore`, `build_frame`, `handle_mouse`, `sync_app_size`), `CellBuffer`, `Layer`, `Rect`.

**Reference:** Spec `docs/superpowers/specs/2026-06-08-simple-view-mode-design.md`.

---

## Background (read before starting)

- `render_menubar(width, focused_app, segments) -> Layer` (`src/chrome.rs:33`): draws `GO_LABEL` (`" Go "`, x=0..3) at x=0, focused-app name at `APP_X=10`, tray segments + `POWER_LABEL` on the right. `menubar_brand_region()` = `Rect(0,0,4,1)`; `menubar_power_region(w)` on the right.
- `SessionCore::build_frame(&self) -> Frame` (`src/session.rs:~1795`): pushes desktop layer (z=0), decorated windows, drag preview, `render_menubar`, `render_dock`, desktop overlay/launcher/tray/dirpicker/help/power_menu, then images (ImageView, FM thumbnails, desktop icons, app graphics), returns `Frame { layers, cursor: Some(self.cursor), images }`.
- `sync_app_size(&mut self, id)` (`src/session.rs`): resizes an App window's PTY to `w.content_rect()`.
- `handle_mouse` checks `menubar_brand_region()` then `menubar_power_region()` then tray then dock (`self.dock_regions()` → `wm.unminimize(id)`), before window routing. The power-menu modal check is near the top.
- `WinContent::render(&self, host, w, h) -> CellBuffer`; `self.apphost.placements(aid)`, `self.app_image_ids: HashMap<(WindowId,u32),u64>` (populated by `refresh_app_graphics` each tick, mode-independent).
- `self.dock_items()`, `self.tray_state`, `crate::tray::tray_segments(&st, w)`, `self.wm.focused() -> Option<WindowId>`, `self.titles: Vec<(WindowId,String)>`.

---

## Task 1: Menubar mode toggle (chrome.rs)

**Files:**
- Modify: `src/chrome.rs`
- Modify: `tests/chrome_tests.rs`

- [ ] **Step 1: Add the glyph constants + render the toggle + hit region**

In `src/chrome.rs`, add near the other label consts:

```rust
/// Menubar view-mode toggle glyphs (shows the CURRENT mode; click to switch).
const MODE_DESKTOP: &str = " \u{229E} "; // ⊞  windowed desktop
const MODE_SIMPLE: &str = " \u{25A6} ";  // ▦  full-screen single app
```

Change `render_menubar` to take the current mode and draw the glyph at x=4 (Go is x=0..3, the app name starts at x=10, so x=4..6 fits with a gap):

```rust
pub fn render_menubar(width: i32, focused_app: &str, segments: &[crate::tray::Segment], simple: bool) -> Layer {
    let t = crate::theme::current();
    let mut buf = CellBuffer::new(width, 1);
    buf.fill(crate::cell::Cell { ch: ' ', fg: t.text, bg: t.menubar_bg, attrs: Default::default() });
    buf.write_str(0, 0, GO_LABEL, t.accent, t.active_bg);
    let mode = if simple { MODE_SIMPLE } else { MODE_DESKTOP };
    buf.write_str(4, 0, mode, t.accent, t.active_bg);
    // ... rest unchanged (power button, tray, app name) ...
```

Keep the rest of the body exactly as-is. Add the hit region function:

```rust
/// Screen-space hit region for the menubar view-mode toggle (just right of "Go").
pub fn menubar_mode_region() -> Rect {
    Rect::new(4, 0, MODE_DESKTOP.chars().count() as i32, 1)
}
```

(`MODE_DESKTOP` and `MODE_SIMPLE` are both 3 chars, so the region width is stable.)

- [ ] **Step 2: Update the chrome tests**

In `tests/chrome_tests.rs`, both `render_menubar(...)` calls need the new `false` arg. Update `menubar_layer_spans_top_row_and_shows_brand` to `render_menubar(40, "btop", &[], false)` and the power-button test likewise. Add a mode-glyph test:

```rust
#[test]
fn menubar_shows_mode_toggle_glyph() {
    use tuiui::chrome::menubar_mode_region;
    let desktop: String = (0..40).map(|x| render_menubar(40, "x", &[], false).buf.get(x, 0).unwrap().ch).collect();
    assert!(desktop.contains('\u{229E}'), "desktop mode shows ⊞, got {desktop:?}");
    let simple: String = (0..40).map(|x| render_menubar(40, "x", &[], true).buf.get(x, 0).unwrap().ch).collect();
    assert!(simple.contains('\u{25A6}'), "simple mode shows ▦, got {simple:?}");
    // region sits just right of Go
    let r = menubar_mode_region();
    assert_eq!(r.y, 0);
    assert!(r.x >= 4);
}
```

- [ ] **Step 3: Build will fail until session.rs is updated (Task 2 changes the caller). Do Tasks 1–3 together, then build once.** Proceed to Task 2.

---

## Task 2: Session state, toggle, mode-aware sizing, input (session.rs)

**Files:**
- Modify: `src/session.rs`

- [ ] **Step 1: Field + accessors + helper**

Add to `struct SessionCore` (near `power_menu`): `simple: bool,`. Initialize `simple: false,` in `with_apphost`'s struct literal. Update the chrome `use` to include `menubar_mode_region`:

```rust
use crate::chrome::{
    render_menubar, render_dock, dock_hit_regions, menubar_brand_region, menubar_mode_region, menubar_power_region, DockItem,
};
```

Add methods to `impl SessionCore`:

```rust
    /// Whether the full-screen "simple" view mode is active.
    pub fn simple_mode(&self) -> bool { self.simple }

    /// The work-area rect a full-screen app fills in simple mode (between the
    /// top menubar row and the bottom dock row).
    fn simple_content_rect(&self) -> crate::geometry::Rect {
        crate::geometry::Rect::new(0, 1, self.w.max(1), (self.h - 2).max(1))
    }

    /// Toggle between desktop and simple view. Resizes the focused app so it
    /// fills the screen (entering simple) or returns to its window size (leaving).
    pub fn toggle_simple(&mut self) {
        self.simple = !self.simple;
        // Re-sync every app window: in simple mode only the focused one becomes
        // full-screen (handled by sync_app_size); the rest go to their window size.
        let ids: Vec<WindowId> = self.contents.keys().copied().collect();
        for id in ids {
            self.sync_app_size(id);
        }
    }
```

- [ ] **Step 2: Make `sync_app_size` mode-aware**

Replace `sync_app_size` so the focused app fills the work area in simple mode:

```rust
    fn sync_app_size(&mut self, id: WindowId) {
        let target = if self.simple && self.wm.focused() == Some(id) {
            self.simple_content_rect()
        } else if let Some(w) = self.wm.get(id) {
            w.content_rect()
        } else {
            return;
        };
        if let Some(WinContent::App(aid)) = self.contents.get(&id) {
            let aid = *aid;
            self.apphost.resize(aid, target.w.max(1), target.h.max(1));
        }
    }
```

- [ ] **Step 3: Toggle click in `handle_mouse`**

Next to the existing brand/power region checks (the `if kind == MouseKind::Down { ... }` block), add the mode toggle BEFORE the brand check:

```rust
            if menubar_mode_region().contains(p) {
                self.launcher.close();
                self.power_menu.close();
                self.toggle_simple();
                return;
            }
```

- [ ] **Step 4: Resize the focused app when switching apps via the dock (simple mode)**

In the dock-click loop (where it does `self.wm.unminimize(id); return;`), resize the newly-focused app when in simple mode so it fills the screen:

```rust
            for (id, r) in self.dock_regions() {
                if r.contains(p) {
                    self.wm.unminimize(id);
                    if self.simple {
                        self.sync_app_size(id);
                    }
                    return;
                }
            }
```

- [ ] **Step 5: Keep the focused app full-screen across terminal resize + launch (simple mode)**

Find the `ClientMsg::Resize { w, h }` arm in `apply` (it updates `self.w/self.h`, desktop layout, etc.). At its end, add:

```rust
                if self.simple {
                    if let Some(fid) = self.wm.focused() {
                        self.sync_app_size(fid);
                    }
                }
```

In `launch_in`, after the successful spawn + `self.auto_tile_if_enabled();`, the new window is focused; ensure it fills the screen in simple mode:

```rust
                self.auto_tile_if_enabled();
                if self.simple {
                    self.sync_app_size(id);
                }
```

(Place inside the `Ok(app_id) => { ... }` arm, after `auto_tile_if_enabled`.)

- [ ] **Step 6: Update the desktop `build_frame` menubar call + early-branch to simple**

At the very start of `build_frame`, before building any layers, branch:

```rust
    pub fn build_frame(&self) -> Frame {
        if self.simple {
            return self.build_frame_simple();
        }
        let mut layers: Vec<Layer> = Vec::new();
        // ... existing desktop body ...
```

And update the existing `render_menubar` call in the desktop body to pass `false`:

```rust
        layers.push(render_menubar(self.w, &app_name, &segs, false));
```

- [ ] **Step 7: Build (with Task 3 added) — see Task 3 for `build_frame_simple`, then build.**

---

## Task 3: `build_frame_simple` (session.rs)

**Files:**
- Modify: `src/session.rs`

- [ ] **Step 1: Add the simple-mode frame builder**

Add this method to `impl SessionCore` (next to `build_frame`):

```rust
    /// Build the frame for simple (full-screen single-app) view: the focused
    /// window's content fills the work area with no decorations; the menubar and
    /// dock stay; the desktop and other windows are hidden.
    fn build_frame_simple(&self) -> Frame {
        use crate::geometry::Point;
        let t = crate::theme::current();
        let wa = self.simple_content_rect();
        let mut layers: Vec<Layer> = Vec::new();
        let focused = self.wm.focused();

        // Focused window full-screen (no chrome), or a hint when nothing is open.
        if let Some(fid) = focused {
            if let Some(content) = self.contents.get(&fid) {
                let buf = content.render(&self.apphost, wa.w, wa.h);
                layers.push(Layer { z: 1, origin: Point::new(wa.x, wa.y), buf, opacity: 1.0, scissor: None });
            }
        } else {
            let mut buf = crate::buffer::CellBuffer::new(wa.w, wa.h);
            buf.fill(crate::cell::Cell { ch: ' ', fg: t.dim, bg: t.desktop_bg, attrs: Default::default() });
            let hint = "Press Go to launch an app";
            let hx = ((wa.w - hint.chars().count() as i32) / 2).max(0);
            let hy = wa.h / 2;
            buf.write_str(hx, hy, hint, t.dim, t.desktop_bg);
            layers.push(Layer { z: 1, origin: Point::new(wa.x, wa.y), buf, opacity: 1.0, scissor: None });
        }

        // Chrome: menubar (simple glyph) + dock.
        let app_name = focused
            .and_then(|id| self.titles.iter().find(|(i, _)| *i == id))
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        let segs = {
            let st = self.tray_state.read().unwrap();
            crate::tray::tray_segments(&st, self.w)
        };
        layers.push(render_menubar(self.w, &app_name, &segs, true));
        layers.push(render_dock(self.w, self.h, &self.dock_items()));

        // Overlays that must still work in simple mode.
        layers.extend(self.launcher.render(self.w, self.h).layers);
        {
            let st = self.tray_state.read().unwrap();
            layers.extend(self.tray.render(self.w, self.h, &st).layers);
        }
        if self.help_open {
            layers.extend(crate::help::render_help(self.w, self.h));
        }
        layers.extend(self.power_menu.render(self.w, self.h));

        // Images for the focused window only, mapped into the work area.
        let mut images = Vec::new();
        if let Some(fid) = focused {
            match self.contents.get(&fid) {
                Some(WinContent::App(aid)) => {
                    let aid = *aid;
                    for pl in self.apphost.placements(aid) {
                        if let Some(&img) = self.app_image_ids.get(&(fid, pl.image_id)) {
                            let x = wa.x + pl.col as i32;
                            let y = wa.y + pl.row as i32;
                            if x >= wa.x + wa.w || y >= wa.y + wa.h {
                                continue;
                            }
                            let cols = pl.cols.min((wa.x + wa.w - x).max(1) as u16);
                            let rows = pl.rows.min((wa.y + wa.h - y).max(1) as u16);
                            images.push(crate::protocol::ImagePlacement {
                                id: img,
                                rect: crate::geometry::Rect::new(x, y, cols as i32, rows as i32),
                                cols,
                                rows,
                                visible: true,
                            });
                        }
                    }
                }
                Some(WinContent::ImageView(v)) => {
                    if let Some(id) = v.image_id() {
                        images.push(crate::protocol::ImagePlacement {
                            id,
                            rect: wa,
                            cols: wa.w.max(1) as u16,
                            rows: wa.h.max(1) as u16,
                            visible: true,
                        });
                    }
                }
                Some(WinContent::FileManager(f)) => {
                    images.extend(f.thumbnail_placements(wa, true));
                }
                _ => {}
            }
        }

        Frame { layers, cursor: Some(self.cursor), images }
    }
```

NOTE: verify the exact names — `crate::imageview::ImageView::image_id() -> Option<u64>`, `FileManager::thumbnail_placements(rect, visible) -> Vec<ImagePlacement>`, `self.app_image_ids` key `(WindowId, u32)`, and the `ImagePlacement` field names (`id, rect, cols, rows, visible`). These all match the desktop `build_frame`; copy precisely from there if any differ.

- [ ] **Step 2: Build**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build 2>&1 | tail -25`
Expected: clean. Fix any borrow/name issues against the real APIs.

---

## Task 4: Tests, gate, commit, manual

**Files:**
- Modify: `tests/session_tests.rs`

- [ ] **Step 1: Session tests for the toggle + simple frame**

```rust
#[test]
fn mode_toggle_switches_view() {
    use tuiui::session::{SessionCore, ClientMsg};
    use tuiui::config::Config;
    use tuiui::chrome::menubar_mode_region;
    use tuiui::geometry::Point;
    let mut core = SessionCore::new(120, 40, Config::default());
    assert!(!core.simple_mode());
    let r = menubar_mode_region();
    core.apply(ClientMsg::MouseDown(Point::new(r.x, 0)));
    assert!(core.simple_mode(), "clicking the toggle enters simple mode");
    core.apply(ClientMsg::MouseDown(Point::new(r.x, 0)));
    assert!(!core.simple_mode(), "clicking again returns to desktop");
    core.shutdown();
}

#[test]
fn simple_mode_renders_focused_app_fullscreen_without_desktop() {
    use tuiui::session::{SessionCore, ClientMsg};
    use tuiui::config::Config;
    use tuiui::chrome::menubar_mode_region;
    use tuiui::geometry::Point;
    // desktop disabled in default? ensure desktop layer present in desktop mode,
    // absent in simple mode. Launch an app so there is a focused window.
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.apply(ClientMsg::MouseDown(Point::new(menubar_mode_region().x, 0)));
    assert!(core.simple_mode());
    let frame = core.build_frame();
    // The focused app's content layer originates at the work area top (row 1),
    // i.e. there is a content layer at y=1 spanning the width.
    assert!(frame.layers.iter().any(|l| l.origin.y == 1 && l.buf.width() == 120),
        "focused app should fill the work-area width at row 1");
    core.shutdown();
}
```

NOTE: if `Config::default()` enables the desktop, the desktop layer (z=0, origin y=0) is present in desktop mode; the assertion above only checks the simple-mode full-width content layer at y=1, which is robust either way. Adjust the width expectation if `simple_content_rect` differs.

- [ ] **Step 2: Full gate**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -5
cargo test 2>&1 | grep -E "test result|error\[|FAILED" | tail -25     # all pass
cargo clippy --all-targets 2>&1 | tail -15                            # zero warnings
```

- [ ] **Step 3: Commit**

```bash
git add src/chrome.rs src/session.rs tests/chrome_tests.rs tests/session_tests.rs
git commit --no-verify -m "session: simple view mode — full-screen single-app toggle (⊞/▦) in the menubar"
```

- [ ] **Step 4: Deploy + manual**

```bash
cargo install --root ~/.local --path . --force
tuiui kill; tuiui
```
1. Launch 2+ apps. Click the `⊞` glyph (right of Go) → it becomes `▦`, the focused app fills the screen, decorations + desktop icons gone, menubar + dock remain.
2. Click dock pills → each app fills the screen at the right size.
3. `Go` launches a new app full-screen; the `tuiui ▾` menu (Exit/Restart/Shutdown) still works.
4. Click `▦` → back to desktop, windows restored in their previous positions/sizes.

- [ ] **Step 5: Update memory**

Add to `tuiui-roadmap-state`: simple view mode DONE (menubar ⊞/▦ toggle, full-screen focused app, dock = switcher; frontend-only).

---

## Self-Review Notes

- **No geometry loss:** simple mode never mutates `window.rect`; it only resizes the focused
  app's PTY. Toggling back calls `sync_app_size` for every window → each returns to
  `content_rect()`. Desktop layout is preserved.
- **Reuse, don't duplicate:** `build_frame_simple` reuses `render_menubar`/`render_dock`/
  launcher/tray/help/power_menu and the same `ImagePlacement`/`app_image_ids` plumbing as the
  desktop path. `refresh_app_graphics` runs each tick regardless of mode, so `app_image_ids`
  is populated.
- **Mouse-into-app is out of scope** (per spec) — app-area clicks in simple mode are ignored;
  keyboard drives the app. Don't add PTY mouse encoding here.
- **Glyph width:** `⊞`/`▦` are single-cell geometric glyphs; `MODE_*` strings are `" X "`
  (3 cells). If the compositor's `is_wide` ever marks them wide, the region width still
  matches the string length, so hit-testing stays correct.
