//! The menubar power menu: the "tuiui ▾" button at the top-right opens a small
//! dropdown (Exit / Restart / Shutdown); each enabled action opens a modal
//! confirmation dialog before it fires.
//!
//! The widget is pure UI state + geometry — it never touches the session. A
//! click that confirms an action is reported back as a [`PowerOutcome`] for the
//! session to carry out.

use crate::buffer::CellBuffer;
use crate::cell::Cell;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};

/// One item in the power dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    /// Detach the client; the daemon and all apps keep running in the background.
    Exit,
    /// Reload the frontend (restart the UI) while apps keep running in the apphost.
    Restart,
    /// Stop tuiui entirely (daemon exits, every app is killed).
    Shutdown,
}

impl PowerAction {
    fn label(self) -> &'static str {
        match self {
            PowerAction::Exit => "Exit",
            PowerAction::Restart => "Restart",
            PowerAction::Shutdown => "Shutdown",
        }
    }
}

/// The dropdown items, top to bottom.
const ITEMS: [PowerAction; 3] = [PowerAction::Exit, PowerAction::Restart, PowerAction::Shutdown];

/// What the session should do after a confirmed power-menu click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerOutcome {
    /// Detach the client, leaving the daemon + apps alive (Exit).
    Detach,
    /// Reload the frontend only — apps stay alive in the apphost.
    Reload,
    /// Full shutdown — daemon exits, apps killed.
    Shutdown,
}

/// Result of routing a click into an open power menu. While the menu is open it
/// is modal, so every click is `Consumed` unless it confirms an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerClick {
    /// The click changed menu state or dismissed it; nothing else to do.
    Consumed,
    /// A confirmed action should fire.
    Act(PowerOutcome),
}

/// Menubar power-button state machine: a dropdown that can raise a modal
/// confirmation dialog over itself.
#[derive(Default)]
pub struct PowerMenu {
    open: bool,
    confirm: Option<PowerAction>,
}

// ── Geometry (shared by render + hit-test so they can never drift) ──────────────

/// The dropdown panel, anchored just under the top-right "tuiui" button.
fn dropdown_rect(w: i32) -> Rect {
    let box_w = 16;
    let box_h = ITEMS.len() as i32 + 2; // +2 for the top/bottom border rows
    let anchor = crate::chrome::menubar_power_region(w);
    // Right-align the panel's right edge with the button's right edge.
    let x = (anchor.x + anchor.w - box_w).max(0);
    Rect::new(x, 1, box_w, box_h)
}

/// Screen rect of dropdown item `i` (the clickable row).
fn item_rect(w: i32, i: usize) -> Rect {
    let d = dropdown_rect(w);
    Rect::new(d.x + 1, d.y + 1 + i as i32, d.w - 2, 1)
}

/// The centered confirmation dialog box.
fn dialog_rect(w: i32, h: i32) -> Rect {
    let box_w = 52.min((w - 2).max(20));
    let box_h = 7.min((h - 1).max(5));
    Rect::new((w - box_w) / 2, ((h - box_h) / 2).max(0), box_w, box_h)
}

/// Screen rects of the dialog's `(Cancel, Confirm)` buttons.
fn dialog_buttons(w: i32, h: i32) -> (Rect, Rect) {
    let d = dialog_rect(w, h);
    let by = d.y + d.h - 2;
    let cancel = Rect::new(d.x + 2, by, 10, 1);
    let confirm_w = 14;
    let confirm = Rect::new(d.x + d.w - 2 - confirm_w, by, confirm_w, 1);
    (cancel, confirm)
}

impl PowerMenu {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the menu is showing anything (dropdown or confirm dialog). When
    /// true the menu is modal and the session should route clicks to it.
    pub fn is_open(&self) -> bool {
        self.open || self.confirm.is_some()
    }

    /// Toggle the dropdown (the menubar button). If anything is open, close it.
    pub fn toggle(&mut self) {
        if self.is_open() {
            self.close();
        } else {
            self.open = true;
        }
    }

    /// Close everything.
    pub fn close(&mut self) {
        self.open = false;
        self.confirm = None;
    }

    /// Route a click while the menu is open. Returns the action to fire, if any.
    pub fn on_click(&mut self, p: Point, w: i32, h: i32) -> PowerClick {
        // The confirm dialog is modal on top of the dropdown — handle it first.
        if let Some(action) = self.confirm {
            let (cancel, confirm) = dialog_buttons(w, h);
            if confirm.contains(p) {
                self.close();
                return match action {
                    PowerAction::Exit => PowerClick::Act(PowerOutcome::Detach),
                    PowerAction::Restart => PowerClick::Act(PowerOutcome::Reload),
                    PowerAction::Shutdown => PowerClick::Act(PowerOutcome::Shutdown),
                };
            }
            if cancel.contains(p) {
                // Back to the dropdown so the user can pick again.
                self.confirm = None;
                self.open = true;
                return PowerClick::Consumed;
            }
            // A click outside the modal dismisses everything; inside is ignored.
            if !dialog_rect(w, h).contains(p) {
                self.close();
            }
            return PowerClick::Consumed;
        }

        if self.open {
            for (i, action) in ITEMS.iter().enumerate() {
                if item_rect(w, i).contains(p) {
                    self.confirm = Some(*action);
                    self.open = false;
                    return PowerClick::Consumed;
                }
            }
            if !dropdown_rect(w).contains(p) {
                self.close();
            }
            return PowerClick::Consumed;
        }

        PowerClick::Consumed
    }

    /// Compositor layers for the dropdown and/or confirm dialog (empty when
    /// closed). The dialog sits above the dropdown and all other chrome.
    pub fn render(&self, w: i32, h: i32) -> Vec<Layer> {
        let t = crate::theme::current();
        let mut layers = Vec::new();

        if self.open {
            let d = dropdown_rect(w);
            let mut buf = CellBuffer::new(d.w, d.h);
            buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
            let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
            for x in 0..d.w {
                buf.set(x, 0, b('─'));
                buf.set(x, d.h - 1, b('─'));
            }
            for y in 0..d.h {
                buf.set(0, y, b('│'));
                buf.set(d.w - 1, y, b('│'));
            }
            buf.set(0, 0, b('╭'));
            buf.set(d.w - 1, 0, b('╮'));
            buf.set(0, d.h - 1, b('╰'));
            buf.set(d.w - 1, d.h - 1, b('╯'));
            for (i, action) in ITEMS.iter().enumerate() {
                let y = 1 + i as i32;
                let label = format!(" {}", action.label());
                buf.write_str(1, y, &label, t.text, t.window_bg);
            }
            layers.push(Layer { z: 5200, origin: Point::new(d.x, d.y), buf, opacity: 1.0, scissor: None });
        }

        if let Some(action) = self.confirm {
            let d = dialog_rect(w, h);
            let mut buf = CellBuffer::new(d.w, d.h);
            buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
            // Title bar.
            for x in 0..d.w {
                buf.set(x, 0, Cell { ch: ' ', fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
            }
            buf.write_str(2, 0, " tuiui ", t.title_fg, t.title_focus);
            // Border.
            let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
            for y in 1..d.h {
                buf.set(0, y, b('│'));
                buf.set(d.w - 1, y, b('│'));
            }
            for x in 0..d.w {
                buf.set(x, d.h - 1, b('─'));
            }
            buf.set(0, d.h - 1, b('╰'));
            buf.set(d.w - 1, d.h - 1, b('╯'));
            // Message + confirm-button label.
            let (msg, confirm_label) = match action {
                PowerAction::Exit => ("Exit tuiui? Apps keep running in the background.", "Exit"),
                PowerAction::Restart => ("Restart tuiui? The UI reloads; your apps keep running.", "Restart"),
                PowerAction::Shutdown => ("Shut down tuiui? All running apps will close.", "Shut Down"),
            };
            buf.write_str(2, 2, msg, t.text, t.window_bg);
            // Buttons (local coords; pad the labels to fill their hit rects).
            let (cancel, confirm) = dialog_buttons(w, h);
            let cancel_s = format!("{:^width$}", "Cancel", width = cancel.w as usize);
            buf.write_str(cancel.x - d.x, cancel.y - d.y, &cancel_s, t.text, t.active_bg);
            let confirm_s = format!("{:^width$}", confirm_label, width = confirm.w as usize);
            buf.write_str(confirm.x - d.x, confirm.y - d.y, &confirm_s, t.close_fg, t.accent);
            layers.push(Layer { z: 6000, origin: Point::new(d.x, d.y), buf, opacity: 1.0, scissor: None });
        }

        layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(r: Rect) -> Point {
        Point::new(r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn toggle_opens_and_closes() {
        let mut m = PowerMenu::new();
        assert!(!m.is_open());
        m.toggle();
        assert!(m.is_open());
        m.toggle();
        assert!(!m.is_open());
    }

    #[test]
    fn exit_then_confirm_detaches() {
        let (w, h) = (120, 40);
        let mut m = PowerMenu::new();
        m.toggle();
        assert_eq!(m.on_click(center(item_rect(w, 0)), w, h), PowerClick::Consumed);
        assert!(m.confirm.is_some());
        let (_, confirm) = dialog_buttons(w, h);
        assert_eq!(m.on_click(center(confirm), w, h), PowerClick::Act(PowerOutcome::Detach));
        assert!(!m.is_open());
    }

    #[test]
    fn shutdown_then_confirm_shuts_down() {
        let (w, h) = (120, 40);
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 2)), w, h); // Shutdown is item 2
        let (_, confirm) = dialog_buttons(w, h);
        assert_eq!(m.on_click(center(confirm), w, h), PowerClick::Act(PowerOutcome::Shutdown));
    }

    #[test]
    fn restart_then_confirm_reloads() {
        let (w, h) = (120, 40);
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 1)), w, h); // Restart is item 1
        assert!(m.confirm.is_some(), "Restart now opens a confirm dialog");
        let (_, confirm) = dialog_buttons(w, h);
        assert_eq!(m.on_click(center(confirm), w, h), PowerClick::Act(PowerOutcome::Reload));
    }

    #[test]
    fn cancel_returns_to_dropdown() {
        let (w, h) = (120, 40);
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 0)), w, h); // Exit -> confirm
        let (cancel, _) = dialog_buttons(w, h);
        m.on_click(center(cancel), w, h);
        assert!(m.confirm.is_none());
        assert!(m.open);
    }

    #[test]
    fn click_outside_dropdown_closes() {
        let (w, h) = (120, 40);
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(Point::new(1, 20), w, h); // far from the top-right dropdown
        assert!(!m.is_open());
    }

    #[test]
    fn click_outside_dialog_dismisses_everything() {
        let (w, h) = (120, 40);
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 0)), w, h); // open Exit confirm
        m.on_click(Point::new(1, 1), w, h); // corner, outside the centered dialog
        assert!(!m.is_open());
    }
}
