# Apphost Phase 2b — Separate Process + IPC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the running apps into a separate long-lived `apphost` process and have the frontend (the current daemon) drive them over a Unix socket via a `RemoteAppHost`, so the apps survive a frontend restart.

**Architecture:** A new `tuiui --apphost` process owns a `LocalAppHost` behind a socket, pushing per-app frames (grid + placements + image blobs) and accepting commands. The frontend daemon ensures the apphost is running, connects a `RemoteAppHost` (implements the `AppHost` trait from Phase 2a, caches the pushed frames), and injects it into `SessionCore::with_apphost`. `tuiui kill` stops both; a mere detach or frontend crash leaves the apphost (and its apps) alive.

**Tech Stack:** Rust 2021, `std::os::unix::net::{UnixListener, UnixStream}`, `serde`/`serde_json` (newline-JSON, matching the existing client protocol), `base64` 0.22, the Phase 2a `AppHost` trait + `LocalAppHost`.

**Reference:** Spec `docs/superpowers/specs/2026-06-07-apphost-frontend-split-design.md`. Phase 2a plan (the trait seam) is already implemented — `AppHost`, `LocalAppHost`, `SessionCore::with_apphost(w,h,cfg, Box<dyn AppHost>)` all exist.

---

## Background (read before starting)

- The existing client↔daemon protocol (`src/protocol.rs`) is **newline-delimited JSON**: `serde_json::to_vec(&msg)` + `b'\n'`, read with `BufReader::read_line` + `serde_json::from_str`. Sockets live in a per-user 0700 dir (`socket_dir()`); `socket_path()` = `<dir>/daemon.sock`.
- `daemon.rs::run()` binds the daemon socket, builds `SessionCore::new(w,h,cfg)`, auto-launches `cfg.apps`, then loops `serve_client` per attached client. `serve_client`'s loop calls `core.reap_dead()`, `core.refresh_app_graphics()`, `core.build_frame()` every 16ms — all via the `AppHost` trait, so it works unchanged with any host.
- `AppHost` (Phase 2a) methods: `spawn/input/resize/kill/is_alive/snapshot/placements/image_png/list/set_meta/meta/remove` + default-no-op `inject_test_image`. `placements(id) -> Vec<Placement>`, `image_png(id, image_id) -> Option<Vec<u8>>`, `snapshot(id) -> Option<CellBuffer>`, `meta(id) -> Option<Vec<u8>>`.
- `Placement` = `{ image_id: u32, col: u16, row: u16, cols: u16, rows: u16 }` (`src/kittygfx.rs`).
- `CellBuffer` (`src/buffer.rs`) has private fields `w: i32, h: i32, cells: Vec<Cell>`; `Cell` already derives serde. `width()`/`height()` accessors exist.
- base64: encode with `crate::kitty::b64(&[u8]) -> String`; decode with `use base64::Engine; base64::engine::general_purpose::STANDARD.decode(&s)`.
- `main.rs` dispatches `--daemon` / `kill` / `attach` / (none→attach). `spawn_daemon()` spawns `current_exe() --daemon` detached (`process_group(0)`, null stdio).

---

## Task 1: Wire protocol — serde derives + `proto.rs` + apphost socket path

**Files:**
- Modify: `src/buffer.rs` (derive serde + PartialEq on `CellBuffer`)
- Modify: `src/kittygfx.rs` (derive serde on `Placement`)
- Modify: `src/protocol.rs` (add `apphost_socket_path()`)
- Create: `src/apphost/proto.rs`
- Modify: `src/apphost/mod.rs` (add `pub mod proto;`)

- [ ] **Step 1: Derive serde + PartialEq on `CellBuffer`**

In `src/buffer.rs`, change:
```rust
#[derive(Clone, Debug)]
pub struct CellBuffer { w: i32, h: i32, cells: Vec<Cell> }
```
to:
```rust
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CellBuffer { w: i32, h: i32, cells: Vec<Cell> }
```
(`Cell` already derives `Serialize/Deserialize/PartialEq`, so this compiles. `PartialEq` is needed by the server's change detection in Task 2.)

- [ ] **Step 2: Derive serde on `Placement`**

In `src/kittygfx.rs`:
```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Placement {
```

- [ ] **Step 3: Add the apphost socket path**

In `src/protocol.rs`, next to `socket_path()`:
```rust
/// Path to the apphost socket (apps live behind this; survives frontend restarts).
pub fn apphost_socket_path() -> PathBuf {
    socket_dir().join("apphost.sock")
}
```

- [ ] **Step 4: Create `src/apphost/proto.rs`**

```rust
//! Wire protocol between the frontend (`RemoteAppHost`) and the apphost server.
//! Newline-delimited JSON, mirroring `crate::protocol`. Image bytes ride as
//! base64 strings; grids ride as serialized `CellBuffer`s.

use crate::buffer::CellBuffer;
use crate::kittygfx::Placement;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};

/// frontend → apphost.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum HostReq {
    Spawn { req_id: u64, cmd: String, args: Vec<String>, cwd: Option<String>, cols: i32, rows: i32 },
    Input { app: u64, bytes: Vec<u8> },
    Resize { app: u64, cols: i32, rows: i32 },
    SetMeta { app: u64, meta: Vec<u8> },
    Kill { app: u64 },
    /// Stop the apphost process entirely (full shutdown / `tuiui kill`).
    Shutdown,
}

/// A PNG the app transmitted, base64-encoded, sent once per (frontend, app, id).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ImgBlob {
    pub image_id: u32,
    pub png_b64: String,
}

/// One app's metadata in the on-connect roster.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RosterEntry {
    pub app: u64,
    pub meta: Vec<u8>,
}

/// apphost → frontend.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum HostEvt {
    Spawned { req_id: u64, app: u64 },
    SpawnFailed { req_id: u64, error: String },
    /// The app's current grid + placements, plus any not-yet-sent image blobs.
    Frame {
        app: u64,
        grid: CellBuffer,
        placements: Vec<Placement>,
        images: Vec<ImgBlob>,
        alive: bool,
    },
    /// The app's child exited.
    Gone { app: u64 },
    /// Sent right after a frontend connects so it can rebuild its window list.
    Roster { apps: Vec<RosterEntry> },
}

/// Write a newline-JSON message. Returns `Err` if the peer is gone.
pub fn send<T: Serialize, W: Write>(w: &mut W, msg: &T) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    buf.push(b'\n');
    w.write_all(&buf)
}

/// Read one newline-JSON message into `T`. `Ok(None)` on EOF.
pub fn recv<T: for<'de> Deserialize<'de>, R: BufRead>(r: &mut R) -> std::io::Result<Option<T>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let msg = serde_json::from_str(line.trim()).map_err(std::io::Error::other)?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Cell;

    #[test]
    fn req_round_trips() {
        let msgs = vec![
            HostReq::Spawn { req_id: 7, cmd: "sh".into(), args: vec!["-c".into()], cwd: None, cols: 80, rows: 24 },
            HostReq::Input { app: 3, bytes: vec![1, 2, 3] },
            HostReq::Resize { app: 3, cols: 100, rows: 40 },
            HostReq::SetMeta { app: 3, meta: vec![9, 9] },
            HostReq::Kill { app: 3 },
            HostReq::Shutdown,
        ];
        for m in msgs {
            let mut buf: Vec<u8> = Vec::new();
            send(&mut buf, &m).unwrap();
            let mut r = std::io::BufReader::new(&buf[..]);
            let back: HostReq = recv(&mut r).unwrap().unwrap();
            assert_eq!(format!("{m:?}"), format!("{back:?}"));
        }
    }

    #[test]
    fn frame_round_trips_with_grid() {
        let mut grid = CellBuffer::new(4, 2);
        grid.set(1, 0, Cell { ch: 'X', ..Default::default() });
        let evt = HostEvt::Frame {
            app: 5,
            grid: grid.clone(),
            placements: vec![Placement { image_id: 1, col: 0, row: 0, cols: 2, rows: 1 }],
            images: vec![ImgBlob { image_id: 1, png_b64: "QUJD".into() }],
            alive: true,
        };
        let mut buf: Vec<u8> = Vec::new();
        send(&mut buf, &evt).unwrap();
        let mut r = std::io::BufReader::new(&buf[..]);
        let back: HostEvt = recv(&mut r).unwrap().unwrap();
        match back {
            HostEvt::Frame { app, grid: g, placements, images, alive } => {
                assert_eq!(app, 5);
                assert_eq!(g, grid);
                assert_eq!(placements.len(), 1);
                assert_eq!(images[0].png_b64, "QUJD");
                assert!(alive);
            }
            _ => panic!("wrong variant"),
        }
    }
}
```

NOTE: `Cell` must implement `Default` for the test (`..Default::default()`); it does (Phase context shows `#[derive(... Default ...)]` on `Cell`). If not, construct the cell explicitly. Also confirm `CellBuffer::set(x,y,cell)` and `CellBuffer::new(w,h)` are public (they are).

- [ ] **Step 5: Register the module**

In `src/apphost/mod.rs` add: `pub mod proto;`.

- [ ] **Step 6: Build + test + clippy + commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test apphost::proto 2>&1 | tail -15      # both round-trip tests pass
cargo build 2>&1 | tail -5
cargo clippy --all-targets 2>&1 | tail -10     # zero warnings
git add src/buffer.rs src/kittygfx.rs src/protocol.rs src/apphost/proto.rs src/apphost/mod.rs
git commit --no-verify -m "apphost: wire protocol (proto.rs) + serde on CellBuffer/Placement + apphost socket path"
```

---

## Task 2: The apphost server (`tuiui --apphost`)

**Files:**
- Create: `src/apphost/server.rs`
- Modify: `src/apphost/mod.rs` (add `pub mod server;`)

The server owns a `LocalAppHost` that persists across frontend (re)connections. It serves one frontend at a time; each connection gets a fresh per-connection change-tracking state (so a reconnecting frontend receives full state).

- [ ] **Step 1: Write `src/apphost/server.rs`**

```rust
//! The apphost process: owns the live apps behind a socket, pushing per-app
//! frames and accepting commands from the frontend. Started as `tuiui --apphost`
//! (normally spawned automatically by the frontend daemon). The apps it owns
//! survive a frontend restart because this process keeps running.

use crate::apphost::proto::{recv, send, HostEvt, HostReq, ImgBlob, RosterEntry};
use crate::apphost::{AppHost, AppId, LocalAppHost};
use crate::protocol::{apphost_socket_path, socket_dir};
use std::collections::{HashMap, HashSet};
use std::fs::Permissions;
use std::io::BufReader;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::time::Duration;

/// Run the apphost event loop until a frontend sends `Shutdown`.
pub fn run() -> std::io::Result<()> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, Permissions::from_mode(0o700))?;
    let path = apphost_socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, Permissions::from_mode(0o600))?;
    crate::dbg_log("apphost: listening");

    let mut local = LocalAppHost::new();
    let mut shutdown = false;

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        serve_frontend(&mut local, stream, &mut shutdown);
        if shutdown {
            break;
        }
        crate::dbg_log("apphost: frontend detached; apps stay alive");
    }

    // Full shutdown: kill every app, then remove the socket.
    for id in local.list() {
        local.kill(id);
    }
    let _ = std::fs::remove_file(&path);
    crate::dbg_log("apphost: shut down");
    Ok(())
}

/// Serve a single connected frontend. Per-connection change state is fresh so
/// the frontend always gets full grids on (re)connect.
fn serve_frontend(local: &mut LocalAppHost, stream: UnixStream, shutdown: &mut bool) {
    let Ok(reader_stream) = stream.try_clone() else { return };
    let mut writer = stream;

    // Frontend → channel on a reader thread.
    let (tx, rx) = mpsc::channel::<HostReq>();
    std::thread::spawn(move || {
        let mut r = BufReader::new(reader_stream);
        loop {
            match recv::<HostReq, _>(&mut r) {
                Ok(Some(req)) => {
                    if tx.send(req).is_err() {
                        break;
                    }
                }
                Ok(None) | Err(_) => break, // EOF / parse error => frontend gone
            }
        }
    });

    // Roster so the frontend can rebuild its windows.
    let roster = HostEvt::Roster {
        apps: local
            .list()
            .into_iter()
            .map(|id| RosterEntry { app: id.0, meta: local.meta(id).unwrap_or_default() })
            .collect(),
    };
    if send(&mut writer, &roster).is_err() {
        return;
    }

    // Per-connection change tracking.
    let mut last_grid: HashMap<AppId, crate::buffer::CellBuffer> = HashMap::new();
    let mut last_placements: HashMap<AppId, Vec<crate::kittygfx::Placement>> = HashMap::new();
    let mut sent_images: HashMap<AppId, HashSet<u32>> = HashMap::new();

    loop {
        // Drain commands.
        loop {
            match rx.try_recv() {
                Ok(HostReq::Spawn { req_id, cmd, args, cwd, cols, rows }) => {
                    let cwd_path = cwd.as_deref().map(std::path::Path::new);
                    let evt = match local.spawn(&cmd, &args, cwd_path, cols, rows) {
                        Ok(id) => HostEvt::Spawned { req_id, app: id.0 },
                        Err(e) => HostEvt::SpawnFailed { req_id, error: e.to_string() },
                    };
                    if send(&mut writer, &evt).is_err() {
                        return;
                    }
                }
                Ok(HostReq::Input { app, bytes }) => local.input(AppId(app), &bytes),
                Ok(HostReq::Resize { app, cols, rows }) => local.resize(AppId(app), cols, rows),
                Ok(HostReq::SetMeta { app, meta }) => local.set_meta(AppId(app), meta),
                Ok(HostReq::Kill { app }) => local.kill(AppId(app)),
                Ok(HostReq::Shutdown) => {
                    *shutdown = true;
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return, // frontend gone
            }
        }

        // Push frames for every app whose grid / placements / images changed.
        for id in local.list() {
            let alive = local.is_alive(id);
            if !alive {
                let _ = send(&mut writer, &HostEvt::Gone { app: id.0 });
                local.remove(id);
                last_grid.remove(&id);
                last_placements.remove(&id);
                sent_images.remove(&id);
                continue;
            }
            let Some(grid) = local.snapshot(id) else { continue };
            let placements = local.placements(id);

            // New image blobs (not yet sent to this frontend).
            let seen = sent_images.entry(id).or_default();
            let mut images = Vec::new();
            for pl in &placements {
                if !seen.contains(&pl.image_id) {
                    if let Some(png) = local.image_png(id, pl.image_id) {
                        images.push(ImgBlob { image_id: pl.image_id, png_b64: crate::kitty::b64(&png) });
                        seen.insert(pl.image_id);
                    }
                }
            }

            let grid_changed = last_grid.get(&id) != Some(&grid);
            let placements_changed = last_placements.get(&id) != Some(&placements);
            if grid_changed || placements_changed || !images.is_empty() {
                let evt = HostEvt::Frame { app: id.0, grid: grid.clone(), placements: placements.clone(), images, alive: true };
                if send(&mut writer, &evt).is_err() {
                    return;
                }
                last_grid.insert(id, grid);
                last_placements.insert(id, placements);
            }
        }

        std::thread::sleep(Duration::from_millis(16));
    }
}
```

- [ ] **Step 2: Register the module**

In `src/apphost/mod.rs` add `pub mod server;`.

- [ ] **Step 3: Build + clippy**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build 2>&1 | tail -15 && cargo clippy --all-targets 2>&1 | tail -15`
Expected: clean, zero warnings. (The server is exercised end-to-end by the Task 3 loopback test; no standalone unit test here.)

- [ ] **Step 4: Commit**

```bash
git add src/apphost/server.rs src/apphost/mod.rs
git commit --no-verify -m "apphost: server process (LocalAppHost behind a socket, pushes frames)"
```

---

## Task 3: `RemoteAppHost` + loopback integration test

**Files:**
- Create: `src/apphost/remote.rs`
- Modify: `src/apphost/mod.rs` (add `mod remote; pub use remote::RemoteAppHost;`)

- [ ] **Step 1: Write `src/apphost/remote.rs`**

```rust
//! Frontend-side handle to the apphost: implements [`AppHost`] by talking to the
//! apphost server over a socket and serving reads from a locally-cached copy of
//! the last frame each app pushed.

use crate::apphost::proto::{recv, send, HostEvt, HostReq};
use crate::apphost::{AppHost, AppId};
use crate::buffer::CellBuffer;
use crate::kittygfx::Placement;
use std::collections::HashMap;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Cached {
    grid: Option<CellBuffer>,
    placements: Vec<Placement>,
    images: HashMap<u32, Vec<u8>>,
    alive: bool,
}

#[derive(Default)]
struct Cache {
    apps: HashMap<AppId, Cached>,
    meta: HashMap<AppId, Vec<u8>>,
}

type Pending = Arc<Mutex<HashMap<u64, mpsc::Sender<Result<AppId, String>>>>>;

pub struct RemoteAppHost {
    writer: UnixStream,
    cache: Arc<Mutex<Cache>>,
    pending: Pending,
    next_req: AtomicU64,
}

impl RemoteAppHost {
    /// Connect to an already-running apphost at `path`.
    pub fn connect(path: &Path) -> std::io::Result<Self> {
        let writer = UnixStream::connect(path)?;
        let reader = writer.try_clone()?;
        let cache: Arc<Mutex<Cache>> = Arc::new(Mutex::new(Cache::default()));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        {
            let cache = cache.clone();
            let pending = pending.clone();
            std::thread::spawn(move || reader_loop(reader, cache, pending));
        }
        Ok(RemoteAppHost { writer, cache, pending, next_req: AtomicU64::new(1) })
    }

    /// Tell the apphost process to exit (full shutdown). Best-effort.
    pub fn shutdown_apphost(&mut self) {
        let _ = send(&mut self.writer, &HostReq::Shutdown);
    }
}

fn reader_loop(reader: UnixStream, cache: Arc<Mutex<Cache>>, pending: Pending) {
    let mut r = BufReader::new(reader);
    loop {
        match recv::<HostEvt, _>(&mut r) {
            Ok(Some(evt)) => apply_evt(evt, &cache, &pending),
            Ok(None) | Err(_) => break, // apphost gone
        }
    }
}

fn apply_evt(evt: HostEvt, cache: &Arc<Mutex<Cache>>, pending: &Pending) {
    match evt {
        HostEvt::Spawned { req_id, app } => {
            if let Some(tx) = pending.lock().unwrap().remove(&req_id) {
                let _ = tx.send(Ok(AppId(app)));
            }
        }
        HostEvt::SpawnFailed { req_id, error } => {
            if let Some(tx) = pending.lock().unwrap().remove(&req_id) {
                let _ = tx.send(Err(error));
            }
        }
        HostEvt::Frame { app, grid, placements, images, alive } => {
            let mut c = cache.lock().unwrap();
            let entry = c.apps.entry(AppId(app)).or_default();
            entry.grid = Some(grid);
            entry.placements = placements;
            entry.alive = alive;
            for b in images {
                use base64::Engine;
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b.png_b64) {
                    entry.images.insert(b.image_id, bytes);
                }
            }
        }
        HostEvt::Gone { app } => {
            if let Some(e) = cache.lock().unwrap().apps.get_mut(&AppId(app)) {
                e.alive = false;
            }
        }
        HostEvt::Roster { apps } => {
            let mut c = cache.lock().unwrap();
            for entry in apps {
                let id = AppId(entry.app);
                c.meta.insert(id, entry.meta);
                c.apps.entry(id).or_default().alive = true;
            }
        }
    }
}

impl AppHost for RemoteAppHost {
    fn spawn(&mut self, cmd: &str, args: &[String], cwd: Option<&Path>, cols: i32, rows: i32) -> std::io::Result<AppId> {
        let req_id = self.next_req.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(req_id, tx);
        let req = HostReq::Spawn {
            req_id,
            cmd: cmd.to_string(),
            args: args.to_vec(),
            cwd: cwd.map(|p| p.to_string_lossy().into_owned()),
            cols,
            rows,
        };
        send(&mut self.writer, &req)?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(e)) => Err(std::io::Error::other(e)),
            Err(_) => {
                self.pending.lock().unwrap().remove(&req_id);
                Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "apphost spawn timed out"))
            }
        }
    }

    fn input(&mut self, id: AppId, bytes: &[u8]) {
        let _ = send(&mut self.writer, &HostReq::Input { app: id.0, bytes: bytes.to_vec() });
    }

    fn resize(&mut self, id: AppId, cols: i32, rows: i32) {
        let _ = send(&mut self.writer, &HostReq::Resize { app: id.0, cols, rows });
    }

    fn kill(&mut self, id: AppId) {
        let _ = send(&mut self.writer, &HostReq::Kill { app: id.0 });
    }

    fn is_alive(&mut self, id: AppId) -> bool {
        self.cache.lock().unwrap().apps.get(&id).map(|c| c.alive).unwrap_or(false)
    }

    fn snapshot(&self, id: AppId) -> Option<CellBuffer> {
        self.cache.lock().unwrap().apps.get(&id).and_then(|c| c.grid.clone())
    }

    fn placements(&self, id: AppId) -> Vec<Placement> {
        self.cache.lock().unwrap().apps.get(&id).map(|c| c.placements.clone()).unwrap_or_default()
    }

    fn image_png(&self, id: AppId, image_id: u32) -> Option<Vec<u8>> {
        self.cache.lock().unwrap().apps.get(&id).and_then(|c| c.images.get(&image_id).cloned())
    }

    fn list(&self) -> Vec<AppId> {
        self.cache.lock().unwrap().apps.keys().copied().collect()
    }

    fn set_meta(&mut self, id: AppId, meta: Vec<u8>) {
        self.cache.lock().unwrap().meta.insert(id, meta.clone());
        let _ = send(&mut self.writer, &HostReq::SetMeta { app: id.0, meta });
    }

    fn meta(&self, id: AppId) -> Option<Vec<u8>> {
        self.cache.lock().unwrap().meta.get(&id).cloned()
    }

    fn remove(&mut self, id: AppId) {
        let mut c = self.cache.lock().unwrap();
        c.apps.remove(&id);
        c.meta.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apphost::AppHost;

    #[test]
    fn loopback_spawn_and_frame() {
        // Start a server on a throwaway socket inside the per-user dir.
        let path = crate::protocol::socket_dir().join("apphost-test.sock");
        let dir = crate::protocol::socket_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

        // Minimal server thread: own a LocalAppHost, serve one frontend.
        let server = std::thread::spawn(move || {
            use crate::apphost::LocalAppHost;
            let mut local = LocalAppHost::new();
            let mut shutdown = false;
            if let Some(Ok(stream)) = listener.incoming().next() {
                super::super::server_serve_for_test(&mut local, stream, &mut shutdown);
            }
        });

        // Give the listener a moment, then connect a RemoteAppHost.
        std::thread::sleep(Duration::from_millis(50));
        let mut remote = RemoteAppHost::connect(&path).unwrap();
        let id = remote.spawn("cat", &[], None, 40, 10).expect("spawn over socket");

        // Poll until a frame for the app arrives with the requested grid size.
        let mut got = false;
        for _ in 0..100 {
            if let Some(g) = remote.snapshot(id) {
                if g.width() == 40 && g.height() == 10 {
                    got = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(got, "expected a frame for the spawned app");
        assert!(remote.is_alive(id));

        remote.kill(id);
        remote.shutdown_apphost();
        let _ = std::fs::remove_file(&path);
        let _ = server.join();
    }
}
```

NOTE on the test: it calls `super::super::server_serve_for_test`, a thin `#[doc(hidden)]` wrapper the test needs because `serve_frontend` is private to `server.rs`. Add this to `src/apphost/server.rs`:

```rust
/// Test-only re-export of the per-frontend serve loop (used by remote.rs's
/// loopback test). Not part of the public API.
#[doc(hidden)]
pub fn server_serve_for_test(local: &mut LocalAppHost, stream: UnixStream, shutdown: &mut bool) {
    serve_frontend(local, stream, shutdown);
}
```
and reference it from the test as `crate::apphost::server::server_serve_for_test(...)` (adjust the path in the test to that — simpler than `super::super`). Use whichever path resolves; the canonical one is `crate::apphost::server::server_serve_for_test`.

- [ ] **Step 2: Register the module**

In `src/apphost/mod.rs`: `mod remote; pub use remote::RemoteAppHost;`.

- [ ] **Step 3: Build + test (the loopback test spawns a real `cat`)**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test apphost::remote 2>&1 | tail -25`
Expected: `loopback_spawn_and_frame` passes. If it is flaky under load, increase the poll budget — do NOT mark it `#[ignore]` without reporting why.
Run: `cargo clippy --all-targets 2>&1 | tail -15` — zero warnings.

- [ ] **Step 4: Commit**

```bash
git add src/apphost/remote.rs src/apphost/server.rs src/apphost/mod.rs
git commit --no-verify -m "apphost: RemoteAppHost (socket-backed AppHost) + loopback test"
```

---

## Task 4: Orchestration — `tuiui --apphost`, frontend uses RemoteAppHost, `tuiui kill` stops both

**Files:**
- Modify: `src/apphost/api.rs` (add `shutdown_host` to the trait)
- Modify: `src/apphost/host.rs` and `src/apphost/remote.rs` (impl `shutdown_host`)
- Modify: `src/session.rs` (`shutdown` calls `apphost.shutdown_host()`)
- Modify: `src/daemon.rs` (ensure apphost up; inject `RemoteAppHost`)
- Modify: `src/main.rs` (add `--apphost`; `kill` also stops the apphost)

- [ ] **Step 1: Add `shutdown_host` to the trait**

In `src/apphost/api.rs`, add to the `AppHost` trait:
```rust
    /// Stop the underlying app host process, if any (default no-op for the
    /// in-process host). The frontend calls this on full shutdown.
    fn shutdown_host(&mut self) {}
```
In `src/apphost/host.rs` `impl AppHost for LocalAppHost`: nothing to add (the default no-op is correct — apps die with the process).
In `src/apphost/remote.rs` `impl AppHost for RemoteAppHost`, add:
```rust
    fn shutdown_host(&mut self) {
        let _ = send(&mut self.writer, &HostReq::Shutdown);
    }
```
(You may keep or remove the inherent `shutdown_apphost` method — having both is fine; the trait method is what `SessionCore` calls. If you remove the inherent one, update the loopback test to call `AppHost::shutdown_host(&mut remote)`.)

- [ ] **Step 2: `SessionCore::shutdown` tears down the apphost**

In `src/session.rs`, at the END of `pub fn shutdown(&mut self)` (after the kill loop + `self.contents.clear()`), add:
```rust
        self.apphost.shutdown_host();
```

- [ ] **Step 3: Frontend ensures apphost is up and injects `RemoteAppHost`**

In `src/daemon.rs::run()`, replace the line `let mut core = SessionCore::new(w, h, cfg.clone());` with apphost startup + injection:
```rust
    let apphost = ensure_apphost()?;
    let mut core = SessionCore::with_apphost(w, h, cfg.clone(), Box::new(apphost));
```
Add this helper to `src/daemon.rs`:
```rust
/// Ensure the apphost process is running and return a connected handle. Spawns
/// `tuiui --apphost` (detached) if its socket is absent, then connects.
fn ensure_apphost() -> std::io::Result<crate::apphost::RemoteAppHost> {
    use crate::protocol::apphost_socket_path;
    let path = apphost_socket_path();
    if UnixStream::connect(&path).is_err() {
        let exe = std::env::current_exe()?;
        std::process::Command::new(exe)
            .arg("--apphost")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()?;
        for _ in 0..100 {
            if UnixStream::connect(&path).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    crate::apphost::RemoteAppHost::connect(&path)
}
```
Add the needed imports to `daemon.rs` if missing: `use std::os::unix::process::CommandExt;` (for `process_group`). `UnixStream` and `Duration` are already imported.

- [ ] **Step 4: `main.rs` — add `--apphost` and stop the apphost on `kill`**

In `src/main.rs`, add a match arm:
```rust
        Some("--apphost") => tuiui::apphost::server::run(),
```
And in `kill()`, after messaging the frontend daemon, also tell the apphost to exit (covers the case where the frontend already died but the apphost is orphaned):
```rust
    // Also stop the apphost (it outlives the frontend by design).
    if let Ok(mut s) = UnixStream::connect(tuiui::protocol::apphost_socket_path()) {
        let req = tuiui::apphost::proto::HostReq::Shutdown;
        if let Ok(mut buf) = serde_json::to_vec(&req) {
            buf.push(b'\n');
            let _ = s.write_all(&buf);
        }
    }
```
Add `use serde_json;` only if not already available (it is a dependency; `serde_json::to_vec` is used already in `kill`). Update the `--daemon`/usage help comment to mention `--apphost` is internal.

- [ ] **Step 5: Build + full suite + clippy**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build 2>&1 | tail -20`
Expected: clean.
Run: `cargo test 2>&1 | grep -E "test result|error\[|FAILED" | tail -25`
Expected: all pass (217+ from Phase 2a, plus the proto + loopback tests). In-process `SessionCore::new` tests still use `LocalAppHost`, so they are unaffected.
Run: `cargo clippy --all-targets 2>&1 | tail -20` — zero warnings.

- [ ] **Step 6: Commit**

```bash
git add src/apphost/api.rs src/apphost/host.rs src/apphost/remote.rs src/session.rs src/daemon.rs src/main.rs
git commit --no-verify -m "apphost: run apps in a separate --apphost process; frontend uses RemoteAppHost"
```

---

## Task 5: End-to-end verification + manual smoke

**Files:** none (verification)

- [ ] **Step 1: Full gate**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build 2>&1 | tail -3 && cargo test 2>&1 | grep -cE "test result: ok" && cargo clippy --all-targets 2>&1 | grep -cE "warning:|error:"`
Expected: build OK; test-suite count ≥ Phase 2a; clippy `0`.

- [ ] **Step 2: Deploy + manual smoke**

```bash
cargo install --root ~/.local --path . --force
tuiui kill; tuiui
```
Verify:
1. `pgrep -fa 'tuiui --apphost'` shows the apphost process, and `pgrep -fa 'tuiui --daemon'` shows the frontend — two processes.
2. Launch an app (a shell), type into it, resize/move its window, open a graphics app (chafa/yazi) — all identical to before.
3. **Survival test (the Phase 2 payoff):** with an app running, kill ONLY the frontend daemon (`pkill -f 'tuiui --daemon'`), then run `tuiui` again. The apphost (and the child process) is still alive — confirm with `pgrep -fa 'tuiui --apphost'` and that the app's child PID is unchanged. (The window won't visually rebuild yet — that's Phase 3's Roster-driven restore — but the process survived, proving the split.)
4. `tuiui kill` stops BOTH processes (`pgrep -fa tuiui` shows neither `--daemon` nor `--apphost`).

- [ ] **Step 3: Update memory**

Update the `tuiui-roadmap-state` memory: apphost Phase 2 (separate process + IPC) DONE; note Phase 3 (reload UX + Roster-driven window restore + enabling the menubar Restart action) is next.

---

## Risks & notes

- **Frame volume:** full grid as JSON per changed frame. Grids are ~84×30 and only sent on change; acceptable locally. Per-app cell-diff compression is a later optimization (spec "Out of scope").
- **Input latency:** input → apphost → child → grid push → frontend cache → render is one extra hop (~16ms). Acceptable locally; the manual smoke confirms typing feels fine.
- **Startup race:** `ensure_apphost` waits up to 5s for the apphost socket; `spawn`'s reply also has a 5s timeout. If the apphost binary is missing/old, spawn fails cleanly (window is dropped) rather than hanging forever.
- **Two debug logs:** the apphost calls `dbg_log` (NOT `dbg_init`, to avoid truncating the frontend's log). Both append to `~/tuiui-debug.log`; lines are timestamped. Acceptable.
- **Phase 3 (not here):** `tuiui reload` (restart frontend only, keep apphost), Roster-driven window rebuild from `meta`, the in-app "Update & Reload" button, and enabling the menubar **Restart** action.
