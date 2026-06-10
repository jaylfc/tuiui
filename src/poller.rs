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

/// Battery is reported only on hosts that have one. Absent (the mini) → `None`,
/// which hides the segment. Populating it on laptop hosts is a follow-up.
fn read_battery() -> Option<BatteryInfo> {
    None
}
