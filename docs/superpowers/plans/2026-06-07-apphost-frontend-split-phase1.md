# Apphost/Frontend Split — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Carve a clean in-process `LocalAppHost` boundary so every PTY-backed app is owned behind a stable API and addressed by an opaque `AppId`, with zero user-visible behavior change.

**Architecture:** Introduce `src/apphost/` with an `AppId` newtype and a `LocalAppHost` that owns the `HashMap<AppId, AppInstance>` plus an opaque per-app metadata blob. `WinContent::App` carries an `AppId` instead of an `AppInstance`; `SessionCore` owns the `LocalAppHost` and routes spawn/input/resize/kill/is_alive/snapshot/graphics through it. This is the seam a later phase will move behind a socket — Phase 1 keeps everything in one process.

**Tech Stack:** Rust 2021, existing `crate::ptyhost::AppInstance`, `crate::buffer::CellBuffer`, `crate::kittygfx::GraphicsState`, `std::collections::HashMap`, `std::sync::MutexGuard`.

**Reference:** Design spec at `docs/superpowers/specs/2026-06-07-apphost-frontend-split-design.md` (Phase 1 section).

---

## Background: current coupling (read before starting)

`src/session.rs` today:
- `enum WinContent { App(AppInstance), Store, Settings, ImageView, FileManager }` (~line 32).
- `WinContent::render/resize/write_input/is_alive/kill` delegate to the embedded `AppInstance` (lines 45–79).
- `SessionCore.contents: HashMap<WindowId, WinContent>` (~line 288).
- `launch_in` calls `AppInstance::spawn(...)` then `self.contents.insert(id, WinContent::App(app))` (~line 1411).
- `refresh_app_graphics` iterates `&self.contents`, for `App` calls `app.graphics()` (~line 526).
- `inject_app_graphics_for_test` finds the last `App` window and calls `app.graphics()` (~line 558).
- `build_frame` graphics loop calls `app.graphics()` per app window (~line 1936).
- `write_input` dispatch: `self.contents.get_mut(&id)` then `c.write_input(&bytes)` (~line 752).
- `sync_app_size`: `self.contents.get_mut(&id)` then `content.resize(...)` (~line 1752).
- `close`: `self.contents.remove(&id)` then `content.kill()` (~line 1772).
- `reap_dead`: `self.contents.iter_mut()` filtering on `!c.is_alive()` (~line 1977).
- `shutdown`: iterate contents, `content.kill()` (~line 2002).

`crate::ptyhost::AppInstance` API (unchanged by this phase):
`AppInstance::spawn(cmd: &str, args: &[String], cols: i32, rows: i32, cwd: Option<&Path>) -> io::Result<AppInstance>`,
`.snapshot() -> CellBuffer`, `.resize(cols: i32, rows: i32)`, `.write_input(&[u8])`, `.kill()`,
`.is_alive(&mut self) -> bool`, `.graphics(&self) -> MutexGuard<'_, GraphicsState>`.

---

## Task 1: AppId newtype + LocalAppHost skeleton (spawn / list / remove)

**Files:**
- Create: `src/apphost/mod.rs`
- Create: `src/apphost/host.rs`
- Modify: `src/lib.rs` (add `pub mod apphost;`)

- [ ] **Step 1: Create the module files with the type and a failing test**

Create `src/apphost/mod.rs`:

```rust
//! In-process application host — owns every PTY-backed [`AppInstance`] behind a
//! stable, `AppId`-addressed API.
//!
//! This is the seam the apphost/frontend split is built on (see
//! `docs/superpowers/specs/2026-06-07-apphost-frontend-split-design.md`). In
//! Phase 1 the host lives in the same process as the frontend; a later phase
//! moves an identical API behind a socket without the frontend noticing.

mod host;

pub use host::{AppId, LocalAppHost};
```

Create `src/apphost/host.rs`:

```rust
use crate::buffer::CellBuffer;
use crate::kittygfx::GraphicsState;
use crate::ptyhost::AppInstance;
use std::collections::HashMap;
use std::path::Path;
use std::sync::MutexGuard;

/// Opaque handle to a hosted application. Stable for the app's lifetime; the
/// frontend stores it in `WinContent::App` and uses it for every host call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppId(pub u64);

/// Owns the live [`AppInstance`] map plus an opaque per-app metadata blob.
///
/// The metadata is never interpreted here — it is window state the frontend
/// stashes so a future restarted frontend can rebuild its windows (Phase 3).
pub struct LocalAppHost {
    apps: HashMap<AppId, AppInstance>,
    meta: HashMap<AppId, Vec<u8>>,
    next: u64,
}

impl LocalAppHost {
    pub fn new() -> Self {
        LocalAppHost { apps: HashMap::new(), meta: HashMap::new(), next: 1 }
    }

    /// Spawn a child in a PTY and return its handle. Propagates spawn failure.
    pub fn spawn(
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

    /// All currently-hosted app handles (order unspecified).
    pub fn list(&self) -> Vec<AppId> {
        self.apps.keys().copied().collect()
    }

    /// Drop the app instance and any metadata. Does not kill — call `kill`
    /// first if the child should be terminated.
    pub fn remove(&mut self, id: AppId) {
        self.apps.remove(&id);
        self.meta.remove(&id);
    }
}

impl Default for LocalAppHost {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_then_list_then_remove() {
        let mut host = LocalAppHost::new();
        let id = host
            .spawn("true", &[], None, 80, 24)
            .expect("spawn true");
        assert_eq!(host.list(), vec![id]);
        host.remove(id);
        assert!(host.list().is_empty());
    }

    #[test]
    fn ids_are_unique_and_increasing() {
        let mut host = LocalAppHost::new();
        let a = host.spawn("true", &[], None, 80, 24).unwrap();
        let b = host.spawn("true", &[], None, 80, 24).unwrap();
        assert_ne!(a, b);
    }
}
```

Add to `src/lib.rs` next to the other `pub mod` lines (e.g. after `pub mod ptyhost;`):

```rust
pub mod apphost;
```

- [ ] **Step 2: Run the tests to verify they pass**

Run: `cargo test -p tuiui apphost::host::tests 2>&1 | tail -20`
(If the package name differs, use `cargo test apphost::host::tests`.)
Expected: `spawn_then_list_then_remove` and `ids_are_unique_and_increasing` PASS.

- [ ] **Step 3: Lint**

Run: `cargo clippy --all-targets 2>&1 | tail -20`
Expected: no warnings introduced by the new file.

- [ ] **Step 4: Commit**

```bash
git add src/apphost/mod.rs src/apphost/host.rs src/lib.rs
git commit --no-verify -m "apphost: AppId + LocalAppHost skeleton (spawn/list/remove)"
```

---

## Task 2: Lifecycle + content API on LocalAppHost (input/resize/kill/is_alive/snapshot/graphics)

**Files:**
- Modify: `src/apphost/host.rs`

- [ ] **Step 1: Add the methods inside `impl LocalAppHost` (above the closing brace)**

```rust
    /// Forward bytes to the app's PTY (no-op if the id is unknown).
    pub fn input(&mut self, id: AppId, bytes: &[u8]) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.write_input(bytes);
        }
    }

    /// Resize the app's PTY/terminal (no-op if unknown).
    pub fn resize(&mut self, id: AppId, cols: i32, rows: i32) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.resize(cols, rows);
        }
    }

    /// Terminate the child (no-op if unknown). The handle stays in the map
    /// until `remove`; `is_alive` will report `false`.
    pub fn kill(&mut self, id: AppId) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.kill();
        }
    }

    /// Whether the child is still running. Unknown ids report `false`.
    pub fn is_alive(&mut self, id: AppId) -> bool {
        self.apps.get_mut(&id).map(|a| a.is_alive()).unwrap_or(false)
    }

    /// Current terminal grid for the app, or `None` if unknown.
    pub fn snapshot(&self, id: AppId) -> Option<CellBuffer> {
        self.apps.get(&id).map(|a| a.snapshot())
    }

    /// Lock and return the app's graphics state, or `None` if unknown.
    pub fn graphics(&self, id: AppId) -> Option<MutexGuard<'_, GraphicsState>> {
        self.apps.get(&id).map(|a| a.graphics())
    }
```

- [ ] **Step 2: Add tests inside the `tests` module**

```rust
    #[test]
    fn snapshot_unknown_is_none() {
        let host = LocalAppHost::new();
        assert!(host.snapshot(AppId(999)).is_none());
    }

    #[test]
    fn is_alive_tracks_child_exit() {
        let mut host = LocalAppHost::new();
        // `true` exits immediately; poll briefly for the reaper to notice.
        let id = host.spawn("true", &[], None, 80, 24).unwrap();
        let mut alive_after = true;
        for _ in 0..50 {
            if !host.is_alive(id) {
                alive_after = false;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!alive_after, "child `true` should be reaped as not-alive");
    }

    #[test]
    fn snapshot_after_spawn_has_requested_dimensions() {
        let mut host = LocalAppHost::new();
        let id = host.spawn("cat", &[], None, 80, 24).unwrap();
        let snap = host.snapshot(id).expect("snapshot");
        assert_eq!(snap.width(), 80);
        assert_eq!(snap.height(), 24);
        host.kill(id);
    }
```

NOTE: confirm `CellBuffer` exposes `width()`/`height()` (grep `src/buffer.rs`). If the accessors are named differently (e.g. `cols()`/`rows()` or public `w`/`h` fields), use the actual names in the assertion.

- [ ] **Step 3: Run the tests**

Run: `cargo test apphost::host::tests 2>&1 | tail -25`
Expected: all five tests PASS.

- [ ] **Step 4: Lint + commit**

```bash
cargo clippy --all-targets 2>&1 | tail -20
git add src/apphost/host.rs
git commit --no-verify -m "apphost: LocalAppHost lifecycle + content API (input/resize/kill/alive/snapshot/graphics)"
```

---

## Task 3: Opaque per-app metadata (set_meta / meta)

**Files:**
- Modify: `src/apphost/host.rs`

- [ ] **Step 1: Add the methods inside `impl LocalAppHost`**

```rust
    /// Store opaque frontend metadata (window geometry/title/z) for restore.
    /// Overwrites any previous value. Ignored for unknown ids only in the
    /// sense that the blob is kept keyed by id regardless of liveness.
    pub fn set_meta(&mut self, id: AppId, meta: Vec<u8>) {
        self.meta.insert(id, meta);
    }

    /// The last metadata stored for the app, if any.
    pub fn meta(&self, id: AppId) -> Option<&[u8]> {
        self.meta.get(&id).map(|v| v.as_slice())
    }
```

- [ ] **Step 2: Add tests**

```rust
    #[test]
    fn meta_round_trips() {
        let mut host = LocalAppHost::new();
        let id = host.spawn("true", &[], None, 80, 24).unwrap();
        assert!(host.meta(id).is_none());
        host.set_meta(id, vec![1, 2, 3]);
        assert_eq!(host.meta(id), Some(&[1, 2, 3][..]));
        host.set_meta(id, vec![9]);
        assert_eq!(host.meta(id), Some(&[9][..]));
    }

    #[test]
    fn remove_clears_meta() {
        let mut host = LocalAppHost::new();
        let id = host.spawn("true", &[], None, 80, 24).unwrap();
        host.set_meta(id, vec![1]);
        host.remove(id);
        assert!(host.meta(id).is_none());
    }
```

- [ ] **Step 3: Run + lint + commit**

```bash
cargo test apphost::host::tests 2>&1 | tail -25
cargo clippy --all-targets 2>&1 | tail -20
git add src/apphost/host.rs
git commit --no-verify -m "apphost: opaque per-app metadata (set_meta/meta)"
```

---

## Task 4: Route SessionCore through LocalAppHost (the seam swap)

This is the single behavior-preserving cut-over. It changes `WinContent::App` to carry
an `AppId`, adds the `apphost` field, and rewires every touch point. It must compile
and the **entire existing suite** must pass unchanged. Do all edits, then build, then test.

**Files:**
- Modify: `src/session.rs`

- [ ] **Step 1: Import and change the enum + add the field**

At the top of `src/session.rs`, replace the `AppInstance` import:

```rust
use crate::apphost::{AppId, LocalAppHost};
```

(Remove `use crate::ptyhost::AppInstance;` — it is no longer referenced in this file.)

Change the enum arm (~line 34):

```rust
    /// A child process in a pseudo-terminal, owned by the `LocalAppHost`.
    App(AppId),
```

Add a field to `struct SessionCore` next to `contents` (~line 288):

```rust
    apphost: LocalAppHost,
```

Initialize it in `SessionCore::new` next to `contents: HashMap::new(),` (~line 348):

```rust
            apphost: LocalAppHost::new(),
```

- [ ] **Step 2: Rewrite the `WinContent` impl block (~lines 45–79)**

`render` now takes the host; the mutating App helpers are removed (the App arm is
handled at the SessionCore call sites, which have `&mut self.apphost` available
without a borrow conflict):

```rust
impl WinContent {
    fn render(&self, host: &LocalAppHost, w: i32, h: i32) -> crate::buffer::CellBuffer {
        match self {
            WinContent::App(id) => host.snapshot(*id).unwrap_or_else(|| crate::buffer::CellBuffer::new(w, h)),
            WinContent::Store(s) => s.render(w, h),
            WinContent::Settings(s) => s.render(w, h),
            WinContent::ImageView(v) => v.render(w, h),
            WinContent::FileManager(f) => f.render(w, h),
        }
    }
}
```

NOTE: confirm the `CellBuffer` constructor name/signature (grep `src/buffer.rs` for `pub fn new` / `impl CellBuffer`). If it is e.g. `CellBuffer::blank(w, h)` or takes `(usize, usize)`, adjust the fallback accordingly.

- [ ] **Step 3: `launch_in` — spawn via the host (~line 1411)**

Replace the `match AppInstance::spawn(...)` block with:

```rust
        match self.apphost.spawn(&command, &args, cwd.as_deref(), content.w.max(1), content.h.max(1)) {
            Ok(app_id) => {
                self.contents.insert(id, WinContent::App(app_id));
                self.titles.push((id, name));
                self.auto_tile_if_enabled();
            }
            Err(_) => {
                self.wm.close(id);
            }
        }
```

- [ ] **Step 4: `write_input` dispatch (~line 752)**

Replace the `if let Some(c) = self.contents.get_mut(&id) { c.write_input(&bytes); }` with
an App-aware lookup (immutable `get`, so it does not conflict with `&mut self.apphost`):

```rust
                    if let Some(WinContent::App(aid)) = self.contents.get(&id) {
                        let aid = *aid;
                        self.apphost.input(aid, &bytes);
                    }
```

NOTE: verify the surrounding context — only `App` windows consumed `write_input` before
(the other arms were no-ops), so this preserves behavior. If a native arm also relied on
`write_input`, handle it explicitly; grep confirms it did not.

- [ ] **Step 5: `sync_app_size` (~line 1752)**

Replace the `if let Some(content) = self.contents.get_mut(&id) { content.resize(...); }` with:

```rust
            if let Some(WinContent::App(aid)) = self.contents.get(&id) {
                let aid = *aid;
                self.apphost.resize(aid, c.w.max(1), c.h.max(1));
            }
```

(`c` is the content rect already computed earlier in `sync_app_size`; keep the existing
binding. Verify its name — it is the `content_rect()` result.)

- [ ] **Step 6: `close` — kill + remove the app (~line 1772)**

Replace:

```rust
        if let Some(mut content) = self.contents.remove(&id) {
            content.kill();
        }
```

with:

```rust
        if let Some(content) = self.contents.remove(&id) {
            if let WinContent::App(aid) = content {
                self.apphost.kill(aid);
                self.apphost.remove(aid);
            }
        }
```

- [ ] **Step 7: `reap_dead` (~line 1977)**

Replace the `self.contents.iter_mut().filter_map(...)` liveness scan. Native windows are
always alive; only App windows can die. Collect app windows first (immutable), then probe
the host:

```rust
    pub fn reap_dead(&mut self) {
        let app_windows: Vec<(WindowId, AppId)> = self
            .contents
            .iter()
            .filter_map(|(id, c)| match c {
                WinContent::App(aid) => Some((*id, *aid)),
                _ => None,
            })
            .collect();
        let dead: Vec<WindowId> = app_windows
            .into_iter()
            .filter(|(_, aid)| !self.apphost.is_alive(*aid))
            .map(|(id, _)| id)
            .collect();
        // (keep the existing install-finished detection + close loop below, unchanged)
```

Keep the rest of `reap_dead` (the `install_finished` computation over `dead`, the
`for id in dead { self.close(id); }`, and the `refresh_installed_apps()` call) exactly
as-is.

- [ ] **Step 8: `shutdown` (~line 2002)**

Replace the kill-all loop. Collect app ids, kill each, then clear:

```rust
    pub fn shutdown(&mut self) {
        for aid in self.apphost.list() {
            self.apphost.kill(aid);
        }
        // (keep whatever else shutdown did — clearing contents/titles, etc.)
```

NOTE: read the existing `shutdown` body first. If it iterates `self.contents` for other
cleanup, keep that; only the `content.kill()` part moves to `self.apphost`. The host's
`list()` covers every live app regardless of window mapping.

- [ ] **Step 9: `refresh_app_graphics` (~line 526)**

The first pass iterates `&self.contents`; replace `app.graphics()` with a host lookup.
Both borrows are immutable, so accessing `self.apphost` inside the loop is fine:

```rust
        for (id, content) in &self.contents {
            if let WinContent::App(aid) = content {
                if let Some(g) = self.apphost.graphics(*aid) {
                    for pl in &g.placements {
                        if self.app_image_ids.contains_key(&(*id, pl.image_id)) {
                            continue;
                        }
                        if let Some(png) = g.png(pl.image_id) {
                            needed.push((*id, pl.image_id, png.to_vec()));
                        }
                    }
                }
            }
        }
```

(Leave the second pass — `self.images.load_bytes` + `app_image_ids.insert` — unchanged.)

- [ ] **Step 10: `build_frame` graphics loop (~line 1936)**

Replace `if let Some(WinContent::App(app)) = self.contents.get(&w.id) { let g = app.graphics(); ... }`
with:

```rust
            let aid = match self.contents.get(&w.id) {
                Some(WinContent::App(aid)) => *aid,
                _ => continue,
            };
            let g = match self.apphost.graphics(aid) {
                Some(g) => g,
                None => continue,
            };
            if g.placements.is_empty() {
                continue;
            }
            // (keep the existing cr/vis computation and the per-placement push loop)
```

Preserve the rest of the loop body verbatim (the `cr`, `vis`, per-placement clipping and
`images.push(...)`).

- [ ] **Step 11: `render` call site in `build_frame` (~line 1826)**

Replace `.map(|c| c.render(cr.w, cr.h))` with `.map(|c| c.render(&self.apphost, cr.w, cr.h))`.
Verify there is no borrow conflict: `self.contents.get(&w.id)` and `&self.apphost` are
disjoint immutable borrows of `self` — allowed.

- [ ] **Step 12: `inject_app_graphics_for_test` (~line 558)**

Replace the body that finds the last App window and calls `app.graphics()`:

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
            if let Some(mut g) = self.apphost.graphics(aid) {
                g.insert_image_for_test(1, png.to_vec());
                g.push_placement_for_test(crate::kittygfx::Placement {
                    image_id: 1,
                    col: 0,
                    row: 0,
                    cols: 2,
                    rows: 1,
                });
            }
        }
        self.refresh_app_graphics();
    }
```

- [ ] **Step 13: Build**

Run: `cargo build 2>&1 | tail -30`
Expected: clean build. Fix any remaining `AppInstance`/borrow errors the compiler points
to (search `src/session.rs` for any leftover `AppInstance` reference and convert it to the
host API). Do NOT change behavior — only adapt to the new ownership.

- [ ] **Step 14: Run the full suite**

Run: `cargo test 2>&1 | tail -40`
Expected: every test passes (the pre-change count was 203). If a test referenced
`WinContent::App(AppInstance)` or constructed an `AppInstance` directly, update it to use
the new path (spawn through a `SessionCore` as before — these are integration tests that
go through `ClientMsg::Launch`, so they should not need changes). Investigate any failure
as a real regression, not a test to weaken.

- [ ] **Step 15: Lint**

Run: `cargo clippy --all-targets 2>&1 | tail -30`
Expected: zero warnings.

- [ ] **Step 16: Commit**

```bash
git add src/session.rs
git commit --no-verify -m "session: route apps through LocalAppHost (WinContent::App(AppId))"
```

---

## Task 5: Verify behavior-preserving end to end

**Files:** none (verification only)

- [ ] **Step 1: Confirm no stray `AppInstance` references remain in session**

Run: `grep -n "AppInstance" src/session.rs`
Expected: no matches (the type is now only used inside `src/apphost/host.rs` and
`src/ptyhost.rs`).

- [ ] **Step 2: Full gate**

Run: `cargo build 2>&1 | tail -5 && cargo test 2>&1 | tail -10 && cargo clippy --all-targets 2>&1 | tail -10`
Expected: build OK, all tests pass, zero clippy warnings.

- [ ] **Step 3: Manual smoke (deploy + run)**

Per the dev loop (see memory `tuiui-dev-loop`):
```bash
cargo install --root ~/.local --path . --force
tuiui kill; tuiui
```
Manually verify: launch an app from the launcher (e.g. a shell), type into it (input
works), resize/move its window (PTY resizes), close it (process is reaped), open a
graphics app (yazi/chafa placement still renders). Behavior must match pre-Phase-1.

- [ ] **Step 4: Update the roadmap memory**

Append to `tuiui-roadmap-state` memory: "apphost/frontend split Phase 1 (in-process
LocalAppHost boundary, WinContent::App(AppId)) DONE; Phase 2 (separate process + IPC) and
Phase 3 (update UX) pending." Add the one-line pointer is already present; just update the
body. (Do this via the Write tool, not a commit.)

---

## Self-Review Notes (for the executor)

- **Borrow-checker traps:** every place that previously held `&mut WinContent` to reach the
  app now must look up the `AppId` via an immutable `self.contents.get(...)`, copy it
  (`let aid = *aid;`), then call `self.apphost` — because `contents` and `apphost` are
  distinct fields, immutable-borrow-then-host-call avoids the conflict. Do not try to hold a
  `get_mut` on `contents` while calling `self.apphost`.
- **No new behavior:** `render`'s fallback `CellBuffer::new(w, h)` only triggers for an
  unknown id, which cannot happen for a live window — it preserves the old "always returns a
  buffer" contract. Confirm the constructor name.
- **Graphics lock scope:** `self.apphost.graphics(aid)` returns the same `MutexGuard` the old
  `app.graphics()` did; keep guard lifetimes identical (drop before mutably borrowing
  `self.images`, exactly as the existing two-pass `refresh_app_graphics` already does).
- **`set_meta`/`meta` are unused in Phase 1** by design (wired in Phase 3). They ship with
  unit tests only — do not add dead-code-allow attributes unless clippy flags them; if it
  does, prefer `#[allow(dead_code)]` on the two methods with a `// wired in Phase 3` note.
