//! The apphost process: owns the live apps behind a socket, pushing per-app
//! frames and accepting commands from its frontends. Started as `tuiui --apphost`
//! (normally spawned automatically by the frontend daemon). The apps it owns
//! survive a frontend restart because this process keeps running.
//!
//! Since protocol v3 it serves **several frontends at once** — the desktop
//! daemon plus any number of short-lived `tuiui ps` / `tuiui kill-app`
//! connections and `tuiui view` viewers — each on its own thread with its own
//! change state, all sharing one [`LocalAppHost`] behind a mutex. Before v3 a
//! second connection queued behind the attached daemon forever (so `tuiui ps`
//! hung while a desktop was attached).

use crate::apphost::proto::{recv, send, AppListEntry, HostEvt, HostReq, ImgBlob, RosterEntry};
use crate::apphost::{AppHost, AppId, LocalAppHost};
use crate::protocol::{apphost_socket_path, socket_dir};
use std::collections::{HashMap, HashSet};
use std::fs::Permissions;
use std::io::BufReader;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::Duration;

/// The app table every frontend connection shares.
pub type SharedHost = Arc<Mutex<LocalAppHost>>;

/// Lock the shared host, recovering a poisoned lock (one panicking connection
/// thread must not wedge every other frontend — or the apps — forever).
fn lock(local: &Mutex<LocalAppHost>) -> MutexGuard<'_, LocalAppHost> {
    local.lock().unwrap_or_else(|e| e.into_inner())
}

/// Run the apphost event loop until a frontend sends `Shutdown`.
pub fn run() -> std::io::Result<()> {
    let path = apphost_socket_path();
    // If another apphost is already listening (e.g. the service started it, or the
    // daemon spawned it), exit cleanly rather than rebinding the socket. Combined
    // with "restart on failure only", a redundant service start won't loop.
    if UnixStream::connect(&path).is_ok() {
        crate::dbg_log("apphost: another instance already running; exiting");
        return Ok(());
    }
    let dir = socket_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, Permissions::from_mode(0o700))?;
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, Permissions::from_mode(0o600))?;
    crate::dbg_log("apphost: listening");

    let local: SharedHost = Arc::new(Mutex::new(LocalAppHost::new()));
    serve(listener, &path, &local);

    // Full shutdown: kill every app, then remove the socket.
    {
        let mut host = lock(&local);
        for id in host.list() {
            host.kill(id);
        }
    }
    let _ = std::fs::remove_file(&path);
    crate::dbg_log("apphost: shut down");
    Ok(())
}

/// Accept frontends on `listener` (bound at `path`) until one sends
/// `Shutdown`, serving each on its own thread. Returns once shut down; the
/// caller kills the apps.
pub fn serve(listener: UnixListener, path: &Path, local: &SharedHost) {
    let shutdown = Arc::new(AtomicBool::new(false));
    for stream in listener.incoming() {
        if shutdown.load(Ordering::SeqCst) {
            break; // woken by the thread that saw Shutdown
        }
        let Ok(stream) = stream else { continue };
        let local = Arc::clone(local);
        let flag = Arc::clone(&shutdown);
        let wake_path = path.to_path_buf();
        std::thread::spawn(move || {
            serve_frontend(&local, stream, &flag);
            if flag.load(Ordering::SeqCst) {
                // Unblock the accept loop so it sees the flag and exits.
                let _ = UnixStream::connect(&wake_path);
            } else {
                crate::dbg_log("apphost: frontend detached; apps stay alive");
            }
        });
    }
}

/// Per-connection change tracking. Fresh for every connection, so a frontend
/// always gets full grids on (re)connect.
#[derive(Default)]
struct ConnState {
    last_grid: HashMap<AppId, crate::buffer::CellBuffer>,
    last_placements: HashMap<AppId, Vec<crate::kittygfx::Placement>>,
    last_mouse: HashMap<AppId, crate::mouse::AppMouse>,
    last_title: HashMap<AppId, Option<String>>,
    last_cursor: HashMap<AppId, Option<(u16, u16)>>,
    sent_images: HashMap<AppId, HashSet<u32>>,
    /// Bell total / clipboard sequence already forwarded on THIS connection.
    bells_seen: HashMap<AppId, u64>,
    clip_seen: HashMap<AppId, u64>,
    /// Apps this connection has been streaming (so it can be told `Gone` even
    /// when another connection's thread reaped the app first).
    known: HashSet<AppId>,
    /// `Some` once the frontend sent `Watch`: only these apps get frames, and
    /// the connection is a viewer (never takes bells / clipboard stores).
    watch: Option<HashSet<u64>>,
}

impl ConnState {
    fn forget(&mut self, id: AppId) {
        self.last_grid.remove(&id);
        self.last_placements.remove(&id);
        self.last_mouse.remove(&id);
        self.last_title.remove(&id);
        self.last_cursor.remove(&id);
        self.sent_images.remove(&id);
        self.bells_seen.remove(&id);
        self.clip_seen.remove(&id);
        self.known.remove(&id);
    }

    fn watches(&self, id: AppId) -> bool {
        self.watch.as_ref().map(|w| w.contains(&id.0)).unwrap_or(true)
    }
}

/// Serve a single connected frontend until it disconnects or any frontend
/// requests `Shutdown` (which sets `shutdown`).
fn serve_frontend(local: &Mutex<LocalAppHost>, stream: UnixStream, shutdown: &AtomicBool) {
    let Ok(reader_stream) = stream.try_clone() else { return };
    let mut writer = stream;

    // Frontend → channel on a reader thread.
    let (tx, rx) = mpsc::channel::<HostReq>();
    std::thread::spawn(move || {
        let mut r = BufReader::new(reader_stream);
        while let Ok(Some(req)) = recv::<HostReq, _>(&mut r) {
            if tx.send(req).is_err() {
                break;
            }
        }
    });

    let mut st = ConnState::default();
    // Roster so the frontend can rebuild its windows. Bells/clipboard stores
    // that predate this connection were already delivered (or are stale), so
    // start this connection's counters at the current totals.
    let roster = {
        let host = lock(local);
        let apps = host
            .list()
            .into_iter()
            .map(|id| {
                st.bells_seen.insert(id, host.bell_total(id));
                st.clip_seen.insert(id, host.clipboard_latest(id).0);
                RosterEntry { app: id.0, meta: host.meta(id).unwrap_or_default(), pid: host.pid(id) }
            })
            .collect();
        HostEvt::Roster { apps, proto: crate::apphost::proto::PROTO_VERSION }
    };
    // Best-effort: a short-lived connection (e.g. `tuiui kill` sending Shutdown)
    // may close before we finish writing the roster. Don't abandon the
    // connection on that — we still want to read any pending command (Shutdown).
    let _ = send(&mut writer, &roster);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        // Drain commands. Each one holds the host lock only while it runs;
        // replies are written after the lock is released, so a slow peer can
        // only ever stall its own thread.
        loop {
            let req = match rx.try_recv() {
                Ok(req) => req,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return, // frontend gone
            };
            let reply = match req {
                HostReq::Shutdown => {
                    shutdown.store(true, Ordering::SeqCst);
                    return;
                }
                HostReq::Watch { apps } => {
                    st.watch = Some(apps.into_iter().collect());
                    // Stop tracking apps that are now filtered out.
                    let drop: Vec<AppId> = st.known.iter().copied().filter(|id| !st.watches(*id)).collect();
                    for id in drop {
                        st.forget(id);
                    }
                    None
                }
                other => handle_req(&mut lock(local), other),
            };
            if let Some(evt) = reply {
                if send(&mut writer, &evt).is_err() {
                    return;
                }
            }
        }

        let out = collect_frames(&mut lock(local), &mut st);
        for evt in &out {
            if send(&mut writer, evt).is_err() {
                return;
            }
        }

        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Apply one (non-Shutdown/Watch) request to the host; returns the reply, if any.
fn handle_req(host: &mut LocalAppHost, req: HostReq) -> Option<HostEvt> {
    match req {
        HostReq::Spawn { req_id, cmd, args, cwd, cols, rows } => {
            let cwd_path = cwd.as_deref().map(std::path::Path::new);
            Some(match host.spawn(&cmd, &args, cwd_path, cols, rows) {
                Ok(id) => HostEvt::Spawned { req_id, app: id.0, pid: host.pid(id) },
                Err(e) => HostEvt::SpawnFailed { req_id, error: e.to_string() },
            })
        }
        HostReq::Input { app, bytes } => {
            host.input(AppId(app), &bytes);
            None
        }
        HostReq::Resize { app, cols, rows } => {
            host.resize(AppId(app), cols, rows);
            None
        }
        HostReq::Scroll { app, lines } => {
            host.scroll(AppId(app), lines);
            None
        }
        HostReq::SetMeta { app, meta } => {
            host.set_meta(AppId(app), meta);
            None
        }
        HostReq::Kill { app } => {
            host.kill(AppId(app));
            None
        }
        HostReq::ListApps => Some(HostEvt::AppList { apps: list_apps(host) }),
        // Handled by the caller (they touch connection / process state).
        HostReq::Shutdown | HostReq::Watch { .. } => None,
    }
}

/// One `AppListEntry` per hosted app (the `ListApps` reply).
fn list_apps(host: &mut LocalAppHost) -> Vec<AppListEntry> {
    let now = std::time::Instant::now();
    let mut apps = Vec::new();
    for id in host.list() {
        let (cmd, args) = host
            .launch_cmd(id)
            .map(|(c, a)| (c.to_string(), a.to_vec()))
            .unwrap_or_else(|| (String::new(), Vec::new()));
        let (cols, rows) = host.dims(id).unwrap_or((0, 0));
        let age_secs = host.spawn_time(id).map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
        apps.push(AppListEntry {
            app: id.0,
            cmd,
            args,
            pid: host.pid(id),
            cols,
            rows,
            age_secs,
            alive: host.is_alive(id),
            meta: host.meta(id).unwrap_or_default(),
            title: AppHost::title(host, id),
        });
    }
    apps.sort_by_key(|a| a.app);
    apps
}

/// Build this connection's outgoing events for one tick: `Gone` for apps that
/// exited (or that another connection already reaped), and a `Frame` for
/// every watched app whose grid / placements / mouse mode / title / cursor
/// changed or that has new images, bells or a clipboard store.
fn collect_frames(host: &mut LocalAppHost, st: &mut ConnState) -> Vec<HostEvt> {
    let mut out = Vec::new();
    let ids: Vec<AppId> = host.list();
    let live: HashSet<AppId> = ids.iter().copied().collect();
    let vanished: Vec<AppId> = st.known.iter().copied().filter(|id| !live.contains(id)).collect();
    for id in vanished {
        out.push(HostEvt::Gone { app: id.0 });
        st.forget(id);
    }
    for id in ids {
        if !host.is_alive(id) {
            // First connection to notice reaps it; the others see it vanish
            // from `list()` on their next tick and send their own `Gone`.
            out.push(HostEvt::Gone { app: id.0 });
            host.remove(id);
            st.forget(id);
            continue;
        }
        if !st.watches(id) {
            continue;
        }
        st.known.insert(id);
        let Some(grid) = host.snapshot(id) else { continue };
        let placements = host.placements(id);
        let mouse = host.mouse_mode(id);
        let title = AppHost::title(host, id);
        let cursor = host.cursor(id);

        // New image blobs (not yet sent to this frontend).
        let seen = st.sent_images.entry(id).or_default();
        let mut images = Vec::new();
        for pl in &placements {
            if !seen.contains(&pl.image_id) {
                if let Some(png) = host.image_png(id, pl.image_id) {
                    images.push(ImgBlob { image_id: pl.image_id, png_b64: crate::kitty::b64(&png) });
                    seen.insert(pl.image_id);
                }
            }
        }

        // Bells / clipboard: per-connection deltas over the host's monotonic
        // counters. Viewers never take them (the desktop owns notifications).
        let viewer = st.watch.is_some();
        let total = host.bell_total(id);
        let seen_bells = st.bells_seen.entry(id).or_insert(0);
        let bells = if viewer { 0 } else { total.saturating_sub(*seen_bells).min(u32::MAX as u64) as u32 };
        *seen_bells = total;
        let (seq, text) = host.clipboard_latest(id);
        let seen_clip = st.clip_seen.entry(id).or_insert(0);
        let clip = if !viewer && seq > *seen_clip { text } else { None };
        *seen_clip = seq;

        let changed = st.last_grid.get(&id) != Some(&grid)
            || st.last_placements.get(&id) != Some(&placements)
            || st.last_mouse.get(&id) != Some(&mouse)
            || st.last_title.get(&id) != Some(&title)
            || st.last_cursor.get(&id) != Some(&cursor);
        if changed || !images.is_empty() || bells > 0 || clip.is_some() {
            out.push(HostEvt::Frame {
                app: id.0,
                grid: grid.clone(),
                placements: placements.clone(),
                images,
                alive: true,
                mouse,
                bells,
                clip,
                title: title.clone(),
                cursor,
            });
            st.last_grid.insert(id, grid);
            st.last_placements.insert(id, placements);
            st.last_mouse.insert(id, mouse);
            st.last_title.insert(id, title);
            st.last_cursor.insert(id, cursor);
        }
    }
    out
}

/// Test-only re-export of the per-frontend serve loop (used by remote.rs's
/// loopback tests). Not part of the public API.
#[doc(hidden)]
pub fn server_serve_for_test(local: &SharedHost, stream: UnixStream, shutdown: &AtomicBool) {
    serve_frontend(local, stream, shutdown);
}
