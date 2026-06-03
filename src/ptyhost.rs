use crate::buffer::CellBuffer;
use crate::cell::{Cell, CellAttrs, Rgba};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// Hosts a child process running inside a pseudo-terminal.
///
/// Spawns the child, pumps its output through a `vt100::Parser` on a
/// dedicated reader thread, and exposes a snapshot of the parsed screen
/// as a [`CellBuffer`].  Supports resize and raw-byte input.
pub struct AppInstance {
    parser: Arc<Mutex<vt100::Parser>>,
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
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let mut builder = CommandBuilder::new(cmd);
        for a in args {
            builder.arg(a);
        }
        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Drop the slave end so EOF is delivered when the child exits.
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            rows as u16,
            cols as u16,
            0,
        )));

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let pclone = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut p) = pclone.lock() {
                            p.process(&buf[..n]);
                        }
                    }
                }
            }
        });

        Ok(AppInstance {
            parser,
            master: pair.master,
            writer,
            child,
            cols: cols as u16,
            rows: rows as u16,
        })
    }

    /// Return a snapshot of the current terminal screen as a [`CellBuffer`].
    pub fn snapshot(&self) -> CellBuffer {
        let p = self.parser.lock().unwrap();
        let screen = p.screen();
        let mut buf = CellBuffer::new(self.cols as i32, self.rows as i32);
        for y in 0..self.rows {
            for x in 0..self.cols {
                if let Some(c) = screen.cell(y, x) {
                    let ch_str = c.contents();
                    let ch = ch_str.chars().next().unwrap_or('\0');
                    buf.set(
                        x as i32,
                        y as i32,
                        Cell {
                            ch: if ch == '\0' { ' ' } else { ch },
                            fg: vt_color(c.fgcolor(), Rgba::rgb(200, 208, 220)),
                            bg: vt_color(c.bgcolor(), Rgba::rgb(17, 20, 29)),
                            attrs: CellAttrs {
                                bold: c.bold(),
                                italic: c.italic(),
                                underline: c.underline(),
                                inverse: c.inverse(),
                            },
                        },
                    );
                }
            }
        }
        buf
    }

    /// Resize the PTY and the internal parser to `cols × rows`.
    pub fn resize(&mut self, cols: i32, rows: i32) {
        self.cols = cols as u16;
        self.rows = rows as u16;
        let _ = self.master.resize(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows as u16, cols as u16);
        }
    }

    /// Write raw bytes to the child's stdin (e.g. keyboard input).
    pub fn write_input(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Send SIGHUP / terminate to the child process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    /// Returns `true` if the child process has not yet exited.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Map a `vt100::Color` to our `Rgba`, using `default` for the `Default` variant.
fn vt_color(c: vt100::Color, default: Rgba) -> Rgba {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => Rgba::rgb(r, g, b),
        vt100::Color::Idx(i) => idx_to_rgb(i),
    }
}

/// Approximate an xterm 256-color index as an RGB triple.
fn idx_to_rgb(i: u8) -> Rgba {
    // Basic 16-color table (matches VS Code / xterm defaults).
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 49, 49),
        (13, 188, 121),
        (229, 229, 16),
        (36, 114, 200),
        (188, 63, 188),
        (17, 168, 205),
        (229, 229, 229),
        (102, 102, 102),
        (241, 76, 76),
        (35, 209, 139),
        (245, 245, 67),
        (59, 142, 234),
        (214, 112, 214),
        (41, 184, 219),
        (255, 255, 255),
    ];
    if (i as usize) < 16 {
        let (r, g, b) = BASE[i as usize];
        return Rgba::rgb(r, g, b);
    }
    // Grayscale ramp: indices 232–255.
    if i >= 232 {
        let v = 8 + (i - 232) * 10;
        return Rgba::rgb(v, v, v);
    }
    // 6×6×6 color cube: indices 16–231.
    let i = i - 16;
    let r = i / 36;
    let g = (i % 36) / 6;
    let b = i % 6;
    let s = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
    Rgba::rgb(s(r), s(g), s(b))
}
