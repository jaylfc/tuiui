//! The built-in Logs viewer (launcher → tuiui → Logs): a scrollable window over
//! `~/tuiui-debug.log` with one-key copy of the log to the host terminal's
//! clipboard (OSC 52 — works in Ghostty/Kitty/WezTerm and over ssh).
//!
//! Logging is always on (see [`crate::dbg_log`]), so this window has content
//! without restarting tuiui with an env var.

use crate::buffer::CellBuffer;
use crate::cell::Cell;

/// Keep at most this many lines in memory (the file itself is capped at 4MB).
const MAX_LINES: usize = 5000;

/// Cap the clipboard payload: terminals reject huge OSC 52 writes, and a bug
/// report rarely needs more than the recent tail.
const MAX_COPY_BYTES: usize = 200 * 1024;

pub struct LogsView {
    lines: Vec<String>,
    /// Index of the first visible line. Kept pinned to the bottom while
    /// `follow` is set (new content scrolls into view like `tail -f`).
    scroll: usize,
    follow: bool,
    status: String,
    /// Rows of log shown by the last render (page size for PgUp/PgDn).
    page: std::cell::Cell<usize>,
}

impl Default for LogsView {
    fn default() -> Self {
        Self::new()
    }
}

/// The log file path shown and read by the viewer.
pub fn log_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join("tuiui-debug.log"))
}

impl LogsView {
    pub fn new() -> Self {
        let mut v = LogsView {
            lines: Vec::new(),
            scroll: 0,
            follow: true,
            status: String::new(),
            page: std::cell::Cell::new(20),
        };
        v.reload();
        v
    }

    /// Re-read the log file, keeping the tail and the follow position.
    pub fn reload(&mut self) {
        let Some(path) = log_path() else {
            self.status = "no home directory".into();
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let all: Vec<&str> = text.lines().collect();
                let skip = all.len().saturating_sub(MAX_LINES);
                self.lines = all[skip..].iter().map(|s| s.to_string()).collect();
                self.status = format!("{} lines", self.lines.len());
            }
            Err(_) => {
                self.lines = vec!["(no log yet — this file appears as tuiui logs events)".into()];
                self.status = format!("missing: {}", path.display());
            }
        }
        if self.follow {
            self.scroll = usize::MAX; // clamped to the bottom on render
        }
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let page = self.page.get().max(1);
        let max_top = self.lines.len().saturating_sub(page);
        let cur = self.scroll.min(max_top);
        let next = (cur as i64 + delta as i64).clamp(0, max_top as i64) as usize;
        self.scroll = next;
        self.follow = next == max_top;
    }

    pub fn page_size(&self) -> usize {
        self.page.get().max(1)
    }

    /// The text to put on the host clipboard: the visible tail of the log,
    /// capped so the OSC 52 write stays within terminal limits.
    pub fn copy_payload(&mut self) -> String {
        let joined = self.lines.join("\n");
        let bytes = joined.as_bytes();
        let payload = if bytes.len() > MAX_COPY_BYTES {
            // Cut on a line boundary inside the tail window.
            let tail = &joined[joined.len() - MAX_COPY_BYTES..];
            let start = tail.find('\n').map(|i| i + 1).unwrap_or(0);
            tail[start..].to_string()
        } else {
            joined
        };
        self.status = format!("copied {} lines to the clipboard", payload.lines().count());
        payload
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn render(&self, w: i32, h: i32) -> CellBuffer {
        let t = crate::theme::current();
        let mut buf = CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
        // Header: path + key hints.
        let path = log_path().map(|p| p.display().to_string()).unwrap_or_default();
        let head: String = format!(" {path}").chars().take(w as usize).collect();
        buf.write_str(0, 0, &head, t.accent, t.window_bg);
        let hints = "[c] copy  [r] refresh  [Esc] close ";
        let hx = (w - hints.chars().count() as i32).max(0);
        if hx > head.chars().count() as i32 + 2 {
            buf.write_str(hx, 0, hints, t.dim, t.window_bg);
        }

        let rows = (h - 2).max(1) as usize;
        self.page.set(rows);
        let max_top = self.lines.len().saturating_sub(rows);
        let top = self.scroll.min(max_top);
        for (i, line) in self.lines.iter().skip(top).take(rows).enumerate() {
            let text: String = line.chars().take(w as usize - 1).collect();
            buf.write_str(1, 1 + i as i32, &text, t.text, t.window_bg);
        }

        // Footer: status + scroll position.
        let pos = if self.lines.len() <= rows {
            "all".to_string()
        } else {
            format!("{}–{}/{}", top + 1, (top + rows).min(self.lines.len()), self.lines.len())
        };
        let foot: String = format!(" {}  {}", self.status, pos).chars().take(w as usize).collect();
        buf.write_str(0, h - 1, &foot, t.dim, t.window_bg);
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_lines(n: usize) -> LogsView {
        let mut v = LogsView {
            lines: (0..n).map(|i| format!("line {i}")).collect(),
            scroll: usize::MAX,
            follow: true,
            status: String::new(),
            page: std::cell::Cell::new(10),
        };
        v.page.set(10);
        let _ = &mut v;
        v
    }

    #[test]
    fn scrolling_clamps_and_tracks_follow() {
        let mut v = with_lines(100);
        v.scroll_by(0); // clamp from the follow sentinel
        assert_eq!(v.scroll, 90, "starts at the bottom");
        assert!(v.follow);
        v.scroll_by(-30);
        assert_eq!(v.scroll, 60);
        assert!(!v.follow, "scrolling up leaves follow mode");
        v.scroll_by(1000);
        assert_eq!(v.scroll, 90, "clamped to the last page");
        assert!(v.follow, "hitting bottom re-enables follow");
    }

    #[test]
    fn copy_payload_caps_on_line_boundary() {
        let mut v = with_lines(20_000);
        let s = v.copy_payload();
        assert!(s.len() <= super::MAX_COPY_BYTES);
        assert!(s.starts_with("line "), "starts on a whole line: {:?}", &s[..20]);
        assert!(v.status().contains("copied"));
    }

    #[test]
    fn render_shows_tail_when_following() {
        let v = with_lines(100);
        let buf = v.render(60, 12);
        // 10 content rows → last lines 90..99 visible; check the last one.
        let row: String = (0..60).filter_map(|x| buf.get(x, 10).map(|c| c.ch)).collect();
        assert!(row.contains("line 99"), "{row:?}");
    }
}
