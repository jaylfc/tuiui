use crate::cell::Rgba;
use crate::compositor::CellChange;
use std::io::{Write, Stdout};

/// Terminal capability flags detected from environment variables.
///
/// Used by [`frame_to_ansi`] to choose the right SGR color encoding,
/// and by [`Terminal`] to enable pixel-accurate mouse if the terminal supports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// Whether the terminal supports 24-bit ("truecolor") SGR color sequences.
    /// When `false`, colors are downsampled to the xterm 256-color palette.
    pub truecolor: bool,
    /// Whether the terminal supports SGR 1016 pixel-position mouse events (Kitty / WezTerm / Ghostty).
    pub pixel_mouse: bool,
    /// Whether the terminal supports the Kitty graphics protocol (for images).
    pub kitty_graphics: bool,
}

impl Caps {
    /// Detect capabilities from `COLORTERM` and `TERM_PROGRAM` environment variables.
    pub fn detect() -> Caps {
        let ct = std::env::var("COLORTERM").unwrap_or_default();
        let truecolor = ct.contains("truecolor") || ct.contains("24bit");
        // Pixel mouse (SGR 1016) — conservatively off unless a known supporting terminal.
        let term = std::env::var("TERM_PROGRAM").unwrap_or_default();
        let pixel_mouse = matches!(term.as_str(), "kitty" | "WezTerm" | "ghostty");
        let kitty_graphics = pixel_mouse; // same set of terminals support graphics
        Caps { truecolor, pixel_mouse, kitty_graphics }
    }
}

// ── Pure color helpers ────────────────────────────────────────────────────────

/// Build the SGR foreground color parameter string for a given color and capability set.
fn fg_code(c: Rgba, caps: &Caps) -> String {
    if caps.truecolor {
        format!("38;2;{};{};{}", c.r, c.g, c.b)
    } else {
        format!("38;5;{}", ansi256(c))
    }
}

/// Build the SGR background color parameter string for a given color and capability set.
fn bg_code(c: Rgba, caps: &Caps) -> String {
    if caps.truecolor {
        format!("48;2;{};{};{}", c.r, c.g, c.b)
    } else {
        format!("48;5;{}", ansi256(c))
    }
}

/// Map an [`Rgba`] color to the nearest xterm 256-color index.
///
/// Near-grey colors use the 24-step grayscale ramp (232–255), which has far
/// finer dark-grey resolution than the 6×6×6 color cube — without this, dark
/// greys collapse to black. Other colors use the cube (indices 16–231).
fn ansi256(c: Rgba) -> u8 {
    let (r, g, b) = (c.r as i32, c.g as i32, c.b as i32);
    if (r - g).abs() < 12 && (g - b).abs() < 12 && (r - b).abs() < 12 {
        let grey = (r + g + b) / 3;
        if grey < 4 {
            return 16; // pure black
        }
        if grey > 246 {
            return 231; // pure white
        }
        // Grayscale ramp index n (0..=23) has value 8 + 10*n.
        return (232 + ((grey - 8).clamp(0, 238) / 10)) as u8;
    }
    let q = |v: u8| -> u32 { v as u32 * 5 / 255 };
    (16 + 36 * q(c.r) + 6 * q(c.g) + q(c.b)) as u8
}

// ── Pure ANSI frame writer ────────────────────────────────────────────────────

/// Convert a list of [`CellChange`]s into an ANSI byte string ready for `stdout`.
///
/// Emits a CUP (`\x1b[row;colH`) + SGR for each changed cell, followed by a final
/// SGR reset. Row/column positions are 1-based as required by the VT100 standard.
///
/// This function is **pure** — it has no I/O side effects and can be unit-tested
/// without a real terminal.
///
/// Returns an empty string when `changes` is empty.
pub fn frame_to_ansi(changes: &[CellChange], caps: &Caps) -> String {
    if changes.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    for ch in changes {
        // Continuation cell (right half of a double-width glyph): the glyph to its
        // left already covers this column, so skip it — painting it would erase the
        // glyph's right half.
        if ch.cell.ch == '\0' {
            continue;
        }
        // CUP: move cursor to 1-based (row, col).
        s.push_str(&format!("\x1b[{};{}H", ch.y + 1, ch.x + 1));
        let a = &ch.cell.attrs;
        let mut sgr = vec![fg_code(ch.cell.fg, caps), bg_code(ch.cell.bg, caps)];
        if a.bold      { sgr.push("1".into()); }
        if a.italic    { sgr.push("3".into()); }
        if a.underline { sgr.push("4".into()); }
        if a.inverse   { sgr.push("7".into()); }
        s.push_str(&format!("\x1b[0;{}m", sgr.join(";")));
        s.push(ch.cell.ch);
    }
    s.push_str("\x1b[0m");
    s
}

// ── Terminal lifecycle ────────────────────────────────────────────────────────

/// Owns the raw-mode / alternate-screen / mouse-capture lifecycle.
///
/// Call [`Terminal::enter`] to initialise the terminal; the [`Drop`] implementation
/// tears it down automatically when the value goes out of scope.
///
/// # Note
/// The lifecycle code is not unit-tested (it requires a real PTY); it is exercised
/// through the main render loop. The pure rendering path ([`frame_to_ansi`]) is
/// fully unit-tested separately.
pub struct Terminal {
    out: Stdout,
    /// Detected terminal capabilities (truecolor, pixel mouse).
    pub caps: Caps,
}

impl Terminal {
    /// Enter raw mode, switch to the alternate screen, enable mouse capture, and hide
    /// the cursor. Returns an initialised [`Terminal`] on success.
    pub fn enter() -> std::io::Result<Terminal> {
        use crossterm::{
            terminal,
            execute,
            event::EnableMouseCapture,
            cursor,
        };
        terminal::enable_raw_mode()?;
        let mut out = std::io::stdout();
        execute!(out, terminal::EnterAlternateScreen, EnableMouseCapture, cursor::Hide)?;
        use std::io::Write;
        write!(out, "\x1b[?1003h")?; // all-motion mouse tracking (for launcher hover)
        out.flush()?;
        Ok(Terminal { out, caps: Caps::detect() })
    }

    /// Return the current terminal size as `(columns, rows)`.
    pub fn size() -> std::io::Result<(i32, i32)> {
        let (c, r) = crossterm::terminal::size()?;
        Ok((c as i32, r as i32))
    }

    /// Encode `changes` as ANSI and flush them to stdout.
    pub fn write_frame(&mut self, changes: &[CellChange]) -> std::io::Result<()> {
        let s = frame_to_ansi(changes, &self.caps);
        self.out.write_all(s.as_bytes())?;
        self.out.flush()
    }
}

impl Drop for Terminal {
    /// Restore the terminal: disable mouse capture, leave the alternate screen, show
    /// the cursor, and disable raw mode. Errors are silently discarded (best-effort
    /// teardown at shutdown).
    fn drop(&mut self) {
        use crossterm::{
            terminal,
            execute,
            event::DisableMouseCapture,
            cursor,
        };
        use std::io::Write;
        let _ = write!(self.out, "\x1b[?1003l"); // disable all-motion mouse tracking
        let _ = execute!(
            self.out,
            DisableMouseCapture,
            terminal::LeaveAlternateScreen,
            cursor::Show
        );
        let _ = terminal::disable_raw_mode();
    }
}
