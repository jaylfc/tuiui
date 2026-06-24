//! tuiui entry point — a thin dispatcher over the daemon/client split.
//!
//! - `tuiui`            ensure the daemon is running, then attach a client.
//! - `tuiui attach`     attach to an already-running daemon.
//! - `tuiui --daemon`   run the daemon (normally spawned automatically).
//! - `tuiui --compositor` run the Wayland compositor backend (stub - not yet implemented).
//! - `tuiui kill`       shut the daemon down (closing all windows).
//! - `tuiui reload`     restart the frontend only; apps keep running.
//! - `tuiui service …`  install|uninstall|status the per-user apphost service.
//! - `tuiui launch …`   open a new app window in the running desktop.
//! - `tuiui tile`       tile all windows into the configured grid.
//! - `tuiui theme <t>`  switch the theme.
//! - `tuiui msg '<j>'`  send a raw ClientMsg (the assistant's escape hatch).
//!
//! The daemon owns the windows and child processes and persists across client
//! detaches, so closing a client (or an SSH disconnect) leaves everything running.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::time::Duration;
use tuiui::protocol::socket_path;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let rest: Vec<String> = args.collect();
    match cmd.as_deref() {
        Some("--compositor") => tuiui::run_compositor(),
        Some("--daemon") => tuiui::daemon::run(),
        Some("--apphost") => tuiui::apphost::server::run(),
        Some("kill") => kill(),
        Some("kill-app") => kill_app(&rest),
        Some("ps") => ps(),
        Some("attach") => attach(false),
        Some("reload") => reload(),
        Some("service") => match rest.first().map(String::as_str) {
            Some("install") => tuiui::service::install(),
            Some("uninstall") => tuiui::service::uninstall(),
            Some("status") | None => tuiui::service::status(),
            Some(other) => {
                eprintln!("tuiui service: unknown '{other}' (try: install, uninstall, status)");
                Ok(())
            }
        },
        Some("launch") => {
            let mut rest = std::env::args().skip(2);
            let Some(command) = rest.next() else {
                eprintln!("usage: tuiui launch <command> [args…]");
                return Ok(());
            };
            let args: Vec<String> = rest.collect();
            let name = command.rsplit('/').next().unwrap_or(&command).to_string();
            ctl(&tuiui::session::ClientMsg::Launch { name, command, args })
        }
        Some("tile") => ctl(&tuiui::session::ClientMsg::TileAll),
        Some("theme") => match std::env::args().nth(2) {
            Some(name) => ctl(&tuiui::session::ClientMsg::SetTheme(name)),
            None => {
                eprintln!("usage: tuiui theme <{}>", tuiui::theme::PRESETS.join("|"));
                Ok(())
            }
        },
        Some("msg") => match std::env::args().nth(2) {
            Some(json) => match serde_json::from_str::<tuiui::session::ClientMsg>(&json) {
                Ok(msg) => ctl(&msg),
                Err(e) => {
                    eprintln!("tuiui msg: not a valid ClientMsg: {e}");
                    Ok(())
                }
            },
            None => {
                eprintln!("usage: tuiui msg '<ClientMsg JSON>'  e.g.  tuiui msg '\"MaximizeFocused\"'");
                Ok(())
            }
        },
        Some(other) => {
            eprintln!(
                "tuiui: unknown command '{other}' (try: attach, kill, kill-app, ps, reload, launch, tile, theme, msg, service, --daemon, --compositor)"
            );
            Ok(())
        }
        None => attach(true),
    }
}

/// Send one control message to the running daemon (used by `tuiui launch/tile/
/// theme/msg` — and by the desktop assistant to drive the UI).
fn ctl(msg: &tuiui::session::ClientMsg) -> std::io::Result<()> {
    if send_control(msg)? {
        println!("tuiui: sent");
    } else {
        eprintln!("tuiui: no daemon running (start it with `tuiui`)");
    }
    Ok(())
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

/// Format a `secs` duration as a short human-readable age ("12s", "3m", "1h12m",
/// "2d04h"). Used by `tuiui ps` and the in-app activity monitor.
fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 60 * 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else if secs < 24 * 60 * 60 {
        format!("{}h{:02}m", secs / 3600, (secs / 60) % 60)
    } else {
        format!("{}d{:02}h", secs / 86400, (secs / 3600) % 24)
    }
}

fn format_cmdline(cmd: &str, args: &[String]) -> String {
    if args.is_empty() {
        cmd.to_string()
    } else {
        format!("{} {}", cmd, args.join(" "))
    }
}

/// Connect to the apphost socket, send one `ListApps` request, and print the
/// result as a fixed-width table. Errors with a clean message if the apphost
/// isn't running.
fn ps() -> std::io::Result<()> {
    let path = tuiui::protocol::apphost_socket_path();
    let mut s = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("tuiui ps: no apphost running (start it with `tuiui`)");
            std::process::exit(1);
        }
    };
    use tuiui::apphost::proto::{send, HostEvt, HostReq};
    send(&mut s, &HostReq::ListApps)?;
    let mut r = std::io::BufReader::new(s);
    let evt: HostEvt = match tuiui::apphost::proto::recv(&mut r)? {
        Some(e) => e,
        None => {
            eprintln!("tuiui ps: apphost closed before replying");
            std::process::exit(1);
        }
    };
    let apps = match evt {
        HostEvt::AppList { apps } => apps,
        other => {
            eprintln!("tuiui ps: unexpected reply: {other:?}");
            std::process::exit(1);
        }
    };
    if apps.is_empty() {
        println!("(no apps running)");
        return Ok(());
    }
    println!(
        "{:>5}  {:>7}  {:<32}  {:>10}  {:>7}  STATE",
        "APPID", "PID", "CMD", "COLSxROWS", "AGE"
    );
    for a in &apps {
        let pid = a.pid.map(|p| p.to_string()).unwrap_or_else(|| "—".into());
        let cmdline = format_cmdline(&a.cmd, &a.args);
        let cmdline = if cmdline.chars().count() > 60 {
            let mut s: String = cmdline.chars().take(59).collect();
            s.push('…');
            s
        } else {
            cmdline
        };
        let state = if a.alive { "alive" } else { "dead" };
        println!(
            "{:>5}  {:>7}  {:<32}  {:>4}x{:<4}  {:>7}  {}",
            a.app,
            pid,
            cmdline,
            a.cols,
            a.rows,
            format_age(a.age_secs),
            state,
        );
    }
    Ok(())
}

/// `tuiui kill-app <id|all>` — send `HostReq::Kill` for one (or all dead)
/// hosted apps. `<id>` may be the apphost's numeric AppId. `all` kills every
/// currently-known app. Errors clearly when the apphost isn't running.
fn kill_app(args: &[String]) -> std::io::Result<()> {
    use tuiui::apphost::proto::{send, HostEvt, HostReq};
    let target = match args.first().map(String::as_str) {
        Some(t) => t,
        None => {
            eprintln!("usage: tuiui kill-app <id|all>");
            std::process::exit(2);
        }
    };
    let path = tuiui::protocol::apphost_socket_path();
    let mut s = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("tuiui kill-app: no apphost running (start it with `tuiui`)");
            std::process::exit(1);
        }
    };
    send(&mut s, &HostReq::ListApps)?;
    let mut r = std::io::BufReader::new(s);
    let evt: HostEvt = match tuiui::apphost::proto::recv(&mut r)? {
        Some(e) => e,
        None => {
            eprintln!("tuiui kill-app: apphost closed before replying");
            std::process::exit(1);
        }
    };
    let apps = match evt {
        HostEvt::AppList { apps } => apps,
        _ => {
            eprintln!("tuiui kill-app: unexpected reply");
            std::process::exit(1);
        }
    };
    let to_kill: Vec<u64> = if target == "all" {
        apps.iter().map(|a| a.app).collect()
    } else {
        let id: u64 = match target.parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("tuiui kill-app: '{target}' is not a numeric id or 'all'");
                std::process::exit(2);
            }
        };
        if !apps.iter().any(|a| a.app == id) {
            let known: Vec<String> = apps.iter().map(|a| a.app.to_string()).collect();
            eprintln!(
                "tuiui kill-app: no such app (have: {})",
                if known.is_empty() { "(none)".into() } else { known.join(", ") }
            );
            std::process::exit(1);
        }
        vec![id]
    };
    if to_kill.is_empty() {
        println!("tuiui kill-app: no apps to kill");
        return Ok(());
    }
    // Reconnect for each Kill — the previous connection's reader consumed
    // the ListApps reply and would also consume any further reply we don't
    // expect, so the cleanest path is one short-lived connection per Kill.
    for id in &to_kill {
        let mut s = match UnixStream::connect(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("tuiui kill-app: connect failed: {e}");
                std::process::exit(1);
            }
        };
        send(&mut s, &HostReq::Kill { app: *id })?;
    }
    if to_kill.len() == 1 {
        println!("tuiui kill-app: sent kill to app {}", to_kill[0]);
    } else {
        println!("tuiui kill-app: sent kill to {} app(s)", to_kill.len());
    }
    Ok(())
}
