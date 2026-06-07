//! Programmatic file-type icons, drawn as small PNGs and rendered through the
//! image layer (Kitty graphics) so desktop / file-manager icons are crisp and
//! genuinely large instead of a single terminal glyph. One rounded-rect "tile"
//! per `Role`, tinted by type, with a simple white symbol.

use crate::openwith::Role;
use image::{Rgba, RgbaImage};

type Px = Rgba<u8>;

const CLEAR: Px = Rgba([0, 0, 0, 0]);
const WHITE: Px = Rgba([245, 248, 252, 255]);

/// Card (tile) colour and ink (symbol) colour for a role.
fn palette(role: Role) -> (Px, Px) {
    let card = match role {
        Role::Directory => Rgba([74, 144, 217, 255]),  // blue
        Role::Image => Rgba([46, 174, 122, 255]),       // green
        Role::Audio => Rgba([150, 99, 214, 255]),       // purple
        Role::Video => Rgba([214, 92, 110, 255]),       // red/pink
        Role::Archive => Rgba([214, 160, 70, 255]),     // amber
        Role::Pdf => Rgba([206, 64, 64, 255]),          // red
        Role::Code => Rgba([83, 110, 150, 255]),        // slate-blue
        Role::Text => Rgba([110, 122, 142, 255]),       // slate
        _ => Rgba([120, 130, 150, 255]),                // grey
    };
    (card, WHITE)
}

/// Render the icon for `role` as PNG bytes sized `w × h` pixels.
pub fn role_icon_png(role: Role, w: u32, h: u32) -> Option<Vec<u8>> {
    let mut img = RgbaImage::from_pixel(w, h, CLEAR);
    let (card, ink) = palette(role);
    let (wi, hi) = (w as i32, h as i32);
    let pad = (w.min(h) as i32 / 9).max(2);
    let radius = (w.min(h) as i32 / 6).max(3);
    fill_rrect(&mut img, pad, pad, wi - pad - 1, hi - pad - 1, radius, card);
    draw_symbol(&mut img, role, w, h, ink);
    encode(&img)
}

fn draw_symbol(img: &mut RgbaImage, role: Role, w: u32, h: u32, ink: Px) {
    let (wi, hi) = (w as i32, h as i32);
    // Symbol bounding box: the centre ~52% of the tile.
    let bx = wi * 24 / 100;
    let by = hi * 26 / 100;
    let bw = wi * 52 / 100;
    let bh = hi * 48 / 100;
    match role {
        Role::Directory => {
            // Folder: a tab on top-left + a body.
            let tab_w = bw * 45 / 100;
            fill_rrect(img, bx, by, bx + tab_w, by + bh * 28 / 100, 2, ink);
            fill_rrect(img, bx, by + bh * 18 / 100, bx + bw, by + bh, 3, ink);
        }
        Role::Video => {
            // Play triangle.
            fill_tri(
                img,
                (bx + bw * 15 / 100, by),
                (bx + bw * 15 / 100, by + bh),
                (bx + bw, by + bh / 2),
                ink,
            );
        }
        Role::Audio => {
            // Note: stem + head.
            let stem_x = bx + bw * 70 / 100;
            fill_rect(img, stem_x, by, stem_x + bw / 12 + 1, by + bh * 78 / 100, ink);
            fill_circle(img, bx + bw * 35 / 100, by + bh * 80 / 100, bw * 24 / 100, ink);
        }
        Role::Image => {
            // Photo: white frame, a "sun" circle and a "mountain" triangle.
            fill_rrect(img, bx, by, bx + bw, by + bh, 2, ink);
            let (card, _) = palette(Role::Image);
            fill_circle(img, bx + bw * 30 / 100, by + bh * 32 / 100, bw * 12 / 100, card);
            fill_tri(
                img,
                (bx + bw * 18 / 100, by + bh * 88 / 100),
                (bx + bw * 55 / 100, by + bh * 45 / 100),
                (bx + bw * 92 / 100, by + bh * 88 / 100),
                card,
            );
        }
        Role::Archive => {
            // Box with a vertical zipper line + a couple of teeth.
            fill_rrect(img, bx, by, bx + bw, by + bh, 2, ink);
            let (card, _) = palette(Role::Archive);
            let zx = bx + bw / 2;
            fill_rect(img, zx - 1, by, zx + 1, by + bh, card);
            for k in 0..3 {
                let yy = by + bh * (20 + k * 25) / 100;
                fill_rect(img, zx - 3, yy, zx + 3, yy + bh / 12 + 1, card);
            }
        }
        _ => {
            // Document: a page with a folded corner + ruled lines.
            fill_rrect(img, bx, by, bx + bw, by + bh, 2, ink);
            let (card, _) = palette(role);
            // folded corner
            let fc = bw * 28 / 100;
            fill_tri(img, (bx + bw - fc, by), (bx + bw, by), (bx + bw, by + fc), card);
            // ruled text lines
            for k in 0..3 {
                let yy = by + bh * (38 + k * 20) / 100;
                fill_rect(img, bx + bw * 16 / 100, yy, bx + bw * 84 / 100, yy + bh / 16 + 1, card);
            }
        }
    }
}

// ── pixel primitives ────────────────────────────────────────────────────────

fn put(img: &mut RgbaImage, x: i32, y: i32, c: Px) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, c);
    }
}

fn fill_rect(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, c: Px) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            put(img, x, y, c);
        }
    }
}

/// Filled rounded rectangle with corner radius `r`.
fn fill_rrect(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, r: i32, c: Px) {
    let r = r.min((x1 - x0) / 2).min((y1 - y0) / 2).max(0);
    for y in y0..=y1 {
        for x in x0..=x1 {
            // Skip the four corner quadrants outside the radius.
            let dx = if x < x0 + r { x0 + r - x } else if x > x1 - r { x - (x1 - r) } else { 0 };
            let dy = if y < y0 + r { y0 + r - y } else if y > y1 - r { y - (y1 - r) } else { 0 };
            if dx * dx + dy * dy <= r * r {
                put(img, x, y, c);
            }
        }
    }
}

fn fill_circle(img: &mut RgbaImage, cx: i32, cy: i32, radius: i32, c: Px) {
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy <= radius * radius {
                put(img, x, y, c);
            }
        }
    }
}

fn fill_tri(img: &mut RgbaImage, a: (i32, i32), b: (i32, i32), c2: (i32, i32), c: Px) {
    let min_x = a.0.min(b.0).min(c2.0);
    let max_x = a.0.max(b.0).max(c2.0);
    let min_y = a.1.min(b.1).min(c2.1);
    let max_y = a.1.max(b.1).max(c2.1);
    let sign = |p: (i32, i32), q: (i32, i32), r: (i32, i32)| {
        (p.0 - r.0) * (q.1 - r.1) - (q.0 - r.0) * (p.1 - r.1)
    };
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let p = (x, y);
            let d1 = sign(p, a, b);
            let d2 = sign(p, b, c2);
            let d3 = sign(p, c2, a);
            let neg = d1 < 0 || d2 < 0 || d3 < 0;
            let pos = d1 > 0 || d2 > 0 || d3 > 0;
            if !(neg && pos) {
                put(img, x, y, c);
            }
        }
    }
}

fn encode(img: &RgbaImage) -> Option<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut buf, image::ImageFormat::Png)
        .ok()?;
    Some(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openwith::Role;

    #[test]
    fn every_role_icon_is_a_valid_png_of_requested_size() {
        for role in [Role::Directory, Role::Image, Role::Audio, Role::Video, Role::Archive, Role::Pdf, Role::Text, Role::Code, Role::Other] {
            let png = role_icon_png(role, 160, 128).expect("icon encodes");
            let img = image::load_from_memory(&png).expect("valid PNG");
            use image::GenericImageView;
            assert_eq!(img.dimensions(), (160, 128), "role {role:?}");
        }
    }
}
