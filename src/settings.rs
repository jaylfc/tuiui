//! The settings panel: a native window with a category sidebar and a content
//! pane of editable settings, backed by [`Config`]. Changes are applied live and
//! persisted to `config.toml` by the session.

use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::config::Config;
use crate::geometry::Point;

const BG: Rgba = Rgba { r: 17, g: 20, b: 29, a: 255 };
const FG: Rgba = Rgba { r: 200, g: 208, b: 220, a: 255 };
const DIM: Rgba = Rgba { r: 120, g: 130, b: 150, a: 255 };
const SEL_BG: Rgba = Rgba { r: 45, g: 58, b: 85, a: 255 };
const ACCENT: Rgba = Rgba { r: 108, g: 182, b: 255, a: 255 };
const GREEN: Rgba = Rgba { r: 126, g: 231, b: 135, a: 255 };

const SIDEBAR_W: i32 = 18;
const SECTIONS: &[&str] = &["Windows", "Appearance", "Updates", "About"];

/// An action the session must perform on behalf of the settings panel
/// (these touch the network / spawn processes, so the panel only requests them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Check upstream for a newer commit.
    CheckUpdates,
    /// Install the latest version.
    InstallUpdate,
}

/// Settings panel state (owns a working copy of the config).
pub struct Settings {
    cfg: Config,
    section: usize,
    sel: usize,
    action: Option<SettingsAction>,
    update_status: String,
}

impl Settings {
    /// Create a settings panel editing a copy of `cfg`.
    pub fn new(cfg: Config) -> Self {
        Self { cfg, section: 0, sel: 0, action: None, update_status: String::new() }
    }

    /// Take a pending action requested by the user (cleared on read).
    pub fn take_action(&mut self) -> Option<SettingsAction> {
        self.action.take()
    }

    /// Set the text shown under the Updates section after a check.
    pub fn set_update_status(&mut self, s: String) {
        self.update_status = s;
    }

    /// The live (edited) config.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Number of interactive rows in the current section.
    fn item_count(&self) -> usize {
        match self.section {
            0 => 2, // snapping, threshold
            1 => 2, // shadows, theme
            2 => 2, // check, install
            _ => 0, // About
        }
    }

    pub fn move_up(&mut self) {
        self.sel = self.sel.saturating_sub(1);
    }
    pub fn move_down(&mut self) {
        if self.sel + 1 < self.item_count() {
            self.sel += 1;
        }
    }
    pub fn prev_section(&mut self) {
        if self.section > 0 {
            self.section -= 1;
            self.sel = 0;
        }
    }
    pub fn next_section(&mut self) {
        if self.section + 1 < SECTIONS.len() {
            self.section += 1;
            self.sel = 0;
        }
    }

    /// Toggle / activate the selected row.
    pub fn toggle(&mut self) {
        self.adjust(0);
    }
    /// Decrease / turn off the selected row.
    pub fn left(&mut self) {
        self.adjust(-1);
    }
    /// Increase / turn on the selected row.
    pub fn right(&mut self) {
        self.adjust(1);
    }

    /// Apply a change to the selected setting. `dir`: 0 = toggle, -1/+1 = down/up.
    fn adjust(&mut self, dir: i32) {
        match (self.section, self.sel) {
            (0, 0) => self.cfg.snapping_enabled = flip(self.cfg.snapping_enabled, dir),
            (0, 1) => {
                self.cfg.snap_threshold = match dir {
                    -1 => (self.cfg.snap_threshold - 1).max(1),
                    1 => (self.cfg.snap_threshold + 1).min(10),
                    // Enter/space on a number wraps 1..=10.
                    _ => if self.cfg.snap_threshold >= 10 { 1 } else { self.cfg.snap_threshold + 1 },
                };
            }
            (1, 0) => self.cfg.window_shadows = flip(self.cfg.window_shadows, dir),
            (1, 1) => {
                let presets = crate::theme::PRESETS;
                let cur_idx = presets.iter().position(|&p| p == self.cfg.theme.as_str()).unwrap_or(0);
                let next_idx = if dir == -1 {
                    (cur_idx + presets.len() - 1) % presets.len()
                } else {
                    (cur_idx + 1) % presets.len()
                };
                self.cfg.theme = presets[next_idx].to_string();
            }
            // Updates section: Enter/Space (dir 0) requests an action from the session.
            (2, 0) if dir == 0 => self.action = Some(SettingsAction::CheckUpdates),
            (2, 1) if dir == 0 => self.action = Some(SettingsAction::InstallUpdate),
            _ => {}
        }
    }

    /// Handle a content-local click; returns `true` if a setting changed.
    pub fn handle_click(&mut self, p: Point, _w: i32, _h: i32) -> bool {
        if p.x < SIDEBAR_W {
            let i = (p.y - 1) as usize;
            if i < SECTIONS.len() {
                self.section = i;
                self.sel = 0;
            }
            return false;
        }
        let row = p.y - 3;
        if row >= 0 && (row as usize) < self.item_count() {
            self.sel = row as usize;
            self.toggle();
            return true;
        }
        false
    }

    /// Render the panel into a `w × h` content buffer.
    pub fn render(&self, w: i32, h: i32) -> CellBuffer {
        let mut buf = CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: FG, bg: BG, attrs: Default::default() });

        // Sidebar.
        for (i, s) in SECTIONS.iter().enumerate() {
            let y = 1 + i as i32;
            let sel = i == self.section;
            let (fg, bg) = if sel { (ACCENT, SEL_BG) } else { (DIM, BG) };
            for x in 0..SIDEBAR_W {
                buf.set(x, y, Cell { ch: ' ', fg, bg, attrs: Default::default() });
            }
            buf.write_str(2, y, s, fg, bg);
        }
        for y in 0..h {
            buf.set(SIDEBAR_W, y, Cell { ch: '\u{2502}', fg: DIM, bg: BG, attrs: Default::default() });
        }

        let cx = SIDEBAR_W + 2;
        buf.write_str(cx, 1, SECTIONS[self.section], ACCENT, BG);

        match self.section {
            0 => {
                self.row(&mut buf, cx, 3, 0, "Drag-to-edge snapping", toggle_val(self.cfg.snapping_enabled));
                self.row(&mut buf, cx, 4, 1, "Snap threshold (cells)", format!("\u{25C2} {} \u{25B8}", self.cfg.snap_threshold));
            }
            1 => {
                self.row(&mut buf, cx, 3, 0, "Window shadows", toggle_val(self.cfg.window_shadows));
                self.row(&mut buf, cx, 4, 1, "Theme", self.cfg.theme.clone());
            }
            2 => {
                self.row(&mut buf, cx, 3, 0, "Check for updates", String::new());
                self.row(&mut buf, cx, 4, 1, "Update tuiui now", String::new());
                let sha = &crate::GIT_SHA[..crate::GIT_SHA.len().min(7)];
                buf.write_str(cx, 6, &format!("installed: v{} ({})", crate::VERSION, sha), DIM, BG);
                if !self.update_status.is_empty() {
                    let col = if self.update_status.contains("Update available") { GREEN } else { DIM };
                    buf.write_str(cx, 7, &self.update_status, col, BG);
                }
            }
            _ => {
                buf.write_str(cx, 3, "tuiui — a desktop environment for the terminal", FG, BG);
                buf.write_str(cx, 5, "Settings are saved to ~/.config/tuiui/config.toml", DIM, BG);
                buf.write_str(cx, 6, "github.com/jaylfc/tuiui", DIM, BG);
            }
        }
        buf
    }

    fn row(&self, buf: &mut CellBuffer, x: i32, y: i32, idx: usize, label: &str, value: String) {
        let sel = idx == self.sel && self.item_count() > 0;
        let marker = if sel { "\u{25B8} " } else { "  " };
        buf.write_str(x, y, marker, ACCENT, BG);
        buf.write_str(x + 2, y, label, if sel { FG } else { DIM }, BG);
        let vx = x + 30;
        let vcol = if value.contains("on") { GREEN } else { FG };
        buf.write_str(vx, y, &value, vcol, BG);
    }
}

fn flip(current: bool, dir: i32) -> bool {
    match dir {
        0 => !current,
        d => d > 0,
    }
}

fn toggle_val(on: bool) -> String {
    if on { "\u{25CF} on".into() } else { "\u{25CB} off".into() }
}
