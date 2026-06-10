//! The thin client: sets up the terminal, renders frames received from the
//! daemon, and forwards input. Holds no session state — it routes keyboard input
//! using the [`Flags`] the daemon sends each frame.

/// How a client session ended.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientExit {
    /// The user detached / shut down — stop.
    Detached,
    /// The daemon asked the client to reconnect (frontend reload).
    Reload,
    /// The user picked a system in the power menu: `main` should run the ssh
    /// switch (and optional first-time setup) in the real terminal, then
    /// re-attach locally when the remote session ends.
    Switch(crate::systems::SwitchSpec),
}

use crate::geometry::{Point, SnapZone};
use crate::protocol::{Flags, FrameMsg};
use crate::session::ClientMsg;
use crate::terminal::{frame_to_ansi, Terminal};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Attach to the daemon over `stream` and run until the user detaches.
pub fn run(stream: UnixStream) -> std::io::Result<ClientExit> {
    let term = Terminal::enter()?;
    let caps = term.caps;
    let (w, h) = Terminal::size()?;

    let mut out_stream = stream.try_clone()?;
    send(&mut out_stream, &ClientMsg::Resize { w, h })?;

    // A per-system theme rides over ssh as TUIUI_THEME: apply it to this
    // daemon's config as soon as we attach.
    if let Ok(theme) = std::env::var("TUIUI_THEME") {
        if !theme.is_empty() {
            send(&mut out_stream, &ClientMsg::SetTheme(theme))?;
        }
    }

    let flags = Arc::new(Mutex::new(Flags::default()));
    let detached = Arc::new(AtomicBool::new(false));
    let reload = Arc::new(AtomicBool::new(false));
    let switch: Arc<Mutex<Option<crate::systems::SwitchSpec>>> = Arc::new(Mutex::new(None));

    // Reader thread: socket frames → ANSI → stdout.
    {
        let flags = flags.clone();
        let detached = detached.clone();
        let reload = reload.clone();
        let switch = switch.clone();
        let reader_stream = stream.try_clone()?;
        std::thread::spawn(move || {
            let mut r = BufReader::new(reader_stream);
            let mut line = String::new();
            let mut out = std::io::stdout();
            // Image ids already transmitted, and the placement geometry currently
            // displayed for each (so we only re-place on move/resize).
            let mut transmitted: std::collections::HashSet<u64> = std::collections::HashSet::new();
            let mut active: std::collections::HashMap<u32, (u64, i32, i32, u16, u16)> = std::collections::HashMap::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(mut msg) = serde_json::from_str::<FrameMsg>(line.trim()) {
                            *flags.lock().unwrap() = msg.flags;
                            if let Some(text) = msg.clipboard.take() {
                                // OSC 52: set the host terminal's clipboard
                                // (Ghostty/Kitty/WezTerm; forwarded over ssh).
                                let b64 = crate::kitty::b64(text.as_bytes());
                                let _ = out.write_all(format!("\x1b]52;c;{b64}\x07").as_bytes());
                            }
                            if let Some(spec) = msg.switch_to.take() {
                                crate::dbg_log(&format!("client: switch requested → {} ({})", spec.name, spec.host));
                                *switch.lock().unwrap() = Some(spec);
                            }
                            if msg.clear {
                                // Re-baseline (attach / resize): erase every cell and
                                // delete all image placements *and* their data. A cell
                                // repaint alone can never remove an image — terminals
                                // composite graphics over text — and after a resize the
                                // emulator's screen/image state can diverge from our
                                // incremental model, leaving orphaned icons on screen.
                                let _ = out.write_all(b"\x1b[0m\x1b[2J\x1b_Ga=d,d=A\x1b\\");
                                transmitted.clear();
                                active.clear();
                            }
                            let ansi = frame_to_ansi(&msg.changes, &caps);
                            let _ = out.write_all(ansi.as_bytes());
                            if caps.kitty_graphics {
                                let g = reconcile_images(&msg, &mut transmitted, &mut active);
                                let _ = out.write_all(g.as_bytes());
                            }
                            let _ = out.flush();
                            if msg.flags.reload {
                                reload.store(true, Ordering::SeqCst);
                                break;
                            }
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

    // On a bare Linux console, gpm provides the mouse (the VT emits no xterm
    // mouse sequences). No-op on GUI terminals / non-Linux.
    crate::gpm::start(flags.clone(), out_stream.try_clone()?);

    let mut leader = false;
    let mut last_click: Option<(Point, std::time::Instant)> = None;
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

                    if f.confirm_close {
                        // The confirm-close dialog is modal: Enter/y confirm, Esc/n cancel.
                        leader = false;
                        match k.code {
                            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                                send(&mut out_stream, &ClientMsg::ConfirmCloseYes)?
                            }
                            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                                send(&mut out_stream, &ClientMsg::ConfirmCloseNo)?
                            }
                            _ => {}
                        }
                    } else if f.power_editing {
                        // The Add Remote form is modal: forward typing + field nav.
                        leader = false;
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::PowerFormCancel)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::PowerFormCommit)?,
                            KeyCode::Tab | KeyCode::Down => send(&mut out_stream, &ClientMsg::PowerFormNext)?,
                            KeyCode::BackTab | KeyCode::Up => send(&mut out_stream, &ClientMsg::PowerFormPrev)?,
                            KeyCode::Left => send(&mut out_stream, &ClientMsg::PowerFormLeft)?,
                            KeyCode::Right => send(&mut out_stream, &ClientMsg::PowerFormRight)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::PowerFormBackspace)?,
                            KeyCode::Char(c) if !ctrl => send(&mut out_stream, &ClientMsg::PowerFormChar(c))?,
                            _ => {}
                        }
                    } else if leader {
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
                            KeyCode::Char('?') | KeyCode::Char('h') => send(&mut out_stream, &ClientMsg::ToggleHelp)?,
                            KeyCode::Char('r') | KeyCode::Char('R') => send(&mut out_stream, &ClientMsg::RenameFocused)?,
                            KeyCode::Char('t') => send(&mut out_stream, &ClientMsg::TileAll)?,
                            KeyCode::Char('T') => send(&mut out_stream, &ClientMsg::ToggleAutoTile)?,
                            KeyCode::Char(c @ '1'..='9') => send(&mut out_stream, &ClientMsg::SendToCell(c as u8 - b'0'))?,
                            KeyCode::Char('q') => break,                       // detach (apps persist)
                            KeyCode::Char('Q') => { send(&mut out_stream, &ClientMsg::Shutdown)?; break; }
                            _ => {}
                        }
                    } else if f.logs_focused {
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::LogsClose)?,
                            KeyCode::Up => send(&mut out_stream, &ClientMsg::LogsUp)?,
                            KeyCode::Down => send(&mut out_stream, &ClientMsg::LogsDown)?,
                            KeyCode::PageUp => send(&mut out_stream, &ClientMsg::LogsPageUp)?,
                            KeyCode::PageDown => send(&mut out_stream, &ClientMsg::LogsPageDown)?,
                            KeyCode::Char('c') | KeyCode::Char('C') => send(&mut out_stream, &ClientMsg::LogsCopy)?,
                            KeyCode::Char('r') | KeyCode::Char('R') => send(&mut out_stream, &ClientMsg::LogsRefresh)?,
                            _ => {}
                        }
                    } else if f.help_open {
                        // The help overlay is modal: any key dismisses it.
                        send(&mut out_stream, &ClientMsg::ToggleHelp)?;
                    } else if f.launcher_open {
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::LauncherEsc)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::LauncherEnter)?,
                            KeyCode::Up => send(&mut out_stream, &ClientMsg::LauncherUp)?,
                            KeyCode::Down => send(&mut out_stream, &ClientMsg::LauncherDown)?,
                            KeyCode::Left => send(&mut out_stream, &ClientMsg::LauncherLeft)?,
                            KeyCode::Right => send(&mut out_stream, &ClientMsg::LauncherRight)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::LauncherBackspace)?,
                            KeyCode::Char(c) if f.spotlight_open && !ctrl => send(&mut out_stream, &ClientMsg::LauncherChar(c))?,
                            _ => {}
                        }
                    } else if f.dirpicker_open && f.dirpicker_creating {
                        // Typing a new folder name.
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::DirPickerCancel)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::DirPickerConfirm)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::DirPickerBackspace)?,
                            KeyCode::Char(c) if !ctrl => send(&mut out_stream, &ClientMsg::DirPickerChar(c))?,
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
                            KeyCode::Char('n') | KeyCode::Char('N') => send(&mut out_stream, &ClientMsg::DirPickerNewFolder)?,
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
                    } else if f.filemanager_focused && f.filemanager_editing {
                        // New-folder / rename overlay: forward typed characters.
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::FileManagerCancel)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::FileManagerCommit)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::FileManagerBackspace)?,
                            KeyCode::Char(c) if !ctrl => send(&mut out_stream, &ClientMsg::FileManagerChar(c))?,
                            _ => {}
                        }
                    } else if f.filemanager_focused {
                        match (k.code, ctrl) {
                            (KeyCode::Esc, _) => send(&mut out_stream, &ClientMsg::FileManagerClose)?,
                            (KeyCode::Up, _) => send(&mut out_stream, &ClientMsg::FileManagerUp)?,
                            (KeyCode::Down, _) => send(&mut out_stream, &ClientMsg::FileManagerDown)?,
                            (KeyCode::Left, _) => send(&mut out_stream, &ClientMsg::FileManagerLeft)?,
                            (KeyCode::Right, _) => send(&mut out_stream, &ClientMsg::FileManagerRight)?,
                            (KeyCode::Enter, _) => send(&mut out_stream, &ClientMsg::FileManagerActivate)?,
                            (KeyCode::Backspace, _) => send(&mut out_stream, &ClientMsg::FileManagerParent)?,
                            (KeyCode::Char('c'), true) => send(&mut out_stream, &ClientMsg::FileManagerCopy)?,
                            (KeyCode::Char('x'), true) => send(&mut out_stream, &ClientMsg::FileManagerCut)?,
                            (KeyCode::Char('v'), true) => send(&mut out_stream, &ClientMsg::FileManagerPaste)?,
                            (KeyCode::Char('n'), true) => send(&mut out_stream, &ClientMsg::FileManagerNewFolder)?,
                            (KeyCode::Delete, _) => send(&mut out_stream, &ClientMsg::FileManagerDelete)?,
                            (KeyCode::F(2), _) => send(&mut out_stream, &ClientMsg::FileManagerRename)?,
                            (KeyCode::Char('1'), false) => send(&mut out_stream, &ClientMsg::FileManagerViewIcon)?,
                            (KeyCode::Char('2'), false) => send(&mut out_stream, &ClientMsg::FileManagerViewList)?,
                            (KeyCode::Char('3'), false) => send(&mut out_stream, &ClientMsg::FileManagerViewColumns)?,
                            (KeyCode::Char(' '), false) => send(&mut out_stream, &ClientMsg::FileManagerTogglePreview)?,
                            (KeyCode::Char('.'), false) => send(&mut out_stream, &ClientMsg::FileManagerToggleHidden)?,
                            (KeyCode::Char('t'), true) => send(&mut out_stream, &ClientMsg::FileManagerNewTab)?,
                            (KeyCode::Char('w'), true) => send(&mut out_stream, &ClientMsg::FileManagerCloseTab)?,
                            (KeyCode::Tab, false) => send(&mut out_stream, &ClientMsg::FileManagerNextTab)?,
                            _ => {}
                        }
                    } else if f.desktop_editing {
                        // Desktop rename / new-folder overlay: forward typed chars.
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::DesktopCancel)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::DesktopCommit)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::DesktopBackspace)?,
                            KeyCode::Char(c) if !ctrl => send(&mut out_stream, &ClientMsg::DesktopChar(c))?,
                            _ => {}
                        }
                    } else if f.renaming {
                        // Window rename overlay: forward typed chars to the rename buffer.
                        match k.code {
                            KeyCode::Esc => send(&mut out_stream, &ClientMsg::RenameCancel)?,
                            KeyCode::Enter => send(&mut out_stream, &ClientMsg::RenameCommit)?,
                            KeyCode::Backspace => send(&mut out_stream, &ClientMsg::RenameBackspace)?,
                            KeyCode::Char(c) if !ctrl => send(&mut out_stream, &ClientMsg::RenameChar(c))?,
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
                Event::Mouse(me) => {
                    let p = Point::new(me.column as i32, me.row as i32);
                    let mods = crate::mouse::MouseMods {
                        shift: me.modifiers.contains(KeyModifiers::SHIFT),
                        ctrl: me.modifiers.contains(KeyModifiers::CONTROL),
                        alt: me.modifiers.contains(KeyModifiers::ALT),
                    };
                    if let Some(ev) = to_mouse_input(&me, p, mods) {
                        route_mouse(&mut out_stream, &f, ev, &mut last_click)?;
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
    if let Some(spec) = switch.lock().unwrap().take() {
        return Ok(ClientExit::Switch(spec));
    }
    if reload.load(Ordering::SeqCst) {
        Ok(ClientExit::Reload)
    } else {
        Ok(ClientExit::Detached)
    }
}

/// Build the Kitty graphics escapes for a frame: transmit not-yet-seen image
/// blobs, (re)place images whose geometry changed, and delete placements that are
/// now hidden or gone. The cursor is saved/restored so cell rendering is intact.
fn reconcile_images(
    msg: &FrameMsg,
    transmitted: &mut std::collections::HashSet<u64>,
    active: &mut std::collections::HashMap<u32, (u64, i32, i32, u16, u16)>,
) -> String {
    use std::fmt::Write;
    let mut g = String::new();
    for blob in &msg.image_data {
        if transmitted.insert(blob.id) {
            g.push_str(&crate::kitty::transmit_b64(blob.id, &blob.png_base64));
        }
    }
    // Key placements by screen position, not image id, so one image (e.g. a shared
    // file-type icon) can appear at many spots. `active` maps placement-key →
    // (image_id, x, y, cols, rows).
    let mut now: std::collections::HashMap<u32, (u64, i32, i32, u16, u16)> = std::collections::HashMap::new();
    for p in &msg.images {
        // A negative origin can't be addressed: CUP parameters are 1-based, and
        // a 0/negative parameter aborts the escape, dropping the image at the
        // current cursor position — i.e. painted at an arbitrary screen spot.
        if p.visible && p.rect.x >= 0 && p.rect.y >= 0 {
            now.insert(place_key(p.rect.x, p.rect.y), (p.id, p.rect.x, p.rect.y, p.cols, p.rows));
        }
    }
    let mut ops = String::new();
    // Remove placements that are gone, or whose image changed at the same slot.
    for (&pk, &(img, ..)) in active.iter() {
        let keep = matches!(now.get(&pk), Some(&(new_img, ..)) if new_img == img);
        if !keep {
            ops.push_str(&crate::kitty::delete(img, pk));
        }
    }
    // (Re)place new or moved/changed placements.
    for (&pk, &(img, x, y, c, r)) in now.iter() {
        if active.get(&pk) != Some(&(img, x, y, c, r)) {
            let _ = write!(ops, "\x1b[{};{}H", y + 1, x + 1);
            ops.push_str(&crate::kitty::place(img, pk, c, r));
        }
    }
    if !ops.is_empty() {
        g.push_str("\x1b[s"); // save cursor
        g.push_str(&ops);
        g.push_str("\x1b[u"); // restore cursor
    }
    *active = now;
    g
}

/// A stable per-position placement id (Kitty `p=`), unique per cell coordinate.
fn place_key(x: i32, y: i32) -> u32 {
    (((x.max(0) as u32) & 0xffff) << 16) | ((y.max(0) as u32) & 0xffff)
}

/// Route one mouse event: into the focused app (passthrough) when the pointer is
/// in `f.app_area`, otherwise via the existing chrome/WM variants. Shared by the
/// crossterm path and the gpm reader so both behave identically.
pub(crate) fn route_mouse(
    out: &mut UnixStream,
    f: &Flags,
    ev: crate::mouse::MouseInput,
    last_click: &mut Option<(Point, std::time::Instant)>,
) -> std::io::Result<()> {
    use crate::mouse::{MouseAction as A, MouseButton as B};
    let p = Point::new(ev.col, ev.row);
    if f.app_area.map(|r| r.contains(p)).unwrap_or(false) {
        return send(out, &ClientMsg::MouseInput(ev));
    }
    match (ev.button, ev.action) {
        (B::Left, A::Down) => {
            let now = std::time::Instant::now();
            let dbl = last_click
                .map(|(lp, lt)| lp == p && now.duration_since(lt) < std::time::Duration::from_millis(400))
                .unwrap_or(false);
            if dbl {
                send(out, &ClientMsg::MouseDouble(p))?;
                *last_click = None;
            } else {
                send(out, &ClientMsg::MouseDown(p))?;
                *last_click = Some((p, now));
            }
        }
        (B::Right, A::Down) => send(out, &ClientMsg::MouseRightDown(p))?,
        (B::Left, A::Drag) => send(out, &ClientMsg::MouseDrag(p))?,
        (B::Left, A::Up) => send(out, &ClientMsg::MouseUp(p))?,
        (_, A::Move) => send(out, &ClientMsg::MouseDrag(p))?,
        (_, A::ScrollUp) if f.store_focused => send(out, &ClientMsg::StoreUp)?,
        (_, A::ScrollDown) if f.store_focused => send(out, &ClientMsg::StoreDown)?,
        (_, A::ScrollUp) if f.filemanager_focused => send(out, &ClientMsg::FileManagerUp)?,
        (_, A::ScrollDown) if f.filemanager_focused => send(out, &ClientMsg::FileManagerDown)?,
        (_, A::ScrollUp) if f.logs_focused => send(out, &ClientMsg::LogsUp)?,
        (_, A::ScrollDown) if f.logs_focused => send(out, &ClientMsg::LogsDown)?,
        // Anywhere else, the wheel scrolls the scrollback of the PTY window
        // under the pointer (a no-op if that's not a non-mouse app window).
        (_, A::ScrollUp) => send(out, &ClientMsg::ScrollAt { p, lines: 3 })?,
        (_, A::ScrollDown) => send(out, &ClientMsg::ScrollAt { p, lines: -3 })?,
        _ => {}
    }
    Ok(())
}

/// Map a crossterm [`event::MouseEvent`] to a [`crate::mouse::MouseInput`] for
/// passthrough to the focused PTY app. Returns `None` only if the event kind is
/// somehow unrecognised (should not happen in practice).
fn to_mouse_input(
    me: &event::MouseEvent,
    p: Point,
    mods: crate::mouse::MouseMods,
) -> Option<crate::mouse::MouseInput> {
    use crate::mouse::{MouseAction, MouseButton, MouseInput};
    use crossterm::event::{MouseButton as XB, MouseEventKind as K};
    let (button, action) = match me.kind {
        K::Down(XB::Left) => (MouseButton::Left, MouseAction::Down),
        K::Down(XB::Middle) => (MouseButton::Middle, MouseAction::Down),
        K::Down(XB::Right) => (MouseButton::Right, MouseAction::Down),
        K::Up(XB::Left) => (MouseButton::Left, MouseAction::Up),
        K::Up(XB::Middle) => (MouseButton::Middle, MouseAction::Up),
        K::Up(XB::Right) => (MouseButton::Right, MouseAction::Up),
        K::Drag(XB::Left) => (MouseButton::Left, MouseAction::Drag),
        K::Drag(XB::Middle) => (MouseButton::Middle, MouseAction::Drag),
        K::Drag(XB::Right) => (MouseButton::Right, MouseAction::Drag),
        K::Moved => (MouseButton::None, MouseAction::Move),
        K::ScrollUp => (MouseButton::None, MouseAction::ScrollUp),
        K::ScrollDown => (MouseButton::None, MouseAction::ScrollDown),
        K::ScrollLeft => (MouseButton::None, MouseAction::ScrollLeft),
        K::ScrollRight => (MouseButton::None, MouseAction::ScrollRight),
    };
    Some(MouseInput { col: p.x, row: p.y, button, action, mods })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::protocol::ImagePlacement;

    fn frame_with(images: Vec<ImagePlacement>) -> FrameMsg {
        FrameMsg {
            changes: Vec::new(),
            cursor: None,
            flags: Flags::default(),
            images,
            image_data: Vec::new(),
            clear: false,
            switch_to: None,
            clipboard: None,
        }
    }

    #[test]
    fn places_visible_placement_with_cup_and_place_ops() {
        let msg = frame_with(vec![ImagePlacement {
            id: 7,
            rect: Rect::new(2, 3, 4, 2),
            cols: 4,
            rows: 2,
            visible: true,
        }]);
        let mut transmitted = std::collections::HashSet::new();
        let mut active = std::collections::HashMap::new();
        let g = reconcile_images(&msg, &mut transmitted, &mut active);
        assert!(g.contains("\x1b[4;3H"), "CUP to 1-based (row 4, col 3): {g:?}");
        assert!(g.contains("a=p"), "place op emitted: {g:?}");
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn skips_placement_with_negative_origin() {
        // A CUP with a 0/negative parameter aborts the escape and the image
        // would land at the current cursor position — never emit one.
        let msg = frame_with(vec![ImagePlacement {
            id: 7,
            rect: Rect::new(-3, 5, 4, 2),
            cols: 4,
            rows: 2,
            visible: true,
        }]);
        let mut transmitted = std::collections::HashSet::new();
        let mut active = std::collections::HashMap::new();
        let g = reconcile_images(&msg, &mut transmitted, &mut active);
        assert!(!g.contains("a=p"), "no place op for an unaddressable rect: {g:?}");
        assert!(active.is_empty());
    }

    #[test]
    fn deletes_placement_that_disappeared() {
        let mut transmitted = std::collections::HashSet::new();
        let mut active = std::collections::HashMap::new();
        active.insert(place_key(2, 3), (7u64, 2, 3, 4u16, 2u16));
        let g = reconcile_images(&frame_with(Vec::new()), &mut transmitted, &mut active);
        assert!(g.contains("a=d,d=i,i=7"), "stale placement deleted: {g:?}");
        assert!(active.is_empty());
    }
}

