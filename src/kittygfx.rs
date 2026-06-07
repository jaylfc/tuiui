//! Kitty graphics protocol capture for hosted PTY apps (A2). A streaming tap
//! separates graphics APC sequences (`ESC _ G … ESC \`) from text so the embedded
//! emulator only sees text, and decodes transmitted images for our compositor.

/// One captured graphics command: control key/value pairs + the raw payload.
#[derive(Clone, Debug, Default)]
pub struct GraphicsCmd {
    pub control: Vec<(char, String)>,
    pub payload: Vec<u8>,
}

impl GraphicsCmd {
    pub fn get(&self, key: char) -> Option<String> {
        self.control.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
    }
    fn parse(control: &[u8], payload: Vec<u8>) -> Self {
        let s = String::from_utf8_lossy(control);
        let control = s
            .split(',')
            .filter_map(|kv| {
                let mut it = kv.splitn(2, '=');
                let k = it.next()?.trim();
                let v = it.next().unwrap_or("").trim();
                let kc = k.chars().next()?;
                Some((kc, v.to_string()))
            })
            .collect();
        GraphicsCmd { control, payload }
    }
}

/// Result of feeding a chunk: the non-graphics bytes (for the emulator) and any
/// completed graphics commands.
#[derive(Debug, Default)]
pub struct Split {
    pub passthrough: Vec<u8>,
    pub commands: Vec<GraphicsCmd>,
}

#[derive(Clone, Copy, PartialEq)]
enum St {
    Text,        // normal bytes
    Esc,         // saw ESC, deciding
    MaybeApc,    // saw ESC _, peeking the marker
    GfxControl,  // inside ESC _ G, before ';' — accumulating control
    GfxPayload,  // after ';' — accumulating payload
    GfxEsc,      // inside graphics APC, saw ESC, expecting '\'
    OtherApc,    // a non-graphics APC, pass through, scan for ESC \
    OtherApcEsc, // in OtherApc, saw ESC
}

/// Streaming graphics separator. Holds partial-APC state across `feed` calls.
pub struct GraphicsTap {
    st: St,
    control: Vec<u8>,
    payload: Vec<u8>,
    other: Vec<u8>, // buffered non-graphics APC bytes (incl. introducer) to pass through
}

impl Default for GraphicsTap {
    fn default() -> Self { Self::new() }
}

impl GraphicsTap {
    pub fn new() -> Self {
        Self { st: St::Text, control: Vec::new(), payload: Vec::new(), other: Vec::new() }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Split {
        let mut out = Split::default();
        for &b in bytes {
            match self.st {
                St::Text => {
                    if b == 0x1b { self.st = St::Esc; } else { out.passthrough.push(b); }
                }
                St::Esc => {
                    if b == b'_' { self.st = St::MaybeApc; }
                    else {
                        // not an APC: emit the ESC and this byte as text
                        out.passthrough.push(0x1b);
                        if b == 0x1b { self.st = St::Esc; } else { out.passthrough.push(b); self.st = St::Text; }
                    }
                }
                St::MaybeApc => {
                    if b == b'G' {
                        self.control.clear();
                        self.payload.clear();
                        self.st = St::GfxControl;
                    } else {
                        // non-graphics APC: pass it through verbatim (ESC _ already consumed)
                        self.other.clear();
                        self.other.extend_from_slice(b"\x1b_");
                        self.other.push(b);
                        self.st = if b == 0x1b { St::OtherApcEsc } else { St::OtherApc };
                    }
                }
                St::GfxControl => {
                    if b == b';' { self.st = St::GfxPayload; }
                    else if b == 0x1b { self.st = St::GfxEsc; } // payload-less command (e.g. a=q)
                    else { self.control.push(b); }
                }
                St::GfxPayload => {
                    if b == 0x1b { self.st = St::GfxEsc; } else { self.payload.push(b); }
                }
                St::GfxEsc => {
                    if b == b'\\' {
                        out.commands.push(GraphicsCmd::parse(&self.control, std::mem::take(&mut self.payload)));
                        self.control.clear();
                        self.st = St::Text;
                    } else {
                        // ESC inside payload that wasn't ST — keep it as payload
                        self.payload.push(0x1b);
                        if b == 0x1b { self.st = St::GfxEsc; } else { self.payload.push(b); self.st = St::GfxPayload; }
                    }
                }
                St::OtherApc => {
                    self.other.push(b);
                    if b == 0x1b { self.st = St::OtherApcEsc; }
                }
                St::OtherApcEsc => {
                    self.other.push(b);
                    if b == b'\\' {
                        out.passthrough.append(&mut self.other);
                        self.st = St::Text;
                    } else if b != 0x1b {
                        self.st = St::OtherApc;
                    }
                }
            }
        }
        out
    }
}

use std::collections::HashMap;

/// Parse exactly one APC command from `bytes` (test/helper convenience).
pub fn parse_one(bytes: &[u8]) -> GraphicsCmd {
    let mut tap = GraphicsTap::new();
    tap.feed(bytes).commands.into_iter().next().unwrap_or_default()
}

struct Pending {
    format: u32,         // 24/32/100
    medium: char,        // d/f/t
    width: u32,
    height: u32,
    zlib: bool,
    // Display intent is declared on the OPENING chunk (`a=T`), but the image can
    // only be placed once transmission completes (the final `m=0` chunk, which
    // carries no action). So we remember the intent + placement from the opener.
    display: bool,
    place_col: u16,
    place_row: u16,
    cols: u16,
    rows: u16,
    data: Vec<u8>,       // accumulated base64 (direct) or path bytes
}

/// A placed image on the app's grid (cell coordinates relative to the app PTY).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Placement {
    pub image_id: u32,
    pub col: u16,
    pub row: u16,
    pub cols: u16,
    pub rows: u16,
}

/// Captured graphics state for one hosted app.
pub struct GraphicsState {
    images: HashMap<u32, Vec<u8>>,    // id → PNG
    pending: HashMap<u32, Pending>,
    pub placements: Vec<Placement>,
    pub queries: Vec<Vec<u8>>,        // a=q replies to write back to the PTY
    pub generation: u64,              // bumped on any change (session dirty check)
}

impl Default for GraphicsState {
    fn default() -> Self { Self::new() }
}

impl GraphicsState {
    pub fn new() -> Self {
        Self {
            images: HashMap::new(),
            pending: HashMap::new(),
            placements: Vec::new(),
            queries: Vec::new(),
            generation: 0,
        }
    }
    pub fn png(&self, id: u32) -> Option<&[u8]> { self.images.get(&id).map(|v| v.as_slice()) }

    /// Insert a decoded PNG under `id` directly (integration tests).
    #[doc(hidden)]
    pub fn insert_image_for_test(&mut self, id: u32, png: Vec<u8>) {
        self.images.insert(id, png);
    }

    /// Push a placement directly (integration tests).
    #[doc(hidden)]
    pub fn push_placement_for_test(&mut self, p: Placement) {
        self.placements.push(p);
    }

    fn num(cmd: &GraphicsCmd, k: char, default: u32) -> u32 {
        cmd.get(k).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    /// Apply a command at cursor cell `(col,row)`.
    pub fn apply(&mut self, cmd: &GraphicsCmd, col: u16, row: u16) {
        let action = cmd.get('a').unwrap_or_else(|| "t".into());
        match action.as_str() {
            "t" | "T" => {
                self.accumulate(cmd, action == "T", col, row);
                if cmd.get('m').as_deref() != Some("1") {
                    let id = Self::num(cmd, 'i', 0);
                    // Capture the opener's display intent before consuming `pending`.
                    if let Some((display, pc, pr, pcols, prows)) = self.finish_transmit(id) {
                        if display {
                            self.place_resolved(id, pc, pr, pcols, prows);
                        }
                    }
                }
                self.generation += 1;
            }
            "p" => {
                let id = Self::num(cmd, 'i', 0);
                self.place(id, col, row, cmd);
                self.generation += 1;
            }
            "d" => { self.delete(cmd); self.generation += 1; }
            "q" => { self.queries.push(reply_ok(Self::num(cmd, 'i', 0))); }
            _ => {}
        }
    }

    /// Append a transmission chunk. The opener (first chunk for this id) sets the
    /// format/medium/dimensions and the display intent + placement; continuation
    /// chunks (`m=1`, no action) only append payload.
    fn accumulate(&mut self, cmd: &GraphicsCmd, display: bool, col: u16, row: u16) {
        let id = Self::num(cmd, 'i', 0);
        let e = self.pending.entry(id).or_insert_with(|| Pending {
            format: Self::num(cmd, 'f', 32),
            medium: cmd.get('t').and_then(|s| s.chars().next()).unwrap_or('d'),
            width: Self::num(cmd, 's', 0),
            height: Self::num(cmd, 'v', 0),
            zlib: cmd.get('o').as_deref() == Some("z"),
            display,
            place_col: col,
            place_row: row,
            cols: Self::num(cmd, 'c', 0) as u16,
            rows: Self::num(cmd, 'r', 0) as u16,
            data: Vec::new(),
        });
        e.data.extend_from_slice(&cmd.payload);
    }

    /// Finish a transmission: decode + store the image. Returns the opener's
    /// `(display, place_col, place_row, cols, rows)` so the caller can place it.
    fn finish_transmit(&mut self, id: u32) -> Option<(bool, u16, u16, u16, u16)> {
        let p = self.pending.remove(&id)?;
        let intent = (p.display, p.place_col, p.place_row, p.cols, p.rows);
        if let Some(png) = decode(&p) {
            self.images.insert(id, png);
        }
        Some(intent)
    }

    fn place(&mut self, id: u32, col: u16, row: u16, cmd: &GraphicsCmd) {
        let cols = Self::num(cmd, 'c', 0) as u16;
        let rows = Self::num(cmd, 'r', 0) as u16;
        self.place_resolved(id, col, row, cols, rows);
    }

    /// Place image `id` at `(col,row)` spanning `cols×rows` cells (deriving the
    /// footprint from the pixel size when `cols`/`rows` are unspecified).
    fn place_resolved(&mut self, id: u32, col: u16, row: u16, cols: u16, rows: u16) {
        if !self.images.contains_key(&id) {
            return;
        }
        let (cols, rows) = if cols > 0 && rows > 0 {
            (cols, rows)
        } else {
            derive_cells(self.images.get(&id))
        };
        self.placements.retain(|pl| pl.image_id != id); // one placement per id (spike)
        self.placements.push(Placement { image_id: id, col, row, cols, rows });
    }

    fn delete(&mut self, cmd: &GraphicsCmd) {
        match cmd.get('d').as_deref() {
            Some("i") | Some("I") => {
                let id = Self::num(cmd, 'i', 0);
                self.placements.retain(|p| p.image_id != id);
            }
            _ => self.placements.clear(), // a/A or unspecified → all
        }
    }
}

/// `ESC _ G i=<id>;OK ESC \`
fn reply_ok(id: u32) -> Vec<u8> {
    format!("\x1b_Gi={id};OK\x1b\\").into_bytes()
}

/// Decode a completed transmission into PNG bytes.
fn decode(p: &Pending) -> Option<Vec<u8>> {
    use base64::Engine;
    // direct payload is base64; file/temp payload is a base64 path.
    let decoded = base64::engine::general_purpose::STANDARD.decode(&p.data).ok()?;
    let raw: Vec<u8> = match p.medium {
        // File/temp-file transmission carries a path. PTY output can be
        // attacker-controlled (e.g. `cat`ing an untrusted file that embeds a
        // crafted graphics escape), and the decoded image is transmitted to the
        // attached client — so an unrestricted read would be an arbitrary
        // file-read / exfiltration vector. Sandbox it to temp dirs where graphics
        // apps legitimately stage images.
        'f' | 't' => read_sandboxed(&String::from_utf8_lossy(&decoded))?,
        _ => decoded, // 'd'
    };
    let raw = if p.zlib { inflate(&raw)? } else { raw };
    match p.format {
        100 => { image::load_from_memory(&raw).ok()?; Some(raw) } // already PNG; validate
        24 => reencode_raw(&raw, p.width, p.height, false),
        32 => reencode_raw(&raw, p.width, p.height, true),
        _ => image::load_from_memory(&raw).ok().map(|_| raw),
    }
}

/// Read a file referenced by a Kitty `t=f`/`t=t` transmission, sandboxed against
/// arbitrary-file-read via crafted PTY escapes. Only regular files under a temp
/// directory are allowed; the path is canonicalized first (defeating symlink and
/// `..` traversal escapes), and the read is size-capped against device/huge files.
fn read_sandboxed(path_str: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    let canon = std::fs::canonicalize(path_str).ok()?;
    // Allowed roots: the platform temp dir ($TMPDIR) and /tmp (canonicalized, so
    // macOS's /tmp -> /private/tmp resolves correctly).
    let roots = [std::env::temp_dir(), std::path::PathBuf::from("/tmp")];
    let allowed = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .any(|root| canon.starts_with(&root));
    if !allowed {
        return None;
    }
    let meta = std::fs::metadata(&canon).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let mut buf = Vec::new();
    std::fs::File::open(&canon).ok()?.take(MAX_BYTES).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

fn reencode_raw(raw: &[u8], w: u32, h: u32, alpha: bool) -> Option<Vec<u8>> {
    if w == 0 || h == 0 { return None; }
    let img = if alpha {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, raw.to_vec())?)
    } else {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, raw.to_vec())?)
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

/// Approximate a cell footprint from the image pixel size (8x16 px cells).
fn derive_cells(png: Option<&Vec<u8>>) -> (u16, u16) {
    use image::GenericImageView;
    if let Some(bytes) = png {
        if let Ok(img) = image::load_from_memory(bytes) {
            let (w, h) = img.dimensions();
            return (((w / 8).max(1)) as u16, ((h / 16).max(1)) as u16);
        }
    }
    (10, 5)
}
