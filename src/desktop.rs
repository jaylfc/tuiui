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
/// one row below the menubar. The top `ICON_H - 1` rows hold the (image) icon,
/// the last row holds the centered label.
pub const ICON_W: i32 = 14;
pub const ICON_H: i32 = 6;
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

    /// The screen rect of an icon's tile. Columns fill from the **top-right**
    /// (like macOS): logical column 0 is the rightmost on-screen column.
    pub fn tile_rect(&self, cell: (u16, u16)) -> crate::geometry::Rect {
        let screen_col = self.cols.saturating_sub(1).saturating_sub(cell.0);
        crate::geometry::Rect::new(
            screen_col as i32 * ICON_W,
            GRID_TOP + cell.1 as i32 * ICON_H,
            ICON_W,
            ICON_H,
        )
    }

    /// The screen rect of the icon *image* within a tile — centered horizontally,
    /// the top `ICON_H - 1` rows (the last row is the label).
    pub fn icon_image_rect(&self, cell: (u16, u16)) -> crate::geometry::Rect {
        let t = self.tile_rect(cell);
        let iw = (ICON_W - 2).max(2);
        let ih = (ICON_H - 1).max(1);
        crate::geometry::Rect::new(t.x + (ICON_W - iw) / 2, t.y, iw, ih)
    }

    /// The icon under `p`, if any.
    pub fn icon_at(&self, p: Point) -> Option<usize> {
        self.icons.iter().position(|i| self.tile_rect(i.cell).contains(p))
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
            let tile = self.tile_rect(icon.cell);
            let ir = self.icon_image_rect(icon.cell);
            let selected = self.selection.contains(&i);
            let bg = if selected { SEL_BG } else { transparent };
            // Glyph fallback, centered in the icon area (the image layer covers it
            // on Kitty-graphics terminals; this shows on the rest).
            let glyph = glyph_for(icon.role);
            buf.set(
                ir.x + ir.w / 2,
                ir.y + ir.h / 2,
                Cell { ch: glyph, fg: FG, bg: transparent, attrs: Default::default() },
            );
            // Label: centered on the tile's last row, truncated to the tile width.
            let label_y = tile.y + ICON_H - 1;
            let name: String = icon.label.chars().take(ICON_W as usize).collect();
            let len = name.chars().count() as i32;
            let lx = tile.x + (ICON_W - len).max(0) / 2;
            for x in tile.x..tile.x + ICON_W {
                buf.set(x, label_y, Cell { ch: ' ', fg: FG, bg, attrs: Default::default() });
            }
            buf.write_str(lx, label_y, &name, FG, bg);
        }
        buf
    }

    pub fn begin_drag(&mut self, p: Point) {
        if let Some(i) = self.icon_at(p) {
            let r = self.tile_rect(self.icons[i].cell);
            self.drag = Some((i, Point::new(p.x - r.x, p.y - r.y)));
            self.selection.clear();
            self.selection.insert(i);
        }
    }

    pub fn drag_to(&mut self, _p: Point) { /* ghost position is render-only; no-op for state */
    }

    /// Finish a drag: snap the dragged icon to the nearest free cell under `p`.
    /// Returns true if its cell changed.
    pub fn end_drag(&mut self, p: Point) -> bool {
        let Some((i, _grab)) = self.drag.take() else {
            return false;
        };
        // Invert the right-aligned screen column back to a logical column.
        let max_col = self.cols.max(1) as i32 - 1;
        let screen_col = (p.x / ICON_W).clamp(0, max_col);
        let col = (max_col - screen_col).clamp(0, max_col) as u16;
        let row = (((p.y - GRID_TOP) / ICON_H).clamp(0, self.rows.max(1) as i32 - 1)) as u16;
        let target = self.nearest_free((col, row), i);
        let changed = self.icons[i].cell != target;
        self.icons[i].cell = target;
        changed
    }

    /// The cell `(col,row)` if free, else the nearest free cell (spiral-ish scan).
    fn nearest_free(&self, want: (u16, u16), ignore: usize) -> (u16, u16) {
        let occupied = |c: (u16, u16)| {
            self.icons
                .iter()
                .enumerate()
                .any(|(j, ic)| j != ignore && ic.cell == c)
        };
        if !occupied(want) {
            return want;
        }
        for radius in 1..(self.cols.max(self.rows) as i32 + 1) {
            for dc in -radius..=radius {
                for dr in -radius..=radius {
                    let c = (want.0 as i32 + dc, want.1 as i32 + dr);
                    if c.0 < 0 || c.1 < 0 || c.0 >= self.cols as i32 || c.1 >= self.rows as i32 {
                        continue;
                    }
                    let cell = (c.0 as u16, c.1 as u16);
                    if !occupied(cell) {
                        return cell;
                    }
                }
            }
        }
        want
    }

    /// The current cell of the icon with key `key` (for persistence).
    pub fn position_of(&self, key: &str) -> Option<(u16, u16)> {
        self.icons
            .iter()
            .find(|i| Self::key_of(i) == key)
            .map(|i| i.cell)
    }

    /// All current positions (for bulk persistence after Clean Up).
    pub fn positions(&self) -> BTreeMap<String, (u16, u16)> {
        self.icons.iter().map(|i| (Self::key_of(i), i.cell)).collect()
    }

    /// Re-flow every icon to column-major order (Clean Up).
    pub fn clean_up(&mut self) {
        let mut taken = BTreeSet::new();
        for icon in &mut self.icons {
            let cell = Self::first_free(&taken, self.cols.max(1), self.rows.max(1));
            icon.cell = cell;
            taken.insert(cell);
        }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    // ── Context menus + rename / new-folder / trash ───────────────────────────

    /// Right-click: open a context menu over an icon, else the empty-desktop menu.
    pub fn right_click(&mut self, p: Point) {
        match self.icon_at(p) {
            Some(i) => {
                self.selection.clear();
                self.selection.insert(i);
                self.overlay = Some(DesktopOverlay::Context { idx: i, anchor: p });
            }
            None => self.overlay = Some(DesktopOverlay::DesktopMenu { anchor: p }),
        }
    }

    /// Begin a rename overlay for a folder-sourced icon (pins can't be renamed).
    pub fn begin_rename(&mut self, idx: usize) {
        if let Some(icon) = self.icons.get(idx) {
            if matches!(icon.source, IconSource::Folder) {
                self.overlay = Some(DesktopOverlay::Rename { idx, name: icon.label.clone() });
            }
        }
    }

    /// Begin a new-folder overlay (empty name field).
    pub fn begin_new_folder(&mut self) {
        self.overlay = Some(DesktopOverlay::NewFolder { name: String::new() });
    }

    /// Dismiss any open overlay / menu.
    pub fn cancel_overlay(&mut self) {
        self.overlay = None;
    }

    /// Append a character to the active rename / new-folder text field.
    pub fn overlay_char(&mut self, c: char) {
        match &mut self.overlay {
            Some(DesktopOverlay::Rename { name, .. }) | Some(DesktopOverlay::NewFolder { name }) => {
                name.push(c);
            }
            _ => {}
        }
    }

    /// Delete the last character of the active rename / new-folder text field.
    pub fn overlay_backspace(&mut self) {
        match &mut self.overlay {
            Some(DesktopOverlay::Rename { name, .. }) | Some(DesktopOverlay::NewFolder { name }) => {
                name.pop();
            }
            _ => {}
        }
    }

    /// Commit the active rename / new-folder overlay via the fs. Returns `true`
    /// when the folder changed (so the session reloads the icon list).
    pub fn overlay_commit(&mut self) -> bool {
        match self.overlay.take() {
            Some(DesktopOverlay::Rename { idx, name }) if !name.trim().is_empty() => {
                if let Some(path) = self.icons.get(idx).map(|i| i.path.clone()) {
                    let _ = self.fs.rename(&path, name.trim());
                }
                true
            }
            Some(DesktopOverlay::NewFolder { name }) if !name.trim().is_empty() => {
                let dir = self.desktop_dir.clone();
                let _ = self.fs.mkdir(&dir, name.trim());
                true
            }
            _ => false,
        }
    }

    /// Move the selected folder icons to Trash. Returns `true` if anything moved.
    pub fn trash_selection(&mut self) -> bool {
        let paths: Vec<PathBuf> = self
            .selection
            .iter()
            .filter_map(|&i| self.icons.get(i))
            .filter(|i| matches!(i.source, IconSource::Folder))
            .map(|i| i.path.clone())
            .collect();
        let mut any = false;
        for p in paths {
            if self.fs.trash(&p).is_ok() {
                any = true;
            }
        }
        self.overlay = None;
        any
    }

    // ── Thumbnails (A1 image placements) ──────────────────────────────────────

    /// Image-role icons (with a non-empty path) needing a thumbnail loaded.
    pub fn thumbnail_requests(&self) -> Vec<(usize, PathBuf)> {
        self.icons
            .iter()
            .enumerate()
            .filter(|(_, i)| i.role == Role::Image && !i.path.as_os_str().is_empty())
            .map(|(i, ic)| (i, ic.path.clone()))
            .collect()
    }

    /// Record a loaded thumbnail id for an icon.
    pub fn set_thumb(&mut self, idx: usize, id: u64) {
        if let Some(i) = self.icons.get_mut(idx) {
            i.thumb = Some(id);
        }
    }

    /// Image placements for every icon: a loaded photo thumbnail when available,
    /// otherwise the generated icon for the entry's `role` (from `role_icons`).
    /// `visible(tile_rect)` decides occlusion by windows.
    pub fn icon_placements(
        &self,
        role_icons: &std::collections::HashMap<Role, u64>,
        visible: impl Fn(crate::geometry::Rect) -> bool,
    ) -> Vec<crate::protocol::ImagePlacement> {
        let mut out = Vec::new();
        for icon in &self.icons {
            let id = icon.thumb.or_else(|| role_icons.get(&icon.role).copied());
            if let Some(id) = id {
                let tile = self.tile_rect(icon.cell);
                let r = self.icon_image_rect(icon.cell);
                out.push(crate::protocol::ImagePlacement {
                    id,
                    rect: r,
                    cols: r.w.max(1) as u16,
                    rows: r.h.max(1) as u16,
                    visible: visible(tile),
                });
            }
        }
        out
    }

    /// The icon index a context menu is anchored on, if any.
    pub fn context_idx(&self) -> Option<usize> {
        match self.overlay {
            Some(DesktopOverlay::Context { idx, .. }) => Some(idx),
            _ => None,
        }
    }
    pub fn icon_is_pinned(&self, idx: usize) -> bool {
        self.icons.get(idx).map(|i| matches!(i.source, IconSource::Pinned)).unwrap_or(false)
    }
    pub fn icon_command(&self, idx: usize) -> Option<String> {
        self.icons.get(idx).and_then(|i| i.command.clone())
    }
    pub fn icon_path(&self, idx: usize) -> Option<PathBuf> {
        self.icons.get(idx).map(|i| i.path.clone())
    }

    /// The menu items for the currently-open overlay, top to bottom. Empty when
    /// no menu (or a text overlay) is open.
    fn menu_items(&self) -> Vec<DesktopMenuItem> {
        match self.overlay {
            Some(DesktopOverlay::Context { idx, .. }) => {
                if self.icon_is_pinned(idx) {
                    vec![DesktopMenuItem::Open, DesktopMenuItem::Unpin]
                } else {
                    vec![
                        DesktopMenuItem::Open,
                        DesktopMenuItem::OpenWith,
                        DesktopMenuItem::Rename,
                        DesktopMenuItem::Trash,
                    ]
                }
            }
            Some(DesktopOverlay::DesktopMenu { .. }) => {
                vec![DesktopMenuItem::NewFolder, DesktopMenuItem::CleanUp]
            }
            _ => Vec::new(),
        }
    }

    /// The screen rect of the open menu box (anchor + item list), if any. Width is
    /// fixed; height is one row per item plus a one-cell border on each side.
    fn menu_rect(&self) -> Option<crate::geometry::Rect> {
        let anchor = match self.overlay {
            Some(DesktopOverlay::Context { anchor, .. })
            | Some(DesktopOverlay::DesktopMenu { anchor }) => anchor,
            _ => return None,
        };
        let items = self.menu_items();
        if items.is_empty() {
            return None;
        }
        let w = MENU_W;
        let h = items.len() as i32 + 2; // +border rows
        Some(crate::geometry::Rect::new(anchor.x, anchor.y, w, h))
    }

    /// The text-field box for an open rename / new-folder overlay, if any.
    fn field_rect(&self) -> Option<crate::geometry::Rect> {
        match &self.overlay {
            Some(DesktopOverlay::Rename { idx, .. }) => {
                let cell = self.icons.get(*idx)?.cell;
                let r = self.tile_rect(cell);
                Some(crate::geometry::Rect::new(r.x, r.y, ICON_W, ICON_H))
            }
            Some(DesktopOverlay::NewFolder { .. }) => {
                Some(crate::geometry::Rect::new(2, GRID_TOP, MENU_W, 3))
            }
            _ => None,
        }
    }

    /// The menu item under `p` for an open context / desktop menu, if any.
    pub fn menu_item_at(&self, p: Point) -> Option<DesktopMenuItem> {
        let r = self.menu_rect()?;
        let items = self.menu_items();
        if !r.contains(p) {
            return None;
        }
        // Rows are the interior of the box (skip the top border row).
        let row = p.y - (r.y + 1);
        if row < 0 || row as usize >= items.len() {
            return None;
        }
        Some(items[row as usize])
    }

    /// Render the open overlay (menu box or text field) into a screen-sized buffer,
    /// or `None` when no overlay is open. The session composites this above the
    /// windows on a high-z layer.
    pub fn overlay_buffer(&self, w: i32, h: i32) -> Option<crate::buffer::CellBuffer> {
        use crate::cell::{Cell, Rgba};
        self.overlay.as_ref()?;
        const FG: Rgba = Rgba { r: 224, g: 228, b: 238, a: 255 };
        const BG: Rgba = Rgba { r: 30, g: 34, b: 46, a: 245 };
        let transparent = Rgba::TRANSPARENT;
        let mut buf = crate::buffer::CellBuffer::new(w, h);
        buf.fill(Cell { ch: ' ', fg: FG, bg: transparent, attrs: Default::default() });

        if let Some(r) = self.menu_rect() {
            // Box background.
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    buf.set(x, y, Cell { ch: ' ', fg: FG, bg: BG, attrs: Default::default() });
                }
            }
            // Item labels, one per interior row.
            for (i, item) in self.menu_items().iter().enumerate() {
                buf.write_str(r.x + 1, r.y + 1 + i as i32, item.label(), FG, BG);
            }
        } else if let Some(r) = self.field_rect() {
            let name = match &self.overlay {
                Some(DesktopOverlay::Rename { name, .. })
                | Some(DesktopOverlay::NewFolder { name }) => name.as_str(),
                _ => "",
            };
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    buf.set(x, y, Cell { ch: ' ', fg: FG, bg: BG, attrs: Default::default() });
                }
            }
            let shown: String = name.chars().take((r.w - 2) as usize).collect();
            buf.write_str(r.x + 1, r.y + 1, &shown, FG, BG);
        }
        Some(buf)
    }
}

/// Fixed pixel/cell width of a desktop menu box.
const MENU_W: i32 = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopMenuItem {
    Open,
    OpenWith,
    Rename,
    Trash,
    Unpin,
    NewFolder,
    CleanUp,
}

impl DesktopMenuItem {
    fn label(self) -> &'static str {
        match self {
            DesktopMenuItem::Open => "Open",
            DesktopMenuItem::OpenWith => "Open with…",
            DesktopMenuItem::Rename => "Rename",
            DesktopMenuItem::Trash => "Move to Trash",
            DesktopMenuItem::Unpin => "Unpin",
            DesktopMenuItem::NewFolder => "New Folder",
            DesktopMenuItem::CleanUp => "Clean Up",
        }
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
