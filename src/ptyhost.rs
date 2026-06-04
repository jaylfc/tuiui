use crate::buffer::CellBuffer;
use crate::cell::{Cell, CellAttrs, Rgba};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::Config;
use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor, Processor, StdSyncHandler};
use alacritty_terminal::Term;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// Default foreground/background used for cells that reference the terminal's
/// "default" colors (and as a fallback for unresolved named colors).
const DEFAULT_FG: Rgba = Rgba { r: 200, g: 208, b: 220, a: 255 };
// Matches the window panel color (wm::WIN_BG) so an app's default-background
// cells blend seamlessly into the window rather than showing a mismatched fill.
const DEFAULT_BG: Rgba = Rgba { r: 13, g: 15, b: 22, a: 255 };

/// No-op event listener — Tuiui polls the grid via [`AppInstance::snapshot`]
/// rather than reacting to terminal events (bell, title changes, etc.) yet.
#[derive(Clone)]
struct NoopListener;
impl EventListener for NoopListener {
    fn send_event(&self, _event: Event) {}
}

/// Hosts a child process running inside a pseudo-terminal.
///
/// The child's output is parsed by a full [`alacritty_terminal`] emulator on a
/// dedicated reader thread (chosen over a minimal parser because the desktop must
/// faithfully render demanding TUIs such as `btop`). [`snapshot`](Self::snapshot)
/// converts the current emulator grid into a Tuiui [`CellBuffer`].
pub struct AppInstance {
    term: Arc<Mutex<Term<NoopListener>>>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    cols: u16,
    rows: u16,
}

impl AppInstance {
    /// Spawn `cmd` with `args` inside a PTY of size `cols × rows`.
    ///
    /// Returns `Err` if the PTY or the child process could not be created.
    pub fn spawn(
        cmd: &str,
        args: &[String],
        cols: i32,
        rows: i32,
    ) -> std::io::Result<AppInstance> {
        let (cols, rows) = (cols.max(1) as u16, rows.max(1) as u16);
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let mut builder = CommandBuilder::new(cmd);
        for a in args {
            builder.arg(a);
        }
        // Inherit the parent environment so apps on the user's PATH (e.g.
        // Homebrew binaries in /opt/homebrew/bin) launch and get HOME/LANG.
        for (key, val) in std::env::vars() {
            builder.env(key, val);
        }
        // Pin TERM/COLORTERM to what the embedded emulator implements, letting
        // apps emit 24-bit color (captured here, re-emitted per the real terminal).
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");
        if let Some(home) = dirs::home_dir() {
            builder.cwd(home);
        }

        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        drop(pair.slave);

        let term = Arc::new(Mutex::new(Term::new(
            Config::default(),
            &TermSize::new(cols as usize, rows as usize),
            NoopListener,
        )));

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Reader thread: pump PTY bytes through the emulator. The `Processor`
        // persists across reads so partial escape sequences are handled.
        let tclone = term.clone();
        std::thread::spawn(move || {
            let mut parser = Processor::<StdSyncHandler>::new();
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut t) = tclone.lock() {
                            parser.advance(&mut *t, &buf[..n]);
                        }
                    }
                }
            }
        });

        Ok(AppInstance { term, master: pair.master, writer, child, cols, rows })
    }

    /// Convert the current emulator grid into a Tuiui [`CellBuffer`].
    pub fn snapshot(&self) -> CellBuffer {
        let t = self.term.lock().unwrap();
        let grid = t.grid();
        let mut buf = CellBuffer::new(self.cols as i32, self.rows as i32);
        for y in 0..self.rows as i32 {
            for x in 0..self.cols as usize {
                let cell = &grid[Line(y)][Column(x)];
                let ch = if cell.c == '\0' { ' ' } else { cell.c };
                let flags = cell.flags;
                buf.set(
                    x as i32,
                    y,
                    Cell {
                        ch,
                        fg: resolve_color(cell.fg, DEFAULT_FG),
                        bg: resolve_color(cell.bg, DEFAULT_BG),
                        attrs: CellAttrs {
                            bold: flags.contains(Flags::BOLD),
                            italic: flags.contains(Flags::ITALIC),
                            underline: flags.contains(Flags::UNDERLINE),
                            inverse: flags.contains(Flags::INVERSE),
                        },
                    },
                );
            }
        }
        buf
    }

    /// Resize both the PTY (sends `SIGWINCH`) and the emulator grid.
    pub fn resize(&mut self, cols: i32, rows: i32) {
        self.cols = cols.max(1) as u16;
        self.rows = rows.max(1) as u16;
        let _ = self.master.resize(PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut t) = self.term.lock() {
            t.resize(TermSize::new(self.cols as usize, self.rows as usize));
        }
    }

    /// Forward raw input bytes to the child.
    pub fn write_input(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Kill the child process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    /// Whether the child process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Resolve an alacritty cell color into our [`Rgba`].
fn resolve_color(c: AColor, default: Rgba) -> Rgba {
    match c {
        AColor::Spec(rgb) => Rgba::rgb(rgb.r, rgb.g, rgb.b),
        AColor::Indexed(i) => idx_to_rgb(i),
        AColor::Named(n) => named_to_rgb(n, default),
    }
}

/// Map a named terminal color to RGB; `default` covers the terminal's own
/// default fg/bg and any names we don't special-case (cursor, dim variants).
fn named_to_rgb(n: NamedColor, default: Rgba) -> Rgba {
    use NamedColor::*;
    match n {
        Foreground => DEFAULT_FG,
        Background => DEFAULT_BG,
        Black => idx_to_rgb(0),
        Red => idx_to_rgb(1),
        Green => idx_to_rgb(2),
        Yellow => idx_to_rgb(3),
        Blue => idx_to_rgb(4),
        Magenta => idx_to_rgb(5),
        Cyan => idx_to_rgb(6),
        White => idx_to_rgb(7),
        BrightBlack => idx_to_rgb(8),
        BrightRed => idx_to_rgb(9),
        BrightGreen => idx_to_rgb(10),
        BrightYellow => idx_to_rgb(11),
        BrightBlue => idx_to_rgb(12),
        BrightMagenta => idx_to_rgb(13),
        BrightCyan => idx_to_rgb(14),
        BrightWhite => idx_to_rgb(15),
        _ => default,
    }
}

/// Convert an xterm 256-color index to RGB (16 base + 6×6×6 cube + grayscale ramp).
fn idx_to_rgb(i: u8) -> Rgba {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16),
        (36, 114, 200), (188, 63, 188), (17, 168, 205), (229, 229, 229),
        (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67),
        (59, 142, 234), (214, 112, 214), (41, 184, 219), (255, 255, 255),
    ];
    if (i as usize) < 16 {
        let (r, g, b) = BASE[i as usize];
        return Rgba::rgb(r, g, b);
    }
    if i >= 232 {
        let v = 8 + (i - 232) * 10;
        return Rgba::rgb(v, v, v);
    }
    let i = i - 16;
    let (r, g, b) = (i / 36, (i % 36) / 6, i % 6);
    let s = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
    Rgba::rgb(s(r), s(g), s(b))
}
