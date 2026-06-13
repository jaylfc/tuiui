# Changelog

All notable changes to tuiui are recorded here. The project uses
[semantic versioning](https://semver.org); while pre-1.0, minor versions may
carry user-visible feature work and the occasional breaking config change.

## [Unreleased]

### Added
- **Dock right-click menu**: right-click any dock pill for Minimise /
  Maximise / Close / **Reset size** (centres the window at half the screen —
  the rescue hatch for a mis-placed window).

### Fixed
- **Updater robustness** (from a code review): an invalid or typo'd
  `update_branch` in `config.toml` now falls back to `main` instead of
  producing a malformed update command or check URL.
- **Control-socket resilience**: the daemon no longer holds the control-message
  lock across `apply` (which could block `tuiui launch/tile/theme/msg` from the
  CLI/assistant), and recovers a poisoned lock so one panic can't wedge the
  control path.
- **Remote file browser**: tighter navigation timeouts (ssh ConnectTimeout 3s,
  directory list/home 5s) so an unreachable saved system is a brief hitch
  rather than a multi-second freeze (a full off-render-loop fix is tracked).
- **Add Remote on Debian**: three failure modes fixed — Linux release
  binaries are now built on Ubuntu 22.04 (glibc 2.35) so they run on
  Debian 12 instead of dying with "GLIBC_2.39 not found"; the installer and
  the remote-setup script accept **wget** when curl is absent (stock Debian);
  and **Linux arm64** prebuilt binaries are now published (Raspberry Pi /
  ARM servers).
- Windows can no longer be stranded off-screen when the terminal shrinks
  (e.g. moving to a smaller monitor): every window — minimized ones too —
  is clamped into the new work area on resize, shrinking any window larger
  than the new screen.

### Changed
- **Settings → Updates simplified**: the update check now runs automatically
  (on a background thread) whenever the Updates screen opens or the channel
  changes — no more pressing Update blind and reinstalling the same version.
  One **Update & Reload** button remains.
- The **`+` (new shell)** button and the **`⊞`/`▦` view toggle** swapped
  places: `+` now sits in the menubar next to the brand; the view toggle
  lives at the dock's bottom-left.
- `dirs` bumped to 6 — the last outdated dependency flagged by the update
  build.

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
- **Update safety net**: updates never kill running apps today (the app server
  is a separate process the reload doesn't touch, and the wire protocol is
  skew-tolerant by construction) — but if a future update ever *must* break
  that protocol, a safety dialog now appears after the reload: it warns that
  restarting the app server will close the user's N apps, and offers
  **Keep apps** (save your work first; restart later from a row in
  Settings → Updates) or **Restart app server**. Armed by bumping
  `MIN_COMPAT` in `apphost/proto.rs`; dormant otherwise.

### Changed
- The in-app updater now installs the **prebuilt latest release** by default
  (a download, not a source compile), falling back to `cargo install`; it
  reopens Settings on the Updates screen after the reload instead of vanishing.
- Add Remote / reconnect copies the local terminal's terminfo to the remote and
  always falls back to `xterm-256color`, fixing "cannot find terminfo entry for
  'xterm-ghostty'" (and the same for Kitty and other modern terminals).
- Dependencies refreshed to current: crossterm 0.29, portable-pty 0.9,
  sysinfo 0.39, toml 1.1, infer 0.19. Minimum supported Rust is now **1.95**
  (declared via `rust-version` in Cargo.toml).

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
