//! The desktop AI assistant: a coding agent (Claude Code, opencode, kilo,
//! hermes, openclaw, …) hosted in a persistent right-side panel, opened from
//! the ✦ menubar button.
//!
//! The agent is just another apphost PTY app — so the chat survives detach and
//! frontend reloads like every other window. It is briefed about its role by
//! `CLAUDE.md` / `AGENTS.md` files written into a dedicated working directory
//! (the convention all the major agent CLIs read on startup), and it controls
//! the desktop through the `tuiui` control CLI (`tuiui launch/tile/theme/msg`).

use std::path::PathBuf;

/// Agent CLIs probed on `$PATH`, in preference order, when the config does not
/// pin one (`assistant_command` in config.toml). The recommended pair is
/// **Claude Code** (premium; reads `CLAUDE.md`) and **opencode** (open source,
/// multi-provider incl. local models; reads `AGENTS.md`); **smallcode** covers
/// fully-local small models (reads a `knowledge/` directory). The rest are
/// probed as a courtesy.
pub const AGENT_CLIS: &[&str] = &["claude", "opencode", "smallcode", "kilo", "hermes", "openclaw"];

/// Resolve the agent command: the config override when set, else the first
/// known CLI found on `$PATH`.
pub fn resolve_agent(cfg_command: Option<&str>, cfg_args: &[String]) -> Option<(String, Vec<String>)> {
    if let Some(cmd) = cfg_command {
        if !cmd.trim().is_empty() {
            return Some((cmd.to_string(), cfg_args.to_vec()));
        }
    }
    AGENT_CLIS
        .iter()
        .find(|c| crate::catalog::is_installed(c))
        .map(|c| (c.to_string(), cfg_args.to_vec()))
}

/// The assistant's working directory (`$XDG_DATA_HOME` aware). The briefing
/// files live here and the agent CLI starts here, so its context-file
/// convention picks the briefing up automatically.
pub fn workdir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))?;
    Some(base.join("tuiui").join("assistant"))
}

/// Write the briefing in every convention a supported agent reads on startup:
/// `CLAUDE.md` (Claude Code), `AGENTS.md` (opencode/codex), and
/// `knowledge/tuiui.md` (smallcode). Refreshed on every launch so the host
/// name, version, and command list never go stale.
pub fn write_briefing(dir: &std::path::Path, host: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir.join("knowledge"))?;
    let text = briefing(host);
    std::fs::write(dir.join("CLAUDE.md"), &text)?;
    std::fs::write(dir.join("AGENTS.md"), &text)?;
    std::fs::write(dir.join("knowledge").join("tuiui.md"), &text)?;
    Ok(())
}

/// The injected system briefing: who the agent is, where it runs, how to drive
/// the desktop, and where the source/config/logs live.
pub fn briefing(host: &str) -> String {
    format!(
        r#"# You are the tuiui desktop assistant

You are an AI agent running INSIDE tuiui — a window manager & desktop for the
terminal (floating windows, dock, launcher, app store, mouse) — in a chat
panel on the user's machine `{host}`. tuiui version: {version} (git {sha}).

## Your role

Help the user run their terminal desktop:
- Answer questions about tuiui and the TUI apps it hosts.
- Diagnose problems: app install failures, rendering issues, remote-system
  (ssh) setup. The live log is at `~/tuiui-debug.log` — read it first.
- Arrange the desktop for them (commands below): open apps, tile windows,
  switch themes.
- Fix tuiui itself: the source is at {repo} — clone it, find the bug, and
  open a pull request against that repository. Build with `cargo build`,
  test with `cargo test`. The user installs updates in-app
  (Settings → Updates), which runs `cargo install --git {repo}`.

## Controlling the desktop

These commands talk to the running tuiui daemon (same user, local socket):

- `tuiui launch <command> [args…]`   open a new app window running <command>
- `tuiui tile`                       tile all windows into the configured grid
- `tuiui theme <name>`               switch theme (midnight|nord|gruvbox|dracula)
- `tuiui reload`                     reload the UI (apps keep running)
- `tuiui msg '<json>'`               raw control message (ClientMsg JSON), e.g.
    tuiui msg '"MaximizeFocused"'
    tuiui msg '{{"SnapFocused":"Left"}}'
    tuiui msg '{{"Launch":{{"name":"btop","command":"btop","args":[]}}}}'

## Where things live

- Config:        `~/.config/tuiui/config.toml` (theme, grid, apps, pins)
- Saved systems: `~/.config/tuiui/systems.toml` (ssh remotes for the Systems menu)
- Logs:          `~/tuiui-debug.log` (always on, capped at 4MB)
- Source:        {repo}

## Ground rules

- You are inside a PTY window: keep output narrow-friendly; the panel is
  typically 40-70 columns wide.
- Never run `tuiui kill` (it terminates the user's whole desktop) unless
  they explicitly ask. `tuiui reload` is the safe restart.
- Destructive file operations: confirm with the user first.
"#,
        host = host,
        version = crate::VERSION,
        sha = crate::GIT_SHA,
        repo = crate::REPO_URL,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn briefing_contains_the_essentials() {
        let b = briefing("mini");
        for needle in [
            "tuiui desktop assistant",
            "mini",
            crate::REPO_URL,
            "tuiui launch",
            "tuiui tile",
            "tuiui theme",
            "tuiui msg",
            "tuiui-debug.log",
            "config.toml",
            "systems.toml",
            "pull request",
            "Never run `tuiui kill`",
        ] {
            assert!(b.contains(needle), "briefing must mention {needle:?}");
        }
    }

    #[test]
    fn config_override_wins_over_detection() {
        let (cmd, args) = resolve_agent(Some("my-agent"), &["--flag".into()]).unwrap();
        assert_eq!(cmd, "my-agent");
        assert_eq!(args, vec!["--flag".to_string()]);
        // Blank override falls through to PATH detection (may be None on CI).
        let detected = resolve_agent(Some("  "), &[]);
        if let Some((cmd, _)) = detected {
            assert!(AGENT_CLIS.contains(&cmd.as_str()));
        }
    }

    #[test]
    fn briefing_files_written_and_identical() {
        let dir = std::env::temp_dir().join(format!("tuiui-assist-test-{}", std::process::id()));
        write_briefing(&dir, "host").unwrap();
        let a = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        let b = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
        let k = std::fs::read_to_string(dir.join("knowledge").join("tuiui.md")).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, k, "smallcode's knowledge/ convention gets the same briefing");
        assert!(a.contains("desktop assistant"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
