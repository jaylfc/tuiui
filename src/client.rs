//! The thin client: sets up the terminal, renders frames received from the
//! daemon, and forwards input. Holds no session state — it routes keyboard input
//! using the [`Flags`] the daemon sends each frame.

use crate::geometry::{Point, SnapZone};
use crate::protocol::{Flags, FrameMsg};
use crate::session::ClientMsg;
use crate::terminal::{frame_to_ansi, Terminal};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Attach to the daemon over `stream` and run until the user detaches.
pub fn run(stream: UnixStream) -> std::io::Result<()> {
    let term = Terminal::enter()?;
    let caps = term.caps;
    let (w, h) = Terminal::size()?;

    let mut out_stream = stream.try_clone()?;
    send(&mut out_stream, &ClientMsg::Resize { w, h })?;

    let flags = Arc::new(Mutex::new(Flags::default()));
    let detached = Arc::new(AtomicBool::new(false));

    // Reader thread: socket frames → ANSI → stdout.
    {
        let flags = flags.clone();
        let detached = detached.clone();
        let reader_stream = stream.try_clone()?;
        std::thread::spawn(move || {
            let mut r = BufReader::new(reader_stream);
            let mut line = String::new();
            let mut out = std::io::stdout();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(msg) = serde_json::from_str::<FrameMsg>(line.trim()) {
                            *flags.lock().unwrap() = msg.flags;
                            let ansi = frame_to_ansi(&msg.changes, &caps);
                            let _ = out.write_all(ansi.as_bytes());
                            let _ = out.flush();
                            if msg.flags.detach {
                                break;
                            }
                        }
                    }
                }
            }
            detached.store(true, Ordering::SeqCst);
        });
    }

    let mut leader = false;
    loop {
        if detached.load(Ordering::SeqCst) {
            break;
        }
        if event::poll(Duration::from_millis(16))? {
            let f = *flags.lock().unwrap();
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                    let ctrl_alt = ctrl && k.modifiers.contains(KeyModifiers::ALT);
                    let is_leader = (ctrl && k.code == KeyCode::Char(' ')) || k.code == KeyCode::Null;

                    if leader {
                        leader = false;
                        match k.code {
                            KeyCode::Char(' ') => send(&mut out_stream, &ClientMsg::ToggleSpotlight)?,
                            KeyCode::Char('a') | KeyCode::Char('A') => send(&mut out_stream, &ClientMsg::ToggleMenu)?,
                            KeyCode::Char('m') | KeyCode::Char('M') => send(&mut out_stream, &ClientMsg::MaximizeFocused)?,
                            KeyCode::Char('n') | KeyCode::Char('N') => send(&mut out_stream, &ClientMsg::MinimizeFocused)?,
                            KeyCode::Char('[') | KeyCode::Left => send(&mut out_stream, &ClientMsg::SnapFocused(SnapZone::Left))?,
                            KeyCode::Char(']') | KeyCode::Right => send(&mut out_stream, &ClientMsg::SnapFocused(SnapZone::Right))?,
                            KeyCode::Char('s') | KeyCode::Char('S') => send(&mut out_stream, &ClientMsg::OpenStore)?,
                            KeyCode::Char(',') => send(&mut out_stream, &ClientMsg::OpenSettings)?,
                            KeyCode::Char('t') => send(&mut out_stream, &ClientMsg::TileAll)?,
                            KeyCode::Char('T') => send(&mut out_stream, &ClientMsg::ToggleAutoTile)?,
                            KeyCode::Char(c @ '1'..='9') => send(&mut out_stream, &ClientMsg::SendToCell(c as u8 - b'0'))?,
                            KeyCode::Char('q') => break,                       // detach (apps persist)
                            KeyCode::Char('Q') => { send(&mut out_stream, &ClientMsg::Shutdown)?; break; }
                            _ => {}
                        }
                    } else if f.launcher_open {
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::LauncherEsc)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::LauncherEnter)?,
                            KeyCode::Up => send(&mut out_stream, &ClientMsg::LauncherUp)?,
                            KeyCode::Down => send(&mut out_stream, &ClientMsg::LauncherDown)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::LauncherBackspace)?,
                            KeyCode::Char(c) if f.spotlight_open && !ctrl => send(&mut out_stream, &ClientMsg::LauncherChar(c))?,
                            _ => {}
                        }
                    } else if f.dirpicker_open {
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::DirPickerCancel)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::DirPickerConfirm)?,
                            KeyCode::Up => send(&mut out_stream, &ClientMsg::DirPickerUp)?,
                            KeyCode::Down => send(&mut out_stream, &ClientMsg::DirPickerDown)?,
                            KeyCode::Right => send(&mut out_stream, &ClientMsg::DirPickerExpand)?,
                            KeyCode::Left => send(&mut out_stream, &ClientMsg::DirPickerCollapse)?,
                            KeyCode::Char('.') => send(&mut out_stream, &ClientMsg::DirPickerToggleHidden)?,
                            _ => {}
                        }
                    } else if is_leader {
                        leader = true;
                    } else if f.store_focused {
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::StoreClose)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::StoreActivate)?,
                            KeyCode::Up => send(&mut out_stream, &ClientMsg::StoreUp)?,
                            KeyCode::Down => send(&mut out_stream, &ClientMsg::StoreDown)?,
                            KeyCode::Left => send(&mut out_stream, &ClientMsg::StorePrevCategory)?,
                            KeyCode::Right => send(&mut out_stream, &ClientMsg::StoreNextCategory)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::StoreBackspace)?,
                            KeyCode::Char(c) if !ctrl => send(&mut out_stream, &ClientMsg::StoreChar(c))?,
                            _ => {}
                        }
                    } else if f.settings_focused && f.settings_editing {
                        // Apps add form: forward typed characters into the field.
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::SettingsCancelEdit)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::SettingsToggle)?,
                            KeyCode::Up => send(&mut out_stream, &ClientMsg::SettingsUp)?,
                            KeyCode::Down => send(&mut out_stream, &ClientMsg::SettingsDown)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::SettingsBackspace)?,
                            KeyCode::Char(c) if !ctrl => send(&mut out_stream, &ClientMsg::SettingsChar(c))?,
                            _ => {}
                        }
                    } else if f.settings_focused {
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::SettingsClose)?,
                            KeyCode::Up => send(&mut out_stream, &ClientMsg::SettingsUp)?,
                            KeyCode::Down => send(&mut out_stream, &ClientMsg::SettingsDown)?,
                            KeyCode::Left => send(&mut out_stream, &ClientMsg::SettingsLeft)?,
                            KeyCode::Right => send(&mut out_stream, &ClientMsg::SettingsRight)?,
                            KeyCode::Enter | KeyCode::Char(' ') => send(&mut out_stream, &ClientMsg::SettingsToggle)?,
                            _ => {}
                        }
                    } else if ctrl_alt {
                        match k.code {
                            KeyCode::Char('q') => break,
                            KeyCode::Up => send(&mut out_stream, &ClientMsg::MaximizeFocused)?,
                            KeyCode::Down => send(&mut out_stream, &ClientMsg::MinimizeFocused)?,
                            KeyCode::Left => send(&mut out_stream, &ClientMsg::SnapFocused(SnapZone::Left))?,
                            KeyCode::Right => send(&mut out_stream, &ClientMsg::SnapFocused(SnapZone::Right))?,
                            _ => send(&mut out_stream, &ClientMsg::Key(encode_key(k.code, k.modifiers)))?,
                        }
                    } else {
                        send(&mut out_stream, &ClientMsg::Key(encode_key(k.code, k.modifiers)))?;
                    }
                }
                Event::Mouse(m) => {
                    let p = Point::new(m.column as i32, m.row as i32);
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => send(&mut out_stream, &ClientMsg::MouseDown(p))?,
                        MouseEventKind::Drag(MouseButton::Left) => send(&mut out_stream, &ClientMsg::MouseDrag(p))?,
                        MouseEventKind::Up(MouseButton::Left) => send(&mut out_stream, &ClientMsg::MouseUp(p))?,
                        MouseEventKind::Moved => send(&mut out_stream, &ClientMsg::MouseDrag(p))?,
                        MouseEventKind::ScrollUp if f.store_focused => send(&mut out_stream, &ClientMsg::StoreUp)?,
                        MouseEventKind::ScrollDown if f.store_focused => send(&mut out_stream, &ClientMsg::StoreDown)?,
                        _ => {}
                    }
                }
                Event::Resize(nc, nr) => send(&mut out_stream, &ClientMsg::Resize { w: nc as i32, h: nr as i32 })?,
                _ => {}
            }
        }
    }

    // Detach: dropping `term` restores the screen; dropping the socket signals
    // the daemon, which keeps the session alive.
    drop(term);
    Ok(())
}

/// Serialize a [`ClientMsg`] as a newline-delimited JSON frame.
fn send(stream: &mut UnixStream, msg: &ClientMsg) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    buf.push(b'\n');
    stream.write_all(&buf)
}

/// Encode a key into the bytes forwarded to the focused PTY app.
fn encode_key(code: KeyCode, mods: KeyModifiers) -> Vec<u8> {
    match code {
        KeyCode::Char(c) => {
            if mods.contains(KeyModifiers::CONTROL) {
                vec![(c.to_ascii_uppercase() as u8).wrapping_sub(0x40)]
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
