//! Shared mouse types and the encoder that turns a frontend mouse event into the
//! byte sequence a PTY app expects, honouring the app's reported mouse mode.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton { Left, Middle, Right, None }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction { Down, Up, Drag, Move, ScrollUp, ScrollDown, ScrollLeft, ScrollRight }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MouseMods { pub shift: bool, pub ctrl: bool, pub alt: bool }

/// A mouse event from the client. `col`/`row` are screen cells (0-based) on the
/// wire; the daemon localises them (subtracts the app area origin) before encoding.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseInput {
    pub col: i32,
    pub row: i32,
    pub button: MouseButton,
    pub action: MouseAction,
    pub mods: MouseMods,
}

/// The focused app's terminal mouse mode, derived from `alacritty Term.mode()`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AppMouse {
    pub report_click: bool,
    pub report_drag: bool,
    pub report_motion: bool,
    pub sgr: bool,
    pub utf8: bool,
    pub alternate_scroll: bool,
    pub alt_screen: bool,
}

impl AppMouse {
    /// Whether the app wants click/drag/motion mouse reporting.
    pub fn wants_mouse(&self) -> bool {
        self.report_click || self.report_drag || self.report_motion
    }
    /// Whether the frontend should treat the pointer over this app as "in-app"
    /// (forwardable): true if it wants mouse, or wheel-to-arrows applies.
    pub fn captures_pointer(&self) -> bool {
        self.wants_mouse() || (self.alternate_scroll && self.alt_screen)
    }
}

const ESC: u8 = 0x1b;

/// Encode `ev` (with **0-based, app-local** col/row) for an app in mouse mode
/// `m`. Returns the PTY bytes, or `None` if this event should not be forwarded.
pub fn encode(ev: &MouseInput, m: &AppMouse) -> Option<Vec<u8>> {
    use MouseAction::*;

    // Alternate-scroll: wheel becomes arrow keys when the app is on the alt
    // screen with alternate-scroll set and is NOT in mouse-reporting mode.
    let scroll = matches!(ev.action, ScrollUp | ScrollDown | ScrollLeft | ScrollRight);
    if scroll && !m.wants_mouse() {
        if m.alternate_scroll && m.alt_screen {
            let arrow: &[u8] = match ev.action {
                ScrollUp => b"\x1b[A",
                ScrollDown => b"\x1b[B",
                ScrollLeft => b"\x1b[D",
                ScrollRight => b"\x1b[C",
                _ => unreachable!(),
            };
            return Some(arrow.to_vec());
        }
        return None;
    }
    if !m.wants_mouse() {
        return None;
    }

    // Gate by reporting level.
    match ev.action {
        Move if !m.report_motion => return None,
        Drag if !(m.report_drag || m.report_motion) => return None,
        _ => {}
    }

    // Base button code.
    let mut cb: u32 = match ev.action {
        ScrollUp => 64,
        ScrollDown => 65,
        ScrollLeft => 66,
        ScrollRight => 67,
        _ => match ev.button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::None => 3,
        },
    };
    // Motion bit for drag/move.
    if matches!(ev.action, Drag | Move) {
        cb += 32;
    }
    // Modifiers.
    if ev.mods.shift { cb += 4; }
    if ev.mods.alt   { cb += 8; }
    if ev.mods.ctrl  { cb += 16; }

    // 1-based coordinates.
    let col = ev.col.max(0) as u32 + 1;
    let row = ev.row.max(0) as u32 + 1;

    if m.sgr {
        // ESC [ < Cb ; col ; row (M=press/motion/scroll, m=release)
        let final_ch = if matches!(ev.action, Up) { b'm' } else { b'M' };
        let mut out = Vec::new();
        out.push(ESC);
        out.extend_from_slice(b"[<");
        out.extend_from_slice(cb.to_string().as_bytes());
        out.push(b';');
        out.extend_from_slice(col.to_string().as_bytes());
        out.push(b';');
        out.extend_from_slice(row.to_string().as_bytes());
        out.push(final_ch);
        Some(out)
    } else {
        // Legacy X10: ESC [ M (32+Cb) (32+col) (32+row), release => button 3.
        let cb_legacy = if matches!(ev.action, Up) {
            // keep motion/mod bits but force button bits to release (3)
            (cb & !0b11) + 3
        } else {
            cb
        };
        // Clamp coords to the 223 ceiling (1..=223), then +32.
        let cx  = (col.min(223) + 32) as u8;
        let cy  = (row.min(223) + 32) as u8;
        let cbb = (cb_legacy.min(223) + 32) as u8;
        Some(vec![ESC, b'[', b'M', cbb, cx, cy])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(col: i32, row: i32, button: MouseButton, action: MouseAction) -> MouseInput {
        MouseInput { col, row, button, action, mods: MouseMods::default() }
    }
    fn sgr() -> AppMouse { AppMouse { report_click: true, report_drag: true, report_motion: false, sgr: true, utf8: false, alternate_scroll: false, alt_screen: false } }
    fn legacy() -> AppMouse { AppMouse { report_click: true, report_drag: false, report_motion: false, sgr: false, utf8: false, alternate_scroll: false, alt_screen: false } }

    #[test]
    fn sgr_left_press_release() {
        // app-local (0,0) -> 1;1
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Down), &sgr()).unwrap(), b"\x1b[<0;1;1M");
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Up), &sgr()).unwrap(), b"\x1b[<0;1;1m");
    }

    #[test]
    fn sgr_right_and_middle_and_mods() {
        assert_eq!(encode(&ev(2, 0, MouseButton::Right, MouseAction::Down), &sgr()).unwrap(), b"\x1b[<2;3;1M");
        let mut e = ev(0, 0, MouseButton::Middle, MouseAction::Down);
        e.mods = MouseMods { shift: false, ctrl: true, alt: false };
        assert_eq!(encode(&e, &sgr()).unwrap(), b"\x1b[<17;1;1M"); // 1 + ctrl(16)
    }

    #[test]
    fn sgr_scroll() {
        assert_eq!(encode(&ev(0, 0, MouseButton::None, MouseAction::ScrollUp), &sgr()).unwrap(), b"\x1b[<64;1;1M");
        assert_eq!(encode(&ev(0, 0, MouseButton::None, MouseAction::ScrollDown), &sgr()).unwrap(), b"\x1b[<65;1;1M");
    }

    #[test]
    fn drag_gated_by_report_drag() {
        let mut m = sgr(); m.report_drag = false;
        assert!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Drag), &m).is_none());
        let m2 = sgr(); // report_drag true
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Drag), &m2).unwrap(), b"\x1b[<32;1;1M");
    }

    #[test]
    fn legacy_encoding_and_clamp() {
        // (0,0): ESC [ M  32+0  32+1  32+1
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Down), &legacy()).unwrap(), vec![0x1b, b'[', b'M', 32, 33, 33]);
        // release => button 3
        assert_eq!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Up), &legacy()).unwrap(), vec![0x1b, b'[', b'M', 35, 33, 33]);
    }

    #[test]
    fn alternate_scroll_to_arrows() {
        let m = AppMouse { report_click: false, report_drag: false, report_motion: false, sgr: false, utf8: false, alternate_scroll: true, alt_screen: true };
        assert_eq!(encode(&ev(5, 5, MouseButton::None, MouseAction::ScrollUp), &m).unwrap(), b"\x1b[A");
        assert_eq!(encode(&ev(5, 5, MouseButton::None, MouseAction::ScrollDown), &m).unwrap(), b"\x1b[B");
        // a click in such an app is not forwarded
        assert!(encode(&ev(5, 5, MouseButton::Left, MouseAction::Down), &m).is_none());
    }

    #[test]
    fn no_mouse_means_no_bytes() {
        let m = AppMouse::default();
        assert!(encode(&ev(0, 0, MouseButton::Left, MouseAction::Down), &m).is_none());
    }
}
