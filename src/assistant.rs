//! The desktop AI assistant: the `opencode` coding agent hosted in a persistent
//! panel or window, opened from the ✦ menubar button.
//!
//! The agent is just another apphost PTY app — so the chat survives detach and
//! frontend reloads like every other window. Its instructions come from the
//! repo's `agent/` pack (embedded at build time), stamped as `AGENTS.md` into a
//! dedicated working directory on every launch with live placeholders filled
//! (host name, saved systems, version). That directory is forced as the agent's
//! cwd, and `AGENTS.md` is the convention opencode reads on startup.
//!
//! tuiui standardises on opencode: one model-agnostic, MCP-extensible CLI
//! rather than a menu of frameworks. `assistant_command` in config.toml can
//! still point the panel at a different binary (e.g. a wrapper or an
//! MCP-preconfigured launcher), but there is no per-framework branching here.

use std::path::PathBuf;

// The instruction pack, embedded from the repo's agent/ folder.
const BRIEFING_MD: &str = include_str!("../agent/BRIEFING.md");
const DESKTOP_MD: &str = include_str!("../agent/DESKTOP.md");
const SYSTEMS_MD: &str = include_str!("../agent/SYSTEMS.md");
const TROUBLESHOOTING_MD: &str = include_str!("../agent/TROUBLESHOOTING.md");
const RULES_MD: &str = include_str!("../agent/RULES.md");

/// The agent CLI tuiui launches by default. Override with `assistant_command`
/// in config.toml to point the panel at a different binary.
pub const DEFAULT_AGENT: &str = "opencode";

/// Resolve the agent command + args. An explicit `assistant_command` override
/// is trusted as-is (with `assistant_args`); otherwise we default to opencode,
/// returning `None` when it isn't installed so the caller can steer the user to
/// install it rather than spawning a missing binary.
pub fn resolve_agent(cfg_command: Option<&str>, cfg_args: &[String]) -> Option<(String, Vec<String>)> {
    match cfg_command.map(str::trim).filter(|c| !c.is_empty()) {
        Some(cmd) => Some((cmd.to_string(), cfg_args.to_vec())),
        None => crate::catalog::is_installed(DEFAULT_AGENT)
            .then(|| (DEFAULT_AGENT.to_string(), cfg_args.to_vec())),
    }
}

/// Whether the configured agent command looks runnable, for the Settings
/// status marker. An explicit path (`assistant_command` may be a wrapper or an
/// absolute path) is checked on disk; a bare name is looked up on `$PATH`.
pub fn agent_available(command: &str) -> bool {
    if command.contains('/') {
        std::path::Path::new(command).is_file()
    } else {
        crate::catalog::is_installed(command)
    }
}

/// The assistant's working directory (`$XDG_DATA_HOME` aware). The instruction
/// pack lives here and the agent CLI starts here, so its context-file
/// convention picks the briefing up automatically.
pub fn workdir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))?;
    Some(base.join("tuiui").join("assistant"))
}

/// Markdown table of the user's saved systems for the SYSTEMS guide, or a
/// pointer to Add Remote when none are saved yet.
fn systems_table(systems: &[crate::systems::RemoteSystem]) -> String {
    if systems.is_empty() {
        return "(none saved yet — the user adds machines via the power menu's \
Systems → Add Remote, which installs SSH keys and tuiui on them)"
            .to_string();
    }
    let mut t = String::from("| name | ssh target | port |\n|---|---|---|\n");
    for s in systems {
        t.push_str(&format!(
            "| {} | {} | {} |\n",
            s.name,
            s.host,
            s.port.map(|p| p.to_string()).unwrap_or_else(|| "22".into())
        ));
    }
    t
}

/// Fill the pack's placeholders in `text`.
fn fill(text: &str, host: &str, systems: &[crate::systems::RemoteSystem]) -> String {
    text.replace("{{HOST}}", host)
        .replace("{{VERSION}}", crate::VERSION)
        .replace("{{SHA}}", crate::GIT_SHA)
        .replace("{{REPO}}", crate::REPO_URL)
        .replace("{{SYSTEMS}}", &systems_table(systems))
}

/// The fully-assembled briefing: every section of the pack, placeholders
/// filled. Written as one file for single-context-file conventions.
pub fn briefing(host: &str, systems: &[crate::systems::RemoteSystem]) -> String {
    let joined = [BRIEFING_MD, DESKTOP_MD, SYSTEMS_MD, TROUBLESHOOTING_MD, RULES_MD].join("\n");
    fill(&joined, host, systems)
}

/// Stamp the instruction pack into `dir` as `AGENTS.md` — the context file
/// opencode reads from its cwd on startup. Refreshed on every launch so the
/// host name, machine list, and version never go stale.
pub fn write_briefing(
    dir: &std::path::Path,
    host: &str,
    systems: &[crate::systems::RemoteSystem],
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // Best-effort: clear artifacts from the retired multi-framework stamping so
    // upgraders don't keep stale context files with outdated placeholders. We
    // leave `.env` alone — it may hold the user's provider secrets.
    let _ = std::fs::remove_file(dir.join("CLAUDE.md"));
    let _ = std::fs::remove_file(dir.join("HERMES.md"));
    let _ = std::fs::remove_dir_all(dir.join("knowledge"));
    std::fs::write(dir.join("AGENTS.md"), briefing(host, systems))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems::RemoteSystem;

    fn sys() -> Vec<RemoteSystem> {
        vec![RemoteSystem { name: "ubuntu".into(), host: "me@10.0.0.7".into(), port: Some(2222), theme: None }]
    }

    #[test]
    fn briefing_contains_the_essentials() {
        let b = briefing("mini", &sys());
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
            // Cross-machine operations: the saved system appears with its port,
            // and the scp/ssh recipes are present.
            "me@10.0.0.7",
            "2222",
            "scp -3",
            "BatchMode=yes",
        ] {
            assert!(b.contains(needle), "briefing must mention {needle:?}");
        }
        assert!(!b.contains("{{"), "all placeholders must be filled");
    }

    #[test]
    fn empty_systems_points_at_add_remote() {
        let b = briefing("mini", &[]);
        assert!(b.contains("Add Remote"), "no machines yet → tell the agent how they get added");
    }

    #[test]
    fn explicit_command_overrides_the_default() {
        // An explicit command is trusted verbatim, args and all — installed or not.
        let (cmd, args) = resolve_agent(Some("my-agent"), &["--flag".into()]).unwrap();
        assert_eq!(cmd, "my-agent");
        assert_eq!(args, vec!["--flag".to_string()]);
        // A blank override is treated as "unset" (falls back to the default).
        assert_eq!(
            resolve_agent(Some("   "), &[]).map(|(c, _)| c),
            resolve_agent(None, &[]).map(|(c, _)| c)
        );
    }

    #[test]
    fn pack_written_as_agents_md_and_cleans_retired_files() {
        let dir = std::env::temp_dir().join(format!("tuiui-assist-test-{}", std::process::id()));
        // Simulate an upgrade: a workdir left by the old multi-framework stamping.
        std::fs::create_dir_all(dir.join("knowledge")).unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "stale").unwrap();
        std::fs::write(dir.join("HERMES.md"), "stale").unwrap();
        std::fs::write(dir.join("knowledge/old.md"), "stale").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=keep").unwrap();

        write_briefing(&dir, "host", &sys()).unwrap();

        let a = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
        assert!(a.contains("desktop assistant"));
        assert!(!a.contains("{{"), "placeholders filled");
        // Retired conventions are cleaned up on write…
        assert!(!dir.join("CLAUDE.md").exists());
        assert!(!dir.join("HERMES.md").exists());
        assert!(!dir.join("knowledge").exists());
        // …but a user's .env (possible secrets) is left untouched.
        assert!(dir.join(".env").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_available_handles_paths_and_names() {
        // A bare name that surely isn't installed reports unavailable.
        assert!(!agent_available("definitely-not-a-real-binary-xyz"));
        // An explicit path is checked on disk, not on $PATH.
        let f = std::env::temp_dir().join(format!("tuiui-agent-{}", std::process::id()));
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        assert!(agent_available(&f.display().to_string()), "an existing path is available");
        let _ = std::fs::remove_file(&f);
        assert!(!agent_available("/no/such/path/opencode"), "a missing path is unavailable");
    }
}
