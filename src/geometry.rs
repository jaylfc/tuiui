#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Point { pub x: i32, pub y: i32 }

impl Point {
    pub fn new(x: i32, y: i32) -> Self { Self { x, y } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self { Self { x, y, w, h } }
    pub fn right(&self) -> i32 { self.x + self.w - 1 }
    pub fn bottom(&self) -> i32 { self.y + self.h - 1 }
    /// Integer midpoint of the rect.
    pub fn center(&self) -> Point { Point::new(self.x + self.w / 2, self.y + self.h / 2) }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    /// Intersection, or None if disjoint.
    pub fn intersect(&self, o: Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r < x || b < y { None } else { Some(Rect::new(x, y, r - x + 1, b - y + 1)) }
    }
}

/// A rows×cols tiling grid over a work area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid { pub rows: u8, pub cols: u8 }

impl Grid {
    /// Total cell count (never zero — rows/cols are treated as at least 1).
    pub fn cells(&self) -> u8 { self.rows.max(1) * self.cols.max(1) }

    /// Rect for cell `(row, col)` within `work`, leaving `gap` cells of gutter
    /// between adjacent cells. The last row/column absorbs any remainder so the
    /// grid always reaches the work-area edge.
    pub fn cell_rect(&self, work: Rect, row: u8, col: u8, gap: i32) -> Rect {
        let (rows, cols) = (self.rows.max(1) as i32, self.cols.max(1) as i32);
        let row = (row.min(self.rows.max(1) - 1)) as i32;
        let col = (col.min(self.cols.max(1) - 1)) as i32;
        let cw = (work.w - gap * (cols - 1)).max(cols) / cols;
        let ch = (work.h - gap * (rows - 1)).max(rows) / rows;
        let x = work.x + col * (cw + gap);
        let y = work.y + row * (ch + gap);
        let w = if col == cols - 1 { work.x + work.w - x } else { cw };
        let h = if row == rows - 1 { work.y + work.h - y } else { ch };
        Rect::new(x, y, w.max(1), h.max(1))
    }

    /// The `(row, col)` cell whose region contains `p` (clamped to the grid).
    pub fn cell_at(&self, work: Rect, p: Point) -> (u8, u8) {
        let (rows, cols) = (self.rows.max(1) as i32, self.cols.max(1) as i32);
        let dx = (p.x - work.x).clamp(0, work.w - 1);
        let dy = (p.y - work.y).clamp(0, work.h - 1);
        let col = (dx * cols / work.w.max(1)).clamp(0, cols - 1);
        let row = (dy * rows / work.h.max(1)).clamp(0, rows - 1);
        (row as u8, col as u8)
    }

    /// Row-major cell index for `(row, col)`.
    pub fn index_of(&self, row: u8, col: u8) -> u8 { row * self.cols.max(1) + col }
    /// `(row, col)` for a row-major `index`.
    pub fn row_col(&self, index: u8) -> (u8, u8) {
        let cols = self.cols.max(1);
        (index / cols, index % cols)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SnapZone { Left, Right }

/// Returns a snap zone if `p` is within `threshold` cells of the left/right screen edge.
pub fn snap_zone(p: Point, screen: Rect, threshold: i32) -> Option<SnapZone> {
    if p.x < screen.x + threshold { Some(SnapZone::Left) }
    else if p.x > screen.right() - threshold { Some(SnapZone::Right) }
    else { None }
}

/// The rect a window takes when snapped, given the usable work area.
pub fn snapped_rect(zone: SnapZone, work: Rect) -> Rect {
    let half = work.w / 2;
    match zone {
        SnapZone::Left => Rect::new(work.x, work.y, half, work.h),
        SnapZone::Right => Rect::new(work.x + half, work.y, work.w - half, work.h),
    }
}
