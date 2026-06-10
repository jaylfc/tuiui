//! The menubar power menu: the "host ▾" button at the top-right opens a small
//! dropdown (Exit / Restart / Shutdown / Systems); each power action opens a
//! modal confirmation dialog before it fires, and Systems cascades a submenu of
//! saved machines (local + remotes + "Add Remote…", which opens a form).
//!
//! The widget is pure UI state + geometry — it never touches the session. A
//! click that confirms an action is reported back as a [`PowerOutcome`] for the
//! session to carry out.

use crate::buffer::CellBuffer;
use crate::cell::Cell;
use crate::compositor::Layer;
use crate::geometry::{Point, Rect};
use crate::systems::{RemoteSystem, SwitchSpec};

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

/// The confirmable dropdown items, top to bottom (Systems is appended below).
const ITEMS: [PowerAction; 3] = [PowerAction::Exit, PowerAction::Restart, PowerAction::Shutdown];

/// The "Systems ▸" row index (just below the power actions).
const SYSTEMS_ROW: usize = ITEMS.len();

/// Theme choices offered in the Add Remote form: the remote's own default, then
/// the built-in presets.
fn theme_choices() -> Vec<&'static str> {
    let mut v = vec!["default"];
    v.extend_from_slice(crate::theme::PRESETS);
    v
}

/// What the session should do after a confirmed power-menu click.
#[derive(Debug, Clone, PartialEq)]
pub enum PowerOutcome {
    /// Detach the client, leaving the daemon + apps alive (Exit).
    Detach,
    /// Reload the frontend only — apps stay alive in the apphost.
    Reload,
    /// Full shutdown — daemon exits, apps killed.
    Shutdown,
    /// Switch the client to a saved system over ssh.
    Switch(SwitchSpec),
    /// Save a new system and switch to it with first-time setup (key transfer +
    /// remote install). The password is for setup only and is never persisted.
    AddAndConnect { system: RemoteSystem, password: Option<String> },
    /// Remove a saved system (by name).
    Forget(String),
}

/// Result of routing a click into an open power menu. While the menu is open it
/// is modal, so every click is `Consumed` unless it confirms an action.
#[derive(Debug, Clone, PartialEq)]
pub enum PowerClick {
    /// The click changed menu state or dismissed it; nothing else to do.
    Consumed,
    /// A confirmed action should fire.
    Act(PowerOutcome),
}

/// The Add Remote form state: four fields + Cancel/Connect buttons.
#[derive(Default)]
pub struct AddForm {
    /// Focused field: 0 = Name, 1 = SSH, 2 = Password, 3 = Theme.
    field: usize,
    name: String,
    ssh: String,
    password: String,
    /// Index into [`theme_choices`].
    theme_idx: usize,
}

const FORM_FIELDS: usize = 4;

impl AddForm {
    /// Build the system + setup password from the form, if it is valid (the SSH
    /// target is the only required field; the name defaults to the host).
    fn commit(&self) -> Option<PowerOutcome> {
        let ssh = self.ssh.trim();
        if ssh.is_empty() {
            return None;
        }
        let (host, port) = crate::systems::parse_target(ssh);
        let name = if self.name.trim().is_empty() {
            host.rsplit('@').next().unwrap_or(&host).to_string()
        } else {
            self.name.trim().to_string()
        };
        let theme = (self.theme_idx > 0).then(|| theme_choices()[self.theme_idx].to_string());
        let password = (!self.password.is_empty()).then(|| self.password.clone());
        Some(PowerOutcome::AddAndConnect {
            system: RemoteSystem { name, host, port, theme },
            password,
        })
    }
}

/// Menubar power-button state machine: a dropdown that can raise a Systems
/// submenu, an Add Remote form, or a modal confirmation dialog over itself.
#[derive(Default)]
pub struct PowerMenu {
    open: bool,
    confirm: Option<PowerAction>,
    systems_open: bool,
    form: Option<AddForm>,
}

// ── Geometry (shared by render + hit-test so they can never drift) ──────────────

/// The dropdown panel, anchored just under the top-right "host" button.
fn dropdown_rect(w: i32) -> Rect {
    let box_w = 16;
    let box_h = ITEMS.len() as i32 + 1 + 2; // +1 Systems row, +2 border rows
    // The power button is right-aligned, so its right edge is the screen edge;
    // right-align the panel to it (no need to know the button's label width).
    let x = (w - box_w).max(0);
    Rect::new(x, 1, box_w, box_h)
}

/// Screen rect of dropdown row `i` (the clickable row); row [`SYSTEMS_ROW`] is
/// the Systems entry.
fn item_rect(w: i32, i: usize) -> Rect {
    let d = dropdown_rect(w);
    Rect::new(d.x + 1, d.y + 1 + i as i32, d.w - 2, 1)
}

/// The Systems submenu panel, cascading left from the dropdown's Systems row.
fn systems_rect(w: i32, n_systems: usize) -> Rect {
    let box_w = 30;
    let rows = n_systems as i32 + 2; // Local + systems + Add Remote…
    let box_h = rows + 2; // border rows
    let d = dropdown_rect(w);
    let x = (d.x - box_w + 1).max(0);
    Rect::new(x, item_rect(w, SYSTEMS_ROW).y, box_w, box_h)
}

/// Screen rect of submenu row `i`: 0 = Local, 1..=n = saved systems,
/// n+1 = "Add Remote…".
fn systems_row_rect(w: i32, n_systems: usize, i: usize) -> Rect {
    let s = systems_rect(w, n_systems);
    Rect::new(s.x + 1, s.y + 1 + i as i32, s.w - 2, 1)
}

/// The ✕ (forget) hot-zone at the right end of saved-system row `i` (1-based
/// within the submenu rows).
fn forget_rect(w: i32, n_systems: usize, i: usize) -> Rect {
    let r = systems_row_rect(w, n_systems, i);
    Rect::new(r.x + r.w - 2, r.y, 2, 1)
}

/// The centered Add Remote form box.
fn form_rect(w: i32, h: i32) -> Rect {
    let box_w = 46.min((w - 2).max(24));
    let box_h = 12.min((h - 1).max(8));
    Rect::new((w - box_w) / 2, ((h - box_h) / 2).max(0), box_w, box_h)
}

/// Screen rect of form field `i`'s value box (the clickable/typing area).
fn form_field_rect(w: i32, h: i32, i: usize) -> Rect {
    let f = form_rect(w, h);
    Rect::new(f.x + 12, f.y + 2 + i as i32, f.w - 14, 1)
}

/// Screen rects of the form's `(Cancel, Connect)` buttons.
fn form_buttons(w: i32, h: i32) -> (Rect, Rect) {
    let f = form_rect(w, h);
    let by = f.y + f.h - 2;
    let cancel = Rect::new(f.x + 2, by, 10, 1);
    let connect_w = 13;
    let connect = Rect::new(f.x + f.w - 2 - connect_w, by, connect_w, 1);
    (cancel, connect)
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

    /// Whether the menu is showing anything (dropdown, submenu, form, or confirm
    /// dialog). When true the menu is modal and the session routes clicks to it.
    pub fn is_open(&self) -> bool {
        self.open || self.confirm.is_some() || self.form.is_some()
    }

    /// Whether the Add Remote form is open (the client forwards typed chars).
    pub fn form_open(&self) -> bool {
        self.form.is_some()
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
        self.systems_open = false;
        self.form = None;
    }

    // ── Add Remote form input (keyboard path, driven via ClientMsg) ─────────────

    pub fn form_char(&mut self, c: char) {
        if let Some(f) = self.form.as_mut() {
            match f.field {
                0 => f.name.push(c),
                1 => f.ssh.push(c),
                2 => f.password.push(c),
                _ => {}
            }
        }
    }

    pub fn form_backspace(&mut self) {
        if let Some(f) = self.form.as_mut() {
            match f.field {
                0 => { f.name.pop(); }
                1 => { f.ssh.pop(); }
                2 => { f.password.pop(); }
                _ => {}
            }
        }
    }

    pub fn form_next(&mut self) {
        if let Some(f) = self.form.as_mut() {
            f.field = (f.field + 1) % FORM_FIELDS;
        }
    }

    pub fn form_prev(&mut self) {
        if let Some(f) = self.form.as_mut() {
            f.field = (f.field + FORM_FIELDS - 1) % FORM_FIELDS;
        }
    }

    /// Left/Right cycle the theme when that field is focused.
    pub fn form_left(&mut self) {
        if let Some(f) = self.form.as_mut() {
            if f.field == 3 {
                let n = theme_choices().len();
                f.theme_idx = (f.theme_idx + n - 1) % n;
            }
        }
    }

    pub fn form_right(&mut self) {
        if let Some(f) = self.form.as_mut() {
            if f.field == 3 {
                f.theme_idx = (f.theme_idx + 1) % theme_choices().len();
            }
        }
    }

    /// Enter: advance through the fields, submitting from the last one (or from
    /// the SSH field when it already has a target and the name is set).
    pub fn form_commit(&mut self) -> PowerClick {
        let Some(f) = self.form.as_ref() else { return PowerClick::Consumed };
        if f.field + 1 < FORM_FIELDS {
            self.form_next();
            return PowerClick::Consumed;
        }
        match f.commit() {
            Some(outcome) => {
                self.close();
                PowerClick::Act(outcome)
            }
            None => PowerClick::Consumed,
        }
    }

    pub fn form_cancel(&mut self) {
        if self.form.take().is_some() {
            // Back to the Systems submenu so a typo isn't a full restart.
            self.open = true;
            self.systems_open = true;
        }
    }

    /// Route a click while the menu is open. Returns the action to fire, if any.
    pub fn on_click(&mut self, p: Point, w: i32, h: i32, systems: &[RemoteSystem]) -> PowerClick {
        // The Add Remote form is modal on top of everything — handle it first.
        if let Some(form) = self.form.as_mut() {
            let (cancel, connect) = form_buttons(w, h);
            if connect.contains(p) {
                if let Some(outcome) = form.commit() {
                    self.close();
                    return PowerClick::Act(outcome);
                }
                return PowerClick::Consumed;
            }
            if cancel.contains(p) {
                self.form_cancel();
                return PowerClick::Consumed;
            }
            for i in 0..FORM_FIELDS {
                if form_field_rect(w, h, i).contains(p) {
                    if form.field == 3 && i == 3 {
                        // Clicking the focused theme row cycles forward.
                        form.theme_idx = (form.theme_idx + 1) % theme_choices().len();
                    }
                    form.field = i;
                    return PowerClick::Consumed;
                }
            }
            if !form_rect(w, h).contains(p) {
                self.close();
            }
            return PowerClick::Consumed;
        }

        // The confirm dialog is modal on top of the dropdown.
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

        // The Systems submenu (cascaded next to the dropdown).
        if self.systems_open {
            let n = systems.len();
            // Local row: we're already here — just close the menu.
            if systems_row_rect(w, n, 0).contains(p) {
                self.close();
                return PowerClick::Consumed;
            }
            for (i, sys) in systems.iter().enumerate() {
                let row = i + 1;
                if forget_rect(w, n, row).contains(p) {
                    let name = sys.name.clone();
                    return PowerClick::Act(PowerOutcome::Forget(name));
                }
                if systems_row_rect(w, n, row).contains(p) {
                    self.close();
                    return PowerClick::Act(PowerOutcome::Switch(SwitchSpec::connect(sys)));
                }
            }
            if systems_row_rect(w, n, n + 1).contains(p) {
                // "Add Remote…" opens the form (modal).
                self.open = false;
                self.systems_open = false;
                self.form = Some(AddForm::default());
                return PowerClick::Consumed;
            }
            if systems_rect(w, n).contains(p) {
                return PowerClick::Consumed;
            }
            if item_rect(w, SYSTEMS_ROW).contains(p) {
                self.systems_open = false;
                return PowerClick::Consumed;
            }
            // Fall through: a click on the dropdown itself keeps routing below;
            // anywhere else closes everything.
            if !dropdown_rect(w).contains(p) {
                self.close();
                return PowerClick::Consumed;
            }
            self.systems_open = false;
        }

        if self.open {
            for (i, action) in ITEMS.iter().enumerate() {
                if item_rect(w, i).contains(p) {
                    self.confirm = Some(*action);
                    self.open = false;
                    self.systems_open = false;
                    return PowerClick::Consumed;
                }
            }
            if item_rect(w, SYSTEMS_ROW).contains(p) {
                self.systems_open = !self.systems_open;
                return PowerClick::Consumed;
            }
            if !dropdown_rect(w).contains(p) {
                self.close();
            }
            return PowerClick::Consumed;
        }

        PowerClick::Consumed
    }

    /// Compositor layers for whatever is open (empty when closed). The form and
    /// dialog sit above the dropdown and all other chrome.
    pub fn render(&self, w: i32, h: i32, systems: &[RemoteSystem]) -> Vec<Layer> {
        let t = crate::theme::current();
        let mut layers = Vec::new();

        if self.open {
            let d = dropdown_rect(w);
            let mut buf = CellBuffer::new(d.w, d.h);
            fill_panel(&mut buf, d.w, d.h);
            for (i, action) in ITEMS.iter().enumerate() {
                buf.write_str(1, 1 + i as i32, &format!(" {}", action.label()), t.text, t.window_bg);
            }
            let sys_bg = if self.systems_open { t.active_bg } else { t.window_bg };
            let label = format!("{:<width$}", " Systems", width = (d.w - 2) as usize);
            buf.write_str(1, 1 + SYSTEMS_ROW as i32, &label, t.text, sys_bg);
            buf.write_str(d.w - 3, 1 + SYSTEMS_ROW as i32, "◂", t.accent, sys_bg);
            layers.push(Layer { z: 5200, origin: Point::new(d.x, d.y), buf, opacity: 1.0, scissor: None });
        }

        if self.open && self.systems_open {
            let n = systems.len();
            let s = systems_rect(w, n);
            let mut buf = CellBuffer::new(s.w, s.h);
            fill_panel(&mut buf, s.w, s.h);
            buf.write_str(1, 1, " ● Local (this machine)", t.text, t.window_bg);
            for (i, sys) in systems.iter().enumerate() {
                let y = 2 + i as i32;
                let theme_tag = sys.theme.as_deref().map(|th| format!("  {th}")).unwrap_or_default();
                let label: String = format!(" ⌁ {}{}", sys.name, theme_tag)
                    .chars()
                    .take((s.w - 4) as usize)
                    .collect();
                buf.write_str(1, y, &label, t.text, t.window_bg);
                buf.write_str(s.w - 3, y, "✕", t.close_fg, t.window_bg);
            }
            buf.write_str(1, 2 + n as i32, " + Add Remote…", t.accent, t.window_bg);
            layers.push(Layer { z: 5300, origin: Point::new(s.x, s.y), buf, opacity: 1.0, scissor: None });
        }

        if let Some(form) = &self.form {
            let f = form_rect(w, h);
            let mut buf = CellBuffer::new(f.w, f.h);
            buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
            // Title bar.
            for x in 0..f.w {
                buf.set(x, 0, Cell { ch: ' ', fg: t.title_fg, bg: t.title_focus, attrs: Default::default() });
            }
            buf.write_str(2, 0, " Add Remote System ", t.title_fg, t.title_focus);
            let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
            for y in 1..f.h {
                buf.set(0, y, b('│'));
                buf.set(f.w - 1, y, b('│'));
            }
            for x in 0..f.w {
                buf.set(x, f.h - 1, b('─'));
            }
            buf.set(0, f.h - 1, b('╰'));
            buf.set(f.w - 1, f.h - 1, b('╯'));

            let masked: String = "•".repeat(form.password.chars().count());
            let theme = theme_choices()[form.theme_idx];
            let rows: [(usize, &str, String); 4] = [
                (0, "Name", form.name.clone()),
                (1, "SSH", if form.ssh.is_empty() && form.field != 1 { "user@host[:port]".into() } else { form.ssh.clone() }),
                (2, "Password", masked),
                (3, "Theme", format!("◂ {theme} ▸")),
            ];
            for (i, label, value) in rows {
                let fr = form_field_rect(w, h, i);
                let (ly, lx) = (fr.y - f.y, 2);
                buf.write_str(lx, ly, label, t.dim, t.window_bg);
                let bg = if form.field == i { t.active_bg } else { t.window_bg };
                let fg = if i == 1 && form.ssh.is_empty() && form.field != 1 { t.dim } else { t.text };
                let padded = format!("{:<width$}", format!(" {value}"), width = fr.w as usize);
                let clipped: String = padded.chars().take(fr.w as usize).collect();
                buf.write_str(fr.x - f.x, ly, &clipped, fg, bg);
            }
            buf.write_str(2, f.h - 4, "Password is for the first-time key copy only;", t.dim, t.window_bg);
            buf.write_str(2, f.h - 3, "it is never saved.", t.dim, t.window_bg);

            let (cancel, connect) = form_buttons(w, h);
            let cancel_s = format!("{:^width$}", "Cancel", width = cancel.w as usize);
            buf.write_str(cancel.x - f.x, cancel.y - f.y, &cancel_s, t.text, t.active_bg);
            let connect_s = format!("{:^width$}", "Connect", width = connect.w as usize);
            buf.write_str(connect.x - f.x, connect.y - f.y, &connect_s, t.close_fg, t.accent);
            layers.push(Layer { z: 6000, origin: Point::new(f.x, f.y), buf, opacity: 1.0, scissor: None });
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

/// Fill a buffer with the window background and a rounded border.
fn fill_panel(buf: &mut CellBuffer, w: i32, h: i32) {
    let t = crate::theme::current();
    buf.fill(Cell { ch: ' ', fg: t.text, bg: t.window_bg, attrs: Default::default() });
    let b = |ch: char| Cell { ch, fg: t.border, bg: t.window_bg, attrs: Default::default() };
    for x in 0..w {
        buf.set(x, 0, b('─'));
        buf.set(x, h - 1, b('─'));
    }
    for y in 0..h {
        buf.set(0, y, b('│'));
        buf.set(w - 1, y, b('│'));
    }
    buf.set(0, 0, b('╭'));
    buf.set(w - 1, 0, b('╮'));
    buf.set(0, h - 1, b('╰'));
    buf.set(w - 1, h - 1, b('╯'));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(r: Rect) -> Point {
        Point::new(r.x + r.w / 2, r.y + r.h / 2)
    }

    fn no_systems() -> Vec<RemoteSystem> {
        Vec::new()
    }

    fn one_system() -> Vec<RemoteSystem> {
        vec![RemoteSystem { name: "pi".into(), host: "pi@10.0.0.2".into(), port: None, theme: Some("nord".into()) }]
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
        let s = no_systems();
        let mut m = PowerMenu::new();
        m.toggle();
        assert_eq!(m.on_click(center(item_rect(w, 0)), w, h, &s), PowerClick::Consumed);
        assert!(m.confirm.is_some());
        let (_, confirm) = dialog_buttons(w, h);
        assert_eq!(m.on_click(center(confirm), w, h, &s), PowerClick::Act(PowerOutcome::Detach));
        assert!(!m.is_open());
    }

    #[test]
    fn shutdown_then_confirm_shuts_down() {
        let (w, h) = (120, 40);
        let s = no_systems();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 2)), w, h, &s); // Shutdown is item 2
        let (_, confirm) = dialog_buttons(w, h);
        assert_eq!(m.on_click(center(confirm), w, h, &s), PowerClick::Act(PowerOutcome::Shutdown));
    }

    #[test]
    fn restart_then_confirm_reloads() {
        let (w, h) = (120, 40);
        let s = no_systems();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 1)), w, h, &s); // Restart is item 1
        assert!(m.confirm.is_some(), "Restart now opens a confirm dialog");
        let (_, confirm) = dialog_buttons(w, h);
        assert_eq!(m.on_click(center(confirm), w, h, &s), PowerClick::Act(PowerOutcome::Reload));
    }

    #[test]
    fn cancel_returns_to_dropdown() {
        let (w, h) = (120, 40);
        let s = no_systems();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 0)), w, h, &s); // Exit -> confirm
        let (cancel, _) = dialog_buttons(w, h);
        m.on_click(center(cancel), w, h, &s);
        assert!(m.confirm.is_none());
        assert!(m.open);
    }

    #[test]
    fn click_outside_dropdown_closes() {
        let (w, h) = (120, 40);
        let s = no_systems();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(Point::new(1, 20), w, h, &s); // far from the top-right dropdown
        assert!(!m.is_open());
    }

    #[test]
    fn click_outside_dialog_dismisses_everything() {
        let (w, h) = (120, 40);
        let s = no_systems();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, 0)), w, h, &s); // open Exit confirm
        m.on_click(Point::new(1, 1), w, h, &s); // corner, outside the centered dialog
        assert!(!m.is_open());
    }

    #[test]
    fn systems_row_opens_submenu_and_saved_system_switches() {
        let (w, h) = (120, 40);
        let s = one_system();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, SYSTEMS_ROW)), w, h, &s);
        assert!(m.systems_open, "Systems row cascades the submenu");
        // Row 1 is the saved system (row 0 is Local). Click left of the ✕.
        let row = systems_row_rect(w, 1, 1);
        let click = Point::new(row.x + 1, row.y);
        match m.on_click(click, w, h, &s) {
            PowerClick::Act(PowerOutcome::Switch(spec)) => {
                assert_eq!(spec.host, "pi@10.0.0.2");
                assert_eq!(spec.theme.as_deref(), Some("nord"));
                assert!(!spec.setup, "saved systems switch without re-running setup");
            }
            other => panic!("expected a switch, got {other:?}"),
        }
        assert!(!m.is_open());
    }

    #[test]
    fn forget_x_removes_a_system() {
        let (w, h) = (120, 40);
        let s = one_system();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, SYSTEMS_ROW)), w, h, &s);
        let x = forget_rect(w, 1, 1);
        assert_eq!(
            m.on_click(center(x), w, h, &s),
            PowerClick::Act(PowerOutcome::Forget("pi".into()))
        );
    }

    #[test]
    fn add_remote_form_typed_and_committed() {
        let (w, h) = (120, 40);
        let s = no_systems();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, SYSTEMS_ROW)), w, h, &s);
        // "Add Remote…" is the last submenu row (index n+1 = 1).
        m.on_click(center(systems_row_rect(w, 0, 1)), w, h, &s);
        assert!(m.form_open(), "Add Remote… opens the form");
        // Type an SSH target (field 1) and pick it via keyboard.
        m.form_next(); // Name -> SSH
        for c in "pi@10.0.0.2:2222".chars() {
            m.form_char(c);
        }
        m.form_next(); // -> Password
        for c in "secret".chars() {
            m.form_char(c);
        }
        m.form_next(); // -> Theme
        m.form_right(); // default -> midnight
        match m.form_commit() {
            PowerClick::Act(PowerOutcome::AddAndConnect { system, password }) => {
                assert_eq!(system.host, "pi@10.0.0.2");
                assert_eq!(system.port, Some(2222));
                assert_eq!(system.name, "10.0.0.2", "name defaults to the host");
                assert_eq!(system.theme.as_deref(), Some("midnight"));
                assert_eq!(password.as_deref(), Some("secret"));
            }
            other => panic!("expected AddAndConnect, got {other:?}"),
        }
        assert!(!m.is_open());
    }

    #[test]
    fn form_requires_an_ssh_target() {
        let mut m = PowerMenu::new();
        m.form = Some(AddForm::default());
        for _ in 0..FORM_FIELDS - 1 {
            m.form_next();
        }
        assert_eq!(m.form_commit(), PowerClick::Consumed, "empty target cannot submit");
        assert!(m.form_open());
    }

    #[test]
    fn form_esc_returns_to_systems_submenu() {
        let (w, h) = (120, 40);
        let s = no_systems();
        let mut m = PowerMenu::new();
        m.toggle();
        m.on_click(center(item_rect(w, SYSTEMS_ROW)), w, h, &s);
        m.on_click(center(systems_row_rect(w, 0, 1)), w, h, &s);
        assert!(m.form_open());
        m.form_cancel();
        assert!(!m.form_open());
        assert!(m.systems_open, "cancel goes back to the Systems list");
    }
}
