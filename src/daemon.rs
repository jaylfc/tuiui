//! The tuiui daemon: owns the [`SessionCore`] (windows + PTYs) and serves frames
//! to one attached client at a time over a Unix socket. Apps keep running while
//! detached, so reattaching restores the live session.

use crate::compositor::Compositor;
use crate::config::Config;
use crate::protocol::{daemon_ctl_path, socket_dir, socket_path, Flags, FrameMsg};
use crate::session::{ClientMsg, SessionCore};
use std::fs::Permissions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// Out-of-band control messages (`tuiui launch/tile/theme/msg`) queued by the
/// control-socket thread and drained into the session by the render loop.
type CtlQueue = Arc<std::sync::Mutex<Vec<ClientMsg>>>;

/// Shared out-of-band control state, set by the control-socket thread and polled
/// by the render loop: 0 = none, 1 = shutdown, 2 = reload.
const CTL_NONE: u8 = 0;
const CTL_SHUTDOWN: u8 = 1;
const CTL_RELOAD: u8 = 2;

/// Run the daemon event loop until `tuiui kill` (or a fatal socket error).
pub fn run() -> std::io::Result<()> {
    crate::dbg_init();
    // Confine the socket to a per-user 0700 directory so other local users
    // cannot connect to (and thus drive/observe) the session. Creating the
    // restrictive directory first avoids any window where the socket is exposed.
    let dir = socket_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, Permissions::from_mode(0o700))?;

    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, Permissions::from_mode(0o600))?;

    // Out-of-band control socket so `tuiui kill` / `tuiui reload` are honored even
    // while a client is attached (the client socket is served serially, so a
    // control message there would queue behind the attached client forever). A
    // listener thread reads Shutdown/Reload and flips this shared flag, which the
    // render loop polls each tick.
    let ctl_path = daemon_ctl_path();
    let _ = std::fs::remove_file(&ctl_path);
    let ctl = Arc::new(AtomicU8::new(CTL_NONE));
    let ctl_queue: CtlQueue = Arc::new(std::sync::Mutex::new(Vec::new()));
    if let Ok(ctl_listener) = UnixListener::bind(&ctl_path) {
        let _ = std::fs::set_permissions(&ctl_path, Permissions::from_mode(0o600));
        let ctl_flag = Arc::clone(&ctl);
        let queue = Arc::clone(&ctl_queue);
        std::thread::spawn(move || serve_control(ctl_listener, ctl_flag, queue));
    }

    let cfg = Config::load();
    crate::theme::set(&cfg.theme);
    let (w, h) = (100, 30); // provisional until the first client reports its size
    let apphost = ensure_apphost()?;
    let mut core = SessionCore::with_apphost(w, h, cfg.clone(), Box::new(apphost));
    // Start the background system poller and feed its shared snapshot to the
    // session so the menubar tray reflects live host state. Keep `_poller` bound
    // for the daemon's lifetime so its thread is not detached/dropped.
    let _poller = crate::poller::SystemPoller::start();
    core.attach_tray_state(_poller.state());
    // Rebuild windows for any apps the apphost already owns (reload / crash
    // recovery). Only auto-launch the configured apps on a truly fresh start.
    // Auto-launch the configured apps ONLY on a genuinely fresh apphost. If the
    // apphost already owns apps (reload / crash recovery), restore their windows
    // instead — never re-launch, or we'd spawn duplicates of apps that are still
    // alive (e.g. when a window's restore meta is missing).
    if core.host_app_count() == 0 {
        for app in &cfg.apps {
            core.apply(ClientMsg::Launch {
                name: app.name.clone(),
                command: app.command.clone(),
                args: app.args.clone(),
            });
        }
    } else {
        let restored = core.restore_windows_from_host();
        crate::dbg_log(&format!("frontend: restored {restored} app window(s) from apphost"));
        // Wait briefly for the apphost to stream each restored app's first frame
        // so the first paint shows real content instead of a blank window flash.
        for _ in 0..30 {
            if core.app_windows_ready() {
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
    }
    // Safety check: if the running apphost is older than this binary can
    // safely talk to AND owns live apps, raise the "restart app server"
    // dialog so the user can save work first (instead of silent breakage).
    core.check_apphost_compat();
    // Honour a one-shot UI reopen hint (e.g. reopen Settings → Updates after an
    // in-app update reloaded the frontend).
    core.reopen_ui_from_hint();
    let mut comp = Compositor::new(w, h);

    // While unattached, the loop blocks in `accept()`; a control message can't be
    // seen until a client connects. So `tuiui kill`/`reload` ALSO poke the main
    // socket (a no-op connection) to wake this accept — see `main.rs`. Either way
    // we re-check the control flag on each wake.
    let mut reloading = false;
    for stream in listener.incoming() {
        if apply_ctl(&ctl, &ctl_queue, &mut core) {
            // A control message arrived while unattached; this stream is the wake
            // poke. Don't serve it as a client.
        } else if let Ok(stream) = stream {
            serve_client(&mut core, &mut comp, stream, &ctl, &ctl_queue);
        }
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
    let _ = std::fs::remove_file(&ctl_path);
    if !reloading {
        core.shutdown(); // full stop: kills apps + tells the apphost to exit
    }
    // On reload we just drop `core` (its RemoteAppHost disconnects); the apphost
    // keeps running so the next frontend can restore the apps.
    Ok(())
}

/// Ensure the apphost process is running and return a connected handle. Spawns
/// `tuiui --apphost` (detached) if its socket is absent, then connects.
fn ensure_apphost() -> std::io::Result<crate::apphost::RemoteAppHost> {
    use crate::protocol::apphost_socket_path;
    let path = apphost_socket_path();
    if UnixStream::connect(&path).is_err() {
        // Prefer the installed per-user service so the *supervised* apphost owns
        // the apps. On macOS that one runs in the GUI login session, which is why
        // Keychain-backed logins (e.g. the Claude Code CLI) work inside it — a
        // detached on-demand apphost we spawn here would not. Only spawn our own
        // when no service is installed, or as a fallback if the service didn't
        // bring its socket up in time.
        let used_service = crate::service::ensure_started();
        if !used_service {
            spawn_detached_apphost()?;
        }
        if !wait_for_socket(&path) && used_service {
            // The service was installed but didn't start in time; fall back so the
            // daemon still gets a working apphost rather than failing to connect.
            spawn_detached_apphost()?;
            wait_for_socket(&path);
        }
    }
    crate::apphost::RemoteAppHost::connect(&path)
}

/// Spawn `tuiui --apphost` detached into its own process group.
fn spawn_detached_apphost() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg("--apphost")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(())
}

/// Poll the apphost socket for up to ~5s. Returns whether it came up.
fn wait_for_socket(path: &std::path::Path) -> bool {
    for _ in 0..100 {
        if UnixStream::connect(path).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Serve one attached client until it detaches (socket closes) or asks to detach.
fn serve_client(
    core: &mut SessionCore,
    comp: &mut Compositor,
    stream: UnixStream,
    ctl: &Arc<AtomicU8>,
    ctl_queue: &CtlQueue,
) {
    let Ok(reader_stream) = stream.try_clone() else { return };
    let mut writer = stream;

    // Read client messages on a thread; the main loop drains them and ticks.
    let (tx, rx) = mpsc::channel::<ClientMsg>();
    std::thread::spawn(move || {
        let mut r = BufReader::new(reader_stream);
        let mut line = String::new();
        loop {
            line.clear();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => break, // EOF == detached
                Ok(_) => {
                    if let Ok(msg) = serde_json::from_str::<ClientMsg>(line.trim()) {
                        if tx.send(msg).is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    // Force a full repaint for the freshly attached client.
    comp.resize(comp.width(), comp.height());
    // Image ids whose bytes this client has already received (reset per attach,
    // mirroring the full cell-repaint above).
    let mut sent_image_ids: std::collections::HashSet<u64> = std::collections::HashSet::new();
    // Ask the client to wipe the terminal (cells + images) before the next
    // frame. Set on attach and on every resize: the emulator may have reflowed
    // cells and kept image placements the incremental diff/delete stream no
    // longer knows about, so only a full re-baseline is trustworthy.
    let mut clear_pending = true;

    loop {
        // Apply all pending input.
        loop {
            match rx.try_recv() {
                Ok(ClientMsg::Resize { w, h }) => {
                    comp.resize(w, h);
                    core.apply(ClientMsg::Resize { w, h });
                    clear_pending = true;
                    // The client's wipe frees transmitted image data; re-send it.
                    sent_image_ids.clear();
                }
                Ok(msg) => core.apply(msg),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return, // client gone
            }
        }
        // Out-of-band `tuiui kill` / `tuiui reload` (control socket) — honored even
        // though this client is attached.
        apply_ctl(ctl, ctl_queue, core);
        if core.shutdown_requested() {
            return;
        }

        core.reap_dead();
        core.pump_app_events();
        core.refresh_app_graphics();
        core.sync_app_meta();
        core.pump_thumbnails();
        core.refresh_activity();
        let frame = core.build_frame();
        comp.composite(&frame.layers, frame.cursor);
        let changes = comp.diff();
        let flags = Flags {
            launcher_open: core.launcher_open(),
            spotlight_open: core.spotlight_open(),
            store_focused: core.focused_is_store(),
            settings_focused: core.focused_is_settings(),
            settings_editing: core.settings_editing(),
            dirpicker_open: core.dirpicker_open(),
            dirpicker_creating: core.dirpicker_creating(),
            help_open: core.help_open(),
            filemanager_focused: core.focused_is_filemanager(),
            filemanager_editing: core.filemanager_editing(),
            desktop_editing: core.desktop_editing(),
            renaming: core.renaming(),
            confirm_close: core.confirm_close_open(),
            power_editing: core.power_form_editing(),
            logs_focused: core.focused_is_logs(),
            activity_focused: core.focused_is_activity(),
            activity_confirming: core.activity_confirming(),
            detach: core.quit_requested(),
            reload: core.reload_requested(),
            app_area: core.app_mouse_area(),
        };
        // Send PNG bytes once per image id (base64); later frames carry only the
        // small placement list.
        let mut image_data = Vec::new();
        for p in &frame.images {
            if p.visible && sent_image_ids.insert(p.id) {
                if let Some(png) = core.image_png(p.id) {
                    image_data.push(crate::protocol::ImageBlob {
                        id: p.id,
                        png_base64: crate::kitty::b64(&png),
                    });
                }
            }
        }
        let mut buf = serde_json::to_vec(&FrameMsg {
            changes,
            cursor: frame.cursor,
            flags,
            images: frame.images.clone(),
            image_data,
            clear: clear_pending,
            switch_to: core.switch_spec(),
            clipboard: core.take_clipboard(),
        })
            .unwrap_or_default();
        buf.push(b'\n');
        if writer.write_all(&buf).is_err() {
            return; // client gone
        }
        comp.commit();
        clear_pending = false;

        if core.quit_requested() {
            if let Some(spec) = core.switch_spec() {
                crate::dbg_log(&format!("daemon: switch frame delivered (→ {} {})", spec.name, spec.host));
            }
            return; // detach (flag already delivered)
        }
        if core.reload_requested() {
            return; // reload flag delivered; daemon will restart, apphost untouched
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// If a control message has arrived, apply it to `core` (setting its
/// shutdown/reload flag, which the loops already act on) and return `true`.
fn apply_ctl(ctl: &Arc<AtomicU8>, queue: &CtlQueue, core: &mut SessionCore) -> bool {
    for msg in queue.lock().unwrap().drain(..) {
        core.apply(msg);
    }
    match ctl.swap(CTL_NONE, Ordering::SeqCst) {
        CTL_SHUTDOWN => {
            core.apply(ClientMsg::Shutdown);
            true
        }
        CTL_RELOAD => {
            core.apply(ClientMsg::Reload);
            true
        }
        _ => false,
    }
}

/// Accept control connections for the daemon's lifetime, flipping the shared flag
/// on `Shutdown` / `Reload`. Each connection is one short-lived message from the
/// `tuiui kill` / `tuiui reload` CLI.
fn serve_control(listener: UnixListener, ctl: Arc<AtomicU8>, queue: CtlQueue) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let mut r = BufReader::new(stream);
        let mut line = String::new();
        if r.read_line(&mut line).is_ok() {
            match serde_json::from_str::<ClientMsg>(line.trim()) {
                Ok(ClientMsg::Shutdown) => ctl.store(CTL_SHUTDOWN, Ordering::SeqCst),
                Ok(ClientMsg::Reload) => ctl.store(CTL_RELOAD, Ordering::SeqCst),
                // Any other message (e.g. `tuiui launch/tile/theme/msg` from the
                // CLI or the desktop assistant) queues for the render loop.
                Ok(msg) => {
                    crate::dbg_log(&format!("ctl: queued {msg:?}"));
                    queue.lock().unwrap().push(msg);
                }
                Err(_) => {}
            }
        }
    }
}
