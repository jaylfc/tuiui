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
        while let Ok(Some(req)) = recv::<HostReq, _>(&mut r) {
            if tx.send(req).is_err() {
                break;
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

/// Test-only re-export of the per-frontend serve loop (used by remote.rs's
/// loopback test). Not part of the public API.
#[doc(hidden)]
pub fn server_serve_for_test(local: &mut LocalAppHost, stream: UnixStream, shutdown: &mut bool) {
    serve_frontend(local, stream, shutdown);
}
