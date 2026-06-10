//! The desktop AI assistant: a coding agent (Claude Code, opencode, smallcode,
//! kilo, hermes, openclaw) hosted in a persistent panel or window, opened from
//! the ✦ menubar button.
//!
//! The agent is just another apphost PTY app — so the chat survives detach and
//! frontend reloads like every other window. Its instructions come from the
//! repo's `agent/` pack (embedded at build time), stamped into a dedicated
//! working directory on every launch with live placeholders filled (host name,
//! saved systems, version). The directory is forced as the agent's cwd no
//! matter which framework runs, and the pack is written in every context-file
//! convention the supported CLIs read: `CLAUDE.md` (Claude Code), `AGENTS.md`
//! (opencode/kilo/codex-style), and `knowledge/*.md` (smallcode).

use std::path::PathBuf;

// The instruction pack, embedded from the repo's agent/ folder.
const BRIEFING_MD: &str = include_str!("../agent/BRIEFING.md");
const DESKTOP_MD: &str = include_str!("../agent/DESKTOP.md");
const SYSTEMS_MD: &str = include_str!("../agent/SYSTEMS.md");
const TROUBLESHOOTING_MD: &str = include_str!("../agent/TROUBLESHOOTING.md");
const RULES_MD: &str = include_str!("../agent/RULES.md");

/// Agent CLIs probed on `$PATH`, in preference order, when the config does not
/// pin one (`assistant_command` in config.toml / Settings → Assistant).
pub const AGENT_CLIS: &[&str] = &["claude", "opencode", "smallcode", "kilo", "hermes", "openclaw"];

/// Default launch arguments per framework, used when `assistant_args` is empty.
/// Verified against each CLI's docs: hermes' TUI is behind a `--tui` flag;
/// openclaw's is the `tui` SUBCOMMAND (`--tui` does not exist; the gateway
/// daemon from `openclaw onboard --install-daemon` serves it). The coding
/// agents (claude/opencode/smallcode/kilo) open their TUI bare.
pub fn default_args(command: &str) -> Vec<String> {
    let bin = command.rsplit('/').next().unwrap_or(command);
    match bin {
        "hermes" => vec!["--tui".into()],
        "openclaw" => vec!["tui".into()],
        _ => Vec::new(),
    }
}

/// Resolve the agent command: the config override when set, else the first
/// known CLI found on `$PATH`. Empty configured args fall back to the
/// framework's defaults (e.g. `--tui` for hermes/openclaw).
pub fn resolve_agent(cfg_command: Option<&str>, cfg_args: &[String]) -> Option<(String, Vec<String>)> {
    let command = match cfg_command {
        Some(cmd) if !cmd.trim().is_empty() => cmd.trim().to_string(),
        _ => AGENT_CLIS
            .iter()
            .find(|c| crate::catalog::is_installed(c))?
            .to_string(),
    };
    let args = if cfg_args.is_empty() { default_args(&command) } else { cfg_args.to_vec() };
    Some((command, args))
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

/// Stamp the instruction pack into `dir` in every convention a supported agent
/// reads on startup. Refreshed on every launch so the host name, machine list,
/// and version never go stale.
pub fn write_briefing(
    dir: &std::path::Path,
    host: &str,
    systems: &[crate::systems::RemoteSystem],
) -> std::io::Result<()> {
    let knowledge = dir.join("knowledge");
    std::fs::create_dir_all(&knowledge)?;
    let full = briefing(host, systems);
    // Single-file conventions get the whole pack. HERMES.md is hermes' own
    // highest-priority context file (it beats AGENTS.md, first-match-wins).
    std::fs::write(dir.join("CLAUDE.md"), &full)?;
    std::fs::write(dir.join("AGENTS.md"), &full)?;
    std::fs::write(dir.join("HERMES.md"), &full)?;
    // smallcode's knowledge/ convention favours topical notes.
    for (name, text) in [
        ("tuiui.md", BRIEFING_MD),
        ("desktop.md", DESKTOP_MD),
        ("systems.md", SYSTEMS_MD),
        ("troubleshooting.md", TROUBLESHOOTING_MD),
        ("rules.md", RULES_MD),
    ] {
        std::fs::write(knowledge.join(name), fill(text, host, systems))?;
    }
    Ok(())
}

/// smallcode configures its model/endpoint via a `.env` in its working
/// directory. Seed a commented template ONCE (never overwrite — the user's
/// real keys may be in it) so opening the assistant with smallcode lands on
/// an explainer instead of a connection error.
pub fn seed_smallcode_env(dir: &std::path::Path) -> std::io::Result<()> {
    let env = dir.join(".env");
    if env.exists() {
        return Ok(());
    }
    std::fs::write(
        env,
        "# smallcode configuration (see https://github.com/Doorman11991/smallcode)\n\
# Point at any OpenAI-compatible server. LM Studio default:\n\
SMALLCODE_MODEL=your-local-model-name\n\
SMALLCODE_BASE_URL=http://localhost:1234/v1\n\
# Ollama instead:  SMALLCODE_MODEL=qwen3:8b  SMALLCODE_BASE_URL=http://localhost:11434/v1\n\
# Optional escalation tiers:\n\
# SMALLCODE_MODEL_STRONG=openai/gpt-4o-mini\n\
# SMALLCODE_BASE_URL_STRONG=https://openrouter.ai/api/v1\n",
    )
}

/// OpenClaw is the one supported agent that does NOT read context files from
/// its cwd — its system prompt is assembled from bootstrap files in its own
/// workspace (`~/.openclaw/workspace/AGENTS.md` et al). Politely point it at
/// our pack: append a small, clearly-marked section to that file (created if
/// missing, only when `~/.openclaw` exists — i.e. openclaw is set up).
/// Idempotent via the marker line.
pub fn inject_openclaw_pointer(pack_dir: &std::path::Path) -> std::io::Result<()> {
    const MARKER: &str = "<!-- tuiui-assistant-briefing -->";
    let Some(home) = dirs::home_dir() else { return Ok(()) };
    let oc = home.join(".openclaw");
    if !oc.exists() {
        return Ok(()); // openclaw not set up; nothing to point at
    }
    let ws = oc.join("workspace");
    std::fs::create_dir_all(&ws)?;
    let agents = ws.join("AGENTS.md");
    let existing = std::fs::read_to_string(&agents).unwrap_or_default();
    if existing.contains(MARKER) {
        return Ok(());
    }
    let section = format!(
        "\n{MARKER}\n## Running inside tuiui\n\nWhen you are launched from the tuiui \
terminal desktop, your full briefing (role, the user's machines, desktop \
control commands, rules) is at `{}/AGENTS.md` — read it first.\n",
        pack_dir.display()
    );
    std::fs::write(&agents, existing + &section)?;
    Ok(())
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
    fn config_override_and_default_args() {
        let (cmd, args) = resolve_agent(Some("my-agent"), &["--flag".into()]).unwrap();
        assert_eq!(cmd, "my-agent");
        assert_eq!(args, vec!["--flag".to_string()]);
        // hermes' TUI is a flag; openclaw's is a subcommand (per their docs).
        assert_eq!(default_args("hermes"), vec!["--tui".to_string()]);
        assert_eq!(default_args("openclaw"), vec!["tui".to_string()]);
        assert_eq!(resolve_agent(Some("openclaw"), &[]).unwrap().1, vec!["tui".to_string()]);
        // Coding agents launch bare; explicit args always win.
        assert!(default_args("claude").is_empty());
        assert_eq!(resolve_agent(Some("hermes"), &["chat".into()]).unwrap().1, vec!["chat".to_string()]);
    }

    #[test]
    fn pack_written_in_every_convention() {
        let dir = std::env::temp_dir().join(format!("tuiui-assist-test-{}", std::process::id()));
        write_briefing(&dir, "host", &sys()).unwrap();
        let a = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        let b = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
        assert_eq!(a, b);
        assert!(a.contains("desktop assistant"));
        let h = std::fs::read_to_string(dir.join("HERMES.md")).unwrap();
        assert_eq!(a, h, "hermes' own context file gets the same pack");
        for f in ["tuiui.md", "desktop.md", "systems.md", "troubleshooting.md", "rules.md"] {
            let text = std::fs::read_to_string(dir.join("knowledge").join(f)).unwrap();
            assert!(!text.contains("{{"), "{f}: placeholders filled");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
