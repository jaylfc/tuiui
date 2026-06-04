//! The menubar status tray: indicator segments + click-through popovers.

use crate::geometry::Rect;
use crate::system::{bars_glyph, mem_pct, volume_glyph, SystemState};

/// Which indicator a tray segment represents (used for hit-testing + drop order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    Cpu,
    Mem,
    Battery,
    Volume,
    Bluetooth,
    Wifi,
    Clock,
}

/// A laid-out tray segment: its kind, display text, and screen rect (row 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub text: String,
    pub rect: Rect,
}

/// Reserve this many columns on the right for the Quit button (matches chrome).
const QUIT_RESERVE: i32 = 9;
/// Columns of space between adjacent segments.
const GAP: i32 = 2;

/// Build the right-aligned, ordered list of tray segments for a `width`-wide
/// menubar. Lowest-priority segments (CPU, then Memory, then Battery) drop out
/// first when space is tight; the clock is always kept.
pub fn tray_segments(state: &SystemState, width: i32) -> Vec<Segment> {
    // Display order, left→right.
    let mut texts: Vec<(SegmentKind, String)> = Vec::new();
    texts.push((SegmentKind::Cpu, format!("⊙{}%", state.cpu_pct.round() as u32)));
    texts.push((SegmentKind::Mem, format!("▤{}%", mem_pct(state.mem.used, state.mem.total))));
    if let Some(b) = &state.battery {
        texts.push((
            SegmentKind::Battery,
            format!("{}{}%", if b.charging { "⚡" } else { "🔋" }, b.pct),
        ));
    }
    texts.push((
        SegmentKind::Volume,
        format!("{}{}", volume_glyph(&state.volume), state.volume.level),
    ));
    if state.caps.bluetooth || state.bluetooth.enabled {
        texts.push((SegmentKind::Bluetooth, "⏻bt".to_string()));
    }
    if let Some(w) = &state.wifi {
        let name = if w.ssid.is_empty() { "wifi".to_string() } else { w.ssid.clone() };
        texts.push((SegmentKind::Wifi, format!("{} {}", bars_glyph(w.signal), name)));
    }
    texts.push((SegmentKind::Clock, state.clock.time.clone()));

    // Drop order when out of space: Cpu, Mem, Battery (Clock always kept).
    let avail = width - QUIT_RESERVE - 1;
    let total = |v: &[(SegmentKind, String)]| -> i32 {
        v.iter().map(|(_, t)| t.chars().count() as i32 + GAP).sum()
    };
    for k in [SegmentKind::Cpu, SegmentKind::Mem, SegmentKind::Battery] {
        if total(&texts) <= avail {
            break;
        }
        texts.retain(|(sk, _)| *sk != k);
    }

    // Right-align: lay out from the right edge leftward, then return left→right.
    let mut x = width - QUIT_RESERVE - 1;
    let mut out: Vec<Segment> = Vec::new();
    for (kind, text) in texts.iter().rev() {
        let w = text.chars().count() as i32;
        x -= w;
        out.push(Segment { kind: *kind, text: text.clone(), rect: Rect::new(x, 0, w, 1) });
        x -= GAP;
    }
    out.reverse();
    out
}
