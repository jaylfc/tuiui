# FM image icons, FM right-click, desktop auto-refresh, app variants + launch warning — design

**Date:** 2026-07-19 · **Status:** approved · **Ships as:** 0.2.13

Five user-reported gaps, one release.

## 1. File-manager image-tile icons

The desktop renders `icons.rs` role-icon PNGs as image tiles for every entry;
the FM's Icon view only loads real image thumbnails and falls back to text
glyphs for everything else. Give the FM Icon view the same treatment:

- Non-image entries get `icons::role_icon_png(role, w, h)` tiles, loaded into
  the shared `ImageStore` (cache per role — every folder shares one image).
- Image files keep their real thumbnails (existing path).
- Same gating as today: Kitty-graphics terminals only; glyph fallback
  unchanged elsewhere. Same overlay-suppression rules (images under an open
  menu are dropped).
- Icon view only; List/Columns unchanged.

## 2. File-manager right-click

`FileManager::begin_context()` (rename/delete/… menu) exists but is
keyboard-only; `ClientMsg::MouseRightDown` routes to dock pills and the
desktop only. Wire it: when the topmost window under the point is a FM window
(and no higher overlay is open — same guard set as the dock branch), localize
the point, `hit_test` it, and on an entry hit: move the cursor/selection to
that entry and open the context menu. Non-entry hits do nothing (v1).

## 3. Desktop auto-refresh

The desktop merges `~/Desktop` + pins but rescans only at startup and after
its own actions — a folder created via the FM or a terminal never appears.
Fix: throttled mtime watch in the daemon tick — at most every ~2s, stat the
desktop dir; if its modified-time changed since the last scan, call
`reload_desktop()`. No FS-watcher dependency.

## 4. Catalog app variants (the Claude ⚠️ entry)

New optional catalog field on an app:

```json
"variants": [{
  "suffix": "⚠️",
  "args": ["--dangerously-skip-permissions"],
  "warn": "Runs Claude Code with --dangerously-skip-permissions: it can edit files and run commands without asking. Launch anyway?"
}]
```

- `CatalogApp` gains `#[serde(default)] pub variants: Vec<Variant>`
  (`Variant { suffix, args, warn: Option<String> }`).
- `detect_installed()` emits one extra `AppEntry` per variant of an installed
  app: name = `"<name> <suffix>"`, same bin/category, the variant's args,
  inherited `requires_cwd`, and `warn` carried through.
- Claude Code is the first (only) catalog user of the field.
- Data-driven: any app can declare variants; no special-casing in code.

## 5. Launch-warning dialog

`AppEntry` gains `warn: Option<String>` (serde default, skip-if-none — user
config entries can set it too). Launching an entry that carries `warn` opens
a modal dialog FIRST (new small module `launchwarn.rs`, mirroring
`confirmclose.rs`: pure state + geometry + render, Cancel/Launch buttons,
Enter|y confirms, Esc|n cancels, mouse clickable). On confirm the launch
proceeds exactly as it would have (cwd picker, CLI wrapper, etc.); on cancel
nothing happens. Client routing via `Flags.launch_warn` + two new
`ClientMsg` variants, following the confirm-close pattern (all
`#[serde(default)]`-safe).

## Testing

Per existing patterns (state-machine tests, no terminal): FM right-click
opens the context overlay on an entry; desktop reload triggers on a changed
mtime (logic-level test); variants parse + `detect_installed` emits the ⚠️
entry with warn; launch of a warn-entry opens the dialog, confirm launches /
cancel doesn't; FM role-icon requests produced for non-image entries in Icon
view. Gate: 0 clippy, all tests, CHANGELOG under 0.2.13.
