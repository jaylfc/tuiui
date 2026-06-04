//! The app store: a browseable, searchable view over the bundled catalog
//! ([`crate::catalog`]). Renders itself into a window's content [`CellBuffer`]
//! and handles keyboard navigation. Installing an app is delegated to the
//! session (it spawns the install command in a PTY window).

use crate::buffer::CellBuffer;
use crate::catalog::{self, CatalogApp};
use crate::cell::{Cell, Rgba};
use crate::geometry::Point;

const BG: Rgba = Rgba { r: 17, g: 20, b: 29, a: 255 };
const FG: Rgba = Rgba { r: 200, g: 208, b: 220, a: 255 };
const DIM: Rgba = Rgba { r: 120, g: 130, b: 150, a: 255 };
const SEL_BG: Rgba = Rgba { r: 45, g: 58, b: 85, a: 255 };
const ACCENT: Rgba = Rgba { r: 108, g: 182, b: 255, a: 255 };
const GREEN: Rgba = Rgba { r: 126, g: 231, b: 135, a: 255 };
const PANEL: Rgba = Rgba { r: 22, g: 26, b: 37, a: 255 };

const SIDEBAR_W: i32 = 16;
const LIST_W: i32 = 30;

/// State of the store browser.
pub struct Store {
    categories: Vec<String>,
    cat_index: usize,
    query: String,
    selected: usize,
    scroll: usize,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// Create a store browser (category list derived from the catalog).
    pub fn new() -> Self {
        let mut cats: Vec<String> = Vec::new();
        for c in catalog::catalog() {
            if !cats.iter().any(|x| x == &c.category) {
                cats.push(c.category.clone());
            }
        }
        cats.sort();
        cats.insert(0, "All".to_string());
        Self { categories: cats, cat_index: 0, query: String::new(), selected: 0, scroll: 0 }
    }

    /// Apps matching the current category + query.
    pub fn filtered(&self) -> Vec<&'static CatalogApp> {
        let cat = &self.categories[self.cat_index];
        let q = self.query.to_lowercase();
        catalog::catalog()
            .iter()
            .filter(|c| {
                (cat == "All" || &c.category == cat)
                    && (q.is_empty()
                        || c.name.to_lowercase().contains(&q)
                        || c.description.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// The currently highlighted app.
    pub fn selected_app(&self) -> Option<&'static CatalogApp> {
        self.filtered().into_iter().nth(self.selected)
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
    pub fn move_down(&mut self) {
        let n = self.filtered().len();
        if n > 0 && self.selected + 1 < n {
            self.selected += 1;
        }
    }
    pub fn prev_category(&mut self) {
        if self.cat_index > 0 {
            self.cat_index -= 1;
            self.reset_list();
        }
    }
    pub fn next_category(&mut self) {
        if self.cat_index + 1 < self.categories.len() {
            self.cat_index += 1;
            self.reset_list();
        }
    }
    pub fn type_char(&mut self, c: char) {
        self.query.push(c);
        self.reset_list();
    }
    pub fn backspace(&mut self) {
        self.query.pop();
        self.reset_list();
    }
    fn reset_list(&mut self) {
        self.selected = 0;
        self.scroll = 0;
    }

    /// Render the store into a `w × h` content buffer.
    pub fn render(&self, w: i32, h: i32) -> CellBuffer {
        let mut buf = CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: FG, bg: BG, attrs: Default::default() });

        // Search bar (row 0).
        buf.write_str(1, 0, &format!("\u{2315} {}\u{2588}", self.query), ACCENT, BG);
        let count = self.filtered().len();
        let meta = format!("{count} apps");
        buf.write_str((w - meta.len() as i32 - 1).max(0), 0, &meta, DIM, BG);
        hline(&mut buf, w, 1);

        // Sidebar (categories).
        for (i, cat) in self.categories.iter().enumerate() {
            let y = 2 + i as i32;
            if y >= h {
                break;
            }
            let sel = i == self.cat_index;
            let (fg, bg) = if sel { (ACCENT, SEL_BG) } else { (DIM, BG) };
            for x in 0..SIDEBAR_W {
                buf.set(x, y, Cell { ch: ' ', fg, bg, attrs: Default::default() });
            }
            buf.write_str(1, y, truncate(cat, SIDEBAR_W as usize - 2), fg, bg);
        }
        vline(&mut buf, SIDEBAR_W, 2, h);

        // App list.
        let list_x = SIDEBAR_W + 1;
        let rows = (h - 2).max(0) as usize;
        let apps = self.filtered();
        let scroll = self.scroll_for(rows);
        for (row, app) in apps.iter().skip(scroll).take(rows).enumerate() {
            let idx = scroll + row;
            let y = 2 + row as i32;
            let sel = idx == self.selected;
            let bg = if sel { SEL_BG } else { BG };
            for x in list_x..list_x + LIST_W {
                buf.set(x, y, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() });
            }
            let installed = catalog::is_installed(&app.bin);
            let mark = if installed { "\u{2713} " } else { "  " };
            buf.write_str(list_x, y, mark, GREEN, bg);
            buf.write_str(list_x + 2, y, truncate(&app.name, LIST_W as usize - 3), FG, bg);
        }
        vline(&mut buf, list_x + LIST_W, 2, h);

        // Detail pane.
        let dx = list_x + LIST_W + 1;
        let dw = (w - dx - 1).max(0);
        if let Some(app) = self.selected_app() {
            for y in 2..h {
                for x in dx..w {
                    buf.set(x, y, Cell { ch: ' ', fg: FG, bg: PANEL, attrs: Default::default() });
                }
            }
            buf.write_str(dx + 1, 3, truncate(&app.name, dw as usize - 2), ACCENT, PANEL);
            buf.write_str(dx + 1, 4, truncate(&app.category, dw as usize - 2), DIM, PANEL);
            for (i, line) in wrap(&app.description, dw as usize - 2).into_iter().enumerate() {
                let y = 6 + i as i32;
                if y >= h - 4 {
                    break;
                }
                buf.write_str(dx + 1, y, &line, FG, PANEL);
            }
            buf.write_str(dx + 1, h - 4, truncate(&app.homepage, dw as usize - 2), DIM, PANEL);
            // Verified-recipe badge.
            if let Some(r) = catalog::recipe(&app.name) {
                if r.verified {
                    buf.write_str(dx + 1, h - 3, &format!("\u{2713} verified \u{00B7} {}", r.method), GREEN, PANEL);
                }
            }
            // Action hint.
            let installed = catalog::is_installed(&app.bin);
            let action = if installed { "[ Enter: Launch ]" } else { "[ Enter: Install ]" };
            let acol = if installed { GREEN } else { ACCENT };
            buf.write_str(dx + 1, h - 2, action, acol, PANEL);
        }

        buf
    }

    /// Handle a click at content-local point `p` (within a `w × h` content area).
    ///
    /// Returns `true` if the click should activate (install/launch) the selected
    /// app — i.e. the action button, or a second click on the already-selected row.
    pub fn handle_click(&mut self, p: Point, _w: i32, h: i32) -> bool {
        if p.y < 2 {
            return false; // search bar / divider
        }
        // Category sidebar.
        if p.x < SIDEBAR_W {
            let i = (p.y - 2) as usize;
            if i < self.categories.len() {
                self.cat_index = i;
                self.reset_list();
            }
            return false;
        }
        // App list.
        let list_x = SIDEBAR_W + 1;
        if p.x >= list_x && p.x < list_x + LIST_W {
            let rows = (h - 2).max(0) as usize;
            let idx = self.scroll_for(rows) + (p.y - 2) as usize;
            if idx < self.filtered().len() {
                if idx == self.selected {
                    return true; // second click on the selected row activates
                }
                self.selected = idx;
            }
            return false;
        }
        // Detail pane action button (bottom).
        let dx = list_x + LIST_W + 1;
        if p.x >= dx && p.y == h - 2 {
            return true;
        }
        false
    }

    /// Scroll offset that keeps the selection visible given `rows` visible lines.
    fn scroll_for(&self, rows: usize) -> usize {
        if rows == 0 {
            return 0;
        }
        if self.selected < self.scroll {
            self.selected
        } else if self.selected >= self.scroll + rows {
            self.selected + 1 - rows
        } else {
            self.scroll
        }
    }
}

/// The best-effort install command for an app (run in a shell window).
///
/// Tries the common TUI install paths in turn — Homebrew, then `cargo install`,
/// then `go install <repo>@latest` — and always prints the homepage so the user
/// can finish manually. Per-app recipes will refine this over time.
pub fn install_command(app: &CatalogApp) -> String {
    // A verified curated recipe wins over the heuristic chain.
    if let Some(r) = catalog::recipe(&app.name) {
        if r.verified && !r.install.is_empty() {
            return format!(
                "clear; echo 'Installing {name} …'; echo; {cmd}; echo; echo '────────'; \
echo 'Done. If it succeeded, {name} is now in your launcher.'; \
echo 'Close this window (\u{2715}) when finished.'; exec \"$SHELL\"",
                name = app.name,
                cmd = r.install,
            );
        }
    }
    let gopath = app
        .homepage
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    format!(
        "clear; echo 'Installing {name} — trying brew, then cargo, then go…'; echo; \
( command -v brew >/dev/null 2>&1 && brew install {bin} ) \
|| cargo install {bin} \
|| go install {gopath}@latest \
|| true; \
echo; echo '────────'; \
echo 'If nothing installed automatically, get {name} from:'; echo '  {home}'; \
echo; echo 'Close this window (✕) when done.'; exec \"$SHELL\"",
        name = app.name,
        bin = app.bin,
        gopath = gopath,
        home = app.homepage,
    )
}

fn hline(buf: &mut CellBuffer, w: i32, y: i32) {
    for x in 0..w {
        buf.set(x, y, Cell { ch: '\u{2500}', fg: DIM, bg: BG, attrs: Default::default() });
    }
}
fn vline(buf: &mut CellBuffer, x: i32, y0: i32, h: i32) {
    for y in y0..h {
        buf.set(x, y, Cell { ch: '\u{2502}', fg: DIM, bg: BG, attrs: Default::default() });
    }
}
fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}
fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if line.chars().count() + word.chars().count() + 1 > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
