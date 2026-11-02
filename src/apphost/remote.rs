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
use std::time::{Duration, Instant};

#[derive(Default)]
struct Cached {
    grid: Option<CellBuffer>,
    placements: Vec<Placement>,
    images: HashMap<u32, Vec<u8>>,
    alive: bool,
    mouse: crate::mouse::AppMouse,
    /// Bell rings accumulated since the frontend last drained them.
    bells: u32,
    /// The app's latest OSC-52 clipboard store, not yet forwarded.
    clip: Option<String>,
    /// The app's OS pid, as reported by the apphost on `Spawned`. `None`
    /// for apps the frontend learned about via the connect-time `Roster`
    /// (already running before this frontend came up).
    pid: Option<u32>,
    /// Original `cmd` + `args` the frontend asked the apphost to spawn.
    /// `None` for roster-only entries.
    cmd: Option<String>,
    args: Option<Vec<String>>,
    /// Wall-clock instant the frontend dispatched the `Spawn` request
    /// (the apphost's own spawn time isn't on the wire, so the frontend
    /// stamps this as a close approximation).
    spawned_at: Option<Instant>,
    /// The app's OSC 0/2 title as of its last frame (v3+ apphosts).
    title: Option<String>,
}

#[derive(Default)]
struct Cache {
    apps: HashMap<AppId, Cached>,
    meta: HashMap<AppId, Vec<u8>>,
    /// The apphost's declared protocol version (0 until/unless it says).
    proto: u32,
}

type Pending = Arc<Mutex<HashMap<u64, mpsc::Sender<Result<AppId, String>>>>>;
/// While a `Spawn` is in flight, we also need the cmd/args the frontend sent
/// so we can populate the per-app cache (the apphost's `Spawned` reply
/// doesn't carry cmd/args). Lives next to the reply channel; cleared on
/// reply or timeout.
type Inflight = Arc<Mutex<HashMap<u64, (String, Vec<String>)>>>;

pub struct RemoteAppHost {
    writer: UnixStream,
    cache: Arc<Mutex<Cache>>,
    pending: Pending,
    inflight: Inflight,
    next_req: AtomicU64,
}

impl RemoteAppHost {
    /// Connect to an already-running apphost at `path`.
    pub fn connect(path: &Path) -> std::io::Result<Self> {
        let writer = UnixStream::connect(path)?;
        let reader = writer.try_clone()?;
        let cache: Arc<Mutex<Cache>> = Arc::new(Mutex::new(Cache::default()));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let inflight: Inflight = Arc::new(Mutex::new(HashMap::new()));

        // The server sends Roster first; read it synchronously so the frontend
        // can rebuild windows immediately after connect.
        let mut buf_reader = std::io::BufReader::new(reader);
        if let Ok(Some(evt)) = crate::apphost::proto::recv::<crate::apphost::proto::HostEvt, _>(&mut buf_reader) {
            apply_evt(evt, &cache, &pending, &inflight);
        }

        {
            let cache = cache.clone();
            let pending = pending.clone();
            let inflight = inflight.clone();
            std::thread::spawn(move || reader_loop_buffered(buf_reader, cache, pending, inflight));
        }
        Ok(RemoteAppHost { writer, cache, pending, inflight, next_req: AtomicU64::new(1) })
    }

    /// Tell the apphost process to exit (full shutdown). Best-effort.
    pub fn shutdown_apphost(&mut self) {
        let _ = send(&mut self.writer, &HostReq::Shutdown);
    }
}

fn reader_loop_buffered(
    mut r: BufReader<UnixStream>,
    cache: Arc<Mutex<Cache>>,
    pending: Pending,
    inflight: Inflight,
) {
    while let Ok(Some(evt)) = recv::<HostEvt, _>(&mut r) {
        apply_evt(evt, &cache, &pending, &inflight);
    }
}

fn apply_evt(evt: HostEvt, cache: &Arc<Mutex<Cache>>, pending: &Pending, inflight: &Inflight) {
    match evt {
        HostEvt::Spawned { req_id, app, pid } => {
            // Seed the cache entry as alive immediately, BEFORE replying to the
            // blocked spawn(). Otherwise the window is created before the first
            // AppFrame arrives, is_alive() returns false (no entry yet), and the
            // frontend's per-tick reap_dead instantly closes the brand-new app.
            // Also capture the cmd/args the frontend requested (the apphost
            // doesn't echo them) and stamp the local wall-clock as the spawn
            // time (the apphost's exact `Instant` isn't on the wire; this is
            // within milliseconds of the real spawn, and the activity monitor
            // only needs minute-resolution ages).
            let (cmd, args) = inflight.lock().unwrap().remove(&req_id).unwrap_or_default();
            let mut c = cache.lock().unwrap();
            let entry = c.apps.entry(AppId(app)).or_default();
            entry.alive = true;
            if pid.is_some() {
                entry.pid = pid;
            }
            if !cmd.is_empty() {
                entry.cmd = Some(cmd);
            }
            if !args.is_empty() {
                entry.args = Some(args);
            }
            entry.spawned_at = Some(Instant::now());
            drop(c);
            if let Some(tx) = pending.lock().unwrap().remove(&req_id) {
                let _ = tx.send(Ok(AppId(app)));
            }
        }
        HostEvt::SpawnFailed { req_id, error } => {
            // Clear the inflight stash too — the apphost will never reply
            // with a Spawned for this req_id, so leaving it there would
            // leak the (cmd, args) entry until the apphost disconnects.
            inflight.lock().unwrap().remove(&req_id);
            if let Some(tx) = pending.lock().unwrap().remove(&req_id) {
                let _ = tx.send(Err(error));
            }
        }
        HostEvt::Frame { app, grid, placements, images, alive, mouse, bells, clip, title, cursor: _ } => {
            let mut c = cache.lock().unwrap();
            let entry = c.apps.entry(AppId(app)).or_default();
            entry.grid = Some(grid);
            entry.title = title;
            entry.placements = placements;
            entry.alive = alive;
            entry.mouse = mouse;
            entry.bells = entry.bells.saturating_add(bells);
            if clip.is_some() {
                entry.clip = clip;
            }
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
        HostEvt::Roster { apps, proto } => {
            let mut c = cache.lock().unwrap();
            c.proto = proto;
            for entry in apps {
                let id = AppId(entry.app);
                c.meta.insert(id, entry.meta);
                let cached = c.apps.entry(id).or_default();
                cached.alive = true;
                if entry.pid.is_some() {
                    cached.pid = entry.pid;
                }
            }
        }
        HostEvt::AppList { .. } => {
            // Reply to HostReq::ListApps (used by `tuiui ps` / `tuiui kill-app`).
            // The remote handle isn't the consumer — those CLI commands open a
            // short-lived connection and read the reply themselves, so we drop it.
        }
    }
}

impl AppHost for RemoteAppHost {
    fn spawn(&mut self, cmd: &str, args: &[String], cwd: Option<&Path>, cols: i32, rows: i32) -> std::io::Result<AppId> {
        let req_id = self.next_req.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(req_id, tx);
        // Stash the cmd/args so the Spawned handler can populate the cache
        // (the apphost doesn't echo them back).
        self.inflight.lock().unwrap().insert(req_id, (cmd.to_string(), args.to_vec()));
        let req = HostReq::Spawn {
            req_id,
            cmd: cmd.to_string(),
            args: args.to_vec(),
            cwd: cwd.map(|p| p.to_string_lossy().into_owned()),
            cols,
            rows,
        };
        if let Err(e) = send(&mut self.writer, &req) {
            // The apphost will never see this req, so neither reply will
            // ever arrive — clear both maps to avoid leaking the (cmd,args)
            // and the reply channel until the apphost disconnects.
            self.pending.lock().unwrap().remove(&req_id);
            self.inflight.lock().unwrap().remove(&req_id);
            return Err(e);
        }
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(e)) => {
                // SpawnFailed was already handled in apply_evt (cleared both
                // maps); the timeout path is the only case where we need to
                // clean up here, since the apphost will never reply.
                Err(std::io::Error::other(e))
            }
            Err(_) => {
                self.pending.lock().unwrap().remove(&req_id);
                self.inflight.lock().unwrap().remove(&req_id);
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

    fn scroll(&mut self, id: AppId, lines: i32) {
        let _ = send(&mut self.writer, &HostReq::Scroll { app: id.0, lines });
    }

    fn proto_version(&self) -> u32 {
        self.cache.lock().unwrap().proto
    }

    fn kill(&mut self, id: AppId) {
        let _ = send(&mut self.writer, &HostReq::Kill { app: id.0 });
    }

    fn is_alive(&mut self, id: AppId) -> bool {
        self.cache.lock().unwrap().apps.get(&id).map(|c| c.alive).unwrap_or(false)
    }

    fn take_bells(&mut self, id: AppId) -> u32 {
        self.cache.lock().unwrap().apps.get_mut(&id).map(|c| std::mem::take(&mut c.bells)).unwrap_or(0)
    }

    fn take_clipboard(&mut self, id: AppId) -> Option<String> {
        self.cache.lock().unwrap().apps.get_mut(&id).and_then(|c| c.clip.take())
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

    fn pid(&self, id: AppId) -> Option<u32> {
        self.cache.lock().unwrap().apps.get(&id).and_then(|c| c.pid)
    }

    fn spawn_time(&self, id: AppId) -> Option<std::time::Instant> {
        self.cache.lock().unwrap().apps.get(&id).and_then(|c| c.spawned_at)
    }

    /// Try to recover the original `cmd` + `args` for `id` from the inflight
    /// stash; the session falls back to the concrete `LocalAppHost` first and
    /// only asks here if that didn't match.
    fn launch_cmd(&self, id: AppId) -> Option<(String, Vec<String>)> {
        let c = self.cache.lock().unwrap();
        let entry = c.apps.get(&id)?;
        let cmd = entry.cmd.clone()?;
        let args = entry.args.clone().unwrap_or_default();
        Some((cmd, args))
    }

    fn shutdown_host(&mut self) {
        let _ = send(&mut self.writer, &HostReq::Shutdown);
    }

    fn mouse_mode(&self, id: AppId) -> crate::mouse::AppMouse {
        self.cache.lock().unwrap().apps.get(&id).map(|c| c.mouse).unwrap_or_default()
    }

    fn title(&self, id: AppId) -> Option<String> {
        self.cache.lock().unwrap().apps.get(&id).and_then(|c| c.title.clone())
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
            let local = Arc::new(Mutex::new(LocalAppHost::new()));
            let shutdown = std::sync::atomic::AtomicBool::new(false);
            if let Some(Ok(stream)) = listener.incoming().next() {
                crate::apphost::server::server_serve_for_test(&local, stream, &shutdown);
            }
        });

        // Give the listener a moment, then connect a RemoteAppHost.
        std::thread::sleep(Duration::from_millis(50));
        let mut remote = RemoteAppHost::connect(&path).unwrap();
        let id = remote.spawn("cat", &[], None, 40, 10).expect("spawn over socket");
        // Must report alive IMMEDIATELY (before any frame), or the frontend's
        // per-tick reap_dead would close the brand-new window. (regression guard)
        assert!(remote.is_alive(id), "a freshly spawned app must be alive before its first frame");

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

    /// `tuiui ps` / `tuiui kill-app` depend on `ListApps` round-tripping
    /// correctly through a real server connection. Spawn 3 apps, list, assert
    /// the apphost reports 3 rows with the expected fields.
    #[test]
    fn loopback_list_apps_round_trip() {
        use crate::apphost::proto::{send, HostEvt, HostReq};
        use std::io::BufReader;

        let path = crate::protocol::socket_dir().join("apphost-list-test.sock");
        let dir = crate::protocol::socket_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

        let server = std::thread::spawn(move || {
            use crate::apphost::LocalAppHost;
            let local = Arc::new(Mutex::new(LocalAppHost::new()));
            let shutdown = std::sync::atomic::AtomicBool::new(false);
            if let Some(Ok(stream)) = listener.incoming().next() {
                crate::apphost::server::server_serve_for_test(&local, stream, &shutdown);
            }
        });

        std::thread::sleep(Duration::from_millis(50));
        let mut writer = UnixStream::connect(&path).unwrap();
        let mut r = BufReader::new(writer.try_clone().unwrap());

        // Spawn three apps, then ask for the list. The server processes
        // commands synchronously, so by the time we read AppList all three
        // apps are registered with the shared LocalAppHost.
        for i in 1..=3u64 {
            send(
                &mut writer,
                &HostReq::Spawn {
                    req_id: i,
                    // Long-lived sleep so the child is alive when we read
                    // the AppList. `true` exits instantly and trips
                    // is_alive.
                    cmd: "sh".into(),
                    args: vec!["-c".into(), "sleep 30".into()],
                    cwd: None,
                    cols: 80,
                    rows: 24,
                },
            )
            .unwrap();
        }
        send(&mut writer, &HostReq::ListApps).unwrap();

        // A background reader drains the apphost's frame stream so the
        // server's `send` never blocks on a full kernel buffer. We pass the
        // AppList back through a oneshot channel and ignore everything else.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || loop {
            match crate::apphost::proto::recv::<HostEvt, _>(&mut r) {
                Ok(Some(HostEvt::AppList { apps })) => {
                    let _ = tx.send(apps);
                    // Keep reading: server will continue pushing Frames until
                    // we close the socket. Ignore them.
                }
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            }
        });
        let apps = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("timeout waiting for AppList");
        assert_eq!(apps.len(), 3, "expected 3 rows, got {apps:?}");
        for a in &apps {
            assert_eq!(a.cmd, "sh");
            assert!(a.pid.is_some(), "spawned app must have a pid");
            assert!(a.alive, "freshly spawned sh -c sleep 30 should be alive");
            assert_eq!(a.cols, 80);
            assert_eq!(a.rows, 24);
        }

        // Tell the apphost to shut down; the background reader will hit EOF
        // and exit, the server returns, the thread joins.
        send(&mut writer, &HostReq::Shutdown).unwrap();
        drop(writer);
        let _ = std::fs::remove_file(&path);
        let _ = server.join();
    }

    /// Regression: when the apphost returns `SpawnFailed` for a spawn, the
    /// `inflight` map must clear the (cmd, args) entry — otherwise the next
    /// successful spawn that re-uses the same `req_id` (e.g. after a wrap)
    /// would briefly see stale cmd/args in the cache. The bug was that the
    /// `SpawnFailed` arm of `apply_evt` only cleared `pending`, leaking the
    /// inflight entry until the apphost disconnected.
    #[test]
    fn spawn_failure_clears_inflight() {
        let path = crate::protocol::socket_dir().join("apphost-fail-test.sock");
        let dir = crate::protocol::socket_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

        let server = std::thread::spawn(move || {
            use crate::apphost::LocalAppHost;
            let local = Arc::new(Mutex::new(LocalAppHost::new()));
            let shutdown = std::sync::atomic::AtomicBool::new(false);
            if let Some(Ok(stream)) = listener.incoming().next() {
                crate::apphost::server::server_serve_for_test(&local, stream, &shutdown);
            }
        });

        std::thread::sleep(Duration::from_millis(50));
        let mut remote = RemoteAppHost::connect(&path).unwrap();
        // A bogus command path — the apphost will reply with SpawnFailed.
        // (We expect exactly one error, not a panic or hang.)
        let result = remote.spawn("/nonexistent/command/that/never/exists", &[], None, 80, 24);
        assert!(result.is_err(), "spawn of a missing binary must fail");
        // And a second, different failing spawn should also work — meaning
        // the first failure's bookkeeping was cleaned up and didn't block
        // the next request (no req_id collision, no leaked inflight).
        let result2 = remote.spawn("/also/nonexistent", &[], None, 80, 24);
        assert!(result2.is_err(), "second failing spawn must also be reported cleanly");

        // Ask the apphost to shut down so the server thread returns cleanly
        // (dropping the remote's writer would also work, but a 16ms tick in
        // the server's frame-push loop can occasionally race with the reader
        // thread on close — sending Shutdown is deterministic).
        remote.shutdown_apphost();
        drop(remote);
        let _ = std::fs::remove_file(&path);
        let _ = server.join();
    }

    /// v3 multi-frontend: while one frontend (the "desktop") stays connected
    /// and streaming, a second connection must still get its roster and an
    /// `AppList` (carrying the desktop's meta and the app's OSC title), a
    /// viewer watching one app gets only that app's frames, and `Gone` reaches
    /// every connection.
    #[test]
    fn second_frontend_lists_apps_while_first_is_connected() {
        use crate::apphost::proto::{recv, send, HostEvt, HostReq};
        let dir = crate::protocol::socket_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("apphost-multi-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let local: crate::apphost::server::SharedHost = Arc::new(Mutex::new(crate::apphost::LocalAppHost::new()));
        let server = {
            let path = path.clone();
            let local = local.clone();
            std::thread::spawn(move || crate::apphost::server::serve(listener, &path, &local))
        };

        let mut desktop = RemoteAppHost::connect(&path).unwrap();
        let titled = desktop
            .spawn("sh", &["-c".into(), "printf '\\033]2;✳ Probe\\007'; sleep 30".into()], None, 40, 10)
            .unwrap();
        let other = desktop.spawn("sh", &["-c".into(), "sleep 30".into()], None, 40, 10).unwrap();
        desktop.set_meta(titled, br#"{"title":"Win"}"#.to_vec());
        // The desktop's cache learns the OSC title from the frame stream.
        let mut got_title = false;
        for _ in 0..150 {
            if desktop.title(titled).as_deref() == Some("✳ Probe") {
                got_title = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(got_title, "Frame.title must reach the frontend");

        // A `tuiui ps`-style second connection, with the desktop still attached.
        let mut ps = UnixStream::connect(&path).unwrap();
        ps.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut r = BufReader::new(ps.try_clone().unwrap());
        match recv::<HostEvt, _>(&mut r).unwrap().unwrap() {
            HostEvt::Roster { apps, proto } => {
                assert_eq!(proto, crate::apphost::proto::PROTO_VERSION);
                assert_eq!(apps.len(), 2);
                assert!(apps.iter().all(|a| a.pid.is_some()), "roster carries pids");
            }
            other => panic!("expected roster, got {other:?}"),
        }
        send(&mut ps, &HostReq::Watch { apps: vec![] }).unwrap();
        send(&mut ps, &HostReq::ListApps).unwrap();
        let list = loop {
            match recv::<HostEvt, _>(&mut r).unwrap().unwrap() {
                HostEvt::AppList { apps } => break apps,
                _ => continue,
            }
        };
        let row = list.iter().find(|a| a.app == titled.0).expect("titled app listed");
        assert_eq!(row.meta, br#"{"title":"Win"}"#.to_vec());
        assert_eq!(row.title.as_deref(), Some("✳ Probe"));
        drop(r);
        drop(ps);

        // A viewer of one app: only its frames arrive.
        let mut view = UnixStream::connect(&path).unwrap();
        view.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut vr = BufReader::new(view.try_clone().unwrap());
        let _roster = recv::<HostEvt, _>(&mut vr).unwrap();
        send(&mut view, &HostReq::Watch { apps: vec![other.0] }).unwrap();
        // Frames for both may race the Watch on the first tick; after a
        // resize of `other` we must see its frame, and kill → Gone.
        std::thread::sleep(Duration::from_millis(100));
        desktop.resize(other, 30, 8);
        let mut saw_resized = false;
        for _ in 0..50 {
            match recv::<HostEvt, _>(&mut vr) {
                Ok(Some(HostEvt::Frame { app, grid, .. })) if app == other.0 && grid.width() == 30 => {
                    saw_resized = true;
                    break;
                }
                Ok(Some(_)) => continue,
                _ => break,
            }
        }
        assert!(saw_resized, "viewer gets the watched app's frames");
        desktop.kill(other);
        let mut saw_gone = false;
        for _ in 0..200 {
            match recv::<HostEvt, _>(&mut vr) {
                Ok(Some(HostEvt::Gone { app })) if app == other.0 => {
                    saw_gone = true;
                    break;
                }
                Ok(Some(HostEvt::Frame { app, .. })) => assert_eq!(app, other.0, "viewer only gets watched frames"),
                Ok(Some(_)) => continue,
                _ => break,
            }
        }
        assert!(saw_gone, "viewer is told the watched app is gone");
        // ...and so is the desktop (whichever thread reaped it).
        let mut desktop_gone = false;
        for _ in 0..100 {
            if !desktop.is_alive(other) {
                desktop_gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(desktop_gone, "desktop also learns the app is gone");

        desktop.kill(titled);
        desktop.shutdown_apphost();
        let _ = server.join();
        let _ = std::fs::remove_file(&path);
    }
}
