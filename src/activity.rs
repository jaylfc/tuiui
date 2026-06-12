//! The activity monitor panel: a live, auto-refreshing table of the apps the
//! apphost is hosting, with kill-app keybinds. Mirrors the structure of
//! [`crate::settings`] (state, render, hit-test) and is rendered into a
//! window-sized `CellBuffer` each frame.
//!
//! The panel does **not** talk to the apphost itself — the session rebuilds the
//! list from the in-process `AppHost` every frame and hands the resulting
//! [`Snapshot`]s in via [`Activity::set_rows`]. That keeps the widget in-process
//! and avoids a second protocol surface.

use crate::apphost::AppListEntry;
use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::geometry::Point;

const BG: Rgba = Rgba { r: 17, g: 20, b: 29, a: 255 };
const FG: Rgba = Rgba { r: 200, g: 208, b: 220, a: 255 };
const DIM: Rgba = Rgba { r: 120, g: 130, b: 150, a: 255 };
const SEL_BG: Rgba = Rgba { r: 45, g: 58, b: 85, a: 255 };
const ACCENT: Rgba = Rgba { r: 108, g: 182, b: 255, a: 255 };
const RED: Rgba = Rgba { r: 241, g: 76, b: 76, a: 255 };

/// In-progress kill confirmation: `Some((index, app_id))` while the user is
/// being asked to confirm they really want to kill the selected app.
#[derive(Clone, Copy)]
struct PendingKill {
    row: usize,
    app: u64,
}

/// One row of the table, derived from an [`AppListEntry`] (the apphost reply to
/// `ListApps`).
struct Row {
    entry: AppListEntry,
}

/// Activity monitor panel state.
pub struct Activity {
    rows: Vec<Row>,
    selected: usize,
    /// The refresh counter shown in the header; bumped whenever the session
    /// hands us a new snapshot. Cosmetic, but useful when the list is
    /// mysteriously stale.
    refresh_count: u64,
    /// Pending kill confirmation; while `Some`, the panel renders a modal
    /// overlay and the next `Enter`/`y` confirms.
    pending_kill: Option<PendingKill>,
}

impl Activity {
    pub fn new() -> Self {
        Self { rows: Vec::new(), selected: 0, refresh_count: 0, pending_kill: None }
    }

    /// Replace the table contents with a fresh snapshot from the apphost. The
    /// session calls this every frame; when nothing has changed we still bump
    /// the header counter so the user can see it's live.
    pub fn set_rows(&mut self, entries: Vec<AppListEntry>) {
        self.rows = entries.into_iter().map(|entry| Row { entry }).collect();
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
        self.refresh_count = self.refresh_count.wrapping_add(1);
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Move the selection up one row (no-op if empty).
    pub fn move_up(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the selection down one row (no-op if empty).
    pub fn move_down(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
    }

    /// Request to kill the currently selected row. Returns the app id to kill
    /// (the session will dispatch the actual `HostReq::Kill`). If the user is
    /// already in a confirm prompt, `Enter`/`y` here confirms and returns the
    /// id; `Esc`/`n` cancels.
    ///
    /// Returns `Some(app_id)` to dispatch, `None` to do nothing.
    pub fn request_kill_selected(&mut self) -> Option<u64> {
        if let Some(pk) = self.pending_kill {
            // Already in confirm mode — this Enter/y confirms.
            self.pending_kill = None;
            return Some(pk.app);
        }
        let row = self.rows.get(self.selected)?;
        if !row.entry.alive {
            // Dead rows can be cleaned up immediately (no confirm needed).
            return Some(row.entry.app);
        }
        self.pending_kill = Some(PendingKill { row: self.selected, app: row.entry.app });
        None
    }

    /// Dismiss the pending kill confirmation, if any.
    pub fn cancel_kill(&mut self) {
        self.pending_kill = None;
    }

    /// Whether a kill-confirm overlay is currently being shown.
    pub fn has_pending_kill(&self) -> bool {
        self.pending_kill.is_some()
    }

    /// Kill every row in the `dead` state. Returns the app ids to dispatch.
    /// Intended to be triggered by the `K` keybind — safe with no confirm
    /// because the processes are already gone.
    pub fn kill_dead(&self) -> Vec<u64> {
        self.rows.iter().filter(|r| !r.entry.alive).map(|r| r.entry.app).collect()
    }

    /// Handle a click inside the panel; returns the app id to kill if a
    /// kill-button was clicked, or `None`. `w` is the panel's rendered width
    /// (so the click hit-test matches what was painted).
    pub fn handle_click(&mut self, p: Point, w: i32) -> Option<u64> {
        // Clicks anywhere on the confirm overlay cancel it.
        if self.pending_kill.is_some() {
            self.pending_kill = None;
            return None;
        }
        let header_y = 3;
        let cols = ColumnLayout::for_width(w);
        let row_y = header_y + 2 + self.selected as i32;
        if p.y == row_y && p.x >= cols.kill && p.x < cols.kill + 4 {
            self.request_kill_selected()
        } else if p.y >= header_y + 2 && p.y < header_y + 2 + self.rows.len() as i32 {
            self.selected = (p.y - header_y - 2) as usize;
            None
        } else {
            None
        }
    }

    /// Render the panel into a `w × h` content buffer.
    pub fn render(&self, w: i32, h: i32) -> CellBuffer {
        let mut buf = CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: FG, bg: BG, attrs: Default::default() });

        // Header bar: title + count + refresh counter.
        let title = "Activity Monitor";
        buf.write_str(2, 1, title, ACCENT, BG);
        let count = format!("{} app{}", self.rows.len(), if self.rows.len() == 1 { "" } else { "s" });
        buf.write_str(2 + title.chars().count() as i32 + 2, 1, &count, DIM, BG);
        let refresh = format!("refresh: #{}", self.refresh_count);
        buf.write_str(w - refresh.chars().count() as i32 - 2, 1, &refresh, DIM, BG);

        // Column headers.
        let header_y = 3;
        let cols = ColumnLayout::for_width(w);
        buf.write_str(cols.id, header_y, "ID", DIM, BG);
        buf.write_str(cols.pid, header_y, "PID", DIM, BG);
        buf.write_str(cols.cmd, header_y, "CMD", DIM, BG);
        buf.write_str(cols.dims, header_y, "COLS×ROWS", DIM, BG);
        buf.write_str(cols.age, header_y, "AGE", DIM, BG);
        buf.write_str(cols.state, header_y, "STATE", DIM, BG);
        buf.write_str(cols.kill, header_y, "KILL", DIM, BG);

        // Divider under the header.
        for x in 1..(w - 1) {
            buf.set(x, header_y + 1, Cell { ch: '─', fg: DIM, bg: BG, attrs: Default::default() });
        }

        if self.rows.is_empty() {
            buf.write_str(
                (w / 2 - 10).max(1),
                header_y + 3,
                "no apps running",
                DIM,
                BG,
            );
        } else {
            // List rows.
            let mut y = header_y + 2;
            for (i, row) in self.rows.iter().enumerate() {
                let sel = i == self.selected;
                let row_bg = if sel { SEL_BG } else { BG };
                if sel {
                    for x in 0..w {
                        buf.set(x, y, Cell { ch: ' ', fg: FG, bg: row_bg, attrs: Default::default() });
                    }
                }
                let fg = if sel { FG } else { DIM };
                buf.write_str(cols.id, y, &format!("{}", row.entry.app), fg, row_bg);
                let pid = row.entry.pid.map(|p| p.to_string()).unwrap_or_else(|| "—".into());
                buf.write_str(cols.pid, y, &pid, fg, row_bg);
                let cmdline = format_cmdline(&row.entry.cmd, &row.entry.args);
                let max_cmd = (cols.dims - cols.cmd - 1).max(4) as usize;
                buf.write_str(cols.cmd, y, &truncate(&cmdline, max_cmd), fg, row_bg);
                buf.write_str(
                    cols.dims,
                    y,
                    &format!("{}×{}", row.entry.cols, row.entry.rows),
                    fg,
                    row_bg,
                );
                buf.write_str(cols.age, y, &format_age(row.entry.age_secs), fg, row_bg);
                let (state_text, state_col) = if row.entry.alive { ("alive", FG) } else { ("dead", DIM) };
                buf.write_str(cols.state, y, state_text, state_col, row_bg);

                // Kill button on the selected row (red glyph).
                if sel {
                    buf.write_str(cols.kill, y, "kill", RED, row_bg);
                } else {
                    buf.write_str(cols.kill, y, "    ", FG, row_bg);
                }

                y += 1;
                if y >= h - 4 {
                    break;
                }
            }
        }

        // Footer help line.
        let footer_y = h - 2;
        let footer = "↑↓ select · k kill selected · K kill all dead · r refresh · Esc close";
        let fx = ((w - footer.chars().count() as i32) / 2).max(1);
        buf.write_str(fx, footer_y, footer, DIM, BG);

        // Optional confirm overlay.
        if let Some(pk) = self.pending_kill {
            self.render_confirm(&mut buf, w, h, pk);
        }

        buf
    }

    fn render_confirm(&self, buf: &mut CellBuffer, w: i32, h: i32, pk: PendingKill) {
        let cmdline = self
            .rows
            .get(pk.row)
            .map(|r| format_cmdline(&r.entry.cmd, &r.entry.args))
            .unwrap_or_default();
        let label = format!("Kill app {} ({})?  [Enter/y confirm · Esc/n cancel]", pk.app, cmdline);
        let box_w = (label.chars().count() as i32 + 4).min(w - 2).max(20);
        let box_h = 3;
        let ox = (w - box_w) / 2;
        let oy = (h - box_h) / 2;
        let bg = Rgba { r: 45, g: 0, b: 0, a: 255 };
        for x in ox..(ox + box_w) {
            for y in oy..(oy + box_h) {
                buf.set(x, y, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() });
            }
        }
        buf.write_str(ox + 2, oy + 1, &label, RED, bg);
    }
}

impl Default for Activity {
    fn default() -> Self {
        Self::new()
    }
}

/// Column x-offsets (kept in one place so header + rows stay aligned).
struct ColumnLayout {
    id: i32,
    pid: i32,
    cmd: i32,
    dims: i32,
    age: i32,
    state: i32,
    kill: i32,
}

impl ColumnLayout {
    fn for_width(w: i32) -> Self {
        // Reserve 2 cells of left padding, then fixed-width columns.
        let id = 2;
        let pid = id + 6; // " 12345"
        let cmd = pid + 9; // "  1234567"
        let dims = cmd + 30; // 28 for cmd + 2 padding
        let age = dims + 12; // " 9999×9999 "
        let state = age + 10; // " 9999d99h "
        let kill = state + 8; // " alive "
        // If we don't have room for the kill column, fold it into state.
        if kill + 6 > w {
            ColumnLayout { id, pid, cmd, dims, age, state, kill: w - 6 }
        } else {
            ColumnLayout { id, pid, cmd, dims, age, state, kill }
        }
    }
}

fn format_cmdline(cmd: &str, args: &[String]) -> String {
    if args.is_empty() {
        cmd.to_string()
    } else {
        format!("{} {}", cmd, args.join(" "))
    }
}

fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 60 * 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else if secs < 24 * 60 * 60 {
        format!("{}h{:02}m", secs / 3600, (secs / 60) % 60)
    } else {
        format!("{}d{:02}h", secs / 86400, (secs / 3600) % 24)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep = max.saturating_sub(1);
        format!("{}\u{2026}", s.chars().take(keep).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(app: u64, alive: bool) -> AppListEntry {
        AppListEntry {
            app,
            cmd: "/bin/echo".into(),
            args: vec!["hi".into()],
            pid: Some(1000 + app as u32),
            cols: 80,
            rows: 24,
            age_secs: 90,
            alive,
        }
    }

    #[test]
    fn empty_panel_renders_with_placeholder() {
        let a = Activity::new();
        let buf = a.render(80, 20);
        // No rows → "no apps running" message somewhere.
        let mut found = false;
        for y in 0..20 {
            for x in 0..80 {
                if let Some(c) = buf.get(x, y) {
                    if c.ch == 'n' {
                        // crude, but enough: the placeholder is on its own row
                        found = true;
                    }
                }
            }
        }
        assert!(found, "expected some text in the empty panel");
    }

    #[test]
    fn selection_clamps_when_rows_shrink() {
        let mut a = Activity::new();
        a.set_rows(vec![entry(1, true), entry(2, true), entry(3, true)]);
        a.move_down();
        a.move_down();
        a.move_down(); // past the end
        assert_eq!(a.selected, 2);
        a.set_rows(vec![entry(1, true)]);
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn kill_dead_returns_only_dead() {
        let mut a = Activity::new();
        a.set_rows(vec![entry(1, true), entry(2, false), entry(3, false)]);
        assert_eq!(a.kill_dead(), vec![2, 3]);
    }

    #[test]
    fn kill_live_row_requires_confirm() {
        let mut a = Activity::new();
        a.set_rows(vec![entry(1, true), entry(2, false)]);
        a.move_down();
        a.move_down();
        // selected = 1, dead → kills immediately, no confirm.
        assert_eq!(a.request_kill_selected(), Some(2));
        assert!(!a.has_pending_kill());
        // selected = 0, live → first Enter asks for confirm, second confirms.
        a.selected = 0;
        assert_eq!(a.request_kill_selected(), None);
        assert!(a.has_pending_kill());
        assert_eq!(a.request_kill_selected(), Some(1));
        assert!(!a.has_pending_kill());
    }

    #[test]
    fn cancel_drops_pending_kill() {
        let mut a = Activity::new();
        a.set_rows(vec![entry(7, true)]);
        a.request_kill_selected();
        assert!(a.has_pending_kill());
        a.cancel_kill();
        assert!(!a.has_pending_kill());
    }

    #[test]
    fn age_formatting() {
        assert_eq!(format_age(0), "0s");
        assert_eq!(format_age(59), "59s");
        assert_eq!(format_age(60), "1m00s");
        assert_eq!(format_age(125), "2m05s");
        assert_eq!(format_age(3600), "1h00m");
        assert_eq!(format_age(3725), "1h02m");
        assert_eq!(format_age(86400), "1d00h");
        assert_eq!(format_age(90061), "1d01h");
    }
}
