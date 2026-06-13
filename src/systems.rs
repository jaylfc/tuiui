//! Saved systems for the power-menu "Systems" switcher: a list of remote
//! machines (ssh target + optional theme) persisted next to the config, plus
//! the shell scripts the client runs to set up and attach to one.
//!
//! Switching works with the daemon/client split: the daemon's session decides
//! *what* to switch to and ships a [`SwitchSpec`] to the client in a frame; the
//! client exits back to `main`, which owns the real terminal and runs `ssh -t`
//! there (interactive password/host-key prompts work). When the remote session
//! ends, `main` re-attaches to the local daemon — apps were never stopped.
//!
//! Passwords are used only for the one-time key transfer (`ssh-copy-id`, via
//! `sshpass` when available) and are never written to disk.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One saved remote machine.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RemoteSystem {
    /// Display name in the Systems menu.
    pub name: String,
    /// SSH target: `user@host` (or just `host`).
    pub host: String,
    /// SSH port, when not 22.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Theme applied on the remote session (`None` = the remote's own theme).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

/// What the client should switch to, shipped daemon → client inside a frame.
/// `setup` runs the first-time flow (key transfer + remote install) before
/// attaching; `password` (never persisted) feeds `sshpass` for that flow only.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SwitchSpec {
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub setup: bool,
    #[serde(default)]
    pub password: Option<String>,
}

impl SwitchSpec {
    /// A plain switch to an already-set-up system.
    pub fn connect(s: &RemoteSystem) -> Self {
        SwitchSpec {
            name: s.name.clone(),
            host: s.host.clone(),
            port: s.port,
            theme: s.theme.clone(),
            setup: false,
            password: None,
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
struct SystemsFile {
    #[serde(default)]
    systems: Vec<RemoteSystem>,
}

/// `$XDG_CONFIG_HOME/tuiui/systems.toml` (or `~/.config/tuiui/systems.toml`),
/// mirroring the config path convention on every platform.
fn systems_file_path(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    let base = xdg_config_home
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".config")))?;
    Some(base.join("tuiui").join("systems.toml"))
}

fn default_path() -> Option<PathBuf> {
    systems_file_path(std::env::var_os("XDG_CONFIG_HOME"), dirs::home_dir())
}

/// Load the saved systems (empty on a missing/corrupt file).
pub fn load() -> Vec<RemoteSystem> {
    let Some(p) = default_path() else { return Vec::new() };
    let systems = std::fs::read_to_string(&p)
        .ok()
        .and_then(|text| toml::from_str::<SystemsFile>(&text).ok())
        .map(|f| f.systems)
        .unwrap_or_default();
    crate::dbg_log(&format!("systems: loaded {} from {}", systems.len(), p.display()));
    systems
}

/// Persist the systems list (best-effort; the menu state is the source of truth).
pub fn save(systems: &[RemoteSystem]) -> std::io::Result<()> {
    let path = default_path().ok_or_else(|| std::io::Error::other("no config directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = SystemsFile { systems: systems.to_vec() };
    let toml = toml::to_string_pretty(&file).map_err(std::io::Error::other)?;
    std::fs::write(path, toml)
}

/// Split a `user@host:port` target into (`user@host`, port). A suffix that is
/// not a valid port stays part of the host.
pub fn parse_target(s: &str) -> (String, Option<u16>) {
    if let Some((host, port)) = s.rsplit_once(':') {
        if let Ok(p) = port.parse::<u16>() {
            return (host.to_string(), Some(p));
        }
    }
    (s.to_string(), None)
}

/// Single-quote `s` for POSIX sh (handles embedded single quotes).
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The command run on the remote end of `ssh -t`: guard against a missing
/// terminfo entry for the local terminal (e.g. `xterm-ghostty` on a stock
/// Ubuntu — ncurses apps would die with "cannot find terminfo entry"), extend
/// PATH to the places the installer puts the binary, carry the per-system
/// theme, and start tuiui. The setup flow also installs the real terminfo
/// (see [`switch_script`]); this guard covers systems set up before that, or
/// by hand.
fn remote_run_command(theme: Option<&str>) -> String {
    let theme_env = theme
        .map(|t| format!("TUIUI_THEME={} ", sh_quote(t)))
        .unwrap_or_default();
    format!(
        "if ! infocmp \"$TERM\" >/dev/null 2>&1; then export TERM=xterm-256color; fi; \
PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$PATH\" {theme_env}tuiui"
    )
}

/// `-p <port>` args for ssh/ssh-copy-id, or empty for the default port.
fn port_flag(port: Option<u16>) -> String {
    port.map(|p| format!("-p {p} ")).unwrap_or_default()
}

/// `-P <port>` args for scp (scp's port flag is capitalized).
fn scp_port_flag(port: Option<u16>) -> String {
    port.map(|p| format!("-P {p} ")).unwrap_or_default()
}

/// The local shell script the client runs to switch to `spec`. For a plain
/// switch it just attaches; with `setup` it first transfers a key and installs
/// tuiui (mac / linux / WSL2) + gpm (Linux console mouse) on the remote.
/// The password — if any — is delivered via the `SSHPASS` env var by the
/// spawner, never embedded in the script.
pub fn switch_script(spec: &SwitchSpec) -> String {
    let target = sh_quote(&spec.host);
    let pf = port_flag(spec.port);
    let run = sh_quote(&remote_run_command(spec.theme.as_deref()));
    let mut s = String::new();
    if spec.setup {
        let name = sh_quote(&spec.name);
        let install = sh_quote(&format!(
            "curl -fsSL {repo_raw}/main/install.sh | TUIUI_INSTALL_DEPS=1 sh || \
{{ command -v cargo >/dev/null 2>&1 && cargo install --git {repo}; }}",
            repo = crate::REPO_URL,
            repo_raw = "https://raw.githubusercontent.com/jaylfc/tuiui",
        ));
        let gpm = sh_quote(
            "if [ \"$(uname -s)\" = Linux ] && ! command -v gpm >/dev/null 2>&1; then \
echo 'tuiui: installing gpm (console mouse)…'; \
sudo apt-get install -y gpm 2>/dev/null || sudo dnf install -y gpm 2>/dev/null || \
sudo pacman -S --noconfirm gpm 2>/dev/null || echo 'tuiui: could not install gpm automatically'; \
command -v systemctl >/dev/null 2>&1 && sudo systemctl enable --now gpm 2>/dev/null; fi; true",
        );
        s.push_str(&format!("echo \"── tuiui: setting up\" {name} \"──\"\n"));
        // 1. Make sure we have a key to transfer.
        s.push_str(
            "mkdir -p \"$HOME/.ssh\"\n\
if [ ! -f \"$HOME/.ssh/id_ed25519\" ] && [ ! -f \"$HOME/.ssh/id_rsa\" ]; then\n\
  echo 'tuiui: generating an SSH key (~/.ssh/id_ed25519)…'\n\
  ssh-keygen -t ed25519 -N '' -f \"$HOME/.ssh/id_ed25519\"\n\
fi\n",
        );
        let spf = scp_port_flag(spec.port);
        // 2. Transfer it (sshpass automates the prompt when a password was given).
        //    A failed transfer aborts loudly — every later step would just hang
        //    on the same auth problem.
        s.push_str(&format!(
            "echo \"tuiui: [1/5] copying SSH key to\" {target} \"…\"\n\
if [ -n \"${{SSHPASS:-}}\" ] && command -v sshpass >/dev/null 2>&1; then\n\
  sshpass -e ssh-copy-id {pf}{target} || {{ echo 'tuiui: ERROR — key transfer failed (wrong host/password?)'; exit 1; }}\n\
else\n\
  [ -n \"${{SSHPASS:-}}\" ] && echo 'tuiui: sshpass not installed — enter the password when prompted.'\n\
  ssh-copy-id {pf}{target} || {{ echo 'tuiui: ERROR — key transfer failed (wrong host/password?)'; exit 1; }}\n\
fi\n\
echo 'tuiui: key transfer OK'\n",
        ));
        // 3. Teach the remote this terminal's terminfo. Modern terminals
        //    (Ghostty: xterm-ghostty; Kitty: xterm-kitty) aren't in older
        //    distros' ncurses databases, so without this every curses app on
        //    the remote dies with "cannot find terminfo entry for '$TERM'".
        s.push_str(&format!(
            "echo \"tuiui: [2/5] teaching the remote your terminal type ($TERM)…\"\n\
if command -v infocmp >/dev/null 2>&1 && infocmp -x \"$TERM\" >/dev/null 2>&1; then\n\
  infocmp -x \"$TERM\" | ssh {pf}{target} 'mkdir -p ~/.terminfo && tic -x -' \\\n\
    && echo 'tuiui: terminfo installed' \\\n\
    || echo 'tuiui: terminfo copy failed — remote will fall back to xterm-256color'\n\
else\n\
  echo 'tuiui: no local terminfo for '\"$TERM\"' — remote will fall back to xterm-256color'\n\
fi\n",
        ));
        // 4. Install tuiui + gpm on the remote.
        s.push_str(&format!(
            "echo 'tuiui: [3/5] installing tuiui on the remote (mac/linux/wsl2)…'\n\
ssh -t {pf}{target} {install} || {{ echo 'tuiui: ERROR — remote install failed (output above)'; exit 1; }}\n\
echo 'tuiui: [4/5] checking gpm (Linux console mouse)…'\n\
ssh -t {pf}{target} {gpm}\n",
        ));
        // 5. Sync the saved-systems list so the assistant (and the Systems
        //    menu) on the remote knows the same machines as here.
        s.push_str(&format!(
            "echo 'tuiui: [5/5] syncing your saved systems — connecting…'\n\
if [ -f \"$HOME/.config/tuiui/systems.toml\" ]; then\n\
  ssh {pf}{target} 'mkdir -p ~/.config/tuiui' \\\n\
    && scp {spf}\"$HOME/.config/tuiui/systems.toml\" {target}:.config/tuiui/ >/dev/null \\\n\
    || echo 'tuiui: systems sync failed (non-fatal)'\n\
fi\n",
        ));
    }
    s.push_str(&format!("exec ssh -t {pf}{target} {run}\n"));
    s
}

/// The local shell script that removes *this* machine's public key(s) from a
/// remote's `~/.ssh/authorized_keys` — the inverse of the `ssh-copy-id` that
/// [`switch_script`]'s setup performs. It is best-effort and non-interactive
/// (`BatchMode=yes`, short `ConnectTimeout`): it authenticates with the very key
/// it is revoking, filters that exact full line out of `authorized_keys`
/// (keeping a `.tuiui.bak`), and leaves any other keys untouched. A missing
/// local key or an unreachable host is a quiet no-op, never a hang; any real
/// remote-side failure exits non-zero so the caller can log it truthfully.
pub fn revoke_script(host: &str, port: Option<u16>) -> String {
    let target = sh_quote(host);
    let pf = port_flag(port);
    // Remote side: filter our piped public keys out of authorized_keys, dropping
    // only lines that match one *exactly* (`grep -vxF`). Both the scratch file
    // and the result live *inside* `~/.ssh`, never `/tmp`: a file `mv`d in from
    // `/tmp` can carry a `tmp_t` SELinux label (or cross a filesystem) and make
    // sshd refuse the whole file, silently locking the user out of every key.
    // We `chmod 600` before the swap (shell redirection would leave it
    // world-readable) and back up first, aborting if the backup can't be made.
    // grep rc 0/1 are both success (1 = ours was the only line → file becomes
    // empty); rc >= 2 is a real error. Every failure exits non-zero so the
    // caller logs an honest failure instead of a misleading "done".
    let remote = sh_quote(
        "ak=\"$HOME/.ssh/authorized_keys\"; [ -f \"$ak\" ] || exit 0; \
in=\"$ak.tuiui.in\"; out=\"$ak.tuiui.tmp\"; \
cat > \"$in\" || { rm -f \"$in\"; exit 2; }; \
cp \"$ak\" \"$ak.tuiui.bak\" || { echo 'tuiui: backup failed'; rm -f \"$in\"; exit 2; }; \
grep -vxFf \"$in\" \"$ak\" > \"$out\"; rc=$?; \
if [ \"$rc\" -le 1 ]; then chmod 600 \"$out\"; mv \"$out\" \"$ak\" || { rm -f \"$in\" \"$out\"; exit 2; }; \
else rm -f \"$in\" \"$out\"; exit \"$rc\"; fi; \
rm -f \"$in\"; exit 0",
    );
    // Local side: collect every *readable* local public identity — all
    // `~/.ssh/*.pub` plus any agent-held keys — because `ssh-copy-id` (run
    // without `-i` at setup) may have installed any of them, not just the
    // ed25519/rsa defaults. Bail cleanly when there are none, else pipe them to
    // the remote filter and propagate its exit status.
    format!(
        "kf=$(mktemp) || exit 1; \
{{ for p in \"$HOME\"/.ssh/*.pub; do [ -r \"$p\" ] && cat \"$p\"; done; \
command -v ssh-add >/dev/null 2>&1 && ssh-add -L 2>/dev/null; }} \
| grep -v '^$' | sort -u > \"$kf\"; \
if [ ! -s \"$kf\" ]; then echo 'tuiui: no local SSH public key to revoke'; rm -f \"$kf\"; exit 0; fi; \
ssh -o BatchMode=yes -o ConnectTimeout=8 {pf}{target} {remote} < \"$kf\"; rc=$?; rm -f \"$kf\"; exit \"$rc\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_splits_port() {
        assert_eq!(parse_target("pi@10.0.0.2:2222"), ("pi@10.0.0.2".into(), Some(2222)));
        assert_eq!(parse_target("pi@10.0.0.2"), ("pi@10.0.0.2".into(), None));
        assert_eq!(parse_target("host:notaport"), ("host:notaport".into(), None));
    }

    #[test]
    fn sh_quote_handles_quotes() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn plain_switch_script_is_one_exec_ssh() {
        let spec = SwitchSpec {
            name: "pi".into(),
            host: "pi@10.0.0.2".into(),
            port: Some(2222),
            theme: Some("nord".into()),
            setup: false,
            password: None,
        };
        let s = switch_script(&spec);
        assert!(s.starts_with("exec ssh -t -p 2222 'pi@10.0.0.2'"), "{s}");
        assert!(s.contains("TUIUI_THEME=") && s.contains("nord"), "theme rides over ssh: {s}");
        assert!(s.contains("infocmp"), "guards against missing remote terminfo: {s}");
        assert!(s.contains("TERM=xterm-256color"), "falls back to a universal TERM: {s}");
        assert!(!s.contains("ssh-copy-id"), "no setup steps on a plain switch");
    }

    #[test]
    fn revoke_script_filters_our_key_best_effort() {
        let s = revoke_script("pi@10.0.0.2", Some(2222));
        assert!(s.contains("ssh -o BatchMode=yes"), "non-interactive: {s}");
        assert!(s.contains("ConnectTimeout=8"), "bounded connect: {s}");
        assert!(s.contains("-p 2222 "), "carries the custom port: {s}");
        assert!(s.contains("'pi@10.0.0.2'"), "targets the host: {s}");
        assert!(s.contains("grep -vxFf"), "removes exact full-line key matches: {s}");
        assert!(s.contains(".tuiui.bak"), "keeps a backup: {s}");
        assert!(s.contains("backup failed"), "aborts if the backup can't be made: {s}");
        assert!(s.contains("chmod 600"), "tightens perms before swapping in: {s}");
        assert!(s.contains("$ak.tuiui.tmp"), "writes the result inside ~/.ssh, not /tmp: {s}");
        assert!(s.contains("/.ssh/*.pub"), "considers every local public identity: {s}");
        assert!(s.contains("ssh-add -L"), "also covers agent-held keys: {s}");
        assert!(s.contains("[ -r "), "only reads readable key files: {s}");
        assert!(s.contains("no local SSH public key to revoke"), "no-ops cleanly with no key: {s}");
    }

    #[test]
    fn setup_script_has_key_install_gpm_then_attach() {
        let spec = SwitchSpec {
            name: "mini".into(),
            host: "me@mini.local".into(),
            port: None,
            theme: None,
            setup: true,
            password: Some("hunter2".into()),
        };
        let s = switch_script(&spec);
        let key = s.find("ssh-keygen").expect("generates a key when missing");
        let copy = s.find("ssh-copy-id").expect("transfers the key");
        let terminfo = s.find("tic -x -").expect("copies the local terminfo (xterm-ghostty etc.)");
        let install = s.find("install.sh").expect("installs tuiui remotely");
        let gpm = s.find("gpm").expect("installs gpm for the console mouse");
        let sync = s.find("systems.toml").expect("syncs the saved-systems list");
        let attach = s.find("exec ssh -t").expect("ends by attaching");
        assert!(key < copy && copy < terminfo && terminfo < install && install < gpm && gpm < sync && sync < attach);
        assert!(s.contains("infocmp -x"), "terminfo export uses the extended format");
        assert!(!s.contains("hunter2"), "the password must never be embedded in the script");
        assert!(s.contains("sshpass -e"), "password flows via the SSHPASS env var");
        assert!(!s.contains("TUIUI_THEME"), "no theme env when the system has none");
    }
}
