# CLI tools in the launcher — design

**Date:** 2026-07-04 · **Status:** approved

## Problem

The launcher's `$PATH` scan (`catalog::detect_installed`) surfaces every
installed catalog app as a windowed TUI. Some catalog entries are CLI tools,
not persistent TUIs (himalaya, gum, freeze, khard, nb, vdirsyncer, …): bare
invocation prints-and-exits or demands a subcommand, so "launching" them opens
a window that immediately dies or shows an error. The awesome-tuis sync keeps
adding more of this class.

## Behavior

A catalog app flagged as CLI still appears in the app menu and store, with a
`CLI` badge. Launching it opens a normal PTY window running:

```
sh -lc '<bin> --help; exec "${SHELL:-sh}"'
```

The user sees the tool's commands immediately, then lands in their own
interactive shell in that window, tool on `$PATH`, ready to use. Window title
stays the app name. `requires_cwd` still works — the shell starts in the
picked directory.

## Data model

- `assets/catalog.json`: new optional field `"cli": true`. Absent = TUI (the
  default), so the 600+ existing entries don't churn.
- `CatalogApp`: `#[serde(default)] pub cli: bool` (matches the protocol
  convention: new fields default, never new required shapes).
- `AppEntry` (config.rs): equivalent optional flag so users can mark their own
  config-defined apps too.

## Seams

- `catalog.rs`: `cli` field + `is_cli(name_or_bin)` helper (mirrors
  `category_for` / `requires_cwd_for`); `detect_installed()` passes the flag
  through to `AppEntry`.
- `session.rs launch()`: when the entry is CLI, rewrite command/args to the
  `sh -lc` wrapper. No apphost/protocol change.
- `launcher.rs` + `store.rs`: render the `CLI` badge.

## Catalog audit

One-time subagent sweep over all entries. Flag as CLI when the bare binary
prints-and-exits or needs subcommands with no persistent full-screen UI.
Interactive REPLs (pgcli, litecli, mycli) and hybrid TUIs stay unflagged.
The 6-hourly docs-watch routine keeps future additions honest.

## Testing

- Catalog parse test for the flag; `is_cli` lookup test.
- Session test: CLI-flagged launch spawns the shell wrapper (asserts on the
  spawned command/args), TUI launch unchanged.
- Launcher/store badge tests per the existing UI-test patterns (clicks at
  computed rects, no terminal).

## Out of scope

- Detecting binaries not in the catalog (by design: detection is restricted
  to the curated set).
- npx-cached tools: `npx` installs nothing on `$PATH`, so there is nothing to
  detect; the store recipe (`npm install -g …`) is the supported path.

## Release

CHANGELOG under 0.2.11; merging the version bump cuts the release via CI.
