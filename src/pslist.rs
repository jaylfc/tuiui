//! `tuiui ps --json`: the machine-readable list of hosted apps (used by the
//! Prism Android app to show tuiui and its individual apps). Built from the
//! apphost's `ListApps` reply, decoding the desktop's window metadata for the
//! dock group / label. Keep keys stable — external clients parse them.
//!
//! ```json
//! {"proto":3,"apps":[{"id":3,"pid":4100,"cmd":"claude","args":[],"alive":true,
//!   "age_secs":620,"cols":120,"rows":40,"group":"Claude Code",
//!   "label":"Prism bugs","app_title":"✳ Prism bugs","user_renamed":false,
//!   "on_desktop":true,"minimized":false,
//!   "claude":{"session_id":"ffad21dc-…","name":"Prism bugs"}}]}
//! ```

use crate::apphost::AppListEntry;
use crate::claude::SessionRef;
use serde::Serialize;

/// What the CLI prints when the apphost never answers: before protocol v3 it
/// served one frontend at a time, and the desktop daemon stays connected for
/// its whole life (attached client or not), so every other connection queued.
pub const BUSY_HINT: &str = "tuiui: the app server did not answer within 3s. It is probably an older \
(pre-v3) apphost that serves one connection at a time and is busy with the desktop daemon. \
Restart it to get the multi-client apphost (this closes its apps): `tuiui kill`, then `tuiui`.";

/// One app row.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct PsRow {
    /// The apphost app id (what `tuiui view <id>` / `kill-app <id>` take).
    pub id: u64,
    pub pid: Option<u32>,
    pub cmd: String,
    pub args: Vec<String>,
    pub alive: bool,
    pub age_secs: u64,
    pub cols: i32,
    pub rows: i32,
    /// Dock group (the immutable launch name the dock groups windows by);
    /// falls back to the command's basename when no desktop window exists.
    pub group: String,
    /// Window label: the user's rename, else the label the desktop derived
    /// from the app's title, else the app's title (Claude glyph stripped),
    /// else the group.
    pub label: String,
    /// The app's raw OSC 0/2 title, if it set one.
    pub app_title: Option<String>,
    pub user_renamed: bool,
    /// A desktop window exists for the app (it stored window metadata).
    pub on_desktop: bool,
    pub minimized: bool,
    /// The Claude Code session running in the app's process tree, if any.
    pub claude: Option<PsClaude>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct PsClaude {
    pub session_id: String,
    pub name: Option<String>,
}

/// The whole `ps --json` document.
#[derive(Serialize, Debug)]
pub struct PsList {
    /// The apphost protocol version that answered (0 = too old to say).
    pub proto: u32,
    pub apps: Vec<PsRow>,
}

/// Build the rows; `claude` looks up the Claude session for an app (pure
/// callers/tests pass a stub).
pub fn rows(apps: &[AppListEntry], claude: impl Fn(&AppListEntry) -> Option<SessionRef>) -> Vec<PsRow> {
    apps.iter()
        .map(|a| {
            let meta = crate::session::decode_window_meta(&a.meta);
            let base = a.cmd.rsplit('/').next().unwrap_or(&a.cmd).to_string();
            let group = meta.as_ref().map(|m| m.group.clone()).filter(|g| !g.is_empty()).unwrap_or_else(|| base.clone());
            let from_title = a.title.as_deref().and_then(|t| {
                if crate::claude::is_claude_app(&a.cmd, &group) {
                    crate::claude::label_from_title(t)
                } else {
                    Some(t.trim().to_string()).filter(|t| !t.is_empty())
                }
            });
            let label = meta
                .as_ref()
                .map(|m| m.label.clone())
                .filter(|l| !l.is_empty())
                .or(from_title)
                .unwrap_or_else(|| group.clone());
            PsRow {
                id: a.app,
                pid: a.pid,
                cmd: a.cmd.clone(),
                args: a.args.clone(),
                alive: a.alive,
                age_secs: a.age_secs,
                cols: a.cols,
                rows: a.rows,
                group,
                label,
                app_title: a.title.clone(),
                user_renamed: meta.as_ref().map(|m| m.user_renamed).unwrap_or(false),
                on_desktop: meta.is_some(),
                minimized: meta.as_ref().map(|m| m.minimized).unwrap_or(false),
                claude: claude(a).map(|s| PsClaude { session_id: s.session_id, name: s.name }),
            }
        })
        .collect()
}

/// Look up the Claude session for each app using one process scan.
pub fn claude_lookup() -> impl Fn(&AppListEntry) -> Option<SessionRef> {
    let edges = crate::claude::process_edges();
    let cfg = crate::claude::config_dir();
    move |a: &AppListEntry| {
        let cfg = cfg.as_ref()?;
        let pid = a.pid?;
        crate::claude::find_session_in(cfg, &crate::claude::descendants(pid, &edges))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(app: u64, cmd: &str, meta: &str, title: Option<&str>) -> AppListEntry {
        AppListEntry {
            app,
            cmd: cmd.into(),
            args: vec![],
            pid: Some(100 + app as u32),
            cols: 80,
            rows: 24,
            age_secs: 5,
            alive: true,
            meta: meta.as_bytes().to_vec(),
            title: title.map(str::to_string),
        }
    }

    const META: &str = r#"{"rect":{"x":1,"y":2,"w":40,"h":12},"title":"Renamed","z":3,"minimized":true,"app_key":"Claude Code","user_renamed":true}"#;

    #[test]
    fn row_shape_from_desktop_meta() {
        let rows = rows(&[entry(3, "claude", META, Some("✳ Session x"))], |a| {
            (a.app == 3).then(|| SessionRef { pid: 9, session_id: "sid".into(), name: Some("Session x".into()) })
        });
        let v = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(v["id"], 3);
        assert_eq!(v["pid"], 103);
        assert_eq!(v["cmd"], "claude");
        assert_eq!(v["alive"], true);
        assert_eq!(v["group"], "Claude Code");
        assert_eq!(v["label"], "Renamed");
        assert_eq!(v["app_title"], "✳ Session x");
        assert_eq!(v["user_renamed"], true);
        assert_eq!(v["on_desktop"], true);
        assert_eq!(v["minimized"], true);
        assert_eq!(v["claude"]["session_id"], "sid");
        assert_eq!(v["claude"]["name"], "Session x");
        for key in ["args", "age_secs", "cols", "rows"] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn falls_back_without_meta() {
        // Claude title → stripped label; group = command basename.
        let r = &rows(&[entry(1, "/usr/bin/claude", "", Some("✻ Busy one"))], |_| None)[0];
        assert_eq!((r.group.as_str(), r.label.as_str(), r.on_desktop), ("claude", "Busy one", false));
        assert!(r.claude.is_none());
        // Other app, no title → the group.
        let r = &rows(&[entry(2, "btop", "", None)], |_| None)[0];
        assert_eq!((r.group.as_str(), r.label.as_str()), ("btop", "btop"));
        // Old-style meta (no app_key / user_renamed) still decodes.
        let old = r#"{"rect":{"x":0,"y":0,"w":10,"h":5},"title":"htop","z":1,"minimized":false}"#;
        let r = &rows(&[entry(4, "htop", old, None)], |_| None)[0];
        assert_eq!((r.group.as_str(), r.label.as_str(), r.user_renamed), ("htop", "htop", false));
    }

    #[test]
    fn document_is_an_object_with_apps() {
        let doc = PsList { proto: 3, apps: rows(&[entry(1, "sh", "", None)], |_| None) };
        let v = serde_json::to_value(&doc).unwrap();
        assert_eq!(v["proto"], 3);
        assert!(v["apps"].is_array());
    }
}
