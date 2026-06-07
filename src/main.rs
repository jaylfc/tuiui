//! tuiui entry point — a thin dispatcher over the daemon/client split.
//!
//! - `tuiui`            ensure the daemon is running, then attach a client.
//! - `tuiui attach`     attach to an already-running daemon.
//! - `tuiui --daemon`   run the daemon (normally spawned automatically).
//! - `tuiui kill`       shut the daemon down (closing all windows).
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
        Some(other) => {
            eprintln!("tuiui: unknown command '{other}' (try: attach, kill, --daemon)");
            Ok(())
        }
        None => attach(true),
    }
}

/// Connect to the daemon and run a client. If `spawn_if_missing`, start a daemon
/// first when none is running.
fn attach(spawn_if_missing: bool) -> std::io::Result<()> {
    let path = socket_path();
    if UnixStream::connect(&path).is_err() {
        if !spawn_if_missing {
            eprintln!("tuiui: no daemon running (start it with `tuiui`)");
            return Ok(());
        }
        spawn_daemon()?;
        // Wait for the socket to come up.
        let mut ready = false;
        for _ in 0..100 {
            if UnixStream::connect(&path).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !ready {
            eprintln!("tuiui: daemon failed to start");
            return Ok(());
        }
    }
    let stream = UnixStream::connect(&path)?;
    tuiui::client::run(stream)
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

/// Tell a running daemon to shut down.
fn kill() -> std::io::Result<()> {
    match UnixStream::connect(socket_path()) {
        Ok(mut stream) => {
            let mut buf = serde_json::to_vec(&tuiui::session::ClientMsg::Shutdown)
                .map_err(std::io::Error::other)?;
            buf.push(b'\n');
            stream.write_all(&buf)?;
            println!("tuiui: shutdown requested");
        }
        Err(_) => println!("tuiui: no daemon running"),
    }
    // Also stop the apphost (it outlives the frontend by design).
    if let Ok(mut s) = UnixStream::connect(tuiui::protocol::apphost_socket_path()) {
        let req = tuiui::apphost::proto::HostReq::Shutdown;
        if let Ok(mut buf) = serde_json::to_vec(&req) {
            buf.push(b'\n');
            let _ = s.write_all(&buf);
        }
    }
    Ok(())
}
