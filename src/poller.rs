//! Background poller that refreshes a shared [`SystemState`] on throttled
//! cadences. Clock + CPU/memory update ~every second; the shell-backed metrics
//! (WiFi/volume/Bluetooth) refresh ~every three seconds. All shell-outs are
//! timeout-guarded, so the poller can never hang the desktop.

use crate::system::{backend, BatteryInfo, ClockInfo, SystemState};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Owns a background thread that periodically refreshes a shared snapshot.
pub struct SystemPoller {
    state: Arc<RwLock<SystemState>>,
}

impl SystemPoller {
    /// Spawn the poller thread and return a handle whose `state()` is updated in
    /// place. The thread runs for the lifetime of the process (daemon).
    pub fn start() -> Self {
        let state = Arc::new(RwLock::new(SystemState::default()));
        let worker = state.clone();
        std::thread::spawn(move || run(worker));
        SystemPoller { state }
    }

    /// The shared snapshot handle (cloned into the session).
    pub fn state(&self) -> Arc<RwLock<SystemState>> {
        self.state.clone()
    }
}

fn run(state: Arc<RwLock<SystemState>>) {
    let backend = backend();
    let mut sys = sysinfo::System::new_all();
    let mut last_slow = Instant::now()
        .checked_sub(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);

    // Remote reachability + calendar events refresh on their own (slowest)
    // cadence, on a separate thread so a slow ssh-port probe or `khal` run can
    // never delay the clock/CPU updates.
    {
        let state = state.clone();
        std::thread::spawn(move || loop {
            let online = probe_remotes();
            let events = read_khal_events();
            if let Ok(mut s) = state.write() {
                s.remotes_online = online;
                s.events = events;
            }
            std::thread::sleep(Duration::from_secs(20));
        });
    }

    loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let cpu_pct = sys.global_cpu_usage();
        let (used, total) = (sys.used_memory(), sys.total_memory());
        let clock = now_clock();
        let battery = read_battery();

        // The shell-backed metrics refresh less often than CPU/clock.
        let slow = last_slow.elapsed() >= Duration::from_secs(3);
        let readout = if slow {
            last_slow = Instant::now();
            Some((backend.read(), backend.caps()))
        } else {
            None
        };

        if let Ok(mut s) = state.write() {
            s.clock = clock;
            s.cpu_pct = cpu_pct;
            s.mem.used = used;
            s.mem.total = total;
            s.battery = battery;
            if let Some((r, caps)) = readout {
                s.wifi = r.wifi;
                s.bluetooth = r.bluetooth;
                s.volume = r.volume;
                s.known_networks = r.known_networks;
                s.caps = caps;
            }
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}

fn now_clock() -> ClockInfo {
    // Local time via `date` (cheap, avoids a chrono dependency); uptime via sysinfo.
    let time = crate::system::run_capped("date", &["+%H:%M"], 1)
        .unwrap_or_default()
        .trim()
        .to_string();
    let date = crate::system::run_capped("date", &["+%a %d %b"], 1)
        .unwrap_or_default()
        .trim()
        .to_string();
    // Civil date for the menubar calendar (one call, parsed locally).
    let ymd = crate::system::run_capped("date", &["+%Y-%m-%d"], 1).unwrap_or_default();
    let mut parts = ymd.trim().splitn(3, '-');
    let year = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let month = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let day = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    ClockInfo { time, date, uptime_secs: sysinfo::System::uptime(), year, month, day }
}

/// Battery is reported only on hosts that have one; `None` hides the segment.
/// Linux reads sysfs directly; macOS shells out to `pmset` (timeout-guarded).
fn read_battery() -> Option<BatteryInfo> {
    #[cfg(target_os = "linux")]
    {
        let rd = std::fs::read_dir("/sys/class/power_supply").ok()?;
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("BAT") {
                continue;
            }
            let p = e.path();
            let cap = std::fs::read_to_string(p.join("capacity"))
                .ok()
                .and_then(|s| s.trim().parse::<u8>().ok());
            if let Some(pct) = cap {
                let charging = std::fs::read_to_string(p.join("status"))
                    .map(|s| matches!(s.trim(), "Charging" | "Full"))
                    .unwrap_or(false);
                return Some(BatteryInfo { pct: pct.min(100), charging });
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let out = crate::system::run_capped("pmset", &["-g", "batt"], 1)?;
        parse_pmset_batt(&out)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Probe each saved remote system's ssh port (TCP connect, 400ms cap) so the
/// Systems menu can show a live ●/○ reachability dot.
fn probe_remotes() -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for sys in crate::systems::load() {
        let host = sys.host.rsplit('@').next().unwrap_or(&sys.host).to_string();
        let port = sys.port.unwrap_or(22);
        let online = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port))
            .ok()
            .and_then(|mut addrs| addrs.next())
            .map(|addr| {
                std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
            })
            .unwrap_or(false);
        out.push((sys.name, online));
    }
    out
}

/// Upcoming events for the calendar popover, via `khal` when installed (the
/// catalog's terminal calendar). Missing khal → empty, popover shows none.
fn read_khal_events() -> Vec<crate::system::CalEvent> {
    let Some(out) = crate::system::run_capped(
        "khal",
        &["list", "--format", "{start-date-full}|{start-time} {title}", "today", "30d"],
        4,
    ) else {
        return Vec::new();
    };
    parse_khal_events(&out)
}

/// Parse khal `--format "{start-date-full}|{start-time} {title}"` lines. Day
/// headers and anything without the `|` or a parseable Y-M-D date are skipped,
/// so an unexpected khal config degrades to fewer events, never garbage.
fn parse_khal_events(out: &str) -> Vec<crate::system::CalEvent> {
    let mut events = Vec::new();
    for line in out.lines() {
        let Some((date, text)) = line.split_once('|') else { continue };
        let Some((y, m, d)) = parse_ymd(date.trim()) else { continue };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        events.push(crate::system::CalEvent { year: y, month: m, day: d, text: text.to_string() });
        if events.len() >= 30 {
            break;
        }
    }
    events
}

/// Parse a date as `YYYY-MM-DD` or `DD.MM.YYYY` (khal's `start-date-full`
/// honours the user's locale dateformat; these cover the common defaults).
fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let parts: Vec<&str> = if s.contains('-') {
        s.splitn(3, '-').collect()
    } else if s.contains('.') {
        let mut v: Vec<&str> = s.splitn(3, '.').collect();
        v.reverse(); // DD.MM.YYYY → YYYY MM DD
        v
    } else {
        return None;
    };
    if parts.len() != 3 {
        return None;
    }
    let y: i32 = parts[0].trim().parse().ok()?;
    let m: u32 = parts[1].trim().parse().ok()?;
    let d: u32 = parts[2].trim().parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
        return None;
    }
    Some((y, m, d))
}

/// Parse `pmset -g batt` output, e.g.
/// `Now drawing from 'AC Power'\n -InternalBattery-0 (id=…)  85%; charging; …`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_pmset_batt(out: &str) -> Option<BatteryInfo> {
    let pct = out
        .split_whitespace()
        .find_map(|w| w.strip_suffix("%;").or_else(|| w.strip_suffix('%')))
        .and_then(|n| n.parse::<u8>().ok())?;
    // "discharging" contains "charging", so match the delimited token.
    let charging = out.contains("AC Power") || out.contains("; charging") || out.contains("; charged");
    Some(BatteryInfo { pct: pct.min(100), charging })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmset_output_parses_pct_and_charging() {
        let charging = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=123)\t85%; charging; 0:42 remaining present: true";
        let b = parse_pmset_batt(charging).unwrap();
        assert_eq!(b.pct, 85);
        assert!(b.charging);

        let discharging = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=123)\t47%; discharging; 3:10 remaining present: true";
        let b = parse_pmset_batt(discharging).unwrap();
        assert_eq!(b.pct, 47);
        assert!(!b.charging, "'discharging' must not read as charging");

        assert!(parse_pmset_batt("Now drawing from 'AC Power'").is_none(), "no battery line → None");
    }

    #[test]
    fn khal_lines_parse_and_headers_skip() {
        let out = "Today, 2026-06-10\n2026-06-10|09:30 Standup\n2026-06-11| All-day thing\nnot an event\n15.06.2026|14:00 Dentist\n";
        let ev = parse_khal_events(out);
        assert_eq!(ev.len(), 3);
        assert_eq!((ev[0].year, ev[0].month, ev[0].day), (2026, 6, 10));
        assert_eq!(ev[0].text, "09:30 Standup");
        assert_eq!((ev[2].year, ev[2].month, ev[2].day), (2026, 6, 15), "DD.MM.YYYY accepted");
    }

    #[test]
    fn bad_dates_rejected() {
        assert!(parse_ymd("garbage").is_none());
        assert!(parse_ymd("2026-13-01").is_none());
        assert_eq!(parse_ymd("2026-06-10"), Some((2026, 6, 10)));
        assert_eq!(parse_ymd("10.06.2026"), Some((2026, 6, 10)));
    }
}
