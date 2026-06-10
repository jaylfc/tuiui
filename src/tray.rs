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

/// Columns of space between adjacent segments.
const GAP: i32 = 2;

/// Build the right-aligned, ordered list of tray segments for a `width`-wide
/// menubar, leaving `reserve` columns free on the right for the power button
/// (whose label is the host name, so its width varies per machine). Lowest-
/// priority segments (CPU, then Memory, then Battery) drop out first when
/// space is tight; the clock is always kept.
pub fn tray_segments(state: &SystemState, width: i32, reserve: i32) -> Vec<Segment> {
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
    // Clock shows date + time ("Wed 04 Jun 09:41"); narrows to time-only first.
    let clock_full = if state.clock.date.is_empty() {
        state.clock.time.clone()
    } else {
        format!("{} {}", state.clock.date, state.clock.time)
    };
    texts.push((SegmentKind::Clock, clock_full));

    // When out of space: first shrink the clock to time-only, then drop Cpu,
    // Mem, Battery (the clock itself is always kept).
    let avail = width - reserve - 1;
    let total = |v: &[(SegmentKind, String)]| -> i32 {
        v.iter().map(|(_, t)| t.chars().count() as i32 + GAP).sum()
    };
    if total(&texts) > avail {
        if let Some(c) = texts.iter_mut().find(|(k, _)| *k == SegmentKind::Clock) {
            c.1 = state.clock.time.clone();
        }
    }
    for k in [SegmentKind::Cpu, SegmentKind::Mem, SegmentKind::Battery] {
        if total(&texts) <= avail {
            break;
        }
        texts.retain(|(sk, _)| *sk != k);
    }

    // Right-align: lay out from the right edge leftward, then return left→right.
    let mut x = width - reserve - 1;
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

// ── Interactive tray: open popover state + rendering + hit-testing ─────────────

use crate::buffer::CellBuffer;
use crate::cell::Cell;
use crate::compositor::Layer;
use crate::geometry::Point;
use crate::system::{ControlIntent, SystemState as St};

/// A clickable hot-zone inside an open popover: its screen rect and the intent
/// it produces when clicked.
#[derive(Clone, Debug, PartialEq)]
pub struct PopoverHit {
    pub rect: Rect,
    pub intent: ControlIntent,
}

/// A rendered popover: compositor layers, clickable hits, and the popover's outer
/// bounds (so the session can detect clicks outside it).
#[derive(Default)]
pub struct Rendered {
    pub layers: Vec<Layer>,
    pub hits: Vec<PopoverHit>,
    pub bounds: Option<Rect>,
}

/// The interactive menubar tray: tracks which segment's popover is open (and the
/// x it is anchored under), renders the popover, and maps clicks to intents.
#[derive(Default)]
pub struct Tray {
    open: Option<(SegmentKind, i32)>,
    /// Months the calendar popover is offset from the current month (◂ ▸ nav).
    cal_offset: i32,
}

impl Tray {
    pub fn new() -> Self {
        Self::default()
    }

    /// The kind whose popover is currently open, if any.
    pub fn open(&self) -> Option<SegmentKind> {
        self.open.map(|(k, _)| k)
    }

    /// Open `kind`'s popover at a default anchor (used by tests).
    pub fn force_open(&mut self, kind: SegmentKind) {
        self.open = Some((kind, 20));
        self.cal_offset = 0;
    }

    /// Close any open popover.
    pub fn close(&mut self) {
        self.open = None;
    }

    /// Step the calendar popover by `delta` months (◂ ▸ clicks).
    pub fn calendar_step(&mut self, delta: i32) {
        self.cal_offset += delta;
    }

    /// Handle a click on the menubar row: toggle the popover of the clicked
    /// segment. Returns `true` if the click hit a segment (handled).
    pub fn on_menubar_click(&mut self, p: Point, segments: &[Segment]) -> bool {
        if p.y != 0 {
            return false;
        }
        if let Some(seg) = segments.iter().find(|s| s.rect.contains(p)) {
            self.open = match self.open {
                Some((k, _)) if k == seg.kind => None,
                _ => {
                    self.cal_offset = 0; // calendar always opens on the current month
                    Some((seg.kind, seg.rect.x))
                }
            };
            return true;
        }
        false
    }

    /// Map a click inside the open popover to a `ControlIntent`, if it landed on
    /// a hot-zone.
    pub fn on_popover_click(&self, p: Point, r: &Rendered) -> Option<ControlIntent> {
        r.hits.iter().find(|h| h.rect.contains(p)).map(|h| h.intent.clone())
    }

    /// Render the open popover (if any) into layers + hits.
    pub fn render(&self, w: i32, h: i32, state: &St) -> Rendered {
        let Some((kind, anchor_x)) = self.open else { return Rendered::default() };
        match kind {
            SegmentKind::Volume => self.render_volume(w, h, anchor_x, state),
            SegmentKind::Wifi => self.render_wifi(w, h, anchor_x, state),
            SegmentKind::Bluetooth => self.render_bluetooth(w, h, anchor_x, state),
            SegmentKind::Clock => self.render_calendar(w, h, anchor_x, state),
            SegmentKind::Cpu => self.render_lines(w, h, anchor_x, "CPU", &[
                format!("{:.0}% load", state.cpu_pct),
            ]),
            SegmentKind::Mem => self.render_lines(w, h, anchor_x, "Memory", &[
                format!("{}% used", mem_pct(state.mem.used, state.mem.total)),
            ]),
            SegmentKind::Battery => self.render_lines(w, h, anchor_x, "Battery", &[
                state.battery.map(|b| format!("{}%{}", b.pct, if b.charging { " ⚡" } else { "" }))
                    .unwrap_or_else(|| "no battery".into()),
            ]),
        }
    }

    fn box_origin(&self, w: i32, anchor_x: i32, box_w: i32) -> Point {
        let x = anchor_x.min(w - box_w - 1).max(0);
        Point::new(x, 1)
    }

    fn render_volume(&self, w: i32, _h: i32, anchor_x: i32, st: &St) -> Rendered {
        let box_w = 24;
        let box_h = 4;
        let origin = self.box_origin(w, anchor_x, box_w);
        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h);
        let t = crate::theme::current();
        let v = &st.volume;
        buf.write_str(2, 1, &format!("Volume {}{}", v.level, if v.muted { " (muted)" } else { "" }), t.text, t.window_bg);
        // Row 2: ◂  bar  ▸   speaker
        let filled = (v.level as i32 * 6 / 100).clamp(0, 6) as usize;
        let bar: String = (0..6).map(|i| if i < filled { '▮' } else { '▯' }).collect();
        buf.write_str(2, 2, "◂", t.accent, t.window_bg);
        buf.write_str(5, 2, &bar, t.text, t.window_bg);
        buf.write_str(13, 2, "▸", t.accent, t.window_bg);
        buf.write_str(17, 2, volume_glyph(v), t.text, t.window_bg);
        let hit = |lx: i32, lw: i32, intent: ControlIntent| PopoverHit {
            rect: Rect::new(origin.x + lx, origin.y + 2, lw, 1),
            intent,
        };
        let hits = vec![
            hit(2, 1, ControlIntent::VolumeDown),
            hit(13, 1, ControlIntent::VolumeUp),
            hit(17, 2, ControlIntent::ToggleMute),
        ];
        Rendered { layers: vec![layer(origin, buf)], hits, bounds: Some(Rect::new(origin.x, origin.y, box_w, box_h)) }
    }

    fn render_wifi(&self, w: i32, _h: i32, anchor_x: i32, st: &St) -> Rendered {
        let t = crate::theme::current();
        let enabled = st.wifi.as_ref().map(|x| x.enabled).unwrap_or(false);
        let cur = st.wifi.as_ref().map(|x| x.ssid.clone()).unwrap_or_default();
        let nets: Vec<String> = st.known_networks.iter().take(6).cloned().collect();
        let box_w = 28;
        let box_h = 3 + nets.len() as i32;
        let origin = self.box_origin(w, anchor_x, box_w);
        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h);
        buf.write_str(2, 1, &format!("Wi-Fi  [{}]", if enabled { "on" } else { "off" }), t.accent, t.window_bg);
        let mut hits = vec![PopoverHit {
            rect: Rect::new(origin.x + 9, origin.y + 1, 4, 1),
            intent: ControlIntent::WifiSetEnabled(!enabled),
        }];
        for (i, ssid) in nets.iter().enumerate() {
            let y = 2 + i as i32;
            let mark = if *ssid == cur { "●" } else { " " };
            buf.write_str(2, y, &format!("{} {}", mark, ssid), t.text, t.window_bg);
            hits.push(PopoverHit {
                rect: Rect::new(origin.x + 1, origin.y + y, box_w - 2, 1),
                intent: ControlIntent::WifiConnectKnown(ssid.clone()),
            });
        }
        Rendered { layers: vec![layer(origin, buf)], hits, bounds: Some(Rect::new(origin.x, origin.y, box_w, box_h)) }
    }

    fn render_bluetooth(&self, w: i32, _h: i32, anchor_x: i32, st: &St) -> Rendered {
        let t = crate::theme::current();
        let bt = &st.bluetooth;
        let devs: Vec<_> = bt.devices.iter().take(6).cloned().collect();
        let box_w = 30;
        let box_h = 3 + devs.len().max(1) as i32;
        let origin = self.box_origin(w, anchor_x, box_w);
        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h);
        buf.write_str(2, 1, &format!("Bluetooth  [{}]", if bt.enabled { "on" } else { "off" }), t.accent, t.window_bg);
        let mut hits = vec![PopoverHit {
            rect: Rect::new(origin.x + 13, origin.y + 1, 4, 1),
            intent: ControlIntent::BtSetEnabled(!bt.enabled),
        }];
        if devs.is_empty() && st.caps.bluetooth {
            buf.write_str(2, 2, "(no paired devices)", t.dim, t.window_bg);
        } else if !st.caps.bluetooth {
            buf.write_str(2, 2, "install blueutil to control", t.dim, t.window_bg);
        }
        for (i, d) in devs.iter().enumerate() {
            let y = 2 + i as i32;
            let mark = if d.connected { "●" } else { "○" };
            buf.write_str(2, y, &format!("{} {}", mark, d.name), t.text, t.window_bg);
            hits.push(PopoverHit {
                rect: Rect::new(origin.x + 1, origin.y + y, box_w - 2, 1),
                intent: ControlIntent::BtConnect { addr: d.addr.clone(), connect: !d.connected },
            });
        }
        Rendered { layers: vec![layer(origin, buf)], hits, bounds: Some(Rect::new(origin.x, origin.y, box_w, box_h)) }
    }

    /// The clock popover: a month calendar with ◂ ▸ navigation, today highlighted,
    /// and a time/uptime footer. Falls back to plain text until the first poll
    /// has produced a civil date.
    fn render_calendar(&self, w: i32, h: i32, anchor_x: i32, st: &St) -> Rendered {
        let c = &st.clock;
        if c.year == 0 {
            return self.render_lines(w, h, anchor_x, "Clock", &[
                c.date.clone(),
                format!("up {}h", c.uptime_secs / 3600),
            ]);
        }
        let t = crate::theme::current();
        let (year, month) = crate::calendar::add_months(c.year, c.month, self.cal_offset);
        let weeks = crate::calendar::month_grid(year, month);
        let box_w = 24;
        let box_h = weeks.len() as i32 + 5;
        let origin = self.box_origin(w, anchor_x, box_w);
        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h);
        // Month header with prev/next arrows.
        let title = format!("{} {}", crate::calendar::month_name(month), year);
        let tx = (box_w - title.chars().count() as i32) / 2;
        buf.write_str(2, 1, "◂", t.accent, t.window_bg);
        buf.write_str(tx, 1, &title, t.accent, t.window_bg);
        buf.write_str(box_w - 3, 1, "▸", t.accent, t.window_bg);
        buf.write_str(2, 2, "Mo Tu We Th Fr Sa Su", t.dim, t.window_bg);
        for (wi, week) in weeks.iter().enumerate() {
            let y = 3 + wi as i32;
            for (di, day) in week.iter().enumerate() {
                if let Some(d) = day {
                    let x = 2 + di as i32 * 3;
                    let today = self.cal_offset == 0 && *d == c.day;
                    let (fg, bg) = if today { (t.close_fg, t.accent) } else { (t.text, t.window_bg) };
                    buf.write_str(x, y, &format!("{d:>2}"), fg, bg);
                }
            }
        }
        let footer = format!("{}  up {}h", c.time, c.uptime_secs / 3600);
        buf.write_str(2, box_h - 2, &footer, t.dim, t.window_bg);
        let hits = vec![
            PopoverHit {
                rect: Rect::new(origin.x + 1, origin.y + 1, 3, 1),
                intent: ControlIntent::CalendarPrev,
            },
            PopoverHit {
                rect: Rect::new(origin.x + box_w - 4, origin.y + 1, 3, 1),
                intent: ControlIntent::CalendarNext,
            },
        ];
        Rendered { layers: vec![layer(origin, buf)], hits, bounds: Some(Rect::new(origin.x, origin.y, box_w, box_h)) }
    }

    fn render_lines(&self, w: i32, _h: i32, anchor_x: i32, title: &str, lines: &[String]) -> Rendered {
        let t = crate::theme::current();
        let box_w = 24;
        let box_h = 2 + lines.len() as i32;
        let origin = self.box_origin(w, anchor_x, box_w);
        let mut buf = CellBuffer::new(box_w, box_h);
        fill_box(&mut buf, box_w, box_h);
        buf.write_str(2, 1, title, t.accent, t.window_bg);
        for (i, l) in lines.iter().enumerate() {
            buf.write_str(2, 2 + i as i32, l, t.text, t.window_bg);
        }
        Rendered { layers: vec![layer(origin, buf)], hits: Vec::new(), bounds: Some(Rect::new(origin.x, origin.y, box_w, box_h)) }
    }
}

fn layer(origin: Point, buf: CellBuffer) -> Layer {
    Layer { z: 5000, origin, buf, opacity: 1.0, scissor: None }
}

/// Fill a buffer with the window background and draw a rounded border.
fn fill_box(buf: &mut CellBuffer, w: i32, h: i32) {
    let t = crate::theme::current();
    buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
    let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
    for x in 0..w {
        buf.set(x, 0, b('─'));
        buf.set(x, h - 1, b('─'));
    }
    for y in 0..h {
        buf.set(0, y, b('│'));
        buf.set(w - 1, y, b('│'));
    }
    buf.set(0, 0, b('╭'));
    buf.set(w - 1, 0, b('╮'));
    buf.set(0, h - 1, b('╰'));
    buf.set(w - 1, h - 1, b('╯'));
}
