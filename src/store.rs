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

/// Tag shown on apps flagged `cli` — a tool that prints output and exits (or
/// needs subcommands) rather than opening a persistent full-screen TUI.
const CLI_BADGE: &str = "CLI";

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
                catalog::runs_on_current_os(&c.name)
                    && (cat == "All" || &c.category == cat)
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
            let installed = catalog::is_installed(&app.bin);
            let flags = ListRowFlags { installed, cli: app.cli, sel };
            draw_list_row(&mut buf, list_x, y, LIST_W, &app.name, flags);
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
            let cat_str = truncate(&app.category, dw as usize - 2);
            buf.write_str(dx + 1, 4, cat_str, DIM, PANEL);
            if app.cli {
                let bx = dx + 1 + cat_str.chars().count() as i32 + 2;
                if bx + CLI_BADGE.len() as i32 <= dx + dw {
                    buf.write_str(bx, 4, CLI_BADGE, ACCENT, PANEL);
                }
            }
            let mut y = 6;
            for line in wrap(&app.description, dw as usize - 2) {
                if y >= h - 5 {
                    break;
                }
                buf.write_str(dx + 1, y, &line, FG, PANEL);
                y += 1;
            }
            // Setup tip (e.g. "add models/providers with `hermes model` first").
            if let Some(r) = catalog::recipe(&app.name) {
                if !r.tip.is_empty() {
                    y += 1;
                    for line in wrap(&format!("Setup: {}", r.tip), dw as usize - 2) {
                        if y >= h - 5 {
                            break;
                        }
                        buf.write_str(dx + 1, y, &line, ACCENT, PANEL);
                        y += 1;
                    }
                }
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
            // If the recipe's toolchain (go/cargo/npm/pip/brew) is missing, warn
            // and offer to install it first; declining or a failed install aborts
            // the window before the app install runs.
            let pre = crate::toolchain::preamble(&app.name, &r.method, &app.homepage);
            return format!(
                "clear; {pre}echo 'Installing {name} …'; echo; {cmd}; echo; echo '────────'; \
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

/// Per-row flags for [`draw_list_row`], bundled into one argument to keep the
/// function's parameter count clippy-clean.
struct ListRowFlags {
    /// Whether the app is on `$PATH` (shows the green checkmark).
    installed: bool,
    /// Whether a right-aligned `CLI` tag is drawn (app flagged as a CLI tool).
    cli: bool,
    /// Whether this row is the current selection (swaps to the highlight bg).
    sel: bool,
}

/// Draw one app-list row spanning `w` cells from `x` per `flags` (see
/// [`ListRowFlags`]): the install checkmark, the (possibly truncated) name,
/// and — for CLI-flagged apps (prints output and exits / needs subcommands,
/// rather than a persistent TUI) — a right-aligned `CLI` tag.
fn draw_list_row(buf: &mut CellBuffer, x: i32, y: i32, w: i32, name: &str, flags: ListRowFlags) {
    let ListRowFlags { installed, cli, sel } = flags;
    let bg = if sel { SEL_BG } else { BG };
    for dx in 0..w {
        buf.set(x + dx, y, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() });
    }
    let mark = if installed { "\u{2713} " } else { "  " };
    buf.write_str(x, y, mark, GREEN, bg);
    let badge_w = if cli { CLI_BADGE.len() as i32 + 1 } else { 0 };
    let name_w = (w - 3 - badge_w).max(1) as usize;
    buf.write_str(x + 2, y, truncate(name, name_w), FG, bg);
    if cli {
        buf.write_str(x + w - CLI_BADGE.len() as i32, y, CLI_BADGE, ACCENT, bg);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(buf: &CellBuffer, y: i32, w: i32) -> String {
        (0..w).map(|x| buf.get(x, y).map(|c| c.ch).unwrap_or(' ')).collect()
    }

    /// A `cli`-flagged app's list row shows the "CLI" tag; an unflagged one
    /// doesn't (the badge shown in the store's app list — see `render`).
    #[test]
    fn cli_badge_shown_only_on_flagged_row() {
        let mut buf = CellBuffer::new(30, 2);
        draw_list_row(&mut buf, 0, 0, 30, "himalaya", ListRowFlags { installed: true, cli: true, sel: false });
        draw_list_row(&mut buf, 0, 1, 30, "btop", ListRowFlags { installed: true, cli: false, sel: false });
        assert!(row_text(&buf, 0, 30).contains("CLI"), "flagged row should show the badge");
        assert!(!row_text(&buf, 1, 30).contains("CLI"), "unflagged row should not show the badge");
    }
}
