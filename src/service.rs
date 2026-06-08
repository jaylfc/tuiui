//! Install `tuiui --apphost` as a per-user service (launchd / systemd --user /
//! ~/.profile fallback) so the app host auto-starts and restarts on crash.

use std::io;
use std::path::PathBuf;
use std::process::Command;

const LAUNCHD_LABEL: &str = "co.uk.janlabs.tuiui-apphost";
const SYSTEMD_UNIT: &str = "tuiui-apphost.service";
const PROFILE_START: &str = "# >>> tuiui apphost >>>";
const PROFILE_END: &str = "# <<< tuiui apphost <<<";

/// Which backend this platform uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Launchd,
    Systemd,
    Profile,
    Unsupported,
}

/// Environment to bake into the service so the apphost can find/launch apps
/// (a service's own env is sparse). Captured from the install-time shell.
fn service_env() -> Vec<(String, String)> {
    ["PATH", "HOME", "SHELL", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// launchd LaunchAgent plist (macOS).
pub fn launchd_plist(label: &str, exe: &str, env: &[(String, String)]) -> String {
    let mut env_xml = String::new();
    for (k, v) in env {
        env_xml.push_str(&format!(
            "    <key>{}</key><string>{}</string>\n",
            k,
            xml_escape(v)
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n  \
<key>Label</key><string>{label}</string>\n  \
<key>ProgramArguments</key>\n  <array>\n    <string>{exe}</string>\n    <string>--apphost</string>\n  </array>\n  \
<key>RunAtLoad</key><true/>\n  \
<key>KeepAlive</key>\n  <dict><key>SuccessfulExit</key><false/></dict>\n  \
<key>ProcessType</key><string>Background</string>\n  \
<key>EnvironmentVariables</key>\n  <dict>\n{env_xml}  </dict>\n\
</dict>\n</plist>\n",
        exe = xml_escape(exe)
    )
}

/// systemd user unit (Linux / WSL with systemd).
pub fn systemd_unit(exe: &str, env: &[(String, String)]) -> String {
    let mut env_lines = String::new();
    for (k, v) in env {
        env_lines.push_str(&format!("Environment={k}={v}\n"));
    }
    format!(
        "[Unit]\nDescription=tuiui apphost (keeps your terminal apps alive)\nAfter=default.target\n\n\
[Service]\nType=simple\nExecStart={exe} --apphost\nRestart=on-failure\nRestartSec=2\n{env_lines}\n\
[Install]\nWantedBy=default.target\n"
    )
}

/// Guarded ~/.profile block (no-systemd fallback). Idempotent via the markers.
pub fn profile_block(exe: &str, sock: &str) -> String {
    format!(
        "{PROFILE_START}\n\
# Auto-start the tuiui apphost in the background (no systemd available).\n\
if command -v {exe} >/dev/null 2>&1 && [ ! -S \"{sock}\" ]; then\n  \
( {exe} --apphost >/dev/null 2>&1 & )\nfi\n\
{PROFILE_END}\n"
    )
}

fn current_exe() -> io::Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().into_owned())
}

/// Whether a usable systemd `--user` instance is available.
fn has_user_systemd() -> bool {
    match Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
    {
        Ok(out) => {
            out.status.success()
                && !String::from_utf8_lossy(&out.stderr).contains("Failed to connect")
        }
        Err(_) => false,
    }
}

/// The backend that would be used on this machine.
pub fn backend() -> Backend {
    if cfg!(target_os = "macos") {
        Backend::Launchd
    } else if cfg!(target_os = "linux") {
        if has_user_systemd() {
            Backend::Systemd
        } else {
            Backend::Profile
        }
    } else {
        Backend::Unsupported
    }
}

fn launchd_plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library/LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist"))
    })
}
fn systemd_unit_path() -> Option<PathBuf> {
    dirs::config_dir().map(|c| c.join("systemd/user").join(SYSTEMD_UNIT))
}
fn profile_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".profile"))
}

/// Install + start the per-user apphost service for this platform.
pub fn install() -> io::Result<()> {
    let exe = current_exe()?;
    let env = service_env();
    match backend() {
        Backend::Launchd => {
            let path = launchd_plist_path().ok_or_else(|| io::Error::other("no home dir"))?;
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(&path, launchd_plist(LAUNCHD_LABEL, &exe, &env))?;
            let uid = unsafe { libc::getuid() };
            let domain = format!("gui/{uid}");
            let ps = path.to_string_lossy();
            // Replace any existing instance.
            let _ = Command::new("launchctl")
                .args(["bootout", &domain, &ps])
                .status();
            let st = Command::new("launchctl")
                .args(["bootstrap", &domain, &ps])
                .status()?;
            if !st.success() {
                // Older macOS fallback.
                let _ = Command::new("launchctl").args(["load", "-w", &ps]).status();
            }
            println!("tuiui: apphost service installed (launchd: {LAUNCHD_LABEL}).");
            Ok(())
        }
        Backend::Systemd => {
            let path = systemd_unit_path().ok_or_else(|| io::Error::other("no config dir"))?;
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(&path, systemd_unit(&exe, &env))?;
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            let st = Command::new("systemctl")
                .args(["--user", "enable", "--now", SYSTEMD_UNIT])
                .status()?;
            if !st.success() {
                eprintln!("tuiui: `systemctl --user enable --now {SYSTEMD_UNIT}` failed.");
            }
            println!("tuiui: apphost service installed (systemd --user: {SYSTEMD_UNIT}).");
            println!("tuiui: tip — `loginctl enable-linger $USER` keeps it running across logout.");
            Ok(())
        }
        Backend::Profile => {
            let path = profile_path().ok_or_else(|| io::Error::other("no home dir"))?;
            let sock = crate::protocol::apphost_socket_path()
                .to_string_lossy()
                .into_owned();
            let mut text = std::fs::read_to_string(&path).unwrap_or_default();
            text = strip_profile_block(&text);
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&profile_block(&exe, &sock));
            std::fs::write(&path, text)?;
            // Start it now too (best-effort).
            let _ = Command::new(&exe).arg("--apphost").spawn();
            println!(
                "tuiui: apphost auto-start added to {} (no systemd detected).",
                path.display()
            );
            Ok(())
        }
        Backend::Unsupported => {
            eprintln!("tuiui: service install is not supported on this platform.");
            Ok(())
        }
    }
}

/// Remove the per-user apphost service.
pub fn uninstall() -> io::Result<()> {
    match backend() {
        Backend::Launchd => {
            if let Some(path) = launchd_plist_path() {
                let uid = unsafe { libc::getuid() };
                let _ = Command::new("launchctl")
                    .args(["bootout", &format!("gui/{uid}"), &path.to_string_lossy()])
                    .status();
                let _ = std::fs::remove_file(&path);
            }
            println!("tuiui: apphost service removed (launchd).");
            Ok(())
        }
        Backend::Systemd => {
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", SYSTEMD_UNIT])
                .status();
            if let Some(path) = systemd_unit_path() {
                let _ = std::fs::remove_file(&path);
            }
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            println!("tuiui: apphost service removed (systemd --user).");
            Ok(())
        }
        Backend::Profile => {
            if let Some(path) = profile_path() {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    std::fs::write(&path, strip_profile_block(&text))?;
                }
            }
            println!("tuiui: apphost auto-start removed from ~/.profile.");
            Ok(())
        }
        Backend::Unsupported => Ok(()),
    }
}

/// Print which backend is used and whether the apphost is currently running.
pub fn status() -> io::Result<()> {
    let running =
        std::os::unix::net::UnixStream::connect(crate::protocol::apphost_socket_path()).is_ok();
    println!("tuiui apphost service:");
    println!("  backend:  {:?}", backend());
    println!(
        "  running:  {}",
        if running {
            "yes (socket is up)"
        } else {
            "no"
        }
    );
    match backend() {
        Backend::Launchd => {
            if let Some(p) = launchd_plist_path() {
                println!("  installed: {}", p.exists());
                println!("  plist:    {}", p.display());
            }
        }
        Backend::Systemd => {
            if let Some(p) = systemd_unit_path() {
                println!("  unit:     {} (installed: {})", p.display(), p.exists());
            }
            let _ = Command::new("systemctl")
                .args(["--user", "--no-pager", "status", SYSTEMD_UNIT])
                .status();
        }
        Backend::Profile => {
            if let Some(p) = profile_path() {
                let has = std::fs::read_to_string(&p)
                    .map(|t| t.contains(PROFILE_START))
                    .unwrap_or(false);
                println!("  ~/.profile hook installed: {has}");
            }
        }
        Backend::Unsupported => {}
    }
    Ok(())
}

/// Remove the guarded tuiui block from profile text (idempotent).
fn strip_profile_block(text: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.trim() == PROFILE_START {
            skipping = true;
            continue;
        }
        if line.trim() == PROFILE_END {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn env() -> Vec<(String, String)> {
        vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), "/home/x".into()),
        ]
    }

    #[test]
    fn launchd_plist_has_apphost_keepalive_and_env() {
        let p = launchd_plist(LAUNCHD_LABEL, "/usr/local/bin/tuiui", &env());
        assert!(p.contains("<string>--apphost</string>"));
        assert!(p.contains(LAUNCHD_LABEL));
        assert!(p.contains("<key>RunAtLoad</key><true/>"));
        assert!(p.contains("<key>SuccessfulExit</key><false/>"));
        assert!(p.contains("<key>PATH</key><string>/usr/bin:/bin</string>"));
    }

    #[test]
    fn systemd_unit_has_execstart_restart_and_env() {
        let u = systemd_unit("/usr/local/bin/tuiui", &env());
        assert!(u.contains("ExecStart=/usr/local/bin/tuiui --apphost"));
        assert!(u.contains("Restart=on-failure"));
        assert!(u.contains("Environment=PATH=/usr/bin:/bin"));
        assert!(u.contains("WantedBy=default.target"));
    }

    #[test]
    fn profile_block_is_guarded_and_starts_apphost() {
        let b = profile_block("/usr/local/bin/tuiui", "/run/x/apphost.sock");
        assert!(b.starts_with(PROFILE_START));
        assert!(b.trim_end().ends_with(PROFILE_END));
        assert!(b.contains("--apphost"));
        assert!(b.contains("apphost.sock"));
    }

    #[test]
    fn strip_profile_block_is_idempotent() {
        let base = "export FOO=1\n";
        let with = format!("{base}{}", profile_block("/x/tuiui", "/s.sock"));
        assert_eq!(strip_profile_block(&with), base);
        assert_eq!(strip_profile_block(base), base); // no-op when absent
    }
}
