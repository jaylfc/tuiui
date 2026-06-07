//! Decodes, downscales, and caches images for the native image layer. Each image
//! is keyed by a content hash of its downscaled PNG (`ImageId`).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Content hash of a downscaled PNG.
pub type ImageId = u64;

struct Entry {
    png: Vec<u8>,
    w: u32,
    h: u32,
}

/// Caches decoded + downscaled images by id.
#[derive(Default)]
pub struct ImageStore {
    by_id: HashMap<ImageId, Entry>,
}

impl ImageStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode `bytes`, downscale to fit `max_w × max_h` pixels (aspect preserved,
    /// never upsized), re-encode PNG, and cache it. Returns the content id, or
    /// `None` if the bytes aren't a decodable image.
    pub fn load_bytes(&mut self, bytes: &[u8], max_w: u32, max_h: u32) -> Option<ImageId> {
        let img = image::load_from_memory(bytes).ok()?;
        let scaled = img.thumbnail(max_w.max(1), max_h.max(1));
        let (w, h) = (scaled.width(), scaled.height());
        let mut png = std::io::Cursor::new(Vec::new());
        scaled.write_to(&mut png, image::ImageFormat::Png).ok()?;
        let png = png.into_inner();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        png.hash(&mut hasher);
        let id = hasher.finish();
        self.by_id.entry(id).or_insert(Entry { png, w, h });
        Some(id)
    }

    /// Decode an image file at `path`.
    ///
    /// NOTE: this reads + decodes synchronously on the calling thread. Do NOT call
    /// it on the desktop loop for arbitrary user files — a slow or iCloud-offloaded
    /// ("dataless") file blocks indefinitely and freezes everything. Thumbnails go
    /// through the background [`crate::thumbnail::ThumbLoader`] + [`Self::store_png`].
    pub fn load(&mut self, path: &std::path::Path, max_w: u32, max_h: u32) -> Option<ImageId> {
        let bytes = std::fs::read(path).ok()?;
        self.load_bytes(&bytes, max_w, max_h)
    }

    /// Store an already-decoded+downscaled PNG (e.g. produced off-thread by the
    /// thumbnail loader). No decode happens here — just hash + cache. Returns the id.
    pub fn store_png(&mut self, png: Vec<u8>, w: u32, h: u32) -> ImageId {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        png.hash(&mut hasher);
        let id = hasher.finish();
        self.by_id.entry(id).or_insert(Entry { png, w, h });
        id
    }

    pub fn png_bytes(&self, id: ImageId) -> Option<&[u8]> {
        self.by_id.get(&id).map(|e| e.png.as_slice())
    }

    pub fn dimensions(&self, id: ImageId) -> Option<(u32, u32)> {
        self.by_id.get(&id).map(|e| (e.w, e.h))
    }
}
