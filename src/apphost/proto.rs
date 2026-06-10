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
    /// Stop the apphost process entirely (full shutdown / `tuiui kill`).
    Shutdown,
}

/// A PNG the app transmitted, base64-encoded, sent once per (frontend, app, id).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ImgBlob {
    pub image_id: u32,
    pub png_b64: String,
}

/// One app's metadata in the on-connect roster.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RosterEntry {
    pub app: u64,
    pub meta: Vec<u8>,
}

/// apphost → frontend.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum HostEvt {
    Spawned { req_id: u64, app: u64 },
    SpawnFailed { req_id: u64, error: String },
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
    Roster { apps: Vec<RosterEntry> },
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
    use crate::cell::Cell;

    #[test]
    fn req_round_trips() {
        let msgs = vec![
            HostReq::Spawn { req_id: 7, cmd: "sh".into(), args: vec!["-c".into()], cwd: None, cols: 80, rows: 24 },
            HostReq::Input { app: 3, bytes: vec![1, 2, 3] },
            HostReq::Resize { app: 3, cols: 100, rows: 40 },
            HostReq::SetMeta { app: 3, meta: vec![9, 9] },
            HostReq::Kill { app: 3 },
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
