//! Background thumbnail loader.
//!
//! Reading + decoding image files is done on a dedicated worker thread, never on
//! the desktop loop. A large photo merely takes a moment; an iCloud-offloaded
//! ("dataless") file or one on a stalled mount blocks the *worker* indefinitely
//! instead of freezing the whole desktop (that one thumbnail simply never
//! appears). The session queues paths and drains finished PNGs each frame.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// A finished thumbnail: the source path plus the downscaled PNG and its size.
pub struct ThumbResult {
    pub path: PathBuf,
    pub png: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

/// Queues image paths and yields decoded thumbnails off-thread.
pub struct ThumbLoader {
    req: mpsc::Sender<(PathBuf, u32, u32)>,
    res: mpsc::Receiver<ThumbResult>,
    requested: HashSet<PathBuf>,
}

impl Default for ThumbLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbLoader {
    pub fn new() -> Self {
        let (req_tx, req_rx) = mpsc::channel::<(PathBuf, u32, u32)>();
        let (res_tx, res_rx) = mpsc::channel::<ThumbResult>();
        std::thread::spawn(move || {
            // A blocking read here stalls only this worker, never the desktop loop.
            while let Ok((path, w, h)) = req_rx.recv() {
                if let Some(r) = load_thumb(&path, w, h) {
                    let _ = res_tx.send(r);
                }
            }
        });
        Self { req: req_tx, res: res_rx, requested: HashSet::new() }
    }

    /// Queue `path` for thumbnailing at most once. Non-blocking.
    pub fn request(&mut self, path: PathBuf, w: u32, h: u32) {
        if self.requested.insert(path.clone()) {
            let _ = self.req.send((path, w, h));
        }
    }

    /// Collect any thumbnails finished since the last call. Non-blocking.
    pub fn drain(&self) -> Vec<ThumbResult> {
        let mut out = Vec::new();
        while let Ok(r) = self.res.try_recv() {
            out.push(r);
        }
        out
    }
}

fn load_thumb(path: &Path, w: u32, h: u32) -> Option<ThumbResult> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let thumb = img.thumbnail(w.max(1), h.max(1));
    let (tw, th) = (thumb.width(), thumb.height());
    let mut buf = std::io::Cursor::new(Vec::new());
    thumb.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(ThumbResult { path: path.to_path_buf(), png: buf.into_inner(), w: tw, h: th })
}
