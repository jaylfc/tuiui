//! The tuiui daemon: owns the [`SessionCore`] (windows + PTYs) and serves frames
//! to one attached client at a time over a Unix socket. Apps keep running while
//! detached, so reattaching restores the live session.

use crate::compositor::Compositor;
use crate::config::Config;
use crate::protocol::{socket_dir, socket_path, Flags, FrameMsg};
use crate::session::{ClientMsg, SessionCore};
use std::fs::Permissions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::time::Duration;

/// Run the daemon event loop until `tuiui kill` (or a fatal socket error).
pub fn run() -> std::io::Result<()> {
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

    let cfg = Config::load();
    crate::theme::set(&cfg.theme);
    let (w, h) = (100, 30); // provisional until the first client reports its size
    let mut core = SessionCore::new(w, h, cfg.clone());
    for app in &cfg.apps {
        core.apply(ClientMsg::Launch {
            name: app.name.clone(),
            command: app.command.clone(),
            args: app.args.clone(),
        });
    }
    let mut comp = Compositor::new(w, h);

    for stream in listener.incoming() {
        let stream = stream?;
        serve_client(&mut core, &mut comp, stream);
        core.clear_quit(); // detach was a detach, not a shutdown
        if core.shutdown_requested() {
            break;
        }
    }

    let _ = std::fs::remove_file(&path);
    core.shutdown();
    Ok(())
}

/// Serve one attached client until it detaches (socket closes) or asks to detach.
fn serve_client(core: &mut SessionCore, comp: &mut Compositor, stream: UnixStream) {
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

    loop {
        // Apply all pending input.
        loop {
            match rx.try_recv() {
                Ok(ClientMsg::Resize { w, h }) => {
                    comp.resize(w, h);
                    core.apply(ClientMsg::Resize { w, h });
                }
                Ok(msg) => core.apply(msg),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return, // client gone
            }
        }
        if core.shutdown_requested() {
            return;
        }

        core.reap_dead();
        let frame = core.build_frame();
        comp.composite(&frame.layers, frame.cursor);
        let changes = comp.diff();
        let flags = Flags {
            launcher_open: core.launcher_open(),
            spotlight_open: core.spotlight_open(),
            store_focused: core.focused_is_store(),
            settings_focused: core.focused_is_settings(),
            settings_editing: core.settings_editing(),
            detach: core.quit_requested(),
        };
        let mut buf = serde_json::to_vec(&FrameMsg { changes, cursor: frame.cursor, flags })
            .unwrap_or_default();
        buf.push(b'\n');
        if writer.write_all(&buf).is_err() {
            return; // client gone
        }
        comp.commit();

        if core.quit_requested() {
            return; // detach (flag already delivered)
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}
