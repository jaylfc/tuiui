# Apphost Phase 3 — Reload + Restore Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the live-update payoff user-visible: a fresh frontend rebuilds its app windows from the apphost roster, `tuiui reload` (and the in-app Restart / "Update & Reload") restarts only the frontend while apps keep running, and the menubar Restart item becomes active.

**Architecture:** The frontend serializes each app window's geometry/title/state into the apphost's opaque per-app `meta` each tick (via `set_meta`). On (re)connect, `RemoteAppHost` reads the `Roster` synchronously, and `SessionCore` rebuilds one window per app from its `meta`. A new `ClientMsg::Reload` / `Flags.reload` pair makes the daemon exit **without** tearing down the apphost; the thin client reconnects (spawning a fresh — possibly updated — daemon), which restores the windows.

**Tech Stack:** Rust 2021, the Phase 2 apphost (`AppHost`/`LocalAppHost`/`RemoteAppHost`, `proto.rs`), `serde_json`, `src/window.rs` (`Window { title, rect, z, minimized }`), `src/wm.rs` (`add_window`, `minimize`).

**Reference:** Spec `docs/superpowers/specs/2026-06-07-apphost-frontend-split-design.md` (Phase 3). Phase 2 is implemented: apps run in `tuiui --apphost`; the frontend uses `RemoteAppHost`; `AppHost::{set_meta, meta, shutdown_host}` exist; `SessionCore::with_apphost` injects the host.

---

## Background (read before starting)

- `RemoteAppHost::connect` (`src/apphost/remote.rs`) currently spawns the reader thread immediately; the server sends `HostEvt::Roster` first on every connection. `apply_evt` already stores roster meta + marks apps alive.
- `SessionCore` (`src/session.rs`): `contents: HashMap<WindowId, WinContent>`, `titles: Vec<(WindowId, String)>`, `wm: WindowManager`, `apphost: Box<dyn AppHost>`. `WinContent::App(AppId)`. Flags built in `daemon.rs::serve_client`. `quit`/`shutdown` flags + `quit_requested()`/`shutdown_requested()`/`clear_quit()`.
- `daemon.rs::run()`: `ensure_apphost()` → `SessionCore::with_apphost` → auto-launch `cfg.apps` → `for stream in listener.incoming() { serve_client(...); core.clear_quit(); if shutdown_requested { break } }` → `remove_file(socket)` → `core.shutdown()`.
- `daemon.rs::serve_client()`: 16ms loop; builds `Flags { ..., detach: core.quit_requested() }`; sends `FrameMsg`; `if core.quit_requested() { return }`.
- `client.rs::run(stream) -> io::Result<()>`: reader thread sets `detached=true` on `flags.detach` or EOF; main loop breaks on `detached`. The `q` leader key breaks (detach); `Q` sends Shutdown then breaks.
- `main.rs::attach(spawn_if_missing)`: connects (spawning `--daemon` if missing) then `tuiui::client::run(stream)`.
- `powermenu.rs`: `PowerAction::{Exit,Restart,Shutdown}`; `Restart::enabled()` returns `false` (dimmed "(soon)"); `PowerOutcome::{Detach,Shutdown}`; `on_click` maps confirm → outcome.
- Settings "Install Update" (`session.rs` ~line 861, `SettingsAction::InstallUpdate`) launches a shell running `cargo install --git ... ; <message>`.

---

## Task 1: Window metadata — `WinMeta` + `sync_app_meta`

**Files:**
- Modify: `src/session.rs`

- [ ] **Step 1: Define `WinMeta` and a field to track last-sent meta**

Near the top of `src/session.rs` (after the imports / before `SessionCore`), add:

```rust
/// Opaque per-app window state the frontend stashes in the apphost so a fresh
/// frontend (after `reload` or a crash) can rebuild the window in place.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Debug)]
struct WinMeta {
    rect: crate::geometry::Rect,
    title: String,
    z: i32,
    minimized: bool,
}
```

Add a field to `struct SessionCore` (near `apphost`):

```rust
    /// Last `WinMeta` blob pushed to the apphost per app window (change-gate so
    /// we only send `set_meta` when a window actually moved/retitled/minimized).
    last_meta: HashMap<WindowId, Vec<u8>>,
```

Initialize it in `with_apphost`'s struct literal: `last_meta: HashMap::new(),`.

(`Rect` must derive `Serialize`/`Deserialize` — it does; it is used in `protocol::ImagePlacement`.)

- [ ] **Step 2: Add `sync_app_meta`**

Add this method to `impl SessionCore` (near `refresh_app_graphics`):

```rust
    /// Push each app window's current geometry/state to the apphost as opaque
    /// `meta`, but only when it changed. Called once per frame by the daemon.
    /// For the in-process `LocalAppHost` this just updates a local map (cheap);
    /// for `RemoteAppHost` it sends `SetMeta` so a restarted frontend can restore.
    pub fn sync_app_meta(&mut self) {
        let mut updates: Vec<(AppId, WindowId, Vec<u8>)> = Vec::new();
        for w in self.wm.z_ordered() {
            if let Some(WinContent::App(aid)) = self.contents.get(&w.id) {
                let title = self
                    .titles
                    .iter()
                    .find(|(i, _)| *i == w.id)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_default();
                let meta = WinMeta { rect: w.rect, title, z: w.z, minimized: w.minimized };
                let bytes = serde_json::to_vec(&meta).unwrap_or_default();
                if self.last_meta.get(&w.id) != Some(&bytes) {
                    updates.push((*aid, w.id, bytes));
                }
            }
        }
        for (aid, win, bytes) in updates {
            self.apphost.set_meta(aid, bytes.clone());
            self.last_meta.insert(win, bytes);
        }
    }
```

NOTE: `self.wm.z_ordered()` returns references to `Window`; confirm it borrows `self.wm` immutably so the loop can also read `self.contents`/`self.titles` immutably (it can — all immutable). The `updates` vec defers the `&mut self.apphost` calls until after the immutable borrows end.

- [ ] **Step 3: Build + a unit test**

Add to the session test module (or `tests/session_tests.rs`) a test using the in-process `LocalAppHost` (its `set_meta` stores locally, so `meta` reflects what `sync_app_meta` pushed):

```rust
#[test]
fn sync_app_meta_records_window_state() {
    use tuiui::session::{SessionCore, ClientMsg};
    use tuiui::config::Config;
    let mut core = SessionCore::new(100, 30, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.sync_app_meta();
    // The app's meta is now populated (non-empty) for restore.
    assert!(core.app_meta_count_for_test() > 0);
    core.shutdown();
}
```

Add the tiny test accessor to `impl SessionCore`:

```rust
    #[doc(hidden)]
    pub fn app_meta_count_for_test(&self) -> usize {
        self.last_meta.len()
    }
```

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test sync_app_meta 2>&1 | tail -15` — passes.
Run: `cargo build 2>&1 | tail -5 && cargo clippy --all-targets 2>&1 | tail -10` — clean.

- [ ] **Step 4: Commit**

```bash
git add src/session.rs tests/session_tests.rs
git commit --no-verify -m "session: WinMeta + sync_app_meta (push window state to apphost for restore)"
```

---

## Task 2: Restore windows from the apphost roster

**Files:**
- Modify: `src/apphost/remote.rs` (read `Roster` synchronously on connect)
- Modify: `src/session.rs` (add `restore_windows_from_host`)
- Modify: `src/daemon.rs` (call restore; only auto-launch `cfg.apps` when nothing restored)

- [ ] **Step 1: `RemoteAppHost::connect` reads the roster synchronously**

The frontend must see the roster before `restore_windows_from_host` runs. In `src/apphost/remote.rs::connect`, after `let reader = writer.try_clone()?;` and before spawning the reader thread, read the first message synchronously and apply it if it's the roster:

```rust
        let cache: Arc<Mutex<Cache>> = Arc::new(Mutex::new(Cache::default()));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        // The server sends Roster first; read it synchronously so the frontend
        // can rebuild windows immediately after connect.
        let mut buf_reader = std::io::BufReader::new(reader);
        if let Ok(Some(evt)) = crate::apphost::proto::recv::<crate::apphost::proto::HostEvt, _>(&mut buf_reader) {
            apply_evt(evt, &cache, &pending);
        }

        {
            let cache = cache.clone();
            let pending = pending.clone();
            std::thread::spawn(move || reader_loop_buffered(buf_reader, cache, pending));
        }
        Ok(RemoteAppHost { writer, cache, pending, next_req: AtomicU64::new(1) })
```

Refactor `reader_loop` to take an already-built `BufReader` so the synchronously-read reader is reused (don't drop buffered bytes):

```rust
fn reader_loop_buffered(mut r: std::io::BufReader<UnixStream>, cache: Arc<Mutex<Cache>>, pending: Pending) {
    while let Ok(Some(evt)) = crate::apphost::proto::recv::<crate::apphost::proto::HostEvt, _>(&mut r) {
        apply_evt(evt, &cache, &pending);
    }
}
```

(Remove the old `reader_loop` that built its own `BufReader`, or keep it unused-free — delete it to avoid dead code.)

- [ ] **Step 2: `SessionCore::restore_windows_from_host`**

Add to `impl SessionCore`:

```rust
    /// Rebuild a window for every app the apphost already owns (after a frontend
    /// reload or crash). Returns the number of windows restored. Apps with no
    /// stored `meta` are skipped (they will still be reaped/closed normally if
    /// the user never had a window for them).
    pub fn restore_windows_from_host(&mut self) -> usize {
        // Snapshot ids first so we don't borrow the host across mutations.
        let ids: Vec<AppId> = self.apphost.list();
        let mut restored = 0;
        for aid in ids {
            // Skip if we already have a window bound to this app.
            if self.contents.values().any(|c| matches!(c, WinContent::App(a) if *a == aid)) {
                continue;
            }
            let Some(bytes) = self.apphost.meta(aid) else { continue };
            let Ok(meta) = serde_json::from_slice::<WinMeta>(&bytes) else { continue };
            let id = self.wm.add_window(meta.title.clone(), meta.rect);
            if meta.minimized {
                self.wm.minimize(id);
            }
            self.contents.insert(id, WinContent::App(aid));
            self.titles.push((id, meta.title));
            self.last_meta.insert(id, bytes);
            restored += 1;
        }
        if restored > 0 {
            self.auto_tile_if_enabled();
        }
        restored
    }
```

NOTE: confirm `auto_tile_if_enabled` exists (it is called in `launch_in`). If auto-tiling would fight restored geometry, you may drop that call — but it is a no-op unless the user enabled auto-tile, so keep it for consistency.

- [ ] **Step 3: Daemon calls restore; auto-launch only when nothing restored**

In `src/daemon.rs::run()`, after building `core` with `with_apphost` and BEFORE the existing `for app in &cfg.apps { core.apply(Launch...) }` loop, replace that loop with:

```rust
    // Rebuild windows for any apps the apphost already owns (reload / crash
    // recovery). Only auto-launch the configured apps on a truly fresh start.
    let restored = core.restore_windows_from_host();
    if restored == 0 {
        for app in &cfg.apps {
            core.apply(ClientMsg::Launch {
                name: app.name.clone(),
                command: app.command.clone(),
                args: app.args.clone(),
            });
        }
    } else {
        crate::dbg_log(&format!("frontend: restored {restored} app window(s) from apphost"));
    }
```

- [ ] **Step 4: Daemon calls `sync_app_meta` each tick**

In `src/daemon.rs::serve_client`, next to `core.refresh_app_graphics();`, add `core.sync_app_meta();`:

```rust
        core.reap_dead();
        core.refresh_app_graphics();
        core.sync_app_meta();
        core.pump_thumbnails();
```

- [ ] **Step 5: Build + test + clippy + commit**

Add a restore unit test to `tests/session_tests.rs`:

```rust
#[test]
fn restore_rebuilds_app_window_from_meta() {
    use tuiui::session::{SessionCore, ClientMsg};
    use tuiui::config::Config;
    // Launch an app, push its meta, then simulate a fresh frontend over the SAME
    // in-process host by constructing a new SessionCore around a host that already
    // owns the app. We approximate this by reusing the same core: drop its window
    // mapping, then restore.
    let mut core = SessionCore::new(100, 30, Config::default());
    core.apply(ClientMsg::Launch { name: "shell".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.sync_app_meta();
    let before = core.window_count();
    assert_eq!(before, 1);
    // Forget the window (as a fresh frontend would) but keep the app in the host.
    core.forget_windows_for_test();
    assert_eq!(core.window_count(), 0);
    let restored = core.restore_windows_from_host();
    assert_eq!(restored, 1);
    assert_eq!(core.window_count(), 1);
    core.shutdown();
}
```

Add the test helper to `impl SessionCore`:

```rust
    /// Drop all window bookkeeping while leaving the apphost's apps alive — used
    /// to simulate a fresh frontend in restore tests.
    #[doc(hidden)]
    pub fn forget_windows_for_test(&mut self) {
        let ids: Vec<WindowId> = self.contents.keys().copied().collect();
        for id in ids {
            self.wm.close(id);
        }
        self.contents.clear();
        self.titles.clear();
        self.last_meta.clear();
    }
```

Run:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test restore_rebuilds 2>&1 | tail -15      # passes
cargo test 2>&1 | grep -E "test result|FAILED" | tail -5
cargo clippy --all-targets 2>&1 | tail -10       # zero warnings
```

```bash
git add src/apphost/remote.rs src/session.rs src/daemon.rs tests/session_tests.rs
git commit --no-verify -m "apphost: restore app windows from roster meta on (re)connect"
```

---

## Task 3: Reload plumbing — `ClientMsg::Reload`, `Flags.reload`, daemon reload-exit, client reconnect, `tuiui reload`

**Files:**
- Modify: `src/session.rs` (Reload msg + flag)
- Modify: `src/protocol.rs` (`Flags.reload`)
- Modify: `src/daemon.rs` (send reload flag; exit without tearing down apphost)
- Modify: `src/client.rs` (`ClientExit` enum; reconnect on reload)
- Modify: `src/main.rs` (attach reconnect loop; `tuiui reload`)

- [ ] **Step 1: `ClientMsg::Reload` + a `reload` flag on the core**

In `src/session.rs`: add a variant to `ClientMsg` (near `Shutdown`):
```rust
    /// Restart the frontend only, keeping the apphost (and apps) alive.
    Reload,
```
Add a field to `SessionCore` (near `quit`/`shutdown`): `reload: bool,` and init `reload: false,` in `with_apphost`.
Handle it in `apply` (near `ClientMsg::Shutdown => self.shutdown = true`):
```rust
            ClientMsg::Reload => self.reload = true,
```
Add accessors:
```rust
    /// Whether a frontend-only reload was requested (apps stay alive).
    pub fn reload_requested(&self) -> bool { self.reload }
```
Add `Reload` to the list of mouse-vs-other classification near the top of `apply` ONLY if that match needs exhaustive arms (it does not — it's a `matches!`-style guard; verify the `apply` match is exhaustive and add the arm in the right place).

- [ ] **Step 2: `Flags.reload`**

In `src/protocol.rs` `struct Flags` (which is `#[serde(default)]`), add:
```rust
    /// The daemon is reloading the frontend; the client should reconnect (not
    /// fully detach). Apps stay alive in the apphost.
    pub reload: bool,
```

- [ ] **Step 3: Daemon sends the reload flag and exits without killing the apphost**

In `src/daemon.rs::serve_client`, add `reload: core.reload_requested(),` to the `Flags { ... }` construction, and after the frame is sent, add a reload check next to the quit check:
```rust
        if core.reload_requested() {
            return; // reload flag delivered; daemon will restart, apphost untouched
        }
```

In `src/daemon.rs::run()`, change the accept loop + teardown so a reload exits WITHOUT `core.shutdown()` (which would kill apps + apphost):
```rust
    let mut reloading = false;
    for stream in listener.incoming() {
        let stream = stream?;
        serve_client(&mut core, &mut comp, stream);
        core.clear_quit();
        if core.shutdown_requested() {
            break;
        }
        if core.reload_requested() {
            reloading = true;
            break;
        }
    }
    let _ = std::fs::remove_file(&path);
    if !reloading {
        core.shutdown(); // full stop: kills apps + tells the apphost to exit
    }
    // On reload we just drop `core` (its RemoteAppHost disconnects); the apphost
    // keeps running so the next frontend can restore the apps.
    Ok(())
```

- [ ] **Step 4: Client returns a `ClientExit` and reconnects on reload**

In `src/client.rs`:
- Add at module top:
```rust
/// How a client session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientExit {
    /// The user detached / shut down — stop.
    Detached,
    /// The daemon asked the client to reconnect (frontend reload).
    Reload,
}
```
- Change the signature to `pub fn run(stream: UnixStream) -> std::io::Result<ClientExit>`.
- Add a shared `reload` flag alongside `detached`:
```rust
    let reload = Arc::new(AtomicBool::new(false));
```
- In the reader thread, before the `if msg.flags.detach { break; }`, handle reload:
```rust
                            if msg.flags.reload {
                                reload.store(true, Ordering::SeqCst);
                                break;
                            }
                            if msg.flags.detach {
                                break;
                            }
```
(Clone `reload` into the thread like `detached`.)
- At the END of `run` (after the main loop breaks and terminal restores), return the outcome:
```rust
    if reload.load(Ordering::SeqCst) {
        Ok(ClientExit::Reload)
    } else {
        Ok(ClientExit::Detached)
    }
```
Ensure every existing `return Ok(())`/fall-through in `run` is updated to return a `ClientExit` (the leader-`q`/`Q` `break`s fall through to the final return → `Detached`, which is correct; just fix the final `Ok(())`).

- [ ] **Step 5: `main.rs` reconnect loop + `tuiui reload`**

In `src/main.rs`:
- Change `attach` so it loops on `ClientExit::Reload`, re-attaching (which spawns a fresh — updated — daemon because the old one exited):
```rust
fn attach(spawn_if_missing: bool) -> std::io::Result<()> {
    loop {
        let path = socket_path();
        if UnixStream::connect(&path).is_err() {
            if !spawn_if_missing {
                eprintln!("tuiui: no daemon running (start it with `tuiui`)");
                return Ok(());
            }
            spawn_daemon()?;
            let mut ready = false;
            for _ in 0..100 {
                if UnixStream::connect(&path).is_ok() { ready = true; break; }
                std::thread::sleep(Duration::from_millis(50));
            }
            if !ready {
                eprintln!("tuiui: daemon failed to start");
                return Ok(());
            }
        }
        let stream = UnixStream::connect(&path)?;
        match tuiui::client::run(stream)? {
            tuiui::client::ClientExit::Detached => return Ok(()),
            tuiui::client::ClientExit::Reload => {
                // The daemon is restarting; wait briefly for the old socket to
                // drop, then loop to spawn/connect the fresh daemon.
                for _ in 0..100 {
                    if UnixStream::connect(socket_path()).is_err() { break; }
                    std::thread::sleep(Duration::from_millis(20));
                }
                continue;
            }
        }
    }
}
```
- Add a `Some("reload") => reload(),` arm to `main`, and the function:
```rust
/// Tell a running daemon to reload its frontend (apps keep running via the
/// apphost). An attached client reconnects on its own.
fn reload() -> std::io::Result<()> {
    match UnixStream::connect(socket_path()) {
        Ok(mut stream) => {
            let mut buf = serde_json::to_vec(&tuiui::session::ClientMsg::Reload)
                .map_err(std::io::Error::other)?;
            buf.push(b'\n');
            stream.write_all(&buf)?;
            println!("tuiui: reload requested");
        }
        Err(_) => println!("tuiui: no daemon running"),
    }
    Ok(())
}
```
- Update the `Some(other)` usage line and the top-of-file doc comment to list `reload`.

- [ ] **Step 6: Build + full suite + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -20
cargo test 2>&1 | grep -E "test result|error\[|FAILED" | tail -25     # all pass
cargo clippy --all-targets 2>&1 | tail -20                            # zero warnings
git add src/session.rs src/protocol.rs src/daemon.rs src/client.rs src/main.rs
git commit --no-verify -m "apphost: tuiui reload — restart frontend only, keep apps alive (client reconnects)"
```

---

## Task 4: Enable the menubar Restart action

**Files:**
- Modify: `src/powermenu.rs`
- Modify: `src/session.rs`

- [ ] **Step 1: Add `Reload` to `PowerOutcome`, enable Restart**

In `src/powermenu.rs`:
- Add `Reload` to `PowerOutcome`:
```rust
pub enum PowerOutcome {
    Detach,
    Reload,
    Shutdown,
}
```
- Make Restart enabled:
```rust
    fn enabled(self) -> bool {
        true
    }
```
(Now all three items are enabled; you can simplify `enabled` away entirely OR keep it returning `true`. If you remove it, also remove the `if action.enabled()` branches in `render`/`on_click` so every item is clickable and drawn normally — simplest is to delete `enabled` and the `(soon)`/dim handling.)
- In `on_click`, map the Restart confirm to `Reload`:
```rust
                return match action {
                    PowerAction::Exit => PowerClick::Act(PowerOutcome::Detach),
                    PowerAction::Restart => PowerClick::Act(PowerOutcome::Reload),
                    PowerAction::Shutdown => PowerClick::Act(PowerOutcome::Shutdown),
                };
```
- In `render`'s confirm-dialog `match action`, give Restart a real message + label:
```rust
                PowerAction::Restart => ("Restart tuiui? The UI reloads; your apps keep running.", "Restart"),
```
- Remove the dimming/`(soon)` rendering of disabled items (all items render with `t.text` now).

- [ ] **Step 2: Handle the new outcome in the session**

In `src/session.rs::handle_mouse`, the power-menu click match must handle `Reload`:
```rust
            match self.power_menu.on_click(p, self.w, self.h) {
                PowerClick::Act(PowerOutcome::Detach) => self.quit = true,
                PowerClick::Act(PowerOutcome::Reload) => self.reload = true,
                PowerClick::Act(PowerOutcome::Shutdown) => self.shutdown = true,
                PowerClick::Consumed => {}
            }
```

- [ ] **Step 3: Update powermenu tests**

The `restart_is_disabled_and_opens_no_dialog` test is now wrong (Restart is enabled). Replace it:
```rust
    #[test]
    fn restart_then_confirm_reloads() {
        let (w, h) = (120, 40);
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 1)), w, h); // Restart is item 1
        assert!(m.confirm.is_some(), "Restart now opens a confirm dialog");
        let (_, confirm) = dialog_buttons(w, h);
        assert_eq!(m.on_click(center(confirm), w, h), PowerClick::Act(PowerOutcome::Reload));
    }
```

- [ ] **Step 4: Build + test + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test powermenu 2>&1 | tail -15
cargo test 2>&1 | grep -E "test result|FAILED" | tail -5
cargo clippy --all-targets 2>&1 | tail -10
git add src/powermenu.rs src/session.rs
git commit --no-verify -m "menubar: enable Restart action (reloads the frontend, apps stay alive)"
```

---

## Task 5: Settings "Update & Reload"

**Files:**
- Modify: `src/session.rs` (InstallUpdate command tail)
- Modify: `src/settings.rs` (label, if it names the action — optional)

- [ ] **Step 1: Make the installer trigger a reload when it finishes**

In `src/session.rs`, in the `SettingsAction::InstallUpdate` arm, change the shell command tail so it runs `tuiui reload` after a successful install (the install runs in a hosted shell window inside the apphost, so it survives the reload; `tuiui reload` reconnects the frontend with the new binary):

```rust
                    Some(crate::settings::SettingsAction::InstallUpdate) => {
                        let cmd = format!(
                            "clear; echo 'Updating tuiui from {repo} …'; echo; \
cargo install --git {repo} --force && {{ echo; echo 'Reloading tuiui …'; tuiui reload; }} || \
echo 'Update failed — tuiui not reloaded.'; exec \"$SHELL\"",
                            repo = crate::REPO_URL,
                        );
                        self.launch("update tuiui".into(), "sh".into(), vec!["-lc".into(), cmd]);
                    }
```

NOTE: `tuiui` must be on `$PATH` inside the spawned shell (it is, via `~/.local/bin`; the shell is `-lc` so it loads the login profile). If `tuiui` is not found the `&&` chain just won't reload — safe.

- [ ] **Step 2: (Optional) rename the Settings action label**

If `src/settings.rs` shows a literal like "Install Update", rename to "Update & Reload" for clarity. Grep `src/settings.rs` for the update label; change the display string only (leave the `SettingsAction` enum name alone to avoid churn). If no user-facing label exists, skip.

- [ ] **Step 3: Build + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -5
cargo test 2>&1 | grep -E "test result|FAILED" | tail -5
cargo clippy --all-targets 2>&1 | tail -10
git add src/session.rs src/settings.rs
git commit --no-verify -m "settings: Update & Reload — install then reload the frontend (apps stay alive)"
```

---

## Task 6: End-to-end verification + manual smoke

**Files:** none

- [ ] **Step 1: Full gate**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -3 && cargo test 2>&1 | grep -cE "test result: ok" && cargo clippy --all-targets 2>&1 | grep -cE "warning:|error:"
```
Expected: build OK; suite count ≥ before; clippy `0`.

- [ ] **Step 2: Deploy**

```bash
cargo install --root ~/.local --path . --force
tuiui kill; tuiui
```

- [ ] **Step 3: Manual reload test (the payoff)**

1. Launch an app (e.g. btop or a shell), note its window position and the child PID (`pgrep -fa <app>`).
2. Open the menu (top-right "tuiui ▾") → **Restart** → confirm. The UI should briefly reload and the app window should reappear in place; the child PID is unchanged (apps survived).
3. Confirm two processes throughout: `pgrep -fa 'tuiui --apphost'` (unchanged PID across the reload) and `tuiui --daemon` (NEW PID after reload).
4. `tuiui reload` from a shell window → same effect.
5. Settings → Update & Reload → runs `cargo install` then reloads (only if you want to exercise the network path).
6. Menu → **Shutdown** → confirm: both processes exit.

- [ ] **Step 4: Update memory**

Update `tuiui-roadmap-state`: apphost Phase 3 (reload + restore + Restart enabled + Update & Reload) DONE; the live-update goal is fully realized. Note `[[tuiui-live-update-idea]]` is now implemented.

---

## Risks & notes

- **Reload flash:** the frontend fully restarts, so there is a brief blank/redraw. Acceptable — apps and their terminal state are preserved in the apphost.
- **Auto-launch vs restore:** `cfg.apps` is auto-launched only when `restore_windows_from_host` restored zero windows, preventing duplicate launches on reload. (The user currently has no auto-launch apps, so this is moot today but correct.)
- **Meta volume:** `sync_app_meta` only sends on change, so a static window costs nothing after the first frame.
- **`tuiui reload` with no attached client:** the daemon exits and stays down (apps remain in the apphost); the next `tuiui` rebuilds them. The in-app Restart path always has an attached client that reconnects, so it is seamless.
- **Synchronous roster read:** `RemoteAppHost::connect` blocks on the first message. The server always sends `Roster` first; if a future server change reorders this, restore would mis-handle a non-roster first message — `apply_evt` handles any variant safely, but keep `Roster`-first as an invariant.
