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

    // Whether the previous keypress was the Tuiui leader (Ctrl+Space), so this
    // key completes a leader chord.
    let mut leader = false;

    'outer: loop {
        // Poll for input; 16 ms lets child-app output animate at ~60 fps.
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                // Key events — skip Release so we don't double-send.
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                    let ctrl_alt = ctrl && k.modifiers.contains(KeyModifiers::ALT);
                    // Tuiui leader = Ctrl+Space (reliable everywhere; Alt is not
                    // delivered by default in Ghostty/macOS). Some terminals send
                    // Ctrl+Space as the NUL key.
                    let is_leader = (ctrl && k.code == KeyCode::Char(' ')) || k.code == KeyCode::Null;

                    if leader {
                        // Second key of a leader chord.
                        leader = false;
                        match k.code {
                            KeyCode::Char(' ') => core.apply(ClientMsg::ToggleSpotlight),
                            KeyCode::Char('a') | KeyCode::Char('A') => core.apply(ClientMsg::ToggleMenu),
                            KeyCode::Char('m') | KeyCode::Char('M') => core.apply(ClientMsg::MaximizeFocused),
                            KeyCode::Char('n') | KeyCode::Char('N') => core.apply(ClientMsg::MinimizeFocused),
                            KeyCode::Char('[') | KeyCode::Left => core.apply(ClientMsg::SnapFocused(SnapZone::Left)),
                            KeyCode::Char(']') | KeyCode::Right => core.apply(ClientMsg::SnapFocused(SnapZone::Right)),
                            KeyCode::Char('s') | KeyCode::Char('S') => core.apply(ClientMsg::OpenStore),
                            KeyCode::Char('q') | KeyCode::Char('Q') => break 'outer,
                            _ => {} // Esc / anything else cancels the chord
                        }
                    } else if core.launcher_open() {
                        // An open launcher captures keyboard navigation/typing.
                        match k.code {
                            KeyCode::Esc => core.apply(ClientMsg::LauncherEsc),
                            KeyCode::Enter => core.apply(ClientMsg::LauncherEnter),
                            KeyCode::Up => core.apply(ClientMsg::LauncherUp),
                            KeyCode::Down => core.apply(ClientMsg::LauncherDown),
                            KeyCode::Backspace => core.apply(ClientMsg::LauncherBackspace),
                            KeyCode::Char(c) if core.spotlight_open() && !ctrl => {
                                core.apply(ClientMsg::LauncherChar(c));
                            }
                            _ => {}
                        }
                    } else if is_leader {
                        leader = true;
                    } else if core.focused_is_store() {
                        // The focused store window captures keyboard navigation/search.
                        match k.code {
                            KeyCode::Esc => core.apply(ClientMsg::StoreClose),
                            KeyCode::Enter => core.apply(ClientMsg::StoreActivate),
                            KeyCode::Up => core.apply(ClientMsg::StoreUp),
                            KeyCode::Down => core.apply(ClientMsg::StoreDown),
                            KeyCode::Left => core.apply(ClientMsg::StorePrevCategory),
                            KeyCode::Right => core.apply(ClientMsg::StoreNextCategory),
                            KeyCode::Backspace => core.apply(ClientMsg::StoreBackspace),
                            KeyCode::Char(c) if !ctrl => core.apply(ClientMsg::StoreChar(c)),
                            _ => {}
                        }
                    } else if ctrl_alt {
                        // Legacy Ctrl+Alt chords still work where Alt is delivered.
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
                        // Wheel scrolls the store list when it's focused.
                        MouseEventKind::ScrollUp if core.focused_is_store() => {
                            core.apply(ClientMsg::StoreUp);
                        }
                        MouseEventKind::ScrollDown if core.focused_is_store() => {
                            core.apply(ClientMsg::StoreDown);
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
