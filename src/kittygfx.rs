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
