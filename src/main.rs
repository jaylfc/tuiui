//! Entry point and main render loop.
//!
//! This module is intentionally thin: it owns the I/O boundary (a
//! [`Terminal`] + crossterm event reader) and the compositor
//! ([`Compositor`]), then shuttles events into the pure-logic
//! [`SessionCore`] and pushes the resulting frame diff back out to
//! the terminal.
//!
//! ## Loop structure
//! 1. Poll crossterm for input (16 ms deadline — ~60 fps ceiling).
//! 2. Translate every raw event to a [`ClientMsg`] and `apply` it.
//! 3. Reap any dead child processes.
//! 4. Ask the core to `build_frame`, composite, diff, write, commit.
//!
//! [`Terminal::drop`] restores raw mode and the alternate screen on
//! exit, whether graceful or via panic unwind.

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton,
                       MouseEventKind};
use std::time::Duration;
use tuiui::compositor::Compositor;
use tuiui::config::Config;
use tuiui::geometry::{Point, SnapZone};
use tuiui::session::{ClientMsg, SessionCore};
use tuiui::terminal::Terminal;

fn main() -> std::io::Result<()> {
    let cfg = Config::load();

    // Query terminal size *before* entering raw / alt-screen so that
    // crossterm::terminal::size() still works if enter() fails.
    let (w, h) = Terminal::size()?;

    let mut term = Terminal::enter()?;
    let mut comp = Compositor::new(w, h);
    let mut core = SessionCore::new(w, h, cfg.clone());

    // Launch every app listed in the config.
    for app in &cfg.apps {
        core.apply(ClientMsg::Launch {
            name: app.name.clone(),
            command: app.command.clone(),
            args: app.args.clone(),
        });
    }

    'outer: loop {
        // Poll for input; 16 ms lets child-app output animate at ~60 fps.
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                // Key events — skip Release so we don't double-send.
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    // Reserved Ctrl+Alt window-management chords are intercepted
                    // before forwarding input to the focused app.
                    let ctrl_alt = k.modifiers.contains(KeyModifiers::CONTROL)
                        && k.modifiers.contains(KeyModifiers::ALT);
                    if ctrl_alt {
                        match k.code {
                            KeyCode::Char('q') => break 'outer,
                            KeyCode::Up => core.apply(ClientMsg::MaximizeFocused),
                            KeyCode::Down => core.apply(ClientMsg::MinimizeFocused),
                            KeyCode::Left => core.apply(ClientMsg::SnapFocused(SnapZone::Left)),
                            KeyCode::Right => core.apply(ClientMsg::SnapFocused(SnapZone::Right)),
                            _ => core.apply(ClientMsg::Key(encode_key(k.code, k.modifiers))),
                        }
                    } else {
                        core.apply(ClientMsg::Key(encode_key(k.code, k.modifiers)));
                    }
                }

                // Mouse events — map to the three ClientMsg variants.
                Event::Mouse(m) => {
                    let p = Point::new(m.column as i32, m.row as i32);
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            core.apply(ClientMsg::MouseDown(p));
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            core.apply(ClientMsg::MouseDrag(p));
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            core.apply(ClientMsg::MouseUp(p));
                        }
                        // Moved: update the rendered cursor even when no button is held.
                        MouseEventKind::Moved => {
                            core.apply(ClientMsg::MouseDrag(p));
                        }
                        _ => {}
                    }
                }

                // Terminal resize.
                Event::Resize(nc, nr) => {
                    comp.resize(nc as i32, nr as i32);
                    core.apply(ClientMsg::Resize {
                        w: nc as i32,
                        h: nr as i32,
                    });
                }

                _ => {}
            }
        }

        // Quit if the user clicked the menubar quit button this tick.
        if core.quit_requested() {
            break 'outer;
        }

        // Reap child processes that have exited.
        core.reap_dead();

        // Build frame, composite, diff, write to terminal.
        let frame = core.build_frame();
        let _ = comp.composite(&frame.layers, frame.cursor);
        let changes = comp.diff();
        term.write_frame(&changes)?;
        comp.commit();
    }

    core.shutdown();
    Ok(()) // Terminal::drop restores the screen automatically.
}

/// Encode a crossterm [`KeyCode`] + [`KeyModifiers`] into the byte sequence
/// that should be forwarded to the focused PTY child.
///
/// Covers the most common cases for a shell/TUI child:
/// - Ctrl+letter → single control byte (e.g. Ctrl+C → `\x03`)
/// - Printable characters → UTF-8 bytes
/// - Enter, Backspace, Tab, Esc, arrow keys → standard escape sequences
///
/// Unknown keys are silently dropped (empty `Vec`).
fn encode_key(code: KeyCode, mods: KeyModifiers) -> Vec<u8> {
    match code {
        KeyCode::Char(c) => {
            if mods.contains(KeyModifiers::CONTROL) {
                // Map Ctrl+[A-Za-z@[\]^_] to control bytes 0x01–0x1F.
                let b = (c.to_ascii_uppercase() as u8).wrapping_sub(0x40);
                vec![b]
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        _ => vec![],
    }
}
