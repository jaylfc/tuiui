# Changelog

All notable changes to tuiui are recorded here. The project uses
[semantic versioning](https://semver.org); while pre-1.0, minor versions may
carry user-visible feature work and the occasional breaking config change.

## [0.2.0] — 2026-06-10

The "desktop, networked" release: switch between machines, an AI copilot, and
a lot of polish.

### Added
- **Systems switcher** (power menu → Systems): saved machines with live ●/○
  reachability dots; **Add Remote** transfers an SSH key, installs tuiui + gpm
  + your terminal's terminfo on the remote, syncs your systems list, and
  connects; per-system themes; drop back to the local desktop when the remote
  session ends.
- **AI assistant** (✦ menubar button): a persistent chat panel — or a floating
  window (**Settings → Assistant**) — running any of six agent frameworks
  (Claude Code, opencode, smallcode, Kilo, Hermes, OpenClaw). Instructions live
  in the repo `agent/` pack, stamped into the agent's forced working directory
  in every convention the CLIs read. The agent can drive the desktop
  (`tuiui launch/tile/theme/msg`), read the logs, fix tuiui and open PRs, and
  operate across your saved machines over ssh/scp.
- **Menubar clock + calendar**: date+time, click for a month calendar with
  month navigation and `khal` events.
- **Notifications**: a background app's bell rings a dock attention dot + a 🔔
  tray popover (click to focus); apps' OSC-52 copies are forwarded to the host
  clipboard.
- **Remote files**: browse saved systems over ssh in the file manager; copy
  between machines with Ctrl+C/Ctrl+V (background `scp`, `-3` for remote↔remote).
- **Logs viewer** (launcher → tuiui → Logs): tails `~/tuiui-debug.log`; `c`
  copies it to the host clipboard via OSC 52.
- **Scrollback in app windows**: the mouse wheel scrolls a PTY window's history;
  any keystroke snaps back to the live bottom.
- **Battery** tray segment (Linux sysfs / macOS `pmset`).
- **Control CLI**: `tuiui launch`, `tile`, `theme`, `msg` drive a running
  desktop from any shell (also the assistant's control surface).
- **Updates channel switcher** (Settings → Updates): track `main` (fast
  prebuilt releases) or a `dev` branch (built from source).

### Changed
- The in-app updater now installs the **prebuilt latest release** by default
  (a download, not a source compile), falling back to `cargo install`; it
  reopens Settings on the Updates screen after the reload instead of vanishing.
- Add Remote / reconnect copies the local terminal's terminfo to the remote and
  always falls back to `xterm-256color`, fixing "cannot find terminfo entry for
  'xterm-ghostty'" (and the same for Kitty and other modern terminals).
- Dependencies refreshed to current: crossterm 0.29, portable-pty 0.9,
  sysinfo 0.38, toml 1.1, infer 0.19 (sysinfo held at 0.38: 0.39 needs a newer
  rustc than the project's toolchain).

### Fixed
- Settings' "Check for updates" / "Update & Reload" now respond to **mouse**
  clicks, not only the keyboard.
- The updater window auto-closes after a successful reload.
- Clippy is clean across all targets.

## [0.1.0]

Initial development series: floating windows, persistent daemon/thin client,
app store, settings, theming, menubar tray, grid tiling, native image layer,
file manager, desktop icons, cascading launcher, apphost/frontend split with
live reloads, mouse passthrough, simple view mode, dock grouping, bare-console
(gpm) mouse, apphost service, terminal office suite.
