//! tuiui entry point — a thin dispatcher over the daemon/client split.
//!
//! - `tuiui`            ensure the daemon is running, then attach a client.
//! - `tuiui attach`     attach to an already-running daemon.
//! - `tuiui --daemon`   run the daemon (normally spawned automatically).
//! - `tuiui kill`       shut the daemon down (closing all windows).
//! - `tuiui reload`     restart the frontend only; apps keep running.
//! - `tuiui service …`  install|uninstall|status the per-user apphost service.
//!
//! The daemon owns the windows and child processes and persists across client
//! detaches, so closing a client (or an SSH disconnect) leaves everything running.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::time::Duration;
use tuiui::protocol::socket_path;

fn main() -> std::io::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--daemon") => tuiui::daemon::run(),
        Some("--apphost") => tuiui::apphost::server::run(),
        Some("kill") => kill(),
        Some("attach") => attach(false),
        Some("reload") => reload(),
        Some("service") => match std::env::args().nth(2).as_deref() {
            Some("install") => tuiui::service::install(),
            Some("uninstall") => tuiui::service::uninstall(),
            Some("status") | None => tuiui::service::status(),
            Some(other) => {
                eprintln!("tuiui service: unknown '{other}' (try: install, uninstall, status)");
                Ok(())
            }
        },
        Some(other) => {
            eprintln!(
                "tuiui: unknown command '{other}' (try: attach, kill, reload, service, --daemon)"
            );
            Ok(())
        }
        None => attach(true),
    }
}

/// Connect to the daemon and run a client. If `spawn_if_missing`, start a daemon
/// first when none is running. On a `ClientExit::Reload`, waits for the old
/// daemon socket to drop, then loops to spawn/connect a fresh daemon.
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

/// Spawn the daemon detached into its own process group so it survives the
/// client exiting (and SSH disconnects).
fn spawn_daemon() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg("--daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(())
}

/// Send one newline-framed message, then keep the connection open reading to EOF
/// so the recipient processes the message before our socket closes.
///
/// Without this, a control command (`kill`/`reload`) that writes-then-exits can
/// race the daemon: the daemon hits a broken pipe on its next frame *write* and
/// bails before its reader thread has delivered our queued message. Holding the
/// connection open (until the daemon acts and closes its end) removes the race.
fn send_and_drain(stream: &mut UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Read;
    stream.write_all(bytes)?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut sink = [0u8; 256];
    loop {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    Ok(())
}

/// Tell a running daemon to reload its frontend (apps keep running via the
/// apphost). An attached client reconnects on its own.
fn reload() -> std::io::Result<()> {
    match UnixStream::connect(socket_path()) {
        Ok(mut stream) => {
            let mut buf = serde_json::to_vec(&tuiui::session::ClientMsg::Reload)
                .map_err(std::io::Error::other)?;
            buf.push(b'\n');
            send_and_drain(&mut stream, &buf)?;
            println!("tuiui: reload requested");
        }
        Err(_) => println!("tuiui: no daemon running"),
    }
    Ok(())
}

/// Tell a running daemon to shut down.
fn kill() -> std::io::Result<()> {
    match UnixStream::connect(socket_path()) {
        Ok(mut stream) => {
            let mut buf = serde_json::to_vec(&tuiui::session::ClientMsg::Shutdown)
                .map_err(std::io::Error::other)?;
            buf.push(b'\n');
            send_and_drain(&mut stream, &buf)?;
            println!("tuiui: shutdown requested");
        }
        Err(_) => println!("tuiui: no daemon running"),
    }
    // Also stop the apphost (it outlives the frontend by design).
    if let Ok(mut s) = UnixStream::connect(tuiui::protocol::apphost_socket_path()) {
        let req = tuiui::apphost::proto::HostReq::Shutdown;
        if let Ok(mut buf) = serde_json::to_vec(&req) {
            buf.push(b'\n');
            let _ = send_and_drain(&mut s, &buf);
        }
    }
    Ok(())
}
