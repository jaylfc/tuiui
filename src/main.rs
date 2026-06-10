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
            tuiui::client::ClientExit::Switch(spec) => {
                // Run ssh (and any first-time setup) in the real terminal; when
                // the remote session ends, loop to re-attach to the local
                // daemon — its apps kept running the whole time.
                run_switch(&spec);
                continue;
            }
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

/// Switch to a remote system: run the generated ssh/setup script with inherited
/// stdio so password and host-key prompts are fully interactive. The setup
/// password (if any) is passed via the `SSHPASS` env var — never on a command
/// line and never written anywhere.
fn run_switch(spec: &tuiui::systems::SwitchSpec) {
    println!(
        "tuiui: switching to {} ({}{}){}…",
        spec.name,
        spec.host,
        spec.port.map(|p| format!(":{p}")).unwrap_or_default(),
        if spec.setup { " — first-time setup" } else { "" },
    );
    let script = tuiui::systems::switch_script(spec);
    // With TUIUI_DEBUG set, show exactly what will run (the password is never
    // embedded in the script) and mirror it to ~/tuiui-debug.log.
    if std::env::var_os("TUIUI_DEBUG").is_some() {
        eprintln!("tuiui: switch script:\n{script}");
    }
    tuiui::dbg_log(&format!(
        "switch: name={} host={} port={:?} theme={:?} setup={} password={}",
        spec.name, spec.host, spec.port, spec.theme, spec.setup,
        if spec.password.is_some() { "yes (via SSHPASS)" } else { "no" },
    ));
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&script);
    if let Some(pw) = &spec.password {
        cmd.env("SSHPASS", pw);
    }
    match cmd.status() {
        Ok(status) if status.success() => {
            tuiui::dbg_log("switch: remote session ended cleanly");
            println!("tuiui: remote session ended — back to this machine.");
        }
        Ok(status) => {
            tuiui::dbg_log(&format!("switch: ended with {status}"));
            eprintln!("tuiui: switch to {} ended with {status} — back to this machine.", spec.name);
            eprintln!("tuiui: (re-run with TUIUI_DEBUG=1 to see the exact script; log: ~/tuiui-debug.log)");
        }
        Err(e) => {
            tuiui::dbg_log(&format!("switch: could not run sh/ssh: {e}"));
            eprintln!("tuiui: could not run ssh: {e}");
        }
    }
    // Give the user a beat to read any setup/ssh output before tuiui's
    // alternate screen swallows it on re-attach.
    if spec.setup {
        println!("tuiui: re-attaching to the local session in 3s… (Ctrl-C to stay in the shell)");
        std::thread::sleep(Duration::from_secs(3));
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

/// Send a control message (Shutdown/Reload) to the daemon. Returns whether a
/// daemon was reachable.
///
/// The daemon serves its single attached client serially, so a control message
/// on the main socket would queue behind that client and never be read. We send
/// to the out-of-band control socket (read by the daemon's control thread even
/// while attached), and also poke the main socket so an *unattached* daemon —
/// blocked in `accept()` — wakes and re-checks the control flag (this also covers
/// an older daemon that predates the control socket).
fn send_control(msg: &tuiui::session::ClientMsg) -> std::io::Result<bool> {
    let mut buf = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    buf.push(b'\n');
    let mut reached = false;
    if let Ok(mut s) = UnixStream::connect(tuiui::protocol::daemon_ctl_path()) {
        let _ = send_and_drain(&mut s, &buf);
        reached = true;
    }
    if let Ok(mut s) = UnixStream::connect(socket_path()) {
        // Best-effort wake poke: write and close without draining so we don't
        // block when the daemon is busy serving an attached client.
        let _ = s.write_all(&buf);
        let _ = s.shutdown(std::net::Shutdown::Write);
        reached = true;
    }
    Ok(reached)
}

/// Tell a running daemon to reload its frontend (apps keep running via the
/// apphost). An attached client reconnects on its own.
fn reload() -> std::io::Result<()> {
    if send_control(&tuiui::session::ClientMsg::Reload)? {
        println!("tuiui: reload requested");
    } else {
        println!("tuiui: no daemon running");
    }
    Ok(())
}

/// Tell a running daemon to shut down, and stop the apphost.
fn kill() -> std::io::Result<()> {
    if send_control(&tuiui::session::ClientMsg::Shutdown)? {
        println!("tuiui: shutdown requested");
    } else {
        println!("tuiui: no daemon running");
    }
    // Also stop the apphost directly. When a daemon is attached it shuts the
    // apphost down in-band; this covers the case where the apphost is running
    // with no daemon (e.g. the per-user service apphost on its own).
    if let Ok(mut s) = UnixStream::connect(tuiui::protocol::apphost_socket_path()) {
        let req = tuiui::apphost::proto::HostReq::Shutdown;
        if let Ok(mut buf) = serde_json::to_vec(&req) {
            buf.push(b'\n');
            let _ = send_and_drain(&mut s, &buf);
        }
    }
    Ok(())
}
