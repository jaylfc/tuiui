# Apphost Phase 2a — AppHost Trait Seam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `SessionCore`'s concrete `LocalAppHost` field with a `Box<dyn AppHost>` behind a trait, and redesign the graphics accessor (drop the `MutexGuard` return in favor of owned `placements`/`image_png`) so the same API can later be served over a socket — all with zero user-visible change.

**Architecture:** Define an `AppHost` trait whose methods all return owned/copyable data (no lock guards). `LocalAppHost` implements it. `SessionCore` owns a `Box<dyn AppHost>` injected at construction (defaulting to `LocalAppHost`), so Phase 2b can inject a `RemoteAppHost` instead. Behavior-preserving: the full suite stays green.

**Tech Stack:** Rust 2021, existing `src/apphost/host.rs` (`AppId`, `LocalAppHost`), `crate::buffer::CellBuffer`, `crate::kittygfx::{Placement, GraphicsState}`.

**Reference:** Design spec `docs/superpowers/specs/2026-06-07-apphost-frontend-split-design.md` (Phase 2 section).

---

## Background (read before starting)

- `src/apphost/host.rs` currently exposes `LocalAppHost` with **inherent** methods: `spawn/input/resize/kill/is_alive/snapshot/graphics/list/set_meta/meta/remove`. `graphics(id) -> Option<MutexGuard<'_, GraphicsState>>` is the problem child — a lock guard cannot cross a process boundary.
- `src/session.rs` uses the host via the field `apphost: LocalAppHost` (added in Phase 1):
  - `render` → `host.snapshot(*id)`
  - `refresh_app_graphics` (~line 533) → `self.apphost.graphics(*aid)` then reads `g.placements` and `g.png(pl.image_id)`
  - `build_frame` graphics loop (~line 1948) → `self.apphost.graphics(aid)` then reads `g.placements`
  - `inject_app_graphics_for_test` (~line 565) → `self.apphost.graphics(aid)` then `g.insert_image_for_test` / `g.push_placement_for_test`
- `Placement` (`src/kittygfx.rs:162`) = `{ image_id: u32, col: u16, row: u16, cols: u16, rows: u16 }`, derives `Clone, Debug, PartialEq, Eq`.
- `GraphicsState::png(id: u32) -> Option<&[u8]>`, `placements: Vec<Placement>`, and the test hooks `insert_image_for_test(id, png)` / `push_placement_for_test(Placement)`.

---

## Task 1: Define the `AppHost` trait and implement it for `LocalAppHost`

**Files:**
- Create: `src/apphost/api.rs`
- Modify: `src/apphost/mod.rs`
- Modify: `src/apphost/host.rs`

- [ ] **Step 1: Create the trait**

Create `src/apphost/api.rs`:

```rust
//! The `AppHost` trait — the stable seam between the frontend (`SessionCore`)
//! and whatever owns the running apps. `LocalAppHost` implements it in-process;
//! Phase 2b adds a `RemoteAppHost` that implements the identical API over a
//! socket. Every method returns owned/copyable data (no lock guards) so it can
//! cross a process boundary unchanged.

use super::AppId;
use crate::buffer::CellBuffer;
use crate::kittygfx::Placement;
use std::path::Path;

pub trait AppHost: Send {
    /// Spawn a child in a PTY and return its handle. Propagates spawn failure.
    fn spawn(
        &mut self,
        cmd: &str,
        args: &[String],
        cwd: Option<&Path>,
        cols: i32,
        rows: i32,
    ) -> std::io::Result<AppId>;

    /// Forward bytes to the app's PTY (no-op if unknown).
    fn input(&mut self, id: AppId, bytes: &[u8]);

    /// Resize the app's PTY/terminal (no-op if unknown).
    fn resize(&mut self, id: AppId, cols: i32, rows: i32);

    /// Terminate the child (no-op if unknown).
    fn kill(&mut self, id: AppId);

    /// Whether the child is still running. Unknown ids report `false`.
    fn is_alive(&mut self, id: AppId) -> bool;

    /// Current terminal grid for the app, or `None` if unknown.
    fn snapshot(&self, id: AppId) -> Option<CellBuffer>;

    /// Image placements the app currently declares (cell coords on its grid).
    fn placements(&self, id: AppId) -> Vec<Placement>;

    /// PNG bytes for one of the app's transmitted images, if present.
    fn image_png(&self, id: AppId, image_id: u32) -> Option<Vec<u8>>;

    /// All currently-hosted app handles (order unspecified).
    fn list(&self) -> Vec<AppId>;

    /// Store opaque frontend metadata (window geometry/title/z) for restore.
    fn set_meta(&mut self, id: AppId, meta: Vec<u8>);

    /// The last metadata stored for the app, if any (owned copy).
    fn meta(&self, id: AppId) -> Option<Vec<u8>>;

    /// Drop the host's tracking of the app (does not kill).
    fn remove(&mut self, id: AppId);

    /// Test hook: inject a placement + image into the app's graphics state.
    /// Default no-op; `LocalAppHost` overrides it. Used only by integration tests.
    #[doc(hidden)]
    fn inject_test_image(&self, _id: AppId, _png: &[u8]) {}
}
```

- [ ] **Step 2: Re-export the trait**

In `src/apphost/mod.rs`, add the module and export:

```rust
mod api;
mod host;

pub use api::AppHost;
pub use host::{AppId, LocalAppHost};
```

- [ ] **Step 3: Convert `LocalAppHost`'s inherent methods into the trait impl**

In `src/apphost/host.rs`:
1. Add the import: `use crate::apphost::AppHost;` and `use crate::kittygfx::Placement;` (keep existing imports; `GraphicsState`/`MutexGuard` are no longer needed once `graphics` is gone — remove those two imports).
2. Keep `LocalAppHost::new` and the `spawn` body, but move ALL the public methods (`spawn/input/resize/kill/is_alive/snapshot/list/set_meta/meta/remove`) out of the inherent `impl LocalAppHost` block and into `impl AppHost for LocalAppHost`. Keep `new` as an inherent method (constructors are not part of the trait).
3. Replace `graphics()` with `placements()` + `image_png()`, and add `inject_test_image`. The new graphics-related methods lock the app's `GraphicsState` internally and return owned data:

```rust
impl LocalAppHost {
    pub fn new() -> Self {
        LocalAppHost { apps: HashMap::new(), meta: HashMap::new(), next: 1 }
    }
}

impl AppHost for LocalAppHost {
    fn spawn(
        &mut self,
        cmd: &str,
        args: &[String],
        cwd: Option<&Path>,
        cols: i32,
        rows: i32,
    ) -> std::io::Result<AppId> {
        let app = AppInstance::spawn(cmd, args, cols, rows, cwd)?;
        let id = AppId(self.next);
        self.next += 1;
        self.apps.insert(id, app);
        Ok(id)
    }

    fn input(&mut self, id: AppId, bytes: &[u8]) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.write_input(bytes);
        }
    }

    fn resize(&mut self, id: AppId, cols: i32, rows: i32) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.resize(cols, rows);
        }
    }

    fn kill(&mut self, id: AppId) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.kill();
        }
    }

    fn is_alive(&mut self, id: AppId) -> bool {
        self.apps.get_mut(&id).map(|a| a.is_alive()).unwrap_or(false)
    }

    fn snapshot(&self, id: AppId) -> Option<CellBuffer> {
        self.apps.get(&id).map(|a| a.snapshot())
    }

    fn placements(&self, id: AppId) -> Vec<Placement> {
        self.apps
            .get(&id)
            .map(|a| a.graphics().placements.clone())
            .unwrap_or_default()
    }

    fn image_png(&self, id: AppId, image_id: u32) -> Option<Vec<u8>> {
        self.apps.get(&id).and_then(|a| a.graphics().png(image_id).map(|b| b.to_vec()))
    }

    fn list(&self) -> Vec<AppId> {
        self.apps.keys().copied().collect()
    }

    fn set_meta(&mut self, id: AppId, meta: Vec<u8>) {
        self.meta.insert(id, meta);
    }

    fn meta(&self, id: AppId) -> Option<Vec<u8>> {
        self.meta.get(&id).cloned()
    }

    fn remove(&mut self, id: AppId) {
        self.apps.remove(&id);
        self.meta.remove(&id);
    }

    fn inject_test_image(&self, id: AppId, png: &[u8]) {
        if let Some(app) = self.apps.get(&id) {
            let mut g = app.graphics();
            g.insert_image_for_test(1, png.to_vec());
            g.push_placement_for_test(Placement {
                image_id: 1,
                col: 0,
                row: 0,
                cols: 2,
                rows: 1,
            });
        }
    }
}
```

- [ ] **Step 4: Fix the host unit tests for the new API**

The test module in `host.rs` calls `host.spawn(...)`, `host.list()`, etc. — now trait methods, so add `use crate::apphost::AppHost;` to the `tests` module. The `meta` tests compared against `&[..]` slices; `meta` now returns `Option<Vec<u8>>`:

```rust
        // in meta_round_trips:
        assert_eq!(host.meta(id), Some(vec![1, 2, 3]));
        host.set_meta(id, vec![9]);
        assert_eq!(host.meta(id), Some(vec![9]));
```

- [ ] **Step 5: Build + test the module**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test apphost:: 2>&1 | tail -25`
Expected: all `apphost::host::tests` pass.
Run: `cargo clippy --all-targets 2>&1 | tail -20`
Expected: zero warnings. (Note: `src/session.rs` will NOT compile yet because it calls the old `graphics()` — that is fixed in Task 2. To check this task in isolation, you may temporarily run `cargo test --lib apphost`. The full build is restored in Task 2; do both tasks before the final gate. Commit anyway after Task 1 since the module compiles on its own — but if `cargo build` fails due to session.rs, proceed directly to Task 2 and commit them together. Prefer: do Task 1 + Task 2, then a single commit per task once each compiles.)

NOTE on commit timing: because removing `graphics()` breaks `session.rs` until Task 2, do Task 1 and Task 2 as one continuous edit, then build once, then make the two commits. If you must commit Task 1 alone, leave a temporary `graphics()` shim — but cleaner to land Tasks 1+2 together.

---

## Task 2: Route `SessionCore` through `Box<dyn AppHost>`

**Files:**
- Modify: `src/session.rs`

- [ ] **Step 1: Imports + field type**

Change the import (Phase 1 added `use crate::apphost::{AppId, LocalAppHost};`) to:

```rust
use crate::apphost::{AppHost, AppId, LocalAppHost};
```

Change the struct field:

```rust
    apphost: Box<dyn AppHost>,
```

- [ ] **Step 2: Constructor injection**

Find `SessionCore::new` (it builds the struct literal with `apphost: LocalAppHost::new(),`). Replace the initializer with `apphost,` (field shorthand) and add an injectable constructor. Locate the `pub fn new(w: i32, h: i32, cfg: Config) -> Self` signature; change its body to delegate:

```rust
    pub fn new(w: i32, h: i32, cfg: Config) -> Self {
        Self::with_apphost(w, h, cfg, Box::new(LocalAppHost::new()))
    }

    /// Construct a session backed by a specific [`AppHost`]. The daemon injects
    /// a `RemoteAppHost` here (Phase 2b); tests and in-process use get the
    /// default `LocalAppHost` via [`new`](Self::new).
    pub fn with_apphost(w: i32, h: i32, cfg: Config, apphost: Box<dyn AppHost>) -> Self {
        // ... the existing body of `new`, but with `apphost` bound from the
        // parameter instead of `apphost: LocalAppHost::new()` ...
    }
```

Concretely: move the entire current body of `new` into `with_apphost`, change the struct-literal line `apphost: LocalAppHost::new(),` to `apphost,`, and keep everything else (the `let launcher = ...`, `generate_role_icons`, etc.) identical. Then make `new` the two-line delegator above. Watch for any early `let` bindings in `new` that must move into `with_apphost`.

- [ ] **Step 3: `refresh_app_graphics` — use `placements` + `image_png`**

Replace the first-pass loop body:

```rust
        for (id, content) in &self.contents {
            if let WinContent::App(aid) = content {
                for pl in self.apphost.placements(*aid) {
                    if self.app_image_ids.contains_key(&(*id, pl.image_id)) {
                        continue;
                    }
                    if let Some(png) = self.apphost.image_png(*aid, pl.image_id) {
                        needed.push((*id, pl.image_id, png));
                    }
                }
            }
        }
```

(`png` is now an owned `Vec<u8>` — push it directly; the old code did `png.to_vec()`.) Leave the second pass (`self.images.load_bytes` + `app_image_ids.insert`) unchanged.

- [ ] **Step 4: `build_frame` graphics loop — use `placements`**

Replace the per-window block that acquired the graphics guard:

```rust
            let aid = match self.contents.get(&w.id) {
                Some(WinContent::App(aid)) => *aid,
                _ => continue,
            };
            let placements = self.apphost.placements(aid);
            if placements.is_empty() {
                continue;
            }
            let cr = w.content_rect();
            let vis = self.fully_unobstructed(w);
            for pl in &placements {
                if let Some(&img) = self.app_image_ids.get(&(w.id, pl.image_id)) {
                    let x = cr.x + pl.col as i32;
                    let y = cr.y + pl.row as i32;
                    if x >= cr.x + cr.w || y >= cr.y + cr.h {
                        continue;
                    }
                    let cols = pl.cols.min((cr.x + cr.w - x).max(1) as u16);
                    let rows = pl.rows.min((cr.y + cr.h - y).max(1) as u16);
                    images.push(crate::protocol::ImagePlacement {
                        id: img,
                        rect: crate::geometry::Rect::new(x, y, cols as i32, rows as i32),
                        cols,
                        rows,
                        visible: vis,
                    });
                }
            }
```

Keep the surrounding `for w in self.wm.z_ordered() { if w.minimized { continue; } ... }` wrapper. (The body above replaces what used `g.placements`; preserve the exact clipping math — copy it from the current code if it differs in any detail.)

- [ ] **Step 5: `inject_app_graphics_for_test` — use the trait hook**

Replace its body:

```rust
    #[doc(hidden)]
    pub fn inject_app_graphics_for_test(&mut self, png: &[u8]) {
        let app_id = self
            .titles
            .iter()
            .rev()
            .map(|(id, _)| *id)
            .find_map(|id| match self.contents.get(&id) {
                Some(WinContent::App(aid)) => Some(*aid),
                _ => None,
            });
        if let Some(aid) = app_id {
            self.apphost.inject_test_image(aid, png);
        }
        self.refresh_app_graphics();
    }
```

- [ ] **Step 6: Build + full suite + clippy**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build 2>&1 | tail -20`
Expected: clean. Fix any remaining `graphics()` call site the compiler flags (search `src/session.rs` for `.graphics(` — must be zero).
Run: `cargo test 2>&1 | grep -E "test result|error\[|FAILED" | tail -20`
Expected: every test passes (210+ from Phase 1). The `image_window_emits_a_visible_placement` / A2 graphics tests exercise `inject_app_graphics_for_test` — they must still pass.
Run: `cargo clippy --all-targets 2>&1 | tail -20`
Expected: zero warnings. (A common one: `Box<dyn AppHost>` may need the trait to be object-safe — it is, since no method is generic and `inject_test_image` has a default. If clippy suggests `Box<dyn AppHost + Send>`, the `Send` supertrait already covers it.)

- [ ] **Step 7: Commit (Tasks 1 + 2 together)**

```bash
git add src/apphost/api.rs src/apphost/mod.rs src/apphost/host.rs src/session.rs
git commit --no-verify -m "apphost: introduce AppHost trait; SessionCore owns Box<dyn AppHost>

Replaces the MutexGuard graphics accessor with owned placements()/image_png()
so the API can cross a process boundary. LocalAppHost implements the trait;
SessionCore takes an injected host via with_apphost(). Behavior-preserving."
```

---

## Task 3: Verify behavior-preserving

**Files:** none (verification)

- [ ] **Step 1: No stray lock-guard accessors remain**

Run: `grep -rn "\.graphics(" src/session.rs`
Expected: no matches.

- [ ] **Step 2: Full gate**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build 2>&1 | tail -3 && cargo test 2>&1 | grep -cE "test result: ok" && cargo clippy --all-targets 2>&1 | grep -cE "warning|error"`
Expected: build OK; test-suite count unchanged or higher; clippy count `0`.

- [ ] **Step 3: Smoke (deploy + run)**

```bash
cargo install --root ~/.local --path . --force
tuiui kill; tuiui
```
Launch an app, type into it, open a graphics app (chafa/yazi) — graphics still render. Identical to before.

---

## Self-Review Notes

- **Object safety:** `AppHost` is object-safe (no generics, no `Self`-returning methods, `Send` supertrait). `Box<dyn AppHost>` works. `with_apphost` takes `Box<dyn AppHost>`.
- **No behavior change:** `placements()`/`image_png()` return clones of exactly what the old guard exposed; the build_frame clipping math is copied verbatim. The only difference is a clone per frame of the (tiny) placements vec.
- **Test hook:** `inject_test_image` hard-codes image id `1` + a 2×1 placement at (0,0), matching the previous inline injection exactly — the A2 graphics integration test depends on this.
- **`meta` return type changed** from `Option<&[u8]>` to `Option<Vec<u8>>`; only host unit tests referenced it (updated in Task 1 Step 4). It is unused elsewhere until Phase 3.
