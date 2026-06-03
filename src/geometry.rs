#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapZone { Left, Right }

/// Returns a snap zone if `p` is within `threshold` cells of the left/right screen edge.
pub fn snap_zone(p: Point, screen: Rect, threshold: i32) -> Option<SnapZone> {
    if p.x <= screen.x + threshold - 1 { Some(SnapZone::Left) }
    else if p.x >= screen.right() - threshold + 1 { Some(SnapZone::Right) }
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
