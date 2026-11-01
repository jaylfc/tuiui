//! `tuiui view <app-id>`: show ONE hosted app full-screen in the current
//! terminal, without touching the desktop.
//!
//! The viewer is just another apphost frontend (protocol v3+): it subscribes
//! to that one app's frames (`HostReq::Watch`), renders its grid, and forwards
//! keyboard (and, when the app asked for it, mouse) input. It never sends
//! `SetMeta`, so the app's desktop window is not closed, moved or renamed, and
//! it never takes the app's bells / clipboard stores (the desktop keeps those).
//!
//! **Size.** By default the app's PTY is left at whatever size the desktop
//! gave it: the grid is centred when the terminal is larger and cropped when
//! smaller — anchored top-left and scrolled just enough to keep the cursor on
//! screen (bottom-anchored when the app hides its cursor). `--resize` instead
//! sizes the PTY to this terminal, but only when no visible desktop window
//! shows the app (none, or minimized); `--force-resize` does it regardless.
//! Either way the original size is restored on exit (unless someone else
//! resized the app meanwhile).
//!
//! **Exit:** the tuiui leader `Ctrl+Space` then `q` (leader twice sends a
//! literal `Ctrl+Space`/NUL to the app), or when the app exits.

use crate::apphost::proto::{recv, send, HostEvt, HostReq};
use crate::buffer::CellBuffer;
use crate::compositor::{Compositor, Layer};
use crate::geometry::Point;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

/// Whether `tuiui view` may change the app's PTY size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeMode {
    /// Never resize (default): centre / crop the app's current grid.
    Never,
    /// Resize to the viewer while no visible desktop window shows the app.
    IfHidden,
    /// Resize even if the desktop shows it (its window then shows the
    /// viewer-sized grid until the viewer exits).
    Force,
}

/// Parsed `tuiui view` arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewOpts {
    pub app: u64,
    pub resize: ResizeMode,
}

/// Parse `tuiui view [--resize|--force-resize] <app-id>`.
pub fn parse_args(args: &[String]) -> Result<ViewOpts, String> {
    let mut resize = ResizeMode::Never;
    let mut app = None;
    for a in args {
        match a.as_str() {
            "--resize" => resize = ResizeMode::IfHidden,
            "--force-resize" => resize = ResizeMode::Force,
            s if s.starts_with('-') => return Err(format!("unknown option '{s}'")),
            s => app = Some(s.parse::<u64>().map_err(|_| format!("'{s}' is not a numeric app id (see `tuiui ps`)"))?),
        }
    }
    app.map(|app| ViewOpts { app, resize })
        .ok_or_else(|| "usage: tuiui view [--resize|--force-resize] <app-id>   (ids: `tuiui ps`)".into())
}

/// Where the app's `app` (cols, rows) grid goes on a `view` (cols, rows)
/// screen: centred along an axis where it fits; where it doesn't, anchored
/// at the top/left and shifted just enough to keep the `cursor` (grid
/// coords) on screen — or bottom-anchored when there is no visible cursor.
pub fn place(app: (i32, i32), view: (i32, i32), cursor: Option<(u16, u16)>) -> Point {
    // One axis: `a` = app extent, `v` = view extent, `c` = cursor coordinate.
    fn axis(a: i32, v: i32, c: Option<i32>, no_cursor: i32) -> i32 {
        if a <= v {
            return (v - a) / 2;
        }
        match c {
            Some(c) if c >= v => v - 1 - c, // scroll so the cursor is the last visible cell
            Some(_) => 0,
            None => no_cursor,
        }
    }
    let (cx, cy) = match cursor {
        Some((x, y)) => (Some(x as i32), Some(y as i32)),
        None => (None, None),
    };
    Point::new(axis(app.0, view.0, cx, 0), axis(app.1, view.1, cy, view.1 - app.1))
}

/// What the viewer's key handler decided.
#[derive(Debug, PartialEq, Eq)]
enum KeyOutcome {
    Quit,
    Send(Vec<u8>),
    Nothing,
}

/// Leader-aware key mapping (pure; tested). `leader` is the armed state.
fn handle_key(leader: &mut bool, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let is_leader = (ctrl && code == KeyCode::Char(' ')) || code == KeyCode::Null;
    if *leader {
        *leader = false;
        return match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => KeyOutcome::Quit,
            _ if is_leader => KeyOutcome::Send(vec![0]),
            _ => encoded(code, mods),
        };
    }
    if is_leader {
        *leader = true;
        return KeyOutcome::Nothing;
    }
    encoded(code, mods)
}

fn encoded(code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
    let mut bytes = match code {
        // A few keys the desktop's minimal encoder leaves out.
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        _ => crate::client::encode_key(code, mods),
    };
    if bytes.is_empty() {
        return KeyOutcome::Nothing;
    }
    if mods.contains(KeyModifiers::ALT) && matches!(code, KeyCode::Char(_)) {
        bytes.insert(0, 0x1b);
    }
    KeyOutcome::Send(bytes)
}

/// Raw mode + alternate screen for the viewer's lifetime (mouse capture is
/// toggled to match the app). Restored on drop.
struct Screen {
    mouse: bool,
}

impl Screen {
    fn enter() -> std::io::Result<Screen> {
        use crossterm::{cursor, execute, terminal};
        terminal::enable_raw_mode()?;
        execute!(std::io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Screen { mouse: false })
    }

    fn set_mouse(&mut self, on: bool) {
        use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
        if on != self.mouse {
            let _ = if on {
                crossterm::execute!(std::io::stdout(), EnableMouseCapture)
            } else {
                crossterm::execute!(std::io::stdout(), DisableMouseCapture)
            };
            self.mouse = on;
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        use crossterm::{cursor, execute, terminal};
        self.set_mouse(false);
        let _ = execute!(std::io::stdout(), terminal::LeaveAlternateScreen, cursor::Show);
        let _ = terminal::disable_raw_mode();
    }
}

/// Run the viewer. Returns the process exit code.
pub fn run(opts: ViewOpts) -> std::io::Result<i32> {
    let path = crate::protocol::apphost_socket_path();
    let Ok(stream) = UnixStream::connect(&path) else {
        eprintln!("tuiui view: no apphost running (start it with `tuiui`)");
        return Ok(1);
    };
    let mut writer = stream.try_clone()?;
    let mut r = BufReader::new(stream);
    // A pre-v3 apphost serves one frontend at a time, so while the desktop
    // daemon holds it we would wait forever for the roster.
    r.get_ref().set_read_timeout(Some(Duration::from_secs(3)))?;
    let (proto, meta) = match recv::<HostEvt, _>(&mut r) {
        Ok(Some(HostEvt::Roster { apps, proto })) => {
            let Some(e) = apps.into_iter().find(|e| e.app == opts.app) else {
                eprintln!("tuiui view: no app {} (see `tuiui ps`)", opts.app);
                return Ok(1);
            };
            (proto, e.meta)
        }
        Ok(_) => {
            eprintln!("tuiui view: unexpected first message from the apphost");
            return Ok(1);
        }
        Err(_) => {
            eprintln!("{}", crate::pslist::BUSY_HINT);
            return Ok(1);
        }
    };
    if proto < 3 {
        eprintln!(
            "tuiui view: the running app server speaks protocol v{proto}; viewing needs v3+ \
             (restart the app server — this closes its apps)"
        );
        return Ok(1);
    }
    r.get_ref().set_read_timeout(None)?;
    send(&mut writer, &HostReq::Watch { apps: vec![opts.app] })?;
    let shown_on_desktop = crate::session::decode_window_meta(&meta).map(|m| !m.minimized).unwrap_or(false);
    let do_resize = match opts.resize {
        ResizeMode::Never => false,
        ResizeMode::Force => true,
        ResizeMode::IfHidden => {
            if shown_on_desktop {
                eprintln!("tuiui view: app {} is showing on the desktop; not resizing it (use --force-resize)", opts.app);
                std::thread::sleep(Duration::from_millis(1200));
            }
            !shown_on_desktop
        }
    };
    crate::dbg_log(&format!("view: app {} resize={do_resize}", opts.app));

    // Apphost events → channel.
    let (tx, rx) = mpsc::channel::<HostEvt>();
    std::thread::spawn(move || {
        while let Ok(Some(evt)) = recv::<HostEvt, _>(&mut r) {
            if tx.send(evt).is_err() {
                break;
            }
        }
    });

    crate::theme::set(&crate::config::Config::load().theme);
    let mut screen = Screen::enter()?;
    let (mut vw, mut vh) = crossterm::terminal::size().map(|(c, r)| (c as i32, r as i32))?;
    let mut comp = Compositor::new(vw, vh);
    let caps = crate::terminal::Caps::detect();
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[0m\x1b[2J");

    let mut grid: Option<CellBuffer> = None;
    let mut mouse = crate::mouse::AppMouse::default();
    let mut cursor: Option<(u16, u16)> = None;
    let mut original: Option<(i32, i32)> = None;
    let mut resized_to: Option<(i32, i32)> = None;
    let mut leader = false;
    let mut dirty = true;
    let mut exit_msg: Option<String> = None;

    'main: loop {
        // Apphost events.
        loop {
            match rx.try_recv() {
                Ok(HostEvt::Frame { app, grid: g, mouse: m, cursor: c, .. }) if app == opts.app => {
                    if original.is_none() {
                        original = Some((g.width(), g.height()));
                    }
                    grid = Some(g);
                    mouse = m;
                    cursor = c;
                    dirty = true;
                }
                Ok(HostEvt::Gone { app }) if app == opts.app => {
                    exit_msg = Some(format!("tuiui view: app {} exited", opts.app));
                    break 'main;
                }
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    exit_msg = Some("tuiui view: app server connection closed".into());
                    break 'main;
                }
            }
        }
        if do_resize && original.is_some() && resized_to != Some((vw, vh)) {
            send(&mut writer, &HostReq::Resize { app: opts.app, cols: vw, rows: vh })?;
            resized_to = Some((vw, vh));
        }
        screen.set_mouse(mouse.captures_pointer());

        let origin = grid
            .as_ref()
            .map(|g| place((g.width(), g.height()), (vw, vh), cursor))
            .unwrap_or(Point::new(0, 0));
        if dirty {
            if let Some(g) = &grid {
                let layer = Layer { z: 0, origin, buf: g.clone(), opacity: 1.0, scissor: None };
                let cur = cursor.map(|(x, y)| Point::new(origin.x + x as i32, origin.y + y as i32));
                comp.composite(&[layer], cur);
                let ansi = crate::terminal::frame_to_ansi(&comp.diff(), &caps);
                let _ = out.write_all(ansi.as_bytes());
                let _ = out.flush();
                comp.commit();
            }
            dirty = false;
        }

        if !event::poll(Duration::from_millis(16))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => match handle_key(&mut leader, k.code, k.modifiers) {
                KeyOutcome::Quit => break 'main,
                KeyOutcome::Send(bytes) => send(&mut writer, &HostReq::Input { app: opts.app, bytes })?,
                KeyOutcome::Nothing => {}
            },
            Event::Paste(text) => send(&mut writer, &HostReq::Input { app: opts.app, bytes: text.into_bytes() })?,
            Event::Mouse(me) => {
                let mods = crate::mouse::MouseMods {
                    shift: me.modifiers.contains(KeyModifiers::SHIFT),
                    ctrl: me.modifiers.contains(KeyModifiers::CONTROL),
                    alt: me.modifiers.contains(KeyModifiers::ALT),
                };
                let local = Point::new(me.column as i32 - origin.x, me.row as i32 - origin.y);
                let inside = grid
                    .as_ref()
                    .map(|g| local.x >= 0 && local.y >= 0 && local.x < g.width() && local.y < g.height())
                    .unwrap_or(false);
                if inside {
                    if let Some(ev) = crate::client::to_mouse_input(&me, local, mods) {
                        if let Some(bytes) = crate::mouse::encode(&ev, &mouse) {
                            send(&mut writer, &HostReq::Input { app: opts.app, bytes })?;
                        }
                    }
                }
            }
            Event::Resize(c, r) => {
                (vw, vh) = (c as i32, r as i32);
                comp.resize(vw, vh);
                let _ = out.write_all(b"\x1b[0m\x1b[2J");
                dirty = true;
            }
            _ => {}
        }
    }

    // Put the app back the way the desktop had it — unless someone else
    // (e.g. the desktop window being resized) changed it in the meantime.
    if let (Some(to), Some((ow, oh))) = (resized_to, original) {
        let current = grid.as_ref().map(|g| (g.width(), g.height()));
        if current == Some(to) && to != (ow, oh) {
            let _ = send(&mut writer, &HostReq::Resize { app: opts.app, cols: ow, rows: oh });
        }
    }
    drop(screen);
    if let Some(m) = exit_msg {
        eprintln!("{m}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_view_args() {
        assert_eq!(parse_args(&s(&["7"])), Ok(ViewOpts { app: 7, resize: ResizeMode::Never }));
        assert_eq!(parse_args(&s(&["--resize", "7"])).unwrap().resize, ResizeMode::IfHidden);
        assert_eq!(parse_args(&s(&["7", "--force-resize"])).unwrap().resize, ResizeMode::Force);
        assert!(parse_args(&s(&[])).is_err());
        assert!(parse_args(&s(&["x"])).is_err());
        assert!(parse_args(&s(&["--bogus", "1"])).is_err());
    }

    #[test]
    fn centres_a_small_grid_and_crops_a_big_one() {
        assert_eq!(place((80, 24), (100, 30), None), Point::new(10, 3));
        // Too big, no cursor: left-aligned, bottom rows visible.
        assert_eq!(place((120, 40), (100, 30), None), Point::new(0, -10));
        // Cursor near the top of a too-tall grid: top-anchored.
        assert_eq!(place((80, 40), (80, 30), Some((0, 2))), Point::new(0, 0));
        // Cursor below the visible rows: scrolled until it's the last row.
        assert_eq!(place((80, 40), (80, 30), Some((0, 35))), Point::new(0, -6));
        // Cursor far right of a too-wide grid pulls the view left.
        assert_eq!(place((120, 20), (100, 30), Some((110, 5))), Point::new(-11, 5));
    }

    #[test]
    fn leader_then_q_quits_and_double_leader_sends_nul() {
        let mut l = false;
        let none = KeyModifiers::NONE;
        assert_eq!(handle_key(&mut l, KeyCode::Char('a'), none), KeyOutcome::Send(b"a".to_vec()));
        assert_eq!(handle_key(&mut l, KeyCode::Char(' '), KeyModifiers::CONTROL), KeyOutcome::Nothing);
        assert!(l);
        assert_eq!(handle_key(&mut l, KeyCode::Char('q'), none), KeyOutcome::Quit);
        assert!(!l);
        handle_key(&mut l, KeyCode::Null, none);
        assert_eq!(handle_key(&mut l, KeyCode::Null, none), KeyOutcome::Send(vec![0]));
        // Leader + another key forwards that key.
        handle_key(&mut l, KeyCode::Null, none);
        assert_eq!(handle_key(&mut l, KeyCode::Enter, none), KeyOutcome::Send(b"\r".to_vec()));
        // Alt+char is ESC-prefixed; extra navigation keys are encoded.
        assert_eq!(handle_key(&mut l, KeyCode::Char('b'), KeyModifiers::ALT), KeyOutcome::Send(b"\x1bb".to_vec()));
        assert_eq!(handle_key(&mut l, KeyCode::PageUp, none), KeyOutcome::Send(b"\x1b[5~".to_vec()));
    }
}
