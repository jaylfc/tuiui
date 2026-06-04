use tuiui::system::{ClockInfo, MemInfo, SystemState, VolumeInfo, WifiInfo};
use tuiui::tray::{tray_segments, SegmentKind};

fn sample() -> SystemState {
    SystemState {
        clock: ClockInfo { time: "09:41".into(), date: "Wed 04 Jun".into(), uptime_secs: 0 },
        cpu_pct: 32.0,
        mem: MemInfo { used: 6, total: 10 },
        wifi: Some(WifiInfo { ssid: "wlan".into(), signal: 3, enabled: true }),
        volume: VolumeInfo { level: 60, muted: false },
        ..Default::default()
    }
}

#[test]
fn segments_are_right_aligned_and_on_row_zero() {
    let segs = tray_segments(&sample(), 100);
    let clock = segs.iter().find(|s| s.kind == SegmentKind::Clock).unwrap();
    let max_x = segs.iter().map(|s| s.rect.x + s.rect.w).max().unwrap();
    assert_eq!(clock.rect.x + clock.rect.w, max_x);
    assert!(segs.iter().all(|s| s.rect.y == 0));
}

#[test]
fn narrow_width_drops_cpu_but_keeps_clock() {
    let wide = tray_segments(&sample(), 100);
    let narrow = tray_segments(&sample(), 24);
    assert!(narrow.iter().any(|s| s.kind == SegmentKind::Clock));
    assert!(!narrow.iter().any(|s| s.kind == SegmentKind::Cpu));
    assert!(wide.iter().any(|s| s.kind == SegmentKind::Cpu));
}

#[test]
fn segments_do_not_overlap() {
    let segs = tray_segments(&sample(), 100);
    for pair in segs.windows(2) {
        assert!(pair[0].rect.x + pair[0].rect.w <= pair[1].rect.x);
    }
}

use tuiui::geometry::Point;
use tuiui::system::ControlIntent;
use tuiui::tray::Tray;

#[test]
fn clicking_a_segment_opens_then_closes_its_popover() {
    let mut tray = Tray::new();
    let segs = tray_segments(&sample(), 100);
    let vol = segs.iter().find(|s| s.kind == SegmentKind::Volume).unwrap();
    assert!(tray.on_menubar_click(Point::new(vol.rect.x, 0), &segs));
    assert_eq!(tray.open(), Some(SegmentKind::Volume));
    // Clicking the same segment again closes it.
    assert!(tray.on_menubar_click(Point::new(vol.rect.x, 0), &segs));
    assert_eq!(tray.open(), None);
}

#[test]
fn volume_popover_arrows_yield_intents() {
    let mut tray = Tray::new();
    tray.force_open(SegmentKind::Volume);
    let r = tray.render(100, 30, &sample());
    let up = r.hits.iter().find(|h| h.intent == ControlIntent::VolumeUp).unwrap();
    assert_eq!(tray.on_popover_click(up.rect.center(), &r), Some(ControlIntent::VolumeUp));
    let down = r.hits.iter().find(|h| h.intent == ControlIntent::VolumeDown).unwrap();
    assert_eq!(tray.on_popover_click(down.rect.center(), &r), Some(ControlIntent::VolumeDown));
}

#[test]
fn closed_tray_renders_nothing() {
    let tray = Tray::new();
    let r = tray.render(100, 30, &sample());
    assert!(r.layers.is_empty());
    assert!(r.bounds.is_none());
}
