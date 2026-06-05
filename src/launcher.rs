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

    /// Replace the offered app list (e.g. after a new app is installed), keeping
    /// the open/closed state.
    pub fn set_items(&mut self, items: Vec<AppEntry>) {
        self.items = items;
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
            let (ca, cb) = (cat_of(a), cat_of(b));
            // Pin the "tuiui" section (Store/Settings) to the very top.
            let rank = |c: &str| if c == "tuiui" { 0 } else { 1 };
            rank(&ca)
                .cmp(&rank(&cb))
                .then_with(|| ca.cmp(&cb))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
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

    /// Render the dropdown as a multi-column grid of category blocks, fanning
    /// categories across as many columns as fit under the menubar (Windows
    /// Start-menu style). Columns are height-balanced by greedy packing.
    fn render_menu(&self, w: i32, _h: i32) -> Rendered {
        let filtered = self.filtered();
        let origin = Point::new(0, 1); // directly under the menubar brand
        if filtered.is_empty() {
            let (box_w, box_h) = (18, 3);
            let mut buf = CellBuffer::new(box_w, box_h);
            fill_box(&mut buf, box_w, box_h);
            draw_row(&mut buf, 1, box_w - 2, 1, "(no apps)", false, false);
            return Rendered {
                layers: vec![Layer { z: 5000, origin, buf, opacity: 1.0, scissor: None }],
                items: Vec::new(),
            };
        }

        let blocks = self.blocks(&filtered);
        let name_w = filtered.iter().map(|e| e.name.chars().count()).max().unwrap_or(8) as i32;
        let col_w = (name_w + 4).clamp(14, 26);
        const GAP: i32 = 2;

        // Number of columns that fit under the menubar (anchored at x = 0).
        let avail = (w - 2).max(col_w);
        let ncols = (((avail + GAP) / (col_w + GAP)).max(1) as usize).min(blocks.len());

        // Greedily pack each block into the currently-shortest column. A block is
        // 1 header + its items + a trailing spacer row.
        let mut col_blocks: Vec<Vec<&Block>> = vec![Vec::new(); ncols];
        let mut col_h: Vec<i32> = vec![0; ncols];
        for b in &blocks {
            let bh = 1 + b.items.len() as i32 + 1;
            let c = (0..ncols).min_by_key(|&c| col_h[c]).unwrap();
            col_blocks[c].push(b);
            col_h[c] += bh;
        }

        // Tallest column drives the height (drop its trailing spacer).
        let content_h = col_h.iter().copied().max().unwrap_or(1).saturating_sub(1).max(1);
        let box_w = 2 + ncols as i32 * col_w + (ncols as i32 - 1) * GAP;
        let box_h = content_h + 2;

        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h);

        let mut items = Vec::new();
        for (ci, blocks_in_col) in col_blocks.iter().enumerate() {
            let x0 = 1 + ci as i32 * (col_w + GAP);
            let mut y = 1;
            for b in blocks_in_col {
                draw_header(&mut buf, x0, col_w, y, &b.header);
                y += 1;
                for &i in &b.items {
                    let e = &filtered[i];
                    draw_row(&mut buf, x0, col_w, y, &e.name, i == self.selected, false);
                    items.push((e.clone(), Rect::new(origin.x + x0, origin.y + y, col_w, 1)));
                    y += 1;
                }
                y += 1; // spacer between blocks
            }
        }

        Rendered {
            layers: vec![Layer { z: 5000, origin, buf, opacity: 1.0, scissor: None }],
            items,
        }
    }

    /// Group `filtered` into contiguous category blocks (header + item indices).
    fn blocks(&self, filtered: &[AppEntry]) -> Vec<Block> {
        let mut out: Vec<Block> = Vec::new();
        for (i, e) in filtered.iter().enumerate() {
            let c = cat_of(e);
            if out.last().map(|b| b.header.as_str()) != Some(c.as_str()) {
                out.push(Block { header: c, items: Vec::new() });
            }
            out.last_mut().unwrap().items.push(i);
        }
        out
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
                Row::Header(c) => draw_header(&mut buf, 1, box_w - 2, y, c),
                Row::Item(i) => {
                    let e = &filtered[*i];
                    draw_row(&mut buf, 1, box_w - 2, y, &e.name, *i == self.selected, true);
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

/// A category block for grid layout: a header and the `filtered` indices under it.
struct Block {
    header: String,
    items: Vec<usize>,
}

/// The category an entry belongs to ("Apps" when unset).
fn cat_of(a: &AppEntry) -> String {
    a.category.clone().unwrap_or_else(|| "Apps".into())
}

/// Draw a dimmed, uppercase category header spanning `cw` cells from `x0`.
fn draw_header(buf: &mut CellBuffer, x0: i32, cw: i32, row: i32, cat: &str) {
    for x in x0..x0 + cw {
        buf.set(x, row, Cell { ch: ' ', fg: HINT, bg: MENU_BG, attrs: Default::default() });
    }
    let label: String = cat.to_uppercase().chars().take(cw.max(1) as usize).collect();
    buf.write_str(x0, row, &label, HINT, MENU_BG);
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

/// Draw one app row spanning `cw` cells from `x0`, optionally highlighted, with
/// an optional `▸` selection marker. The name is truncated to fit the column.
fn draw_row(buf: &mut CellBuffer, x0: i32, cw: i32, row: i32, name: &str, highlighted: bool, marker: bool) {
    let (fg, bg) = if highlighted { (SEL_FG, SEL_BG) } else { (MENU_FG, MENU_BG) };
    for x in x0..x0 + cw {
        buf.set(x, row, Cell { ch: ' ', fg, bg, attrs: Default::default() });
    }
    let lead = if marker && highlighted { "\u{25B8} " } else { "  " };
    buf.write_str(x0, row, lead, ACCENT, bg);
    let avail = (cw - 2).max(1) as usize;
    let shown: String = if name.chars().count() > avail {
        name.chars().take(avail.saturating_sub(1)).collect::<String>() + "\u{2026}"
    } else {
        name.to_string()
    };
    buf.write_str(x0 + 2, row, &shown, fg, bg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, cat: &str) -> AppEntry {
        AppEntry { name: name.into(), command: name.into(), args: vec![], category: Some(cat.into()), requires_cwd: None, cwd: None }
    }

    fn many() -> Vec<AppEntry> {
        vec![
            app("alpha", "Git"), app("beta", "Git"),
            app("gamma", "System"), app("delta", "System"),
            app("epsilon", "Net"), app("zeta", "Net"),
            app("eta", "Games"), app("theta", "Games"),
        ]
    }

    #[test]
    fn menu_grid_renders_every_app() {
        let mut l = Launcher::new(many());
        l.toggle_menu();
        let r = l.render(120, 40);
        // Every entry is clickable exactly once.
        assert_eq!(r.items.len(), many().len());
    }

    #[test]
    fn wide_menu_uses_multiple_columns() {
        let mut l = Launcher::new(many());
        l.toggle_menu();
        let r = l.render(120, 40);
        let columns: std::collections::BTreeSet<i32> = r.items.iter().map(|(_, rect)| rect.x).collect();
        assert!(columns.len() > 1, "wide screen should fan categories across columns");
    }

    #[test]
    fn narrow_menu_collapses_to_one_column() {
        let mut l = Launcher::new(many());
        l.toggle_menu();
        let r = l.render(20, 40);
        let columns: std::collections::BTreeSet<i32> = r.items.iter().map(|(_, rect)| rect.x).collect();
        assert_eq!(columns.len(), 1, "narrow screen should use a single column");
        assert_eq!(r.items.len(), many().len());
    }

    #[test]
    fn closed_launcher_renders_nothing() {
        let l = Launcher::new(many());
        let r = l.render(120, 40);
        assert!(r.items.is_empty());
        assert!(r.layers.is_empty());
    }
}
