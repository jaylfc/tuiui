//! The launch-warning dialog: a modal "are you sure?" shown before launching an
//! app entry that carries a `warn` message (see [`crate::config::AppEntry::warn`]
//! / [`crate::catalog::Variant::warn`]) — e.g. a catalog variant that skips a
//! tool's own safety prompts. Confirming runs the launch exactly as it would
//! have without the prompt; cancelling drops it.
//!
//! Like [`crate::confirmclose`], the widget is pure UI state + geometry — it
//! never touches the session or spawns anything itself. A confirmed click/key
//! hands the pending launch back to the caller to actually run.

use crate::buffer::CellBuffer;
use crate::cell::Cell;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};

/// The launch the dialog is guarding, captured before the warning is shown so
/// confirming can run it exactly as `launch_entry` would have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingLaunch {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Whether the entry is CLI-flagged (needs the help-then-shell wrapper).
    pub cli: bool,
    pub requires_cwd: bool,
    pub cwd: Option<String>,
}

/// Modal launch-warning dialog. Holds the pending launch and its warning
/// message, or `None` when nothing is pending.
#[derive(Default)]
pub struct LaunchWarn {
    pending: Option<(PendingLaunch, String)>,
}

// ── Geometry (shared by render + hit-test so they can never drift) ──────────────

/// The centered dialog box. Taller/wider than the confirm-close box so a
/// multi-line custom warning fits.
fn dialog_rect(w: i32, h: i32) -> Rect {
    let box_w = 60.min((w - 2).max(24));
    let box_h = 9.min((h - 1).max(7));
    Rect::new((w - box_w) / 2, ((h - box_h) / 2).max(0), box_w, box_h)
}

/// Screen rects of the dialog's `(Cancel, Launch)` buttons.
fn dialog_buttons(w: i32, h: i32) -> (Rect, Rect) {
    let d = dialog_rect(w, h);
    let by = d.y + d.h - 2;
    let cancel = Rect::new(d.x + 2, by, 10, 1);
    let launch_w = 14;
    let launch = Rect::new(d.x + d.w - 2 - launch_w, by, launch_w, 1);
    (cancel, launch)
}

/// Word-wrap `msg` to lines of at most `width` chars, never splitting a word
/// (an over-long single word still gets its own line rather than panicking on
/// a zero-progress loop).
fn wrap_lines(msg: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in msg.split_whitespace() {
        let extra = if cur.is_empty() { 0 } else { 1 };
        if !cur.is_empty() && cur.chars().count() + extra + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

impl LaunchWarn {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the dialog is showing. While true it is modal and the session
    /// routes clicks/keys to it.
    pub fn is_open(&self) -> bool {
        self.pending.is_some()
    }

    /// Open the dialog for `launch`, showing `message`.
    pub fn open(&mut self, launch: PendingLaunch, message: String) {
        self.pending = Some((launch, message));
    }

    /// Dismiss without launching.
    pub fn close(&mut self) {
        self.pending = None;
    }

    /// Route a click while open. Returns the launch to run when the Launch
    /// button is hit; otherwise the click is consumed (Cancel, or a click
    /// outside the box, dismisses the dialog).
    pub fn on_click(&mut self, p: Point, w: i32, h: i32) -> Option<PendingLaunch> {
        self.pending.as_ref()?;
        let (cancel, launch) = dialog_buttons(w, h);
        if launch.contains(p) {
            return self.confirm();
        }
        if cancel.contains(p) || !dialog_rect(w, h).contains(p) {
            self.close();
        }
        None
    }

    /// Keyboard confirm (Enter / y). Returns the launch to run.
    pub fn confirm(&mut self) -> Option<PendingLaunch> {
        self.pending.take().map(|(l, _)| l)
    }

    /// Keyboard cancel (Esc / n). Dismisses without launching.
    pub fn cancel(&mut self) {
        self.close();
    }

    /// Compositor layer for the dialog (empty when closed). Rendered above the
    /// confirm-close / compat dialogs — the topmost modal of all.
    pub fn render(&self, w: i32, h: i32) -> Vec<Layer> {
        let Some((_, message)) = &self.pending else {
            return Vec::new();
        };
        let t = crate::theme::current();
        let d = dialog_rect(w, h);
        let mut buf = CellBuffer::new(d.w, d.h);
        buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
        // Title bar.
        for x in 0..d.w {
            buf.set(x, 0, Cell { ch: ' ', fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
        }
        buf.write_str(2, 0, " tuiui ", t.title_fg, t.title_focus);
        // Border.
        let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
        for y in 1..d.h {
            buf.set(0, y, b('│'));
            buf.set(d.w - 1, y, b('│'));
        }
        for x in 0..d.w {
            buf.set(x, d.h - 1, b('─'));
        }
        buf.set(0, d.h - 1, b('╰'));
        buf.set(d.w - 1, d.h - 1, b('╯'));
        // Message: word-wrapped to the box width, truncated (with an ellipsis on
        // the last shown line) if it still doesn't fit.
        let avail_w = (d.w as usize).saturating_sub(4);
        let max_lines = (d.h as usize).saturating_sub(4).max(1);
        let mut lines = wrap_lines(message, avail_w);
        let truncated = lines.len() > max_lines;
        lines.truncate(max_lines);
        if truncated {
            if let Some(last) = lines.last_mut() {
                let budget = avail_w.saturating_sub(1);
                let mut s: String = last.chars().take(budget).collect();
                s.push('…');
                *last = s;
            }
        }
        for (i, line) in lines.iter().enumerate() {
            buf.write_str(2, 2 + i as i32, line, t.text, t.window_bg);
        }
        // Buttons (local coords; pad labels to fill their hit rects).
        let (cancel, launch) = dialog_buttons(w, h);
        let cancel_s = format!("{:^width$}", "Cancel", width = cancel.w as usize);
        buf.write_str(cancel.x - d.x, cancel.y - d.y, &cancel_s, t.text, t.active_bg);
        let launch_s = format!("{:^width$}", "Launch", width = launch.w as usize);
        buf.write_str(launch.x - d.x, launch.y - d.y, &launch_s, t.close_fg, t.accent);
        vec![Layer { z: 6600, origin: Point::new(d.x, d.y), buf, opacity: 1.0, scissor: None }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(r: Rect) -> Point {
        Point::new(r.x + r.w / 2, r.y + r.h / 2)
    }

    fn launch(name: &str) -> PendingLaunch {
        PendingLaunch {
            name: name.into(),
            command: name.into(),
            args: vec![],
            cli: false,
            requires_cwd: false,
            cwd: None,
        }
    }

    #[test]
    fn opens_and_reports_pending() {
        let mut lw = LaunchWarn::new();
        assert!(!lw.is_open());
        lw.open(launch("claude"), "careful?".into());
        assert!(lw.is_open());
    }

    #[test]
    fn click_launch_returns_pending_and_dismisses() {
        let (w, h) = (120, 40);
        let mut lw = LaunchWarn::new();
        lw.open(launch("claude"), "careful?".into());
        let (_, launch_btn) = dialog_buttons(w, h);
        let got = lw.on_click(center(launch_btn), w, h);
        assert_eq!(got.map(|l| l.name), Some("claude".into()));
        assert!(!lw.is_open());
    }

    #[test]
    fn click_cancel_dismisses_without_launching() {
        let (w, h) = (120, 40);
        let mut lw = LaunchWarn::new();
        lw.open(launch("claude"), "careful?".into());
        let (cancel, _) = dialog_buttons(w, h);
        assert_eq!(lw.on_click(center(cancel), w, h), None);
        assert!(!lw.is_open());
    }

    #[test]
    fn click_outside_dismisses_without_launching() {
        let (w, h) = (120, 40);
        let mut lw = LaunchWarn::new();
        lw.open(launch("claude"), "careful?".into());
        assert_eq!(lw.on_click(Point::new(0, 0), w, h), None);
        assert!(!lw.is_open());
    }

    #[test]
    fn keyboard_confirm_and_cancel() {
        let mut lw = LaunchWarn::new();
        lw.open(launch("claude"), "careful?".into());
        assert_eq!(lw.confirm().map(|l| l.name), Some("claude".into()));
        assert!(!lw.is_open());

        lw.open(launch("claude"), "careful?".into());
        lw.cancel();
        assert!(!lw.is_open());
    }

    #[test]
    fn render_is_empty_when_closed_and_present_when_open() {
        let lw0 = LaunchWarn::new();
        assert!(lw0.render(120, 40).is_empty());
        let mut lw = LaunchWarn::new();
        lw.open(launch("claude"), "Runs Claude Code with --dangerously-skip-permissions: it can edit files and run commands without asking. Launch anyway?".into());
        assert_eq!(lw.render(120, 40).len(), 1);
    }

    #[test]
    fn wrap_lines_never_splits_a_word_and_respects_width() {
        let msg = "Runs Claude Code with a flag that can edit files and run commands";
        let lines = wrap_lines(msg, 20);
        for line in &lines {
            assert!(line.chars().count() <= 20, "line too wide: {line:?}");
        }
        // Rejoining the wrapped lines reproduces every original word in order.
        let rejoined: Vec<&str> = lines.iter().flat_map(|l| l.split_whitespace()).collect();
        let original: Vec<&str> = msg.split_whitespace().collect();
        assert_eq!(rejoined, original);
    }

    #[test]
    fn wrap_lines_gives_an_over_long_word_its_own_line() {
        let lines = wrap_lines("a-word-that-is-way-longer-than-the-configured-width right", 10);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], "right");
    }
}
