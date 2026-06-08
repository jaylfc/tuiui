# Dock App-Grouping + Window Rename + App Badges — Design

**Status:** Approved (2026-06-08).

## Goal

Make the dock behave like a real taskbar: windows of the same app **group** into one pill;
windows can be **renamed**; every pill carries a **colored letter badge** so an app is
recognizable even after its window is renamed.

Scenario: open Claude → dock shows `Claude`. Rename that window → dock shows `appname`. Open a
second Claude → the two collapse into one `Claude` pill with a count; click it → a popup lists
both windows (`Claude` and `appname`), each clickable; a badge (`C` on orange) sits on every
pill/row so you always know it's Claude.

## Model

Each window already has a display label in `SessionCore.titles: Vec<(WindowId, String)>`. Add a
parallel, immutable **group key** per window:

- `app_keys: HashMap<WindowId, String>` — the grouping key, set once at create time:
  - PTY apps: the launch `name` (e.g. `"Claude"`).
  - Native windows: `"Settings"` / `"Store"` / `"Files"` / `"Image"`.
  - Restored windows (reload): carried in `WinMeta` (add `app_key` field).
- The **display label** stays in `titles` (default = the group key; editable via rename). Renaming
  mutates only `titles`, never `app_keys`, so grouping is stable across renames.

## Badge

A one-cell badge = an uppercase initial on a background color.

- **Letter:** first alphanumeric char of the group key, uppercased (`Claude → C`, `kilo → K`).
- **Color:** resolved each frame (so config edits apply on reload):
  1. config override — `[dock.badges]` map, matched case-insensitively if the group key (or the
     window's command) **contains** the configured keyword;
  2. else a deterministic color hashed from the group key (stable per app, distinct-ish).
- **Config:** new `[dock] badges = { claude = "orange", kilo = "yellow", … }`, seeded with
  `claude`/`kilo` defaults via `#[serde(default)]`. Colors accept named colors (a small table:
  red/orange/amber/yellow/green/teal/cyan/blue/indigo/violet/magenta/pink/gray) or `#rrggbb`.
  Native windows get sensible default colors too (overridable by the same map, e.g. `settings`).

## Dock grouping

A new `dock_groups()` on `SessionCore` builds the pills from `titles` + `app_keys`, preserving
first-appearance order:

- Group windows by `app_key`.
- **1 window** in a group → a **Single** pill: `badge + label` (the window's own/renamed label);
  click focuses/un-minimizes it.
- **≥2 windows** → a **Group** pill: `badge + group_key + count` (e.g. `C Claude ²`); click opens
  a **popup** above the dock listing each window as `badge + its label`; click a row → focus that
  window. Clicking elsewhere closes the popup.

`chrome::DockItem` grows to carry the badge + a kind (`Single(WindowId)` or `Group(Vec<WindowId>)`)
and an optional count; `dock_layout`/`render_dock`/`dock_hit_regions` render the badge cell + label
and return per-pill hit regions. A new `dock_group_popup` render + hit-test (mirroring the power
menu) shows the expanded list; `SessionCore` holds `dock_popup: Option<String>` (the open group's
key) and routes dock clicks: Single → focus; Group → toggle popup; popup row → focus + close.

The dock-popup rect joins the overlay-suppression set (so it never renders behind icon graphics),
and `app_mouse_area()` returns `None` while it's open (consistent with other overlays).

## Window rename

Inline edit of a window's label, two triggers:

- **Double-click the title bar** of a window → start rename (the client already emits
  `MouseDouble`; route a double-click whose point is on a window's titlebar to rename).
- **Leader `Ctrl+Space` then `r`** → rename the focused window.

Editing state on `SessionCore`: `rename: Option<RenameState { win: WindowId, buf: String }>`. While
active, a small text field renders over that window's titlebar (and the dock pill could echo it).
New `ClientMsg::{RenameStart(WindowId)?, RenameChar(char), RenameBackspace, RenameCommit,
RenameCancel}` — actually a single `ClientMsg::RenameFocused` (leader path) + reuse char/backspace/
commit/cancel; double-click path sets the state daemon-side. A `Flags.renaming: bool` tells the
client to forward typed characters to the rename field (mirrors `desktop_editing`). Commit writes
`buf` into `titles` for that window (and `window.title`); empty buf cancels. Esc cancels.

## Out of scope (later)

Drag-to-reorder dock pills; pinning apps to the dock; per-app real icons (the letter badge stands
in); renaming the group key itself; persisting custom labels across a full `tuiui kill` (they
already persist across a frontend **reload** via `WinMeta`).

## Testing

- **Badge:** letter extraction (`Claude→C`, `1pass→1`, empty→`?`); color resolution (config
  override match by substring, hash fallback determinism, named + `#hex` parsing).
- **Grouping:** `dock_groups()` — 1 window → Single with its label; 2 same-key → one Group with
  count 2; mixed apps → separate pills in first-seen order; a renamed window stays in its group.
- **Dock click routing:** Single focuses; Group opens popup; popup row focuses the right window;
  outside click closes.
- **Rename:** leader/double-click starts rename; char/backspace edit the buffer; commit updates
  `titles` (dock + menubar reflect it) without changing `app_keys` (still grouped); cancel/empty
  restores; `Flags.renaming` toggles.
- **Overlay hygiene:** dock popup suppresses overlapped icon images and disables app mouse
  passthrough while open.
