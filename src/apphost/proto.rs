//! Wire protocol between the frontend (`RemoteAppHost`) and the apphost server.
//! Newline-delimited JSON, mirroring `crate::protocol`. Image bytes ride as
//! base64 strings; grids ride as serialized `CellBuffer`s.

use crate::buffer::CellBuffer;
use crate::kittygfx::Placement;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};

/// frontend → apphost.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum HostReq {
    Spawn { req_id: u64, cmd: String, args: Vec<String>, cwd: Option<String>, cols: i32, rows: i32 },
    Input { app: u64, bytes: Vec<u8> },
    Resize { app: u64, cols: i32, rows: i32 },
    /// Scroll the app's scrollback view by `lines` (+ = back into history).
    Scroll { app: u64, lines: i32 },
    SetMeta { app: u64, meta: Vec<u8> },
    Kill { app: u64 },
    /// Ask the apphost to enumerate its hosted apps. The apphost replies with
    /// `HostEvt::AppList`. Used by `tuiui ps` / `tuiui kill-app`.
    ListApps,
    /// Stop the apphost process entirely (full shutdown / `tuiui kill`).
    Shutdown,
}

/// A PNG the app transmitted, base64-encoded, sent once per (frontend, app, id).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ImgBlob {
    pub image_id: u32,
    pub png_b64: String,
}

/// The apphost wire-protocol version this binary speaks. Bump on ANY change
/// to `HostReq`/`HostEvt`.
///
/// v2 — added `HostReq::ListApps` and `HostEvt::AppList` for the activity
/// monitor (`tuiui ps` / `tuiui kill-app` / the in-app panel). Additive only,
/// so an old apphost stays compatible (an old frontend that never sends
/// `ListApps` is unaffected).
///
/// v3 — added an optional `pid` field to `HostEvt::Spawned` so the daemon
/// (via `RemoteAppHost`) can populate the activity monitor's `pid` column.
/// `#[serde(default)]` keeps it backward-compatible: v2 apphosts omit the
/// field and the v3 frontend fills `None`; v2 frontends ignore the unknown
/// field on a v3 wire payload (serde default is permissive). Do NOT bump
/// `MIN_COMPAT`.
pub const PROTO_VERSION: u32 = 3;

/// The OLDEST apphost protocol this frontend can safely talk to. Apphosts
/// predating the `proto` roster field report 0. Bump this to `PROTO_VERSION`
/// ONLY when a change genuinely breaks older apphosts — doing so arms the
/// post-update safety dialog ("restart the app server, closes your apps"),
/// giving users a chance to save work instead of silent breakage.
pub const MIN_COMPAT: u32 = 0;

/// One app's metadata in the on-connect roster.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RosterEntry {
    pub app: u64,
    pub meta: Vec<u8>,
}

/// One row in `HostEvt::AppList` — a snapshot of a hosted app's state. Used by
/// `tuiui ps`. `age_secs` is seconds since the app was spawned (so the client
/// doesn't need a synchronized clock with the apphost).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppListEntry {
    pub app: u64,
    pub cmd: String,
    pub args: Vec<String>,
    pub pid: Option<u32>,
    pub cols: i32,
    pub rows: i32,
    pub age_secs: u64,
    pub alive: bool,
}

/// apphost → frontend.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum HostEvt {
    /// The app's current grid + placements, plus any not-yet-sent image blobs.
    Frame {
        app: u64,
        grid: CellBuffer,
        placements: Vec<Placement>,
        images: Vec<ImgBlob>,
        alive: bool,
        mouse: crate::mouse::AppMouse,
        /// Bell rings since the previous frame (default 0 across version skew).
        #[serde(default)]
        bells: u32,
        /// The app's latest OSC-52 clipboard store, forwarded to the host.
        #[serde(default)]
        clip: Option<String>,
    },
    /// The app's child exited.
    Gone { app: u64 },
    /// Sent right after a frontend connects so it can rebuild its window list.
    Roster {
        apps: Vec<RosterEntry>,
        /// The apphost's [`PROTO_VERSION`] (0 = an apphost too old to say).
        #[serde(default)]
        proto: u32,
    },
    SpawnFailed { req_id: u64, error: String },
    /// Reply to `HostReq::Spawn`. The apphost now also reports the child's
    /// OS pid (v3+; v2 apphosts omit the field, the frontend fills `None`).
    /// The daemon's activity monitor uses this to fill the panel's `pid`
    /// column in normal daemon mode.
    Spawned {
        req_id: u64,
        app: u64,
        #[serde(default)]
        pid: Option<u32>,
    },
    /// Reply to `HostReq::ListApps`.
    AppList { apps: Vec<AppListEntry> },
}

/// Write a newline-JSON message. Returns `Err` if the peer is gone.
pub fn send<T: Serialize, W: Write>(w: &mut W, msg: &T) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    buf.push(b'\n');
    w.write_all(&buf)
}

/// Read one newline-JSON message into `T`. `Ok(None)` on EOF.
pub fn recv<T: for<'de> Deserialize<'de>, R: BufRead>(r: &mut R) -> std::io::Result<Option<T>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let msg = serde_json::from_str(line.trim()).map_err(std::io::Error::other)?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_proto_round_trips_and_legacy_defaults_to_zero() {
        let evt = HostEvt::Roster { apps: vec![], proto: PROTO_VERSION };
        let mut buf: Vec<u8> = Vec::new();
        send(&mut buf, &evt).unwrap();
        let mut r = std::io::BufReader::new(&buf[..]);
        match recv::<HostEvt, _>(&mut r).unwrap().unwrap() {
            HostEvt::Roster { proto, .. } => assert_eq!(proto, PROTO_VERSION),
            _ => panic!("wrong variant"),
        }
        // A roster from an apphost that predates the field parses with proto 0.
        let legacy = br#"{"Roster":{"apps":[]}}
"#;
        let mut r = std::io::BufReader::new(&legacy[..]);
        match recv::<HostEvt, _>(&mut r).unwrap().unwrap() {
            HostEvt::Roster { proto, .. } => assert_eq!(proto, 0),
            _ => panic!("wrong variant"),
        }
    }
    #[test]
    fn spawned_with_pid_round_trips_and_legacy_omits_pid() {
        // Modern v3 wire: Spawned carries the child pid.
        let evt = HostEvt::Spawned { req_id: 42, app: 7, pid: Some(12345) };
        let mut buf: Vec<u8> = Vec::new();
        send(&mut buf, &evt).unwrap();
        let mut r = std::io::BufReader::new(&buf[..]);
        match recv::<HostEvt, _>(&mut r).unwrap().unwrap() {
            HostEvt::Spawned { req_id, app, pid } => {
                assert_eq!(req_id, 42);
                assert_eq!(app, 7);
                assert_eq!(pid, Some(12345));
            }
            _ => panic!("wrong variant"),
        }
        // A v2 apphost that predates the pid field still parses fine on a
        // v3 frontend (#[serde(default)] on `pid`).
        let legacy = br#"{"Spawned":{"req_id":1,"app":2}}
"#;
        let mut r = std::io::BufReader::new(&legacy[..]);
        match recv::<HostEvt, _>(&mut r).unwrap().unwrap() {
            HostEvt::Spawned { req_id, app, pid } => {
                assert_eq!(req_id, 1);
                assert_eq!(app, 2);
                assert_eq!(pid, None);
            }
            _ => panic!("wrong variant"),
        }
    }
    use crate::cell::Cell;

    #[test]
    fn req_round_trips() {
        let msgs = vec![
            HostReq::Spawn { req_id: 7, cmd: "sh".into(), args: vec!["-c".into()], cwd: None, cols: 80, rows: 24 },
            HostReq::Input { app: 3, bytes: vec![1, 2, 3] },
            HostReq::Resize { app: 3, cols: 100, rows: 40 },
            HostReq::SetMeta { app: 3, meta: vec![9, 9] },
            HostReq::Kill { app: 3 },
            HostReq::ListApps,
            HostReq::Shutdown,
        ];
        for m in msgs {
            let mut buf: Vec<u8> = Vec::new();
            send(&mut buf, &m).unwrap();
            let mut r = std::io::BufReader::new(&buf[..]);
            let back: HostReq = recv(&mut r).unwrap().unwrap();
            assert_eq!(format!("{m:?}"), format!("{back:?}"));
        }
    }

    #[test]
    fn frame_round_trips_with_grid() {
        let mut grid = CellBuffer::new(4, 2);
        grid.set(1, 0, Cell { ch: 'X', ..Default::default() });
        let evt = HostEvt::Frame {
            app: 5,
            grid: grid.clone(),
            placements: vec![Placement { image_id: 1, col: 0, row: 0, cols: 2, rows: 1 }],
            images: vec![ImgBlob { image_id: 1, png_b64: "QUJD".into() }],
            alive: true,
            mouse: Default::default(),
            bells: 2,
            clip: Some("copied".into()),
        };
        let mut buf: Vec<u8> = Vec::new();
        send(&mut buf, &evt).unwrap();
        let mut r = std::io::BufReader::new(&buf[..]);
        let back: HostEvt = recv(&mut r).unwrap().unwrap();
        match back {
            HostEvt::Frame { app, grid: g, placements, images, alive, mouse, bells, clip } => {
                assert_eq!(app, 5);
                assert_eq!(g, grid);
                assert_eq!(placements.len(), 1);
                assert_eq!(images[0].png_b64, "QUJD");
                assert!(alive);
                assert_eq!(mouse, Default::default());
                assert_eq!(bells, 2);
                assert_eq!(clip.as_deref(), Some("copied"));
            }
            _ => panic!("wrong variant"),
        }
    }
}
