//! Claude Code integration: keep a window's label and the Claude Code session
//! name in step. Isolated here so no other app is affected.
//!
//! - **Claude → tuiui.** Claude Code puts its session name in the terminal
//!   title (OSC 0/2) as `<status glyph> <name>`, e.g. `✳ My session`; the glyph
//!   is a spinner while it works. [`label_from_title`] strips the glyph, and
//!   `SessionCore::sync_app_titles` makes a Claude window's label follow it
//!   (until the user explicitly renames the window).
//! - **tuiui → Claude.** On a user rename, [`PendingRename`] finds the Claude
//!   session running in that window's process tree (Claude registers each
//!   interactive process in `~/.claude/sessions/<pid>.json`) and, once it is
//!   safe, types `/rename <name>` + Enter into it — exactly what the user
//!   would do. "Safe" means all of: the session reports `status: idle`; the
//!   screen shows an empty input box (nothing typed, no dialog, not vim
//!   NORMAL mode, no "esc to interrupt"); the screen has been unchanged and
//!   nobody typed into the window for 0.8s. Until then it waits (no deadline);
//!   it is cancelled by a newer rename, closing the window, or the session
//!   ending, and does not survive a daemon reload.
//!
//!   Why not write the transcript instead: Claude Code does adopt a
//!   `custom-title` record another process appends, but only lazily, and the
//!   live process re-appends its own in-memory name after every turn, so an
//!   external record never stays the last one while the session runs. The
//!   trade-off of typing is that the rename shows in the conversation as a
//!   normal `/rename` (and Claude gets its usual "user named this session"
//!   note), and a keystroke landing in the same few milliseconds as ours
//!   could interleave.

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
    /// Claude's own activity status (`"idle"`, `"busy"`, …).
    pub status: Option<String>,
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
        let status = v.get("status").and_then(|n| n.as_str()).map(str::to_string);
        return Some(SessionRef { pid, session_id: sid.to_string(), name, status });
    }
    None
}

/// The Claude session running under the app process `root_pid`, if any.
pub fn find_session(root_pid: u32) -> Option<SessionRef> {
    find_session_in(&config_dir()?, &process_tree(root_pid))
}

/// The keystrokes for a rename: the `/rename <name>` text (sent first) and
/// the Enter that submits it (sent separately, after a pause, so Claude reads
/// the text as typing rather than a paste). Control characters are dropped.
pub fn rename_keystrokes(name: &str) -> (Vec<u8>, Vec<u8>) {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    (format!("/rename {}", clean.trim()).into_bytes(), b"\r".to_vec())
}

/// Whether Claude Code's screen shows an EMPTY prompt that is safe to type a
/// slash command into: the bottom-most input box (a `❯` row between two `─`
/// rules) holds nothing but whitespace or Claude's `Try "…"` placeholder, no
/// vim NORMAL mode, and no "esc to interrupt" (busy) footer.
pub fn prompt_is_empty(grid: &crate::buffer::CellBuffer) -> bool {
    let row = |y: i32| -> String { (0..grid.width()).map(|x| grid.get(x, y).map(|c| c.ch).unwrap_or(' ')).collect() };
    let rows: Vec<String> = (0..grid.height()).map(row).collect();
    let text = rows.join("\n");
    if text.contains("-- NORMAL --") || text.to_lowercase().contains("esc to interrupt") {
        return false;
    }
    let is_rule = |r: &str| {
        let n = r.chars().count().max(1);
        r.chars().filter(|&c| c == '─').count() * 2 > n
    };
    for y in (1..rows.len().saturating_sub(1)).rev() {
        let line = rows[y].trim_start();
        let Some(rest) = line.strip_prefix('❯') else { continue };
        if !is_rule(&rows[y - 1]) || !is_rule(&rows[y + 1]) {
            continue;
        }
        let rest = rest.trim();
        return rest.is_empty() || rest.starts_with("Try \"");
    }
    false
}

/// A user rename waiting to be typed into a Claude Code window as `/rename`.
/// A watcher thread polls the session's registry file (`status`, `name`) off
/// the render loop; the session checks the screen and types when it's safe.
pub struct PendingRename {
    pub name: String,
    shared: std::sync::Arc<RenameWatch>,
    /// Last screen seen and since when it has been unchanged.
    stable: Option<(crate::buffer::CellBuffer, std::time::Instant)>,
    /// When the `/rename` text went in (Enter follows after a pause).
    typed_at: Option<std::time::Instant>,
}

#[derive(Default)]
struct RenameWatch {
    /// A Claude session lives in the window's tree, it is idle, and its name
    /// differs from the requested one.
    ready: std::sync::atomic::AtomicBool,
    /// Finished: renamed, superseded, not a Claude window, or session gone.
    done: std::sync::atomic::AtomicBool,
}

/// How long the screen must sit unchanged (no typing echo, no output) before
/// we type — covers keystrokes from any client, including `tuiui view`.
const STABLE_FOR: std::time::Duration = std::time::Duration::from_millis(800);
/// Pause between the `/rename` text and its Enter.
const ENTER_AFTER: std::time::Duration = std::time::Duration::from_millis(300);

/// What the session should do for a pending rename this tick.
#[derive(Debug, PartialEq, Eq)]
pub enum RenameStep {
    Wait,
    Type(Vec<u8>),
    /// Send these bytes and drop the pending rename.
    Finish(Vec<u8>),
    /// Drop it (renamed already, not a Claude window, session gone).
    Abandon,
}

impl PendingRename {
    /// Start watching the Claude session under `root_pid` for a moment to
    /// rename it to `name`.
    pub fn start(root_pid: u32, name: String) -> PendingRename {
        let shared = std::sync::Arc::new(RenameWatch::default());
        let watch = std::sync::Arc::downgrade(&shared);
        let want = name.clone();
        std::thread::spawn(move || {
            use std::sync::atomic::Ordering::SeqCst;
            let Some(cfg) = config_dir() else {
                if let Some(w) = watch.upgrade() { w.done.store(true, SeqCst); }
                return;
            };
            let mut claude_pid: Option<u32> = None;
            let mut misses = 0;
            // Runs until the session drops the rename (the Weak dies) or done.
            while let Some(w) = watch.upgrade() {
                if w.done.load(SeqCst) {
                    return;
                }
                let found = match claude_pid {
                    Some(pid) => find_session_in(&cfg, &[pid]),
                    None => find_session_in(&cfg, &process_tree(root_pid)),
                };
                match found {
                    Some(s) => {
                        misses = 0;
                        if claude_pid.is_none() {
                            crate::dbg_log(&format!("claude: rename of session {} to {want:?} pending", s.session_id));
                        }
                        claude_pid = Some(s.pid);
                        if s.name.as_deref() == Some(want.as_str()) {
                            crate::dbg_log(&format!("claude: session {} is now named {want:?}", s.session_id));
                            w.done.store(true, SeqCst);
                            return;
                        }
                        w.ready.store(s.status.as_deref() == Some("idle"), SeqCst);
                    }
                    None => {
                        w.ready.store(false, SeqCst);
                        misses += 1;
                        // Not a Claude window (or its session ended): give up.
                        if misses >= 3 {
                            w.done.store(true, SeqCst);
                            return;
                        }
                    }
                }
                drop(w);
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
        PendingRename { name, shared, stable: None, typed_at: None }
    }

    /// Decide this tick's action from the app's current screen and how long
    /// ago the user last typed into it.
    pub fn step(&mut self, grid: Option<&crate::buffer::CellBuffer>, since_input: std::time::Duration) -> RenameStep {
        use std::sync::atomic::Ordering::SeqCst;
        let now = std::time::Instant::now();
        if let Some(t) = self.typed_at {
            return if now.duration_since(t) >= ENTER_AFTER {
                self.shared.done.store(true, SeqCst);
                RenameStep::Finish(rename_keystrokes(&self.name).1)
            } else {
                RenameStep::Wait
            };
        }
        if self.shared.done.load(SeqCst) {
            return RenameStep::Abandon;
        }
        let Some(grid) = grid else { return RenameStep::Wait };
        match &self.stable {
            Some((g, _)) if g == grid => {}
            _ => self.stable = Some((grid.clone(), now)),
        }
        let still = self.stable.as_ref().map(|(_, t)| now.duration_since(*t)).unwrap_or_default();
        if self.shared.ready.load(SeqCst) && still >= STABLE_FOR && since_input >= STABLE_FOR && prompt_is_empty(grid) {
            self.typed_at = Some(now);
            crate::dbg_log(&format!("claude: typing /rename {:?} into an idle, empty prompt", self.name));
            return RenameStep::Type(rename_keystrokes(&self.name).0);
        }
        RenameStep::Wait
    }

    /// Test hook: mark the watcher's verdict without a real Claude session.
    #[doc(hidden)]
    pub fn force_ready_for_test(&self, ready: bool) {
        self.shared.ready.store(ready, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for PendingRename {
    fn drop(&mut self) {
        self.shared.done.store(true, std::sync::atomic::Ordering::SeqCst);
    }
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
        std::fs::write(
            d.join("sessions/4242.json"),
            r#"{"pid":4242,"sessionId":"abc-123","cwd":"/tmp/proj","name":"Old","status":"idle"}"#,
        )
        .unwrap();
        d
    }

    #[test]
    fn finds_the_session_in_a_process_tree() {
        let cfg = fake_cfg("find");
        // The window's app is a shell (pid 100); claude (4242) runs under it.
        let s = find_session_in(&cfg, &[100, 4242]).expect("session");
        assert_eq!(
            s,
            SessionRef { pid: 4242, session_id: "abc-123".into(), name: Some("Old".into()), status: Some("idle".into()) }
        );
        assert!(find_session_in(&cfg, &[100, 101]).is_none());
        let _ = std::fs::remove_dir_all(&cfg);
    }

    fn screen(lines: &[&str]) -> crate::buffer::CellBuffer {
        let w = 40;
        let mut g = crate::buffer::CellBuffer::new(w, lines.len() as i32);
        for (y, l) in lines.iter().enumerate() {
            for (x, ch) in l.chars().enumerate().take(w as usize) {
                g.set(x as i32, y as i32, crate::cell::Cell { ch, ..Default::default() });
            }
        }
        g
    }

    const RULE: &str = "────────────────────────────────────────";

    #[test]
    fn empty_prompt_detection() {
        assert!(prompt_is_empty(&screen(&["● OK", RULE, "❯ ", RULE, "  ⏵⏵ auto mode on"])));
        assert!(prompt_is_empty(&screen(&["", RULE, "❯ Try \"fix lint errors\"", RULE])));
        // A draft in the prompt: never type.
        assert!(!prompt_is_empty(&screen(&["", RULE, "❯ half a thought", RULE])));
        // A dialog (no input box), busy footer, vim NORMAL mode.
        assert!(!prompt_is_empty(&screen(&["Do you trust this folder?", "❯ No, exit", "  Yes"])));
        assert!(!prompt_is_empty(&screen(&["✻ Working… (esc to interrupt)", RULE, "❯ ", RULE])));
        assert!(!prompt_is_empty(&screen(&["", RULE, "❯ ", RULE, "-- NORMAL --"])));
    }

    #[test]
    fn rename_keystrokes_strip_control_chars() {
        let (text, enter) = rename_keystrokes("  My\nname\x1b ");
        assert_eq!(text, b"/rename Myname".to_vec());
        assert_eq!(enter, b"\r".to_vec());
    }

    #[test]
    fn pending_rename_waits_for_a_stable_empty_prompt_then_types() {
        let mut p = PendingRename { name: "New".into(), shared: Default::default(), stable: None, typed_at: None };
        let empty = screen(&["", RULE, "❯ ", RULE]);
        let draft = screen(&["", RULE, "❯ abc", RULE]);
        let long = std::time::Duration::from_secs(5);
        // Not ready (busy / unknown): wait.
        assert_eq!(p.step(Some(&empty), long), RenameStep::Wait);
        p.force_ready_for_test(true);
        // Ready, but the screen only just appeared: wait for it to settle.
        assert_eq!(p.step(Some(&empty), long), RenameStep::Wait);
        p.stable = Some((empty.clone(), std::time::Instant::now() - long));
        // The user typed a moment ago: wait.
        assert_eq!(p.step(Some(&empty), std::time::Duration::from_millis(100)), RenameStep::Wait);
        // A draft: never.
        p.stable = Some((draft.clone(), std::time::Instant::now() - long));
        assert_eq!(p.step(Some(&draft), long), RenameStep::Wait);
        // Stable, idle, empty: type the text, then Enter after the pause.
        p.stable = Some((empty.clone(), std::time::Instant::now() - long));
        assert_eq!(p.step(Some(&empty), long), RenameStep::Type(b"/rename New".to_vec()));
        assert_eq!(p.step(Some(&empty), long), RenameStep::Wait);
        p.typed_at = Some(std::time::Instant::now() - long);
        assert_eq!(p.step(Some(&empty), long), RenameStep::Finish(b"\r".to_vec()));
    }
}
