# Dock Grouping + Window Rename + App Badges — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Group dock pills by app, give each a colored letter badge (config-overridable per app),
and let windows be renamed (double-click titlebar or Ctrl+Space r) — grouping stays stable across
renames.

**Architecture:** A per-window immutable `app_key` (grouping key) beside the existing editable
`titles` label. A pure `src/badge.rs` resolves a `(letter, color)` badge from the key + config. The
dock renders grouped pills with badges; groups of ≥2 expand to a popup. Rename edits only `titles`.

**Tech Stack:** Rust 2021, `crate::cell::Rgba`, `serde`, existing dock (`src/chrome.rs`), window
model (`src/window.rs`/`wm.rs`), `SessionCore` (`src/session.rs`).

**Reference:** Spec `docs/superpowers/specs/2026-06-08-dock-grouping-rename-design.md`.

---

## Background (verified)

- `SessionCore.titles: Vec<(WindowId, String)>` = dock label per window (also the menubar
  focused-app name + `WinMeta.title` for restore). `dock_items()` maps it to `chrome::DockItem`.
- `chrome::DockItem { id, label, focused }`; `render_dock(width,height,&[DockItem]) -> Layer`;
  `dock_hit_regions(width,height,&[DockItem]) -> Vec<(WindowId,Rect)>`; private `dock_layout`.
- Windows: `Window { id, title, rect, z, minimized, … }`; `content_rect()` excludes the 1-row
  titlebar; the titlebar row is `rect.y`. `Window::control_at(p)`/`control_columns()` for the
  min/max/close buttons. Native opens push `titles` ("Settings"/"Store"/"Files"). `launch_in(name,
  command, args, cwd)` pushes `titles.push((id, name))`. `WinMeta { rect, title, z, minimized }`
  drives reload restore (`restore_windows_from_host`, `sync_app_meta`).
- `Rgba { r,g,b,a }`, `Rgba::rgb(r,g,b)`. `Config` is serde with `#[serde(default)]` fields.
- `MouseDouble(p)` handler at `session.rs:1125`. Client leader keys at `client.rs:99` (e.g.
  `KeyCode::Char('t') => TileAll`). Desktop rename overlay uses `DesktopChar/Backspace/Commit`
  + `Flags.desktop_editing` (client forwards typed chars when set).

---

## Task 1: `src/badge.rs` — app badge (letter + color) + config

**Files:**
- Create: `src/badge.rs`
- Modify: `src/lib.rs` (`pub mod badge;`)
- Modify: `src/config.rs` (`[dock] badges`)

- [ ] **Step 1: Config field**

In `src/config.rs` `struct Config`, add:
```rust
    /// Per-app dock badge colors: keyword (matched case-insensitively as a
    /// substring of the app name/command) → color (named or `#rrggbb`).
    #[serde(default = "default_dock_badges", skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub dock_badges: std::collections::BTreeMap<String, String>,
```
Add the default seeding the user's known apps:
```rust
fn default_dock_badges() -> std::collections::BTreeMap<String, String> {
    [("claude", "orange"), ("kilo", "yellow")]
        .iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}
```
Add `dock_badges: default_dock_badges(),` to any non-serde `Config` construction (e.g. a `Default`
impl or test builder — grep `Config {` to find struct literals and add the field, or rely on
`..Default::default()` if present; if `Config` derives `Default` via serde defaults only, ensure the
manual constructors compile).

- [ ] **Step 2: Badge module**

Create `src/badge.rs`:
```rust
//! Dock app badges: a one-cell colored initial that identifies an app even when
//! its window has been renamed. Color comes from config (per-app override) or a
//! deterministic hash of the app key.

use crate::cell::Rgba;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    pub letter: char,
    pub color: Rgba,
}

/// First alphanumeric char of `key`, uppercased; `'?'` if none.
fn initial(key: &str) -> char {
    key.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_ascii_uppercase()).unwrap_or('?')
}

/// A small set of named colors, plus `#rrggbb`. Returns `None` if unrecognized.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Rgba::rgb(r, g, b));
        }
        return None;
    }
    let c = match s.to_ascii_lowercase().as_str() {
        "red" => (220, 60, 60),
        "orange" => (230, 130, 40),
        "amber" => (220, 160, 40),
        "yellow" => (220, 200, 50),
        "green" => (70, 180, 90),
        "teal" => (40, 180, 170),
        "cyan" => (50, 180, 210),
        "blue" => (70, 130, 230),
        "indigo" => (90, 90, 210),
        "violet" => (150, 100, 220),
        "magenta" => (200, 70, 190),
        "pink" => (230, 110, 160),
        "gray" | "grey" => (130, 130, 140),
        _ => return None,
    };
    Some(Rgba::rgb(c.0, c.1, c.2))
}

/// Deterministic fallback color from the key (stable per app).
fn hashed_color(key: &str) -> Rgba {
    // FNV-1a over the lowercased key → pick a hue from a fixed palette.
    let mut h: u32 = 2166136261;
    for b in key.to_ascii_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    const PALETTE: &[(u8, u8, u8)] = &[
        (220, 60, 60), (230, 130, 40), (220, 200, 50), (70, 180, 90),
        (40, 180, 170), (70, 130, 230), (150, 100, 220), (200, 70, 190),
    ];
    let (r, g, b) = PALETTE[(h as usize) % PALETTE.len()];
    Rgba::rgb(r, g, b)
}

/// Resolve the badge for an app group key, honoring config overrides (matched as
/// a case-insensitive substring of the key).
pub fn badge_for(key: &str, overrides: &BTreeMap<String, String>) -> Badge {
    let lower = key.to_ascii_lowercase();
    let color = overrides
        .iter()
        .find(|(kw, _)| !kw.is_empty() && lower.contains(&kw.to_ascii_lowercase()))
        .and_then(|(_, c)| parse_color(c))
        .unwrap_or_else(|| hashed_color(key));
    Badge { letter: initial(key), color }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ov(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn initial_letter() {
        assert_eq!(badge_for("Claude", &ov(&[])).letter, 'C');
        assert_eq!(badge_for("kilo", &ov(&[])).letter, 'K');
        assert_eq!(badge_for("1pass", &ov(&[])).letter, '1');
        assert_eq!(badge_for("", &ov(&[])).letter, '?');
    }

    #[test]
    fn config_override_by_substring() {
        let o = ov(&[("claude", "orange")]);
        assert_eq!(badge_for("Claude Code", &o).color, parse_color("orange").unwrap());
    }

    #[test]
    fn hash_fallback_is_stable() {
        assert_eq!(badge_for("btop", &ov(&[])).color, badge_for("btop", &ov(&[])).color);
    }

    #[test]
    fn parse_named_and_hex() {
        assert_eq!(parse_color("#ff8000"), Some(Rgba::rgb(255, 128, 0)));
        assert!(parse_color("orange").is_some());
        assert!(parse_color("nope").is_none());
    }
}
```

- [ ] **Step 3: register + test + clippy + commit**
```bash
export PATH="$HOME/.cargo/bin:$PATH"
# add `pub mod badge;` to src/lib.rs
cargo test badge:: 2>&1 | tail -15
cargo build 2>&1 | tail -5
cargo clippy --all-targets 2>&1 | tail -10
git add src/badge.rs src/lib.rs src/config.rs
git commit --no-verify -m "badge: per-app dock badge (letter + config/hash color) + [dock] badges config"
```

---

## Task 2: per-window `app_key` + grouped dock model

**Files:**
- Modify: `src/session.rs`
- Modify: `src/chrome.rs`
- Modify: `tests/chrome_tests.rs`

- [ ] **Step 1: Track `app_key` per window**

Add field `app_keys: HashMap<WindowId, String>` to `SessionCore` (init empty). Set it everywhere a
window is created:
- `launch_in`: `self.app_keys.insert(id, name.clone());` next to `self.titles.push((id, name))`.
- native opens (Settings/Store/Files/Image — grep `self.titles.push((id, "…"`): insert the same
  literal as the key, e.g. `self.app_keys.insert(id, "Settings".into());`.
- `restore_windows_from_host`: `self.app_keys.insert(id, meta.app_key.clone());`.
- on `close`: `self.app_keys.remove(&id);` (find where `titles.retain`/remove happens).

Add `app_key: String` to `WinMeta` (in `session.rs`): `sync_app_meta` writes
`app_key: self.app_keys.get(&w.id).cloned().unwrap_or_default()`; `restore_windows_from_host` reads
it (default to the title if empty). Update `WinMeta`'s struct + both serializers/deserializers.

- [ ] **Step 2: Rework `chrome::DockItem` into grouped pills**

Replace `DockItem` with a richer type:
```rust
/// What a dock pill activates.
#[derive(Clone, Debug)]
pub enum DockKind {
    /// A single window — clicking focuses it.
    Single(WindowId),
    /// A group of windows of the same app — clicking expands a chooser.
    Group(String, Vec<WindowId>), // (app_key, window ids)
}

/// One pill in the dock.
#[derive(Clone, Debug)]
pub struct DockItem {
    pub kind: DockKind,
    /// Text shown after the badge (single → its label; group → app key).
    pub label: String,
    /// Number of windows (>1 ⇒ a group; shown as a count).
    pub count: usize,
    /// Badge letter + color.
    pub badge_letter: char,
    pub badge_color: crate::cell::Rgba,
    /// Whether any window in this pill is focused.
    pub focused: bool,
}
```

- [ ] **Step 3: Build grouped pills in `SessionCore`**

Replace `dock_items()` with grouping. Preserve first-seen order by iterating `self.titles`:
```rust
    fn dock_items(&self) -> Vec<crate::chrome::DockItem> {
        use crate::chrome::{DockItem, DockKind};
        let focused = self.wm.focused();
        let mut order: Vec<String> = Vec::new();      // group keys in first-seen order
        let mut groups: std::collections::HashMap<String, Vec<WindowId>> = std::collections::HashMap::new();
        for (id, _) in &self.titles {
            let key = self.app_keys.get(id).cloned().unwrap_or_else(|| {
                self.titles.iter().find(|(i, _)| i == id).map(|(_, t)| t.clone()).unwrap_or_default()
            });
            if !groups.contains_key(&key) { order.push(key.clone()); }
            groups.entry(key).or_default().push(*id);
        }
        order.into_iter().map(|key| {
            let wins = groups.remove(&key).unwrap_or_default();
            let badge = crate::badge::badge_for(&key, &self.cfg.dock_badges);
            let focused = wins.iter().any(|w| Some(*w) == focused);
            if wins.len() == 1 {
                let id = wins[0];
                let label = self.titles.iter().find(|(i, _)| *i == id).map(|(_, t)| t.clone()).unwrap_or_else(|| key.clone());
                DockItem { kind: DockKind::Single(id), label, count: 1, badge_letter: badge.letter, badge_color: badge.color, focused }
            } else {
                let count = wins.len();
                DockItem { kind: DockKind::Group(key.clone(), wins), label: key, count, badge_letter: badge.letter, badge_color: badge.color, focused }
            }
        }).collect()
    }
```

- [ ] **Step 4: Render badges + counts; per-pill hit regions**

In `src/chrome.rs` rewrite `dock_layout`/`render_dock`/`dock_hit_regions` for the new `DockItem`.
Each pill renders: a badge cell (the letter, `fg` = a readable contrast (`t.title_fg` or white),
`bg` = `badge_color`), a space, the label, and for `count>1` a ` ²`-style suffix (use a superscript
for 2–9, else `·N`). `dock_layout` returns `(usize index, Rect, rendered_pieces)`; keep the
formatting in one place. `render_dock` writes the badge cell with its color then the rest with the
focused/unfocused bg. `dock_hit_regions` returns `Vec<(usize pill_index, Rect)>` (index into the
items slice) so the session can map a click to the pill's `DockKind`. Helper for the superscript:
```rust
fn count_suffix(n: usize) -> String {
    const SUP: [char; 10] = ['⁰','¹','²','³','⁴','⁵','⁶','⁷','⁸','⁹'];
    if n <= 1 { String::new() }
    else if n <= 9 { format!(" {}", SUP[n]) }
    else { format!(" ·{n}") }
}
```
Update `src/session.rs`'s `dock_regions()` (used by handle_mouse) to the new return shape — it now
yields `(pill_index, Rect)`; the click handler reads `items[pill_index].kind`. (Find the dock-click
loop in `handle_mouse` and adapt in Task 3.)

- [ ] **Step 5: chrome tests**

Update `tests/chrome_tests.rs` `dock_*` tests for the new `DockItem` shape (construct with
`kind: DockKind::Single(WindowId(1)), label, count: 1, badge_letter: 'B', badge_color: …, focused`)
and assert the badge letter renders and a group pill shows its count glyph. Keep the bottom-row
assertions.

- [ ] **Step 6: build + suite + clippy + commit**
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -15
cargo test 2>&1 | grep -E "test result|FAILED" | tail -15
cargo clippy --all-targets 2>&1 | tail -10
git add src/session.rs src/chrome.rs tests/chrome_tests.rs
git commit --no-verify -m "dock: per-window app_key + grouped pills with app badges"
```

---

## Task 3: dock group popup + click routing

**Files:**
- Modify: `src/session.rs`
- Modify: `src/chrome.rs`

- [ ] **Step 1: Popup state + render**

Add `dock_popup: Option<String>` (the open group's app_key) to `SessionCore` (init `None`). Add a
`chrome::render_dock_popup(width, height, key, &[(WindowId, label, badge)], focused) -> (Vec<Layer>,
Vec<(WindowId, Rect)>)` (or two fns) that draws a small bordered box ABOVE the dock at the group's x,
one row per window (`badge + label`), and returns each row's screen Rect for hit-testing. Mirror the
power-menu render style (opaque bordered panel, z ≈ 5200).

- [ ] **Step 2: Dock click routing**

In `handle_mouse`, the dock-region loop becomes:
```rust
            let items = self.dock_items();
            for (idx, r) in self.dock_regions() {     // (pill_index, Rect)
                if r.contains(p) {
                    match &items[idx].kind {
                        crate::chrome::DockKind::Single(id) => {
                            let id = *id;
                            self.dock_popup = None;
                            self.wm.unminimize(id);
                            if self.simple { self.sync_app_size(id); }
                        }
                        crate::chrome::DockKind::Group(key, _) => {
                            // toggle the chooser popup for this group
                            self.dock_popup = if self.dock_popup.as_deref() == Some(key.as_str()) { None } else { Some(key.clone()) };
                        }
                    }
                    return;
                }
            }
```
Before the dock loop (and the menubar checks), if `self.dock_popup.is_some()`, hit-test the popup
rows first: clicking a row focuses that window (`wm.unminimize` + `sync_app_size` if simple) and
clears the popup; clicking elsewhere clears the popup. (Mirror the tray-popover modal block.)

- [ ] **Step 3: Render the popup + overlay hygiene**

In `build_frame`, after the other overlays and within the `overlay_start..` span (so its rect joins
`overlay_rects` and suppresses overlapped icons), push the dock popup layers when `dock_popup` is
set and resolves to a current group. Add `self.dock_popup.is_some()` to the `app_mouse_area()`
overlay guard. Stash the popup row rects for the click hit-test (recompute in `handle_mouse` from
`render_dock_popup`, like the tray popover recomputes on click).

- [ ] **Step 4: build + suite + clippy + commit**
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -15
cargo test 2>&1 | grep -E "test result|FAILED" | tail -10
cargo clippy --all-targets 2>&1 | tail -10
git add src/session.rs src/chrome.rs
git commit --no-verify -m "dock: group chooser popup (click a grouped pill to pick a window)"
```

---

## Task 4: window rename (double-click titlebar + leader r)

**Files:**
- Modify: `src/session.rs`, `src/protocol.rs`, `src/client.rs`, `src/wm.rs` (render field)

- [ ] **Step 1: State + messages + flag**

`SessionCore`: `rename: Option<(WindowId, String)>` (win + buffer), init `None`. Add accessor
`pub fn renaming(&self) -> bool { self.rename.is_some() }`. `Flags.renaming: bool` (in protocol.rs,
`#[serde(default)]`). `ClientMsg`: add `RenameFocused`, `RenameChar(char)`, `RenameBackspace`,
`RenameCommit`, `RenameCancel`. In `apply`:
- `RenameFocused` → if a window is focused, `self.rename = Some((id, current_label))`.
- `RenameChar(c)` → push c to buffer (ignore control chars).
- `RenameBackspace` → pop.
- `RenameCommit` → if buffer non-empty, set `titles[win].1 = buf` and `wm.get_mut(win).title = buf`;
  clear `rename`. (Empty buffer = cancel.)
- `RenameCancel` → `self.rename = None`.
Add `RenameChar(_)`/etc. to the top-of-`apply` `matches!` logging-suppression filter as appropriate.

- [ ] **Step 2: Double-click titlebar starts rename**

In the `MouseDouble(p)` handler, BEFORE the desktop/content checks, add: if `p` is on a window's
titlebar (topmost non-minimized window with `rect.y == p.y`, `rect` contains `p.x`, and not on a
control button via `Window::control_at`), start rename of that window and `return`. Add a helper
`topmost_window_titlebar_at(&self, p) -> Option<WindowId>`.

- [ ] **Step 3: Client wiring**

`client.rs`: in the leader branch add `KeyCode::Char('r') => send(RenameFocused)`. Add a top-level
routing branch (like `f.desktop_editing`) `else if f.renaming { match key { Esc→RenameCancel,
Enter→RenameCommit, Backspace→RenameBackspace, Char(c) if !ctrl →RenameChar(c), _→{} } }`. Set
`renaming: core.renaming()` in the daemon `Flags`.

- [ ] **Step 4: Render the rename field**

While `rename` is `Some((win,buf))`, draw the buffer over that window's titlebar (a high-z 1-row
layer at the window's `rect.y`, spanning the title area, showing `buf` + a cursor block). Simplest:
in `build_frame`, after windows render, push a small layer for the rename field. Reuse theme
`title_focus`/`title_fg`. (A text-field look like the desktop rename overlay.)

- [ ] **Step 5: build + suite + clippy + commit**
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -15
cargo test 2>&1 | grep -E "test result|FAILED" | tail -10
cargo clippy --all-targets 2>&1 | tail -10
git add src/session.rs src/protocol.rs src/client.rs src/wm.rs
git commit --no-verify -m "windows: rename via double-click titlebar or Ctrl+Space r (grouping stays by app_key)"
```

---

## Task 5: tests, gate, manual, memory

- [ ] **Step 1: Session tests**

In `tests/session_tests.rs`:
```rust
#[test]
fn two_same_app_windows_group_in_dock() {
    use tuiui::session::{SessionCore, ClientMsg};
    use tuiui::config::Config;
    let mut core = SessionCore::new(120, 40, Config::default());
    let launch = |c: &mut SessionCore| c.apply(ClientMsg::Launch { name: "Claude".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    launch(&mut core);
    assert_eq!(core.dock_pill_count_for_test(), 1); // one window → one pill
    launch(&mut core);
    assert_eq!(core.dock_pill_count_for_test(), 1); // two Claude → still ONE (grouped) pill
    assert_eq!(core.window_count(), 2);
    core.shutdown();
}

#[test]
fn rename_changes_label_not_grouping() {
    use tuiui::session::{SessionCore, ClientMsg};
    use tuiui::config::Config;
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::Launch { name: "Claude".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    core.apply(ClientMsg::RenameFocused);
    for c in "appname".chars() { core.apply(ClientMsg::RenameChar(c)); }
    core.apply(ClientMsg::RenameCommit);
    assert_eq!(core.focused_label_for_test(), "appname");
    // a second Claude still groups with the renamed one (same app_key)
    core.apply(ClientMsg::Launch { name: "Claude".into(), command: "sh".into(), args: vec!["-c".into(), "sleep 5".into()] });
    assert_eq!(core.dock_pill_count_for_test(), 1);
    core.shutdown();
}
```
Add the tiny test accessors to `SessionCore`: `dock_pill_count_for_test(&self) -> usize`
(`self.dock_items().len()`) and `focused_label_for_test(&self) -> String` (focused window's
`titles` label).

- [ ] **Step 2: Full gate**
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -3 && cargo test 2>&1 | grep -cE "test result: ok" && cargo clippy --all-targets 2>&1 | grep -cE "warning:|error:"
```

- [ ] **Step 3: Deploy + manual**
```bash
cargo install --root ~/.local --path . --force
tuiui kill; tuiui
```
Open Claude → dock shows `C Claude`. Double-click its titlebar (or Ctrl+Space r), type `appname`,
Enter → dock shows `C appname`. Open a second Claude → dock collapses to `C Claude ²`; click it →
popup lists `C Claude` and `C appname`; click either to focus. Confirm the badge color = orange for
Claude (config), and add `[dock] badges` entries (e.g. `kilo = "yellow"`) take effect after reload.

- [ ] **Step 4: Update memory + README**

`tuiui-roadmap-state`: dock grouping + window rename + app badges DONE. README "What works today":
add dock grouping + rename + per-app badges; document `[dock.badges]` in the Configuration section.

---

## Self-Review Notes

- **Grouping stable across rename:** rename mutates `titles`/`window.title` only; `app_keys` is the
  group key and never changes. Tests assert this.
- **Order:** dock pills follow first-seen order (iterate `titles`), so windows don't jump around.
- **Overlay hygiene:** the group popup joins `overlay_rects` (suppresses overlapped icon graphics)
  and disables app mouse passthrough (`app_mouse_area` guard), consistent with other menus.
- **Restore:** `WinMeta.app_key` keeps grouping + (later) badges correct after a frontend reload.
- **Badge contrast:** draw the letter with a light fg on the colored bg; if a configured color is
  very light this could be low-contrast — acceptable for v1 (user controls the colors).
