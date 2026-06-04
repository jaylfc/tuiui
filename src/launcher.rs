//! The app launcher: a menubar dropdown (mouse) and a Spotlight overlay
//! (keyboard), sharing one app list and selection state.
//!
//! [`Launcher`] owns its open/closed state, the typed query (Spotlight), and the
//! highlighted index. It renders itself to compositor [`Layer`]s and reports
//! clickable hit regions, so the session core just forwards events and launches
//! whatever entry the launcher resolves.

use crate::buffer::CellBuffer;
use crate::cell::{Cell, Rgba};
use crate::compositor::Layer;
use crate::config::AppEntry;
use crate::geometry::{Point, Rect};

const MENU_BG: Rgba = Rgba { r: 24, g: 28, b: 40, a: 255 };
const MENU_FG: Rgba = Rgba { r: 200, g: 208, b: 220, a: 255 };
const SEL_BG: Rgba = Rgba { r: 45, g: 58, b: 85, a: 255 };
const SEL_FG: Rgba = Rgba { r: 207, g: 224, b: 255, a: 255 };
const BORDER: Rgba = Rgba { r: 58, g: 68, b: 88, a: 255 };
const HINT: Rgba = Rgba { r: 120, g: 130, b: 150, a: 255 };
const ACCENT: Rgba = Rgba { r: 108, g: 182, b: 255, a: 255 };

/// How the launcher is currently presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherMode {
    /// Apple-menu-style dropdown anchored under the menubar brand (mouse-first).
    Menu,
    /// Centered, filterable search overlay (keyboard-first).
    Spotlight,
}

/// The launcher widget: state + rendering + hit-testing.
pub struct Launcher {
    items: Vec<AppEntry>,
    open: Option<LauncherMode>,
    query: String,
    /// Highlighted row index into the currently *filtered* list.
    selected: usize,
}

/// A rendered launcher frame: layers plus the clickable regions for this frame.
pub struct Rendered {
    /// Compositor layers (drawn above all chrome).
    pub layers: Vec<Layer>,
    /// `(entry, screen_rect)` for each visible, clickable app row.
    pub items: Vec<(AppEntry, Rect)>,
}

impl Launcher {
    /// Create a launcher offering `items`, initially closed.
    pub fn new(items: Vec<AppEntry>) -> Self {
        Self { items, open: None, query: String::new(), selected: 0 }
    }

    /// Whether the launcher is currently visible.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The current presentation mode, if open.
    pub fn mode(&self) -> Option<LauncherMode> {
        self.open
    }

    /// Open (or, if already in `Menu`, close) the dropdown menu.
    pub fn toggle_menu(&mut self) {
        self.open = if self.open == Some(LauncherMode::Menu) { None } else { Some(LauncherMode::Menu) };
        self.reset_selection();
    }

    /// Open (or, if already in `Spotlight`, close) the search overlay.
    pub fn toggle_spotlight(&mut self) {
        self.open = if self.open == Some(LauncherMode::Spotlight) { None } else { Some(LauncherMode::Spotlight) };
        self.reset_selection();
    }

    /// Close the launcher.
    pub fn close(&mut self) {
        self.open = None;
        self.reset_selection();
    }

    fn reset_selection(&mut self) {
        self.query.clear();
        self.selected = 0;
    }

    /// Append a character to the Spotlight query.
    pub fn type_char(&mut self, c: char) {
        if self.open == Some(LauncherMode::Spotlight) {
            self.query.push(c);
            self.selected = 0;
        }
    }

    /// Delete the last character of the Spotlight query.
    pub fn backspace(&mut self) {
        if self.open == Some(LauncherMode::Spotlight) {
            self.query.pop();
            self.selected = 0;
        }
    }

    /// Move the highlight up (saturating).
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the highlight down (clamped to the filtered list).
    pub fn move_down(&mut self) {
        let n = self.filtered().len();
        if n > 0 && self.selected + 1 < n {
            self.selected += 1;
        }
    }

    /// The apps matching the current query (all apps in `Menu` mode), sorted by
    /// category then name so groups are contiguous for header rendering.
    pub fn filtered(&self) -> Vec<AppEntry> {
        let mut v: Vec<AppEntry> = if self.query.is_empty() {
            self.items.clone()
        } else {
            let q = self.query.to_lowercase();
            self.items.iter().filter(|a| a.name.to_lowercase().contains(&q)).cloned().collect()
        };
        v.sort_by(|a, b| {
            cat_of(a).cmp(&cat_of(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        v
    }

    /// The highlighted entry (for Enter in Spotlight).
    pub fn selected_entry(&self) -> Option<AppEntry> {
        self.filtered().into_iter().nth(self.selected)
    }

    /// Render the launcher for a `w × h` screen.
    pub fn render(&self, w: i32, h: i32) -> Rendered {
        match self.open {
            Some(LauncherMode::Menu) => self.render_menu(w, h),
            Some(LauncherMode::Spotlight) => self.render_spotlight(w, h),
            None => Rendered { layers: Vec::new(), items: Vec::new() },
        }
    }

    /// Build the ordered list of rendered rows (category headers + item rows).
    fn rows(&self, filtered: &[AppEntry]) -> Vec<Row> {
        let mut out = Vec::new();
        let mut last: Option<String> = None;
        for (i, e) in filtered.iter().enumerate() {
            let c = cat_of(e);
            if last.as_deref() != Some(c.as_str()) {
                out.push(Row::Header(c.clone()));
                last = Some(c);
            }
            out.push(Row::Item(i));
        }
        out
    }

    fn render_menu(&self, _w: i32, _h: i32) -> Rendered {
        let filtered = self.filtered();
        let rows = self.rows(&filtered);
        let name_w = filtered.iter().map(|e| e.name.chars().count()).max().unwrap_or(8) as i32;
        let inner_w = (name_w + 4).max(16);
        let box_w = inner_w + 2;
        let box_h = rows.len() as i32 + 2;
        let origin = Point::new(0, 1); // directly under the menubar brand

        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h);

        let mut items = Vec::new();
        for (ri, row) in rows.iter().enumerate() {
            let y = 1 + ri as i32;
            match row {
                Row::Header(c) => draw_header(&mut buf, box_w, y, c),
                Row::Item(i) => {
                    let e = &filtered[*i];
                    draw_row(&mut buf, box_w, y, &e.name, *i == self.selected, false);
                    items.push((e.clone(), Rect::new(origin.x + 1, origin.y + y, inner_w, 1)));
                }
            }
        }

        Rendered {
            layers: vec![Layer { z: 5000, origin, buf, opacity: 1.0, scissor: None }],
            items,
        }
    }

    fn render_spotlight(&self, w: i32, _h: i32) -> Rendered {
        let mut filtered = self.filtered();
        filtered.truncate(8);
        let rows = self.rows(&filtered);
        let box_w = 46.min(w - 4).max(24);
        let inner_w = box_w - 2;
        let body = rows.len().max(1) as i32;
        let box_h = body + 4; // top border + query + separator + body + bottom border
        let origin = Point::new((w - box_w) / 2, 3);

        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h);

        // Query row + separator.
        buf.write_str(2, 1, &format!("\u{2318} {}\u{2588}", self.query), ACCENT, MENU_BG);
        for x in 1..box_w - 1 {
            buf.set(x, 2, Cell { ch: '\u{2500}', fg: BORDER, bg: MENU_BG, attrs: Default::default() });
        }

        let mut items = Vec::new();
        if rows.is_empty() {
            buf.write_str(2, 3, "no matches", HINT, MENU_BG);
        }
        for (ri, row) in rows.iter().enumerate() {
            let y = 3 + ri as i32;
            match row {
                Row::Header(c) => draw_header(&mut buf, box_w, y, c),
                Row::Item(i) => {
                    let e = &filtered[*i];
                    draw_row(&mut buf, box_w, y, &e.name, *i == self.selected, true);
                    items.push((e.clone(), Rect::new(origin.x + 1, origin.y + y, inner_w, 1)));
                }
            }
        }

        Rendered {
            layers: vec![Layer { z: 5000, origin, buf, opacity: 1.0, scissor: None }],
            items,
        }
    }
}

/// A rendered launcher row: a category header or an item (index into `filtered`).
enum Row {
    Header(String),
    Item(usize),
}

/// The category an entry belongs to ("Apps" when unset).
fn cat_of(a: &AppEntry) -> String {
    a.category.clone().unwrap_or_else(|| "Apps".into())
}

/// Draw a dimmed, uppercase category header row.
fn draw_header(buf: &mut CellBuffer, w: i32, row: i32, cat: &str) {
    for x in 1..w - 1 {
        buf.set(x, row, Cell { ch: ' ', fg: HINT, bg: MENU_BG, attrs: Default::default() });
    }
    buf.write_str(1, row, &cat.to_uppercase(), HINT, MENU_BG);
}

/// Fill a buffer with the menu background and draw a rounded border.
fn fill_box(buf: &mut CellBuffer, w: i32, h: i32) {
    buf.fill(Cell { ch: ' ', fg: MENU_FG, bg: MENU_BG, attrs: Default::default() });
    let b = |ch: char| Cell { ch, fg: BORDER, bg: MENU_BG, attrs: Default::default() };
    for x in 0..w {
        buf.set(x, 0, b('\u{2500}'));
        buf.set(x, h - 1, b('\u{2500}'));
    }
    for y in 0..h {
        buf.set(0, y, b('\u{2502}'));
        buf.set(w - 1, y, b('\u{2502}'));
    }
    buf.set(0, 0, b('\u{256D}'));
    buf.set(w - 1, 0, b('\u{256E}'));
    buf.set(0, h - 1, b('\u{2570}'));
    buf.set(w - 1, h - 1, b('\u{256F}'));
}

/// Draw one app row, optionally highlighted, with optional `▸` selection marker.
fn draw_row(buf: &mut CellBuffer, w: i32, row: i32, name: &str, highlighted: bool, marker: bool) {
    let (fg, bg) = if highlighted { (SEL_FG, SEL_BG) } else { (MENU_FG, MENU_BG) };
    for x in 1..w - 1 {
        buf.set(x, row, Cell { ch: ' ', fg, bg, attrs: Default::default() });
    }
    let lead = if marker && highlighted { "\u{25B8} " } else { "  " };
    buf.write_str(1, row, lead, ACCENT, bg);
    buf.write_str(3, row, name, fg, bg);
}
