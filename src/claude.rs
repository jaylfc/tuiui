//! Claude Code integration: keep a window's label and the Claude Code session
//! name in step. Isolated here so no other app is affected.
//!
//! - **Claude → tuiui.** Claude Code puts its session name in the terminal
//!   title (OSC 0/2) as `<status glyph> <name>`, e.g. `✳ My session`; the glyph
//!   is a spinner while it works. [`label_from_title`] strips the glyph, and
//!   `SessionCore::sync_app_titles` makes a Claude window's label follow it
//!   (until the user explicitly renames the window).
//! - **tuiui → Claude.** On a user rename, [`sync_rename_async`] finds the
//!   Claude session running in that window's process tree (Claude registers
//!   each interactive process in `~/.claude/sessions/<pid>.json`) and appends
//!   the same `custom-title` + `agent-name` records that `/rename` writes to
//!   the session transcript. Claude Code documents that a name another process
//!   writes to the transcript is picked up when the live process next reads the
//!   end of its transcript (after ~32 KB of its own writes, or at a compaction)
//!   and it is what `claude --resume` / the picker show from then on. We never
//!   type `/rename` into the PTY: tuiui cannot know the prompt is empty, and
//!   keystrokes injected into a half-typed prompt would corrupt it.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Whether a window launched as `cmd` under dock group `app_key` is Claude Code.
pub fn is_claude_app(cmd: &str, app_key: &str) -> bool {
    let base = cmd.rsplit('/').next().unwrap_or(cmd);
    let key = app_key.to_lowercase();
    base == "claude" || key == "claude" || key.starts_with("claude ")
}

/// The label a Claude Code OSC title stands for: the title minus Claude's
/// leading status glyph (`✳`, `✻`, braille spinners, …). `None` for an empty
/// title.
pub fn label_from_title(title: &str) -> Option<String> {
    let t = title.trim();
    let mut chars = t.chars();
    let label = match (chars.next(), chars.next()) {
        (Some(c), Some(s)) if !c.is_alphanumeric() && !c.is_ascii() && s.is_whitespace() => chars.as_str().trim(),
        _ => t,
    };
    (!label.is_empty()).then(|| label.to_string())
}

/// A live Claude Code session found in a window's process tree.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRef {
    /// The `claude` process's pid.
    pub pid: u32,
    pub session_id: String,
    /// The session's current name as Claude last recorded it.
    pub name: Option<String>,
}

/// Claude Code's config directory (`$CLAUDE_CONFIG_DIR`, else `~/.claude`).
pub fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d)),
        _ => dirs::home_dir().map(|h| h.join(".claude")),
    }
}

/// `root` and all its descendants, breadth-first (root first).
pub fn process_tree(root: u32) -> Vec<u32> {
    descendants(root, &process_edges())
}

/// Every `(parent pid, child pid)` pair on the system (one process scan; reuse
/// it with [`descendants`] when walking several trees).
pub fn process_edges() -> Vec<(u32, u32)> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes()
        .iter()
        .filter_map(|(pid, p)| p.parent().map(|pp| (pp.as_u32(), pid.as_u32())))
        .collect()
}

/// Breadth-first walk of `(parent, child)` edges from `root`, root first.
pub fn descendants(root: u32, edges: &[(u32, u32)]) -> Vec<u32> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        let mut kids: Vec<u32> = edges.iter().filter(|(pp, _)| *pp == p).map(|(_, c)| *c).collect();
        kids.sort_unstable();
        for k in kids {
            if !out.contains(&k) {
                out.push(k);
            }
        }
        i += 1;
    }
    out
}

/// The first of `pids` that Claude registered in `<cfg>/sessions/<pid>.json`.
pub fn find_session_in(cfg: &Path, pids: &[u32]) -> Option<SessionRef> {
    for &pid in pids {
        let Ok(text) = std::fs::read_to_string(cfg.join("sessions").join(format!("{pid}.json"))) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        // The file is keyed by pid; check it really is this process's record.
        if v.get("pid").and_then(|p| p.as_u64()) != Some(pid as u64) {
            continue;
        }
        let Some(sid) = v.get("sessionId").and_then(|s| s.as_str()) else { continue };
        let name = v.get("name").and_then(|n| n.as_str()).map(str::to_string);
        return Some(SessionRef { pid, session_id: sid.to_string(), name });
    }
    None
}

/// The Claude session running under the app process `root_pid`, if any.
pub fn find_session(root_pid: u32) -> Option<SessionRef> {
    find_session_in(&config_dir()?, &process_tree(root_pid))
}

/// The session's transcript: `<cfg>/projects/<project>/<session-id>.jsonl`.
pub fn transcript_path(cfg: &Path, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains('/') || session_id.contains("..") {
        return None;
    }
    let file = format!("{session_id}.jsonl");
    std::fs::read_dir(cfg.join("projects"))
        .ok()?
        .flatten()
        .map(|e| e.path().join(&file))
        .find(|p| p.is_file())
}

/// The transcript records `/rename <name>` appends (one JSON object per line).
pub fn rename_records(session_id: &str, name: &str) -> String {
    let title = serde_json::json!({ "type": "custom-title", "customTitle": name, "sessionId": session_id });
    let agent = serde_json::json!({ "type": "agent-name", "agentName": name, "sessionId": session_id });
    format!("{title}\n{agent}\n")
}

/// Record `name` as the session's name in its transcript (and the
/// `custom-title.json` sidecar Claude keeps beside it).
pub fn rename_session_in(cfg: &Path, session_id: &str, name: &str) -> std::io::Result<PathBuf> {
    let path = transcript_path(cfg, session_id)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no transcript for session"))?;
    // One O_APPEND write of both short lines, so it can't interleave with
    // Claude's own appends.
    let mut f = std::fs::OpenOptions::new().append(true).open(&path)?;
    f.write_all(rename_records(session_id, name).as_bytes())?;
    if let Some(dir) = path.parent().map(|p| p.join(session_id)) {
        if dir.is_dir() {
            let _ = std::fs::write(dir.join("custom-title.json"), serde_json::json!({ "customTitle": name }).to_string());
        }
    }
    Ok(path)
}

/// On a user rename of the window whose app runs as `root_pid`: if a Claude
/// session lives in that process tree, rename it to `name`. Runs on its own
/// thread (process scan + file IO stay off the render loop); every outcome is
/// logged.
pub fn sync_rename_async(root_pid: u32, name: String) {
    std::thread::spawn(move || {
        let Some(cfg) = config_dir() else { return };
        let Some(s) = find_session_in(&cfg, &process_tree(root_pid)) else { return };
        if s.name.as_deref() == Some(name.as_str()) {
            return;
        }
        match rename_session_in(&cfg, &s.session_id, &name) {
            Ok(p) => crate::dbg_log(&format!(
                "claude: renamed session {} (pid {}) to {name:?} via {}",
                s.session_id,
                s.pid,
                p.display()
            )),
            Err(e) => crate::dbg_log(&format!("claude: rename of session {} failed: {e}", s.session_id)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_claude_status_glyphs() {
        assert_eq!(label_from_title("✳ My session").as_deref(), Some("My session"));
        assert_eq!(label_from_title("⠂ Fix the bug ").as_deref(), Some("Fix the bug"));
        assert_eq!(label_from_title("Claude Code").as_deref(), Some("Claude Code"));
        // An ASCII leading char is part of the name, not a glyph.
        assert_eq!(label_from_title("# notes").as_deref(), Some("# notes"));
        assert_eq!(label_from_title("  ").as_deref(), None);
        assert_eq!(label_from_title("✳").as_deref(), Some("✳"));
    }

    #[test]
    fn detects_claude_windows() {
        assert!(is_claude_app("claude", "Claude Code"));
        assert!(is_claude_app("/home/u/.local/bin/claude", "x"));
        assert!(is_claude_app("sh", "Claude Code ⚠️"));
        assert!(!is_claude_app("btop", "btop"));
        assert!(!is_claude_app("claudette", "claudette"));
    }

    #[test]
    fn walks_descendants_breadth_first() {
        let edges = [(1, 2), (2, 3), (2, 4), (9, 10), (3, 5)];
        assert_eq!(descendants(2, &edges), vec![2, 3, 4, 5]);
        assert_eq!(descendants(7, &edges), vec![7]);
    }

    fn fake_cfg(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tuiui-claude-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sessions")).unwrap();
        std::fs::create_dir_all(d.join("projects/-tmp-proj/abc-123")).unwrap();
        std::fs::write(
            d.join("sessions/4242.json"),
            r#"{"pid":4242,"sessionId":"abc-123","cwd":"/tmp/proj","name":"Old"}"#,
        )
        .unwrap();
        std::fs::write(d.join("projects/-tmp-proj/abc-123.jsonl"), "{\"type\":\"user\"}\n").unwrap();
        d
    }

    #[test]
    fn finds_the_session_in_a_process_tree() {
        let cfg = fake_cfg("find");
        // The window's app is a shell (pid 100); claude (4242) runs under it.
        let s = find_session_in(&cfg, &[100, 4242]).expect("session");
        assert_eq!(s, SessionRef { pid: 4242, session_id: "abc-123".into(), name: Some("Old".into()) });
        assert!(find_session_in(&cfg, &[100, 101]).is_none());
        let _ = std::fs::remove_dir_all(&cfg);
    }

    #[test]
    fn rename_appends_the_records_rename_writes() {
        let cfg = fake_cfg("rename");
        rename_session_in(&cfg, "abc-123", "New \"name\"").unwrap();
        let text = std::fs::read_to_string(cfg.join("projects/-tmp-proj/abc-123.jsonl")).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 3, "existing line kept, two appended");
        assert_eq!(lines[1]["type"], "custom-title");
        assert_eq!(lines[1]["customTitle"], "New \"name\"");
        assert_eq!(lines[1]["sessionId"], "abc-123");
        assert_eq!(lines[2]["type"], "agent-name");
        assert_eq!(lines[2]["agentName"], "New \"name\"");
        let side = std::fs::read_to_string(cfg.join("projects/-tmp-proj/abc-123/custom-title.json")).unwrap();
        assert!(side.contains("New \\\"name\\\""));
        // Unknown / path-like session ids are refused.
        assert!(rename_session_in(&cfg, "nope", "x").is_err());
        assert!(transcript_path(&cfg, "../etc").is_none());
        let _ = std::fs::remove_dir_all(&cfg);
    }
}
