//! Desktop icons: a wallpaper-level icon layer merged from the live `~/Desktop`
//! folder and user pins. Not a window — rendered at z=0 beneath all windows and
//! hit-tested only for clicks that fall through to the empty desktop.

use crate::config::AppEntry;
use crate::fileops::{FsOps, StdFs};
use crate::geometry::Point;
use crate::openwith::{classify, Role};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Layout: each icon occupies a tile this many cells wide/tall; the grid starts
/// one row below the menubar.
pub const ICON_W: i32 = 14;
pub const ICON_H: i32 = 3;
pub const GRID_TOP: i32 = 1; // below the menubar row

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSource {
    Folder,
    Pinned,
}

#[derive(Clone, Debug)]
pub struct DesktopIcon {
    pub path: PathBuf,
    pub label: String,
    pub role: Role,
    pub source: IconSource,
    pub command: Option<String>, // pins only
    pub cell: (u16, u16),        // (col, row)
    pub thumb: Option<u64>,
}

/// What the session must effect (the model never touches windows/PTYs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopAction {
    Open(PathBuf),
    Run { command: String, args: Vec<String> },
    Unpin(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopOverlay {
    Context { idx: usize, anchor: Point },
    DesktopMenu { anchor: Point },
    Rename { idx: usize, name: String },
    NewFolder { name: String },
}

pub struct DesktopIcons<F: FsOps = StdFs> {
    fs: F,
    desktop_dir: PathBuf,
    icons: Vec<DesktopIcon>,
    selection: BTreeSet<usize>,
    drag: Option<(usize, Point)>,
    overlay: Option<DesktopOverlay>,
    action: Option<DesktopAction>,
    cols: u16,
    rows: u16,
}

impl DesktopIcons<StdFs> {
    pub fn new(desktop_dir: PathBuf) -> Self {
        Self::with_fs(StdFs, desktop_dir)
    }
}

impl<F: FsOps> DesktopIcons<F> {
    pub fn with_fs(fs: F, desktop_dir: PathBuf) -> Self {
        Self {
            fs,
            desktop_dir,
            icons: Vec::new(),
            selection: BTreeSet::new(),
            drag: None,
            overlay: None,
            action: None,
            cols: 1,
            rows: 1,
        }
    }

    pub fn icons(&self) -> &[DesktopIcon] {
        &self.icons
    }
    pub fn selection(&self) -> Vec<usize> {
        self.selection.iter().copied().collect()
    }
    pub fn overlay(&self) -> Option<&DesktopOverlay> {
        self.overlay.as_ref()
    }
    pub fn is_editing(&self) -> bool {
        matches!(
            self.overlay,
            Some(DesktopOverlay::Rename { .. }) | Some(DesktopOverlay::NewFolder { .. })
        )
    }
    pub fn take_action(&mut self) -> Option<DesktopAction> {
        self.action.take()
    }

    /// Rebuild the icon list from the folder + pins, keeping `positions`.
    pub fn reload(&mut self, pins: &[AppEntry], positions: &BTreeMap<String, (u16, u16)>) {
        let prev_keys: BTreeSet<String> = self
            .selection
            .iter()
            .filter_map(|&i| self.icons.get(i))
            .map(Self::key_of)
            .collect();
        let mut icons = Vec::new();
        if let Ok(entries) = self.fs.list(&self.desktop_dir, false) {
            for e in entries {
                icons.push(DesktopIcon {
                    label: e.name.clone(),
                    role: e.role,
                    path: e.path,
                    source: IconSource::Folder,
                    command: None,
                    cell: (0, 0),
                    thumb: None,
                });
            }
        }
        for p in pins {
            let target = p.cwd.clone().or_else(|| p.args.first().cloned()).unwrap_or_default();
            let path = PathBuf::from(&target);
            let role = if target.is_empty() {
                Role::Other
            } else {
                classify(&path, path.is_dir())
            };
            icons.push(DesktopIcon {
                label: p.name.clone(),
                role,
                path,
                source: IconSource::Pinned,
                command: Some(p.command.clone()),
                cell: (0, 0),
                thumb: None,
            });
        }
        self.icons = icons;
        self.assign_cells(positions);
        // restore selection by key
        self.selection = self
            .icons
            .iter()
            .enumerate()
            .filter(|(_, i)| prev_keys.contains(&Self::key_of(i)))
            .map(|(i, _)| i)
            .collect();
    }

    /// The persistence key for an icon (pin command, else abs path).
    fn key_of(icon: &DesktopIcon) -> String {
        match &icon.command {
            Some(cmd) => cmd.clone(),
            None => icon.path.to_string_lossy().to_string(),
        }
    }
    pub fn icon_key(&self, idx: usize) -> Option<String> {
        self.icons.get(idx).map(Self::key_of)
    }

    /// Recompute the grid dimensions for a `w×h` screen.
    pub fn layout(&mut self, w: i32, h: i32) {
        self.cols = ((w / ICON_W).max(1)) as u16;
        // leave the menubar (1) and dock (1) rows out
        self.rows = (((h - GRID_TOP - 1) / ICON_H).max(1)) as u16;
    }

    /// Assign each icon a cell: saved position if present, else first free cell.
    fn assign_cells(&mut self, positions: &BTreeMap<String, (u16, u16)>) {
        let mut taken: BTreeSet<(u16, u16)> = BTreeSet::new();
        // first pass: saved positions
        for icon in &mut self.icons {
            if let Some(&cell) = positions.get(&Self::key_of(icon)) {
                icon.cell = cell;
                taken.insert(cell);
            }
        }
        // second pass: unplaced icons → first free cell, column-major
        for icon in &mut self.icons {
            if positions.contains_key(&Self::key_of(icon)) {
                continue;
            }
            let cell = Self::first_free(&taken, self.cols.max(1), self.rows.max(1));
            icon.cell = cell;
            taken.insert(cell);
        }
    }

    fn first_free(taken: &BTreeSet<(u16, u16)>, cols: u16, rows: u16) -> (u16, u16) {
        for col in 0..cols.max(1) {
            for row in 0..rows.max(1) {
                if !taken.contains(&(col, row)) {
                    return (col, row);
                }
            }
        }
        (0, 0) // grid full: stack at origin
    }

    /// The screen rect of an icon's tile.
    pub fn tile_rect(cell: (u16, u16)) -> crate::geometry::Rect {
        crate::geometry::Rect::new(
            cell.0 as i32 * ICON_W,
            GRID_TOP + cell.1 as i32 * ICON_H,
            ICON_W,
            ICON_H,
        )
    }

    /// The icon under `p`, if any.
    pub fn icon_at(&self, p: Point) -> Option<usize> {
        self.icons.iter().position(|i| Self::tile_rect(i.cell).contains(p))
    }

    /// Left click: select the icon under `p` (clear others unless `ctrl`); on empty
    /// desktop clear selection and dismiss any overlay.
    pub fn click(&mut self, p: Point, ctrl: bool) {
        self.overlay = None;
        match self.icon_at(p) {
            Some(i) => {
                if ctrl {
                    if !self.selection.remove(&i) {
                        self.selection.insert(i);
                    }
                } else {
                    self.selection.clear();
                    self.selection.insert(i);
                }
            }
            None => self.selection.clear(),
        }
    }

    /// Double click: produce an Open/Run action for the icon under `p`.
    pub fn double_click(&mut self, p: Point) {
        let Some(i) = self.icon_at(p) else {
            return;
        };
        let icon = &self.icons[i];
        self.action = Some(match &icon.command {
            Some(cmd) => DesktopAction::Run {
                command: cmd.clone(),
                args: vec![],
            },
            None => DesktopAction::Open(icon.path.clone()),
        });
    }

    /// Render the icon layer into a `w×h` buffer (transparent background so the
    /// wallpaper shows through; only icon tiles draw).
    pub fn render(&self, w: i32, h: i32) -> crate::buffer::CellBuffer {
        use crate::cell::{Cell, Rgba};
        const FG: Rgba = Rgba { r: 220, g: 226, b: 236, a: 255 };
        const SEL_BG: Rgba = Rgba { r: 45, g: 58, b: 85, a: 200 };
        let transparent = Rgba::TRANSPARENT;
        let mut buf = crate::buffer::CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: FG, bg: transparent, attrs: Default::default() });
        for (i, icon) in self.icons.iter().enumerate() {
            let r = Self::tile_rect(icon.cell);
            let selected = self.selection.contains(&i);
            let bg = if selected { SEL_BG } else { transparent };
            // glyph row
            let glyph = glyph_for(icon.role);
            buf.set(r.x + ICON_W / 2, r.y, Cell { ch: glyph, fg: FG, bg, attrs: Default::default() });
            // label row (centered-ish, truncated)
            let name: String = icon.label.chars().take((ICON_W - 1) as usize).collect();
            // paint selection bg across the label width, then write the string
            for k in 0..name.chars().count() as i32 {
                buf.set(r.x + k, r.y + 1, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() });
            }
            buf.write_str(r.x, r.y + 1, &name, FG, bg);
        }
        buf
    }
}

fn glyph_for(role: Role) -> char {
    match role {
        Role::Directory => '\u{1F4C1}',
        Role::Image => '\u{1F5BC}',
        Role::Audio => '\u{1F3B5}',
        Role::Video => '\u{1F3AC}',
        Role::Archive => '\u{1F4E6}',
        Role::Pdf => '\u{1F4D5}',
        _ => '\u{1F4C4}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tuiui-dtr-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn render_returns_screen_sized_buffer_with_label() {
        let d = tmp("render");
        fs::create_dir(d.join("proj")).unwrap();
        let mut dt = DesktopIcons::new(d.clone());
        dt.reload(&[], &BTreeMap::new());
        dt.layout(100, 30);
        let buf = dt.render(100, 30);
        assert_eq!(buf.width(), 100);
        assert_eq!(buf.height(), 30);
        let _ = fs::remove_dir_all(&d);
    }
}
