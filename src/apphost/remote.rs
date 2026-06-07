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
    mouse: crate::mouse::AppMouse,
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
    }

    /// Tell the apphost process to exit (full shutdown). Best-effort.
    pub fn shutdown_apphost(&mut self) {
        let _ = send(&mut self.writer, &HostReq::Shutdown);
    }
}

fn reader_loop_buffered(mut r: BufReader<UnixStream>, cache: Arc<Mutex<Cache>>, pending: Pending) {
    while let Ok(Some(evt)) = recv::<HostEvt, _>(&mut r) {
        apply_evt(evt, &cache, &pending);
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
        HostEvt::Frame { app, grid, placements, images, alive, mouse } => {
            let mut c = cache.lock().unwrap();
            let entry = c.apps.entry(AppId(app)).or_default();
            entry.grid = Some(grid);
            entry.placements = placements;
            entry.alive = alive;
            entry.mouse = mouse;
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

    fn shutdown_host(&mut self) {
        let _ = send(&mut self.writer, &HostReq::Shutdown);
    }

    fn mouse_mode(&self, id: AppId) -> crate::mouse::AppMouse {
        self.cache.lock().unwrap().apps.get(&id).map(|c| c.mouse).unwrap_or_default()
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
                crate::apphost::server::server_serve_for_test(&mut local, stream, &mut shutdown);
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
