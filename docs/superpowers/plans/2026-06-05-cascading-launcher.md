# Cascading Launcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the launcher's flat-grid `Menu` dropdown with a Windows-95-style cascading flyout — a vertical category menu whose entries open submenus of apps to the right — navigable by hover, click, and keyboard (`↑/↓`, `→` descend, `←` collapse, `Enter`, `Esc`). Spotlight search is unchanged.

**Architecture:** `Launcher` gains a `MenuEntry` tree (built by grouping its `AppEntry` list by category) and an open `path: Vec<usize>`. A shared `panel_geometry()` computes one offset panel rect per open level; `render_menu`, `hover`, and `click` all use it so geometry stays consistent. The session routes Menu-mode mouse-moves → `hover` and clicks → `click`; two new keys (`←`/`→`) descend/collapse. The client enables all-motion mouse tracking (`?1003h`) so hover works.

**Tech Stack:** Rust; existing `launcher`/`session`/`protocol`/`client`, `buffer`/`cell`/`geometry`. No new crates.

**Reference spec:** `docs/superpowers/specs/2026-06-05-cascading-launcher-design.md`.

---

## Current surface (verified)

- `src/launcher.rs`: `pub enum LauncherMode { Menu, Spotlight }`. `pub struct Launcher { items: Vec<AppEntry>, open: Option<LauncherMode>, query: String, selected: usize }`. `pub struct Rendered { pub layers: Vec<Layer>, pub items: Vec<(AppEntry, Rect)> }`. Methods: `new`, `is_open`, `set_items`, `mode`, `toggle_menu`, `toggle_spotlight`, `close`, `reset_selection`, `type_char`, `backspace`, `move_up`, `move_down`, `filtered`, `selected_entry`, `render`, `rows`, `render_menu`, `blocks`, `render_spotlight`. Free fns: `cat_of(a) -> String`, `draw_header`, `fill_box`, `draw_row`. Color consts `MENU_BG/MENU_FG/SEL_BG/SEL_FG/BORDER/HINT/ACCENT`. `render_menu` uses `blocks()`/`Block`; `render_spotlight` uses `filtered()`+`selected`. Inline tests assert the flat grid (`menu_grid_renders_every_app`, `wide_menu_uses_multiple_columns`, `narrow_menu_collapses_to_one_column`, `closed_launcher_renders_nothing`).
- `src/geometry.rs`: `Point::new(x,y)`, `Rect::new(x,y,w,h)`, `Rect::contains(Point)->bool`, `Rect::right()/bottom()`.
- `src/buffer.rs`/`src/cell.rs`: `CellBuffer::{new,width,height,set,write_str,fill}`, `Cell { ch, fg, bg, attrs }`, `Rgba`.
- `src/compositor.rs`: `Layer { z, origin: Point, buf: CellBuffer, opacity: f32, scissor: Option<Rect> }`. The launcher renders at `z: 5000`.
- `src/session.rs`: `handle_mouse(&mut self, kind: MouseKind, p: Point)` has a block `if kind == MouseKind::Down && self.launcher.is_open() { … iterate rendered.items, launch on hit, else close }`. `apply` has `ClientMsg::Launcher{Up,Down,Enter,Esc,Char,Backspace}` arms and `ToggleMenu`/`ToggleSpotlight`. `launch_entry(AppEntry)` dispatches `@store`/`@settings`/`@files`/`@image`/normal. `MouseKind::{Down,Drag,Up}`.
- `src/client.rs`: key routing has `} else if f.launcher_open { match k.code { Esc=>LauncherEsc, Enter=>LauncherEnter, Up=>LauncherUp, Down=>LauncherDown, Backspace=>LauncherBackspace, Char(c)=>LauncherChar(c), _=>{} } }`. Mouse arm sends `MouseEventKind::Moved => MouseDrag(p)`.
- `src/terminal.rs`: setup uses `execute!(out, terminal::EnterAlternateScreen, EnableMouseCapture, cursor::Hide)?;` (line ~136) and teardown `DisableMouseCapture` (line ~167). crossterm's `EnableMouseCapture` does NOT enable all-motion (`?1003h`).

## Conventions

- `export PATH="$HOME/.cargo/bin:$PATH"` before cargo. Build before commit. Per-task: build clean + task tests pass. Final task: full gate (`build && test && clippy --all-targets`), 0 warnings.
- Commit per task with the exact message. No AI attribution. Branch `main`.

---

### Task 1: `MenuEntry` tree + `path` + navigation (no render yet)

**Files:** `src/launcher.rs`; inline `#[cfg(test)] mod tests`.

- [ ] **Step 1: Write the failing inline test** (append to the existing `mod tests`):

```rust
    #[test]
    fn cascade_root_groups_by_category_and_navigates() {
        let mut l = Launcher::new(vec![
            app("Aaa", "Games"), app("Bbb", "Games"), app("Ccc", "Tools"),
        ]);
        l.toggle_menu();
        // root has 2 category submenus (Games, Tools), sorted
        assert_eq!(l.menu_labels(), vec!["Games", "Tools"]);
        assert_eq!(l.path_for_test(), vec![0]); // first root row selected
        // descend into Games → its apps
        l.expand();
        assert_eq!(l.path_for_test(), vec![0, 0]);
        assert_eq!(l.focused_label(), Some("Aaa".to_string()));
        l.move_down();
        assert_eq!(l.focused_label(), Some("Bbb".to_string()));
        // activate a leaf returns the app
        assert_eq!(l.activate().map(|a| a.name), Some("Bbb".to_string()));
        // collapse back to root
        l.toggle_menu(); l.toggle_menu(); // reopen fresh
        l.expand(); l.collapse();
        assert_eq!(l.path_for_test(), vec![0]);
    }

    #[test]
    fn activate_on_category_descends_not_launches() {
        let mut l = Launcher::new(vec![app("Aaa", "Games")]);
        l.toggle_menu();
        assert!(l.activate().is_none()); // category → descend, no launch
        assert_eq!(l.path_for_test(), vec![0, 0]);
        assert_eq!(l.activate().map(|a| a.name), Some("Aaa".to_string())); // now the leaf
    }
```

> `app(name, cat)` helper already exists in the test module. Add the `#[doc(hidden)] pub fn path_for_test(&self) -> Vec<usize>`, `menu_labels`, `focused_label` accessors below (test-only-ish but plain pub is fine).

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --lib launcher::tests::cascade`).

- [ ] **Step 3: Implement.**

Add the tree type (top of `launcher.rs`):

```rust
/// A node in the cascading menu.
#[derive(Clone, Debug)]
enum MenuEntry {
    Launch(AppEntry),
    Submenu { label: String, items: Vec<MenuEntry> },
}

impl MenuEntry {
    fn label(&self) -> &str {
        match self {
            MenuEntry::Launch(a) => &a.name,
            MenuEntry::Submenu { label, .. } => label,
        }
    }
    fn is_submenu(&self) -> bool { matches!(self, MenuEntry::Submenu { .. }) }
}
```

Add fields to `Launcher`:

```rust
    /// Cascade root (Menu mode), rebuilt on open.
    menu_root: Vec<MenuEntry>,
    /// Open chain: selected row at each open level. Non-empty while Menu is open.
    path: Vec<usize>,
```

Initialize in `new`: `menu_root: Vec::new(), path: vec![0],`.

Rebuild on menu open — in `toggle_menu`, when opening, call `self.rebuild_menu();` and set `self.path = vec![0];`:

```rust
    pub fn toggle_menu(&mut self) {
        let opening = self.open != Some(LauncherMode::Menu);
        self.open = if opening { Some(LauncherMode::Menu) } else { None };
        self.reset_selection();
        if opening {
            self.rebuild_menu();
            self.path = vec![0];
        }
    }

    /// Build the cascade root: one Submenu per category (sorted, "tuiui" first),
    /// apps inside (sorted by name).
    fn rebuild_menu(&mut self) {
        use std::collections::BTreeMap;
        let mut by_cat: BTreeMap<String, Vec<AppEntry>> = BTreeMap::new();
        for a in &self.items {
            by_cat.entry(cat_of(a)).or_default().push(a.clone());
        }
        let rank = |c: &str| if c == "tuiui" { 0 } else { 1 };
        let mut cats: Vec<(String, Vec<AppEntry>)> = by_cat.into_iter().collect();
        cats.sort_by(|(a, _), (b, _)| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
        self.menu_root = cats
            .into_iter()
            .map(|(label, mut apps)| {
                apps.sort_by(|x, y| x.name.to_lowercase().cmp(&y.name.to_lowercase()));
                MenuEntry::Submenu { label, items: apps.into_iter().map(MenuEntry::Launch).collect() }
            })
            .collect();
    }
```

Add the level walker + navigation:

```rust
    /// The visible panels: for each open level, (entries, selected_row). Includes a
    /// final auto-expanded panel when the deepest selected entry is a Submenu.
    fn levels(&self) -> Vec<(&[MenuEntry], usize)> {
        let mut out: Vec<(&[MenuEntry], usize)> = Vec::new();
        let mut entries: &[MenuEntry] = &self.menu_root;
        for (k, &sel) in self.path.iter().enumerate() {
            let sel = sel.min(entries.len().saturating_sub(1));
            out.push((entries, sel));
            match entries.get(sel) {
                Some(MenuEntry::Submenu { items, .. }) if k + 1 < self.path.len() => entries = items,
                Some(MenuEntry::Submenu { items, .. }) => {
                    // deepest selected is a submenu → auto-expand one panel (row 0)
                    out.push((items.as_slice(), 0));
                    break;
                }
                _ => break,
            }
        }
        out
    }

    fn focused_entry(&self) -> Option<&MenuEntry> {
        let mut entries: &[MenuEntry] = &self.menu_root;
        let mut last = None;
        for &sel in &self.path {
            let sel = sel.min(entries.len().saturating_sub(1));
            last = entries.get(sel);
            match entries.get(sel) {
                Some(MenuEntry::Submenu { items, .. }) => entries = items,
                _ => break,
            }
        }
        last
    }

    fn focused_len(&self) -> usize {
        // length of the list the focused index points into
        let mut entries: &[MenuEntry] = &self.menu_root;
        for (k, &sel) in self.path.iter().enumerate() {
            if k + 1 == self.path.len() { return entries.len(); }
            match entries.get(sel.min(entries.len().saturating_sub(1))) {
                Some(MenuEntry::Submenu { items, .. }) => entries = items,
                _ => return entries.len(),
            }
        }
        entries.len()
    }

    pub fn expand(&mut self) {
        if let Some(MenuEntry::Submenu { items, .. }) = self.focused_entry() {
            if !items.is_empty() { self.path.push(0); }
        }
    }
    pub fn collapse(&mut self) {
        if self.path.len() > 1 { self.path.pop(); }
    }
    /// Activate the focused entry: descend into a submenu (return None) or launch a
    /// leaf (return the app).
    pub fn activate(&mut self) -> Option<AppEntry> {
        match self.focused_entry().cloned() {
            Some(MenuEntry::Submenu { .. }) => { self.expand(); None }
            Some(MenuEntry::Launch(a)) => Some(a),
            None => None,
        }
    }

    // test/inspection helpers
    #[doc(hidden)]
    pub fn path_for_test(&self) -> Vec<usize> { self.path.clone() }
    #[doc(hidden)]
    pub fn menu_labels(&self) -> Vec<String> { self.menu_root.iter().map(|e| e.label().to_string()).collect() }
    #[doc(hidden)]
    pub fn focused_label(&self) -> Option<String> { self.focused_entry().map(|e| e.label().to_string()) }
```

**Dispatch `move_up`/`move_down` by mode** — change them to operate on `path` in Menu mode, `selected` in Spotlight:

```rust
    pub fn move_up(&mut self) {
        if self.open == Some(LauncherMode::Menu) {
            if let Some(last) = self.path.last_mut() { *last = last.saturating_sub(1); }
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }
    pub fn move_down(&mut self) {
        if self.open == Some(LauncherMode::Menu) {
            let n = self.focused_len();
            if let Some(last) = self.path.last_mut() {
                if n > 0 && *last + 1 < n { *last += 1; }
            }
        } else {
            let n = self.filtered().len();
            if n > 0 && self.selected + 1 < n { self.selected += 1; }
        }
    }
```

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit:**

```bash
git add src/launcher.rs
git commit -m "launcher: cascade MenuEntry tree + path navigation (expand/collapse/activate)"
```

---

### Task 2: Cascade render + hit-test (`panel_geometry`, `hover`, `click`, `point_in_menu`)

**Files:** `src/launcher.rs`; replace the flat-grid inline tests with cascade tests.

- [ ] **Step 1: Replace the flat-grid tests.** Delete `menu_grid_renders_every_app`, `wide_menu_uses_multiple_columns`, `narrow_menu_collapses_to_one_column` (they assert the removed grid). Keep `closed_launcher_renders_nothing`. Add:

```rust
    #[test]
    fn cascade_renders_root_then_submenu_on_hover() {
        let mut l = Launcher::new(vec![app("Aaa", "Games"), app("Bbb", "Tools")]);
        l.toggle_menu();
        let r = l.render(120, 40);
        assert!(!r.layers.is_empty());
        // root panel + auto-expanded submenu of the selected category = 2 panels
        assert_eq!(l.panel_count_for_test(120, 40), 2);
        // hovering the "Tools" root row (second row) selects it
        let rects = l.panel_rects_for_test(120, 40);
        let tools_row = rects.iter().find(|(lvl, row, _)| *lvl == 0 && *row == 1).map(|(_, _, r)| *r).unwrap();
        l.hover(Point::new(tools_row.x + 1, tools_row.y));
        assert_eq!(l.focused_label(), Some("Tools".to_string()));
    }

    #[test]
    fn click_launches_leaf_and_descends_submenu() {
        let mut l = Launcher::new(vec![app("Aaa", "Games")]);
        l.toggle_menu();
        let rects = l.panel_rects_for_test(120, 40);
        // level 1 row 0 is the leaf "Aaa" (auto-expanded under the only category)
        let leaf = rects.iter().find(|(lvl, row, _)| *lvl == 1 && *row == 0).map(|(_, _, r)| *r).unwrap();
        let got = l.click(Point::new(leaf.x + 1, leaf.y));
        assert_eq!(got.map(|a| a.name), Some("Aaa".to_string()));
    }
```

> Add `#[doc(hidden)] pub fn panel_count_for_test(&self, w, h) -> usize` and `panel_rects_for_test(&self, w, h) -> Vec<(usize, usize, Rect)>` that expose `panel_geometry`'s output.

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --lib launcher`).

- [ ] **Step 3: Implement.**

Add a geometry helper shared by render + hit-test:

```rust
    /// One rendered panel: its box rect and the per-row rects + the entry list.
    /// Returns `(level, panel_rect, row_rects)`.
    fn panel_geometry(&self, w: i32, h: i32) -> Vec<(usize, Rect, Vec<Rect>)> {
        let levels = self.levels();
        let mut out = Vec::new();
        let mut x = 0;
        let mut prev_sel_y = 1; // panel 0 top
        for (k, (entries, sel)) in levels.iter().enumerate() {
            let label_w = entries.iter().map(|e| e.label().chars().count()).max().unwrap_or(6) as i32;
            let pw = (label_w + 4).clamp(12, 30); // +marker/padding/border
            let ph = entries.len() as i32 + 2; // border top/bottom
            let px = if k == 0 { 0 } else { x };
            // clamp horizontally on screen
            let px = px.min((w - pw).max(0));
            let py = if k == 0 { 1 } else { prev_sel_y };
            let py = py.min((h - ph).max(1)).max(1);
            let mut rows = Vec::new();
            for i in 0..entries.len() {
                rows.push(Rect::new(px + 1, py + 1 + i as i32, pw - 2, 1));
            }
            out.push((k, Rect::new(px, py, pw, ph), rows));
            // next panel starts to the right; anchored at this panel's selected row
            x = px + pw;
            prev_sel_y = py + 1 + (*sel as i32);
        }
        out
    }
```

Rewrite `render_menu` to draw those panels (replace the whole flat-grid body, keep the `(no apps)` early-return when `menu_root` is empty):

```rust
    fn render_menu(&self, w: i32, h: i32) -> Rendered {
        if self.menu_root.is_empty() {
            let (bw, bh) = (18, 3);
            let mut buf = CellBuffer::new(bw, bh);
            fill_box(&mut buf, bw, bh);
            draw_row(&mut buf, 1, bw - 2, 1, "(no apps)", false, false);
            return Rendered { layers: vec![Layer { z: 5000, origin: Point::new(0, 1), buf, opacity: 1.0, scissor: None }], items: Vec::new() };
        }
        let levels = self.levels();
        let geom = self.panel_geometry(w, h);
        let mut layers = Vec::new();
        let mut items: Vec<(AppEntry, Rect)> = Vec::new();
        for ((k, panel, rows), (entries, sel)) in geom.iter().zip(levels.iter()) {
            let mut buf = CellBuffer::new(panel.w, panel.h);
            fill_box(&mut buf, panel.w, panel.h);
            for (i, e) in entries.iter().enumerate() {
                let highlighted = i == *sel;
                let marker = e.is_submenu();
                let label = e.label();
                draw_row(&mut buf, 1, panel.w - 2, 1 + i as i32, label, highlighted, marker);
                if let MenuEntry::Launch(a) = e {
                    items.push((a.clone(), rows[i]));
                }
            }
            layers.push(Layer { z: 5000 + *k as i32, origin: Point::new(panel.x, panel.y), buf, opacity: 1.0, scissor: None });
        }
        Rendered { layers, items }
    }
```

> `draw_row`'s `marker: bool` currently draws an install/selection marker — repurpose it to draw a `▸` at the right edge of the row for submenu entries (check its current body; if `marker` means something else, add a small `submenu: bool` param or draw the `▸` directly in `render_menu` by writing into `buf` at `panel.w - 2`). Keep it simple: after `draw_row`, if `marker`, write `'\u{25B8}'` at `(panel.w - 2, 1 + i)`.

Add hover/click/point_in_menu using `panel_geometry`:

```rust
    /// Mouse-move: select the (level,row) under `p`, truncating deeper levels.
    pub fn hover(&mut self, p: Point) {
        if self.open != Some(LauncherMode::Menu) { return; }
        // Use a generous screen; geometry only needs w,h to clamp — recompute with a
        // large canvas matching the last render is ideal, but clamping is monotonic.
        let geom = self.panel_geometry(self.last_w.get(), self.last_h.get());
        for (k, _panel, rows) in &geom {
            for (i, r) in rows.iter().enumerate() {
                if r.contains(p) {
                    let mut np: Vec<usize> = self.path.iter().take(*k).copied().collect();
                    np.push(i);
                    self.path = np;
                    return;
                }
            }
        }
    }

    /// Mouse-click: hover then activate (descend a submenu, or launch a leaf).
    pub fn click(&mut self, p: Point) -> Option<AppEntry> {
        if self.open != Some(LauncherMode::Menu) { return None; }
        self.hover(p);
        self.activate()
    }

    /// Whether `p` is inside any visible panel (so an outside click should close).
    pub fn point_in_menu(&self, p: Point) -> bool {
        self.panel_geometry(self.last_w.get(), self.last_h.get())
            .iter()
            .any(|(_, panel, _)| panel.contains(p))
    }
```

> `hover`/`point_in_menu` need the screen size. Add `last_w: std::cell::Cell<i32>, last_h: std::cell::Cell<i32>` to `Launcher` (init `Cell::new(80)`/`Cell::new(24)`), and set them at the top of `render` (`self.last_w.set(w); self.last_h.set(h);`). This mirrors the file manager's `cols_per_row` pattern.

Add the test accessors:

```rust
    #[doc(hidden)]
    pub fn panel_count_for_test(&self, w: i32, h: i32) -> usize { self.panel_geometry(w, h).len() }
    #[doc(hidden)]
    pub fn panel_rects_for_test(&self, w: i32, h: i32) -> Vec<(usize, usize, Rect)> {
        self.panel_geometry(w, h).into_iter()
            .flat_map(|(k, _p, rows)| rows.into_iter().enumerate().map(move |(i, r)| (k, i, r)))
            .collect()
    }
```

> Before calling the `_for_test` geometry, the tests call `render` first OR pass explicit w,h to `panel_geometry`; the `_for_test` helpers take explicit w,h so they work without a prior render. But `hover` uses `last_w/last_h` — in the hover test, call `l.render(120,40)` first (the test does) so `last_w/last_h` are set. Ensure the cascade hover test renders before hovering (the provided test calls `panel_rects_for_test(120,40)` and then `hover`; add `let _ = l.render(120,40);` before hovering so `last_w/last_h` match).

Remove the now-unused flat-grid helpers (`blocks`, `Block`, the `rows`/`Row` machinery if unused by Spotlight — check `render_spotlight`; if it uses `rows()`, keep `rows`/`Row`, else remove). Delete dead code to keep clippy clean.

- [ ] **Step 4: Run → PASS** (`cargo test --offline --lib launcher`).

- [ ] **Step 5: Commit:**

```bash
git add src/launcher.rs
git commit -m "launcher: cascade render (offset panels) + hover/click hit-testing"
```

---

### Task 3: Protocol + client (`←`/`→` + all-motion mouse tracking)

**Files:** `src/session.rs` (ClientMsg), `src/client.rs`, `src/terminal.rs`; Test: inline/protocol round-trip.

- [ ] **Step 1: Add a failing test** (`tests/protocol_tests.rs`, append):

```rust
#[test]
fn launcher_left_right_roundtrip() {
    use tuiui::session::ClientMsg;
    for msg in [ClientMsg::LauncherLeft, ClientMsg::LauncherRight] {
        let s = serde_json::to_string(&msg).unwrap();
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), s);
    }
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.**

(a) `src/session.rs` — add `LauncherLeft` and `LauncherRight` to `enum ClientMsg`. Add apply arms:

```rust
ClientMsg::LauncherLeft => self.launcher.collapse(),
ClientMsg::LauncherRight => self.launcher.expand(),
```

(b) `src/client.rs` — in the `f.launcher_open` key branch add:

```rust
                            KeyCode::Left => send(&mut out_stream, &ClientMsg::LauncherLeft)?,
                            KeyCode::Right => send(&mut out_stream, &ClientMsg::LauncherRight)?,
```

(c) `src/terminal.rs` — enable all-motion tracking so hover works. After the `execute!(out, terminal::EnterAlternateScreen, EnableMouseCapture, cursor::Hide)?;` line, write the raw escape:

```rust
        use std::io::Write;
        write!(out, "\x1b[?1003h")?; // all-motion mouse tracking (for hover)
        out.flush()?;
```

And in teardown, before/after `DisableMouseCapture`, write `\x1b[?1003l`:

```rust
        let _ = write!(out, "\x1b[?1003l");
```

> Match the exact teardown structure in `terminal.rs` (it uses `execute!(out, …, DisableMouseCapture, …)`); add the `?1003l` write alongside. Ensure `out` is the same writer and is flushed.

- [ ] **Step 4: Run → PASS** (`cargo test --offline --test protocol_tests`). Build the client + terminal.

- [ ] **Step 5: Commit:**

```bash
git add src/session.rs src/client.rs src/terminal.rs tests/protocol_tests.rs
git commit -m "launcher: LauncherLeft/Right keys + all-motion mouse tracking for hover"
```

---

### Task 4: Session wiring — Menu-mode hover/click routing

**Files:** `src/session.rs`; Test `tests/session_tests.rs` (append).

- [ ] **Step 1: Append the failing test:**

```rust
#[test]
fn cascade_keyboard_launches_app_from_submenu() {
    let mut core = SessionCore::new(120, 40, Config::default());
    core.apply(ClientMsg::ToggleMenu);
    assert!(core.launcher_open_for_test());
    let before = core.window_count();
    core.apply(ClientMsg::LauncherRight); // descend into the first category
    core.apply(ClientMsg::LauncherEnter); // launch the first app in it
    assert!(!core.launcher_open_for_test()); // menu closed after launch
    assert_eq!(core.window_count(), before + 1);
    core.shutdown();
}
```

> Add `#[doc(hidden)] pub fn launcher_open_for_test(&self) -> bool { self.launcher.is_open() }`. (The default `Config` launcher has the tuiui pins — Files/Store/Settings — so descending the first category and Entering launches one, e.g. opens the Store/Settings/Files window → window_count +1. If the first category's first entry is an `@`-builtin that doesn't create a counted window, pick a category/depth that does, or assert on `launcher_open` flipping to closed instead of window_count. Prefer: assert the menu closed AND that `take_action`/window opened; if `@settings` opens a window (it does — Settings is a WinContent window), count holds.)

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement.**

(a) `LauncherEnter` apply arm — make it use the cascade `activate()` in Menu mode and launch + close on a leaf (keep Spotlight behavior):

```rust
ClientMsg::LauncherEnter => {
    match self.launcher.mode() {
        Some(crate::launcher::LauncherMode::Menu) => {
            if let Some(app) = self.launcher.activate() {
                self.launcher.close();
                self.launch_entry(app);
            }
        }
        Some(crate::launcher::LauncherMode::Spotlight) => {
            if let Some(app) = self.launcher.selected_entry() {
                self.launcher.close();
                self.launch_entry(app);
            }
        }
        None => {}
    }
}
```

> If `LauncherEnter` already has a body that launches `selected_entry`, replace it with the mode-aware version above. `LauncherUp`/`LauncherDown` already call `launcher.move_up()/move_down()`, which are now mode-aware (Task 1) — no change needed.

(b) `handle_mouse` — replace the Menu-mode click handling and add hover. Find the `if kind == MouseKind::Down && self.launcher.is_open() { … }` block. Change it to handle both Menu and Spotlight, and add a `MouseKind::Drag` (move) hover branch:

```rust
        // Launcher captures clicks/moves while open.
        if self.launcher.is_open() {
            match (self.launcher.mode(), kind) {
                (Some(crate::launcher::LauncherMode::Menu), MouseKind::Drag) => {
                    self.launcher.hover(p);
                    return;
                }
                (Some(crate::launcher::LauncherMode::Menu), MouseKind::Down) => {
                    if let Some(app) = self.launcher.click(p) {
                        self.launcher.close();
                        self.launch_entry(app);
                    } else if !self.launcher.point_in_menu(p) {
                        self.launcher.close();
                    }
                    return;
                }
                (Some(crate::launcher::LauncherMode::Spotlight), MouseKind::Down) => {
                    let rendered = self.launcher.render(self.w, self.h);
                    for (entry, r) in rendered.items {
                        if r.contains(p) { self.launcher.close(); self.launch_entry(entry); return; }
                    }
                    self.launcher.close();
                    return;
                }
                _ => {}
            }
        }
```

> This REPLACES the existing `if kind == MouseKind::Down && self.launcher.is_open() { … }` block. Make sure it sits at the same point in `handle_mouse` (before window routing) and that non-`Down`/`Drag` kinds while open fall through harmlessly (the `_ => {}` then continues to the rest of handle_mouse — but you likely want to `return` for any event while the menu is open to avoid leaking clicks to windows; if so, add a trailing `return;` after the match when `self.launcher.is_open()` and the kind was handled. Keep it simple: for Menu mode, `Up`/other kinds can just `return` too. Ensure a mouse-move while the menu is open does NOT fall through to window-drag logic.)

- [ ] **Step 4: Run → PASS.** Then the FULL gate (`build && test && clippy --all-targets`, 0 warnings).

- [ ] **Step 5: Commit:**

```bash
git add src/session.rs tests/session_tests.rs
git commit -m "launcher: session wiring — Menu-mode hover/click/keyboard cascade"
```

---

### Task 5: Manual verification + docs

- [ ] **Step 1:** Full gate green.
- [ ] **Step 2:** `cargo install --path . --root ~/.local --force`; on the host `tuiui kill ; tuiui`. Click `✦ tuiui` (or leader `a`): a vertical category menu appears; **hovering** a category flies out its apps; clicking an app launches it; **keyboard** `↑/↓`, `→` (into submenu), `←` (back), `Enter` (launch), `Esc` (close) all work; Spotlight (`Ctrl+Space` then `Space`) still works unchanged.
- [ ] **Step 3:** Update `README.md` — note the cascading menu in the launcher feature bullet + a controls line (hover/arrows to cascade, Enter to launch). Commit:

```bash
git add README.md
git commit -m "docs: cascading launcher in README"
```

---

## Notes for the implementer
- Keep **Spotlight** behavior identical; only `Menu` mode changes. `move_up`/`move_down`/`Enter` dispatch on `self.open`.
- `panel_geometry` is the single source of truth for both render and hit-testing — never compute row rects twice.
- `last_w`/`last_h` (`Cell<i32>`) let `hover`/`point_in_menu` know the screen size without threading it through the session; set them in `render`. The session calls `render` every frame, so they stay current.
- Remove dead flat-grid code (`blocks`/`Block`, and `rows`/`Row` if Spotlight doesn't use them) so clippy stays at 0 warnings.
- If all-motion tracking (`?1003h`) proves too chatty over SSH later, gate it on `launcher_open` client-side — but v1 enables it globally (also gives free-cursor tracking). Click + keyboard keep the cascade fully usable regardless.
