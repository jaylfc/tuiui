# Cascading Launcher — Design

**Status:** Approved design (2026-06-05). A Windows-95-style cascading/flyout app
menu that replaces the launcher's flat multi-column dropdown. Spotlight search is
unchanged.

**Goal:** The `✦ tuiui` menu (and leader `a`) opens a vertical menu of categories;
hovering or arrowing onto a category flies out a submenu of its apps to the right,
exactly like the classic Start → Programs ▸ cascade. Navigable by **both** mouse
(hover to open, click to launch) **and** keyboard (`↑/↓` move, `→` descend, `←`
collapse, `Enter` activate, `Esc` close).

**Architecture:** The existing `Launcher` keeps its two modes (`Menu`, `Spotlight`),
but `Menu` is re-implemented as a cascade. A `MenuEntry` tree (built by grouping the
launcher's `AppEntry` list by category) plus an open `path: Vec<usize>` drive both
rendering (one offset panel per open level) and hit-testing. The session routes
mouse-move → `hover` and click → `click` while the Menu-mode launcher is open, and
two new keys (`←`/`→`) descend/collapse.

**Tech stack:** Rust; existing `launcher`/`session`/`protocol`/`client`,
`buffer`/`cell`/`geometry`. No new crates.

---

## Module layout

- **Modify `src/launcher.rs`** — replace the flat-grid `Menu` rendering with the
  cascade: a `MenuEntry` tree, `path` state, hover/click/expand/collapse/move/activate
  methods, and a nested-panel renderer + hit-test. Remove the now-unused flat-grid
  helpers (`blocks`, `Block`, `rows`, `Row`, `draw_header` if unused) and replace
  their tests.
- **Modify `src/session.rs`** — add `ClientMsg::LauncherLeft`/`LauncherRight`; in
  `handle_mouse`, when the Menu-mode launcher is open, route mouse-move (`MouseKind::
  Drag`) → `launcher.hover(p)` and `MouseKind::Down` → `launcher.click(p)` (which
  returns an optional `AppEntry` to launch). Keep Spotlight handling as-is.
- **Modify `src/client.rs`** — in the `f.launcher_open` key branch, map `Left` →
  `LauncherLeft` and `Right` → `LauncherRight` (only meaningful in Menu mode; harmless
  in Spotlight).

## The model (`launcher.rs`)

```rust
/// A node in the cascading menu: either a launchable app, or a submenu.
enum MenuEntry {
    Launch(AppEntry),
    Submenu { label: String, items: Vec<MenuEntry> },
}
```

`Launcher` gains (used only in `Menu` mode):

```rust
    /// The cascade root, rebuilt from `items` when the menu opens.
    menu_root: Vec<MenuEntry>,
    /// The open chain: selected row index at each open level. `path[0]` selects in
    /// the root; if that entry is a Submenu, level 1 renders, etc.
    path: Vec<usize>,
```

- **Build root:** group `self.items` by `cat_of`, sorted by category then name; one
  `Submenu { label: category, items: apps.map(Launch) }` per category, categories
  sorted. (Single level of nesting — the catalog is flat. The tree type still allows
  deeper nesting for free.) Built in `toggle_menu`/`open` so it reflects the current
  installed apps.
- **`path` invariant:** `path` is non-empty when the menu is open (`[0]` at least).
  The *focused* level is `path.len() - 1`. A level `k` renders for every `k` in
  `0..path.len()`, plus one extra **auto-expanded** level if the focused entry is a
  `Submenu` (so hovering a category immediately shows its apps without a second
  action).

### Navigation methods

```rust
    pub fn hover(&mut self, p: Point);          // mouse-move: select the (level,row) under p
    pub fn click(&mut self, p: Point) -> Option<AppEntry>; // launch a leaf, or descend a submenu
    pub fn move_up(&mut self);                  // ↑ within the focused level (clamp)
    pub fn move_down(&mut self);                // ↓ within the focused level (clamp)
    pub fn expand(&mut self);                   // → descend into the focused submenu (push index 0)
    pub fn collapse(&mut self);                 // ← pop the focused level (min length 1)
    pub fn activate(&mut self) -> Option<AppEntry>; // Enter: descend if submenu, else launch the leaf
```

- `hover(p)`: find the `(level, row)` whose rendered rect contains `p`; set
  `path = path[..level] + [row]`. No-op if `p` is outside every panel.
- `click(p)`: `hover(p)` then `activate()`.
- `move_up/down`: adjust `*path.last_mut()` within `0..len(focused level)`, clamped.
- `expand`: if the focused entry is a `Submenu` with children, push `0`.
- `collapse`: if `path.len() > 1`, `path.pop()`.
- `activate`: focused entry `Submenu` → `expand()` returns `None`; `Launch(app)` →
  return `Some(app)` (the session launches and closes).
- Existing `move_up`/`move_down`/`selected_entry` for Spotlight stay; the cascade
  versions apply in Menu mode (dispatch on `self.open`).

### Resolving levels (helper)

A private `fn levels(&self) -> Vec<(&[MenuEntry], usize)>` returns, for each visible
panel, its entry slice and the selected row. Built by walking `menu_root` down
`path`: level 0 = `(&menu_root, path[0])`; for each `k`, if `entries[path[k]]` is a
`Submenu`, the next level is `(&its items, path.get(k+1).copied().unwrap_or(0))`.
Stop when the selected entry is a `Launch` or `path` is exhausted (with the
auto-expand: if the deepest selected entry is a `Submenu`, append its children as a
final panel with selected row 0). Both `render` and `hit_test` use `levels()` so
geometry is shared.

## Rendering (`launcher.rs::render_menu`)

For each panel `k` from `levels()`:
- **Width** = `max(label width) + padding` (room for the `▸` marker on submenu rows).
- **x-origin** = `2 + Σ widths of panels 0..k` (each panel offset to the right of the
  previous), clamped so the panel stays on screen (`x + width ≤ w`); if it would
  overflow, clamp `x = w - width` (no edge-flip in v1).
- **y-origin** = panel 0 at `y=1` (under the menubar); panel `k>0` anchored at the
  parent's selected row (`parent_y + parent_selected_row`), clamped to keep the panel
  on screen vertically.
- Draw a bordered box (`fill_box`), then a row per entry: highlight the selected row,
  append `▸` to `Submenu` rows, plain text for `Launch` rows.
- Record each row's screen `Rect` so `hit_test` and the session's click handling can
  map a point back to `(level, row)`.

`Rendered` keeps `items: Vec<(AppEntry, Rect)>` for compatibility (leaf rows only, so
existing dock/launch code that scans `items` still works), and the cascade adds an
internal `row_rects: Vec<(usize /*level*/, usize /*row*/, Rect)>` used by `hover`/
`click`. (Store `row_rects` on `Launcher` as render scratch via `Cell`/`RefCell`, or
recompute geometry in `hover`/`click` from `levels()` — prefer recompute to avoid
interior mutability: a private `fn panel_geometry(&self, w, h) -> Vec<PanelRects>`
shared by `render_menu`, `hover`, and `click`.)

## Session wiring (`session.rs`)

- `handle_mouse`, in the `self.launcher.is_open()` block: if `mode == Menu`:
  - `MouseKind::Down` → `if let Some(app) = self.launcher.click(p) { self.launcher.close(); self.launch_entry(app); }  else if !self.launcher.point_in_menu(p) { self.launcher.close(); }` (a click that hits a submenu descends and keeps the menu open; a click outside all panels closes).
  - `MouseKind::Drag` (mouse-move) → `self.launcher.hover(p);` (open submenus on hover; no launch).
  - Spotlight mode keeps the existing click-to-launch behavior.
- `apply`: `ClientMsg::LauncherLeft => self.launcher.collapse()`, `ClientMsg::
  LauncherRight => self.launcher.expand()`. `LauncherUp/Down/Enter/Esc/Char/Backspace`
  already exist; ensure `LauncherEnter` calls `activate()` and launches when it
  returns `Some(app)` (mirror the existing Spotlight Enter path), and `LauncherUp/
  Down` dispatch to the cascade movers in Menu mode.
- `point_in_menu(p)` — a `Launcher` helper: true if `p` is inside any visible panel
  rect (so the session knows an outside-click should close).

## Client wiring (`client.rs`)

In the `f.launcher_open` key branch add:
- `KeyCode::Left => LauncherLeft`, `KeyCode::Right => LauncherRight`.
Keep `Up/Down/Enter/Esc/Char/Backspace`. (In Spotlight, Left/Right are harmless
no-ops server-side.)

**Hover requires all-motion mouse tracking.** crossterm's `EnableMouseCapture`
(used in `terminal.rs`) enables button/drag tracking (`?1002h`) but **not**
all-motion (`?1003h`), so buttonless mouse-moves aren't currently delivered (the
existing `Moved => MouseDrag` arm is effectively dead). To make hover work, the
client emits `\x1b[?1003h` after `EnableMouseCapture` on setup and `\x1b[?1003l` on
teardown. This also makes the cursor track free movement everywhere. Tradeoff: every
mouse-move now sends an event (more bytes over SSH while the mouse is moving); it's
revertible, and the cascade stays fully usable via click + keyboard if all-motion is
ever disabled. (If we later want to scope the cost, gate `?1003h` on `launcher_open`,
but v1 enables it globally for simplicity and the free-cursor win.)

## Testing

`launcher.rs` (pure, deterministic — build a `Launcher` from a fixed `AppEntry`
list):
- root is built as one submenu per category, categories sorted, apps inside;
- `hover`/`hit_test`: a point on a category row selects it and auto-expands its
  submenu (a second panel appears); a point on an app row selects the leaf;
- `expand`/`collapse`/`move_up`/`move_down` adjust `path` correctly and clamp;
- `activate` on a submenu descends (no launch); on a leaf returns the `AppEntry`;
- `click` on a leaf returns the app; on a submenu descends and returns `None`;
- `render` returns a sized buffer; the number of visible panels matches the open
  depth; a category with N apps shows N rows in its submenu;
- closed launcher renders nothing (existing test kept).

`session`/`client` (lighter): `LauncherRight` then `LauncherEnter` launches an app
from a submenu (window count +1); a mouse-move over a category opens its submenu
(observable via the launcher's panel count / `point_in_menu`).

## Build sequence (informs the plan)

1. `MenuEntry` tree + root build + `path` + `levels()` + navigation methods
   (`hover` stubbed until render geometry exists; test `expand/collapse/move/activate`
   against `path` directly).
2. `panel_geometry` + `render_menu` cascade + `hit_test`/`point_in_menu`; wire `hover`/
   `click` to the geometry. Replace the flat-grid tests.
3. Protocol `LauncherLeft`/`LauncherRight` + client `←`/`→` routing.
4. Session wiring: Menu-mode mouse-move→hover, click→descend/launch, Enter/arrows →
   cascade; outside-click closes.
5. Manual verification + README.

## Out of scope (v2)

- Submenu **scrolling** for very long categories (v1 renders the full list; if it
  overflows vertically it clamps the panel origin — acceptable for our category
  sizes).
- **Sub-subcategories** (the catalog is one level deep; the tree type supports
  deeper nesting if we ever add it).
- **Open-direction flipping** near the right/bottom edge (v1 clamps the panel onto
  screen instead of opening leftward/upward).
- Touch/scroll-wheel navigation of the cascade.
