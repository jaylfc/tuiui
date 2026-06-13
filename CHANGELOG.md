# Changelog

All notable changes to tuiui are recorded here. The project uses
[semantic versioning](https://semver.org); while pre-1.0, minor versions may
carry user-visible feature work and the occasional breaking config change.

## [Unreleased]

## [0.2.8] — 2026-06-13

### Added
- **Switchable assistant agent**: Settings → Assistant now has an **Agent** row
  that flips the ✦ panel between the two supported CLIs — **opencode** (default)
  and **hermes**. The choice is stored in `assistant_command`, so an existing
  config that points at one of them is reflected in the switch (and a
  hand-edited `assistant_command` can still name any binary).

## [0.2.7] — 2026-06-13

### Fixed
- **In-app update could "succeed" without changing the running version**: the
  updater reloads by running a bare `tuiui reload`, but it runs in a
  non-interactive `sh -lc` whose PATH may not include the install dir
  (`~/.local/bin` is added by interactive shell config, not a login `sh`). When
  `tuiui` wasn't found, `install.sh` still wrote the new binary but the daemon
  never restarted onto it — so the version never moved and the update appeared
  to fail every time. The updater now reloads via the **absolute path** of the
  freshly-installed binary (`{install_dir}/tuiui reload`), removing the PATH
  dependency.

### Changed
- **The updater now logs to `~/tuiui-debug.log`**: the in-app update runs in a
  window whose output was otherwise lost. Update checks, the install request,
  and each install step (install.sh / cargo fallback / reload / failure) now
  leave breadcrumbs in the debug log, so a failed update is visible in the log
  users paste.

## [0.2.6] — 2026-06-13

### Added
- **Dock right-click context menu**: right-click a dock pill for Minimise /
  Maximise / Close / Reset size. Grouped pills target the group's focused
  window (else the first); Close routes through the usual confirm dialog;
  Reset re-centres the window at half the work area — the rescue hatch for a
  stranded or mis-sized window. Render and hit-testing share the same geometry
  fns, and the menu y-clamps so it stays on-screen (and clickable) on very
  short terminals. Ported from the `dev` branch (#11, hardened in #20).

### Changed
- **Activity-monitor / apphost follow-ups**: `tuiui ps` and `tuiui kill-app`
  now drain the on-connect roster (and any queued events) before matching the
  `AppList` reply, via a shared `fetch_app_list()` helper; `kill-app all` is
  safe-by-default (only reaps already-dead apps — live apps need an explicit
  id). The apphost protocol gains an optional `pid` field on
  `HostEvt::Spawned` (PROTO_VERSION → 2, additive, `MIN_COMPAT` unchanged) so
  the daemon's `RemoteAppHost` can fill the activity panel's `pid` column, and
  it now stashes each spawn's command + args so the panel shows real data in
  normal daemon mode. Re-landed from the stale `#16` branch onto current
  `main` (keeping main's newer visibility-gated `refresh_activity`).

## [0.2.5] — 2026-06-13

### Fixed
- **Update loop on the `main` channel**: the v0.2.4 release binaries were built
  from the pre-bump commit and reported their version as `0.2.3`, so the in-app
  updater saw a perpetual "update available" (`0.2.3 → 0.2.4`) that installing
  could never settle. Re-released as 0.2.5 (identical code) built from the
  correct commit. The release workflow now fails if the release tag and the
  `Cargo.toml` version disagree, so a version-skewed release can't ship again.

### Changed
- **Releases cut themselves on a version bump**: merging a `Cargo.toml` version
  change to `main` now auto-tags and publishes the release (no tag push
  required), alongside the existing tag-push and manual `workflow_dispatch`
  paths.

## [0.2.4] — 2026-06-13

### Fixed
Robustness fixes from a code review of the networked seams — these had landed
on the `dev` branch but never reached `main`, so they were absent from the
0.2.x releases. Ported to `main`:
- **Log-copy could crash the daemon**: copying a large (>200 KB) log whose
  non-ASCII bytes straddled the tail cut sliced a string mid-UTF-8 and panicked.
  The cut now advances to a char boundary first.
- **Apphost reader resilience**: a single corrupted/blank protocol line used to
  tear down the whole apphost connection (dropping every window's frame stream);
  `recv` now logs-and-skips a bad line while still propagating genuine IO errors.
- **Updater branch validation**: a hand-edited/typo'd `update_branch` flowed
  into a shell command and a URL; it's now sanitized to a git-safe charset and
  falls back to `main`.
- **Control-socket lock resilience**: `apply_ctl` no longer holds the queue lock
  across `core.apply` (which could block the control thread), and both lock
  sites recover a poisoned lock so one panic can't wedge `tuiui launch/tile/
  theme/msg`.
- **Remote file-manager freeze bounded**: tightened SSH `ConnectTimeout` (4→3s)
  and the frequent navigation ops (list/home → 5s) so an unreachable saved
  system is a brief hitch, not a multi-second freeze.

## [0.2.3] — 2026-06-13

### Fixed
- **In-app update could land in a shadowed binary**: when the fast path
  (`install.sh`) failed — e.g. a brief network blip or racing a release's
  asset upload — the updater fell back to `cargo install --git`, which writes to
  cargo's default `~/.cargo/bin`. If that differs from the dir of the running
  binary (e.g. a release install in `~/.local/bin`), the rebuilt binary landed
  in a `$PATH`-shadowed location and the update silently appeared to do nothing.
  The cargo fallback now targets the running binary's own `bin/` dir via
  `--root`, so both update paths replace the binary the user actually runs.

## [0.2.2] — 2026-06-13

### Changed
- **Assistant standardised on opencode**: the ✦ assistant now runs a single
  agent CLI — **opencode** (model-agnostic, MCP-extensible) — instead of a
  six-framework menu. The per-framework machinery is gone (Claude Code / Kilo /
  Hermes / OpenClaw / smallcode plumbing, the `CLAUDE.md` / `HERMES.md` /
  `knowledge/*.md` stamping, the smallcode `.env` and OpenClaw workspace pointer,
  and the Settings framework picker); the briefing is stamped once as `AGENTS.md`.
  `assistant_command` in config.toml still overrides the binary, and broader
  OS/computer-use control is added via MCP servers in opencode's own config.
  Migration: if you had `assistant_command` pinned to `hermes` or `openclaw`,
  add their TUI launch arg to `assistant_args` (`["--tui"]` / `["tui"]`) — the
  per-framework default arguments are no longer applied.

### Added
- **Revoke SSH key when forgetting a system**: the ✕ in the Systems menu now
  opens a confirm with an opt-in *"Also revoke this PC's key on &lt;host&gt;"*
  toggle. When checked, removing the system also strips this machine's public
  key from that remote's `~/.ssh/authorized_keys` (the inverse of the key copy
  that adding it performed). Exact full-line match only (never touches other
  keys), across all your local identities; it rewrites the file in place inside
  `~/.ssh` (preserving its `0600` perms and SELinux context) and keeps a
  `.tuiui.bak`. Best-effort on a background thread — an offline host never blocks
  the removal or the UI, the local forget always succeeds, and any remote-side
  failure is logged rather than silently assumed done.

### Fixed
- **Activity Monitor refresh while hidden**: the per-frame app snapshot was
  still running when the Activity panel was open but **minimized** (or hidden in
  simple mode), so the window-drag stutter could persist in those states. The
  refresh now also requires the panel to be visible.
- **Update check no longer offers downgrades**: the main-channel version
  comparison was a plain string compare, so a local build *newer* than the
  latest release (mid-release-cut, or a dev/source build) showed a bogus
  "update available → older version". It now compares release version numbers
  (`major.minor.patch`) and only offers a strictly-newer release; an
  unparseable release tag reports "couldn't check" instead of guessing.

## [0.2.1] — 2026-06-13

### Added
- **Activity Monitor** (Ctrl+Space → A, or `@activity` in the launcher): a
  live, auto-refreshing table of every app the apphost is hosting (id, pid,
  command, age, dimensions, state) with kill-app controls and an Enter/y vs
  Esc/n confirm for live apps. Also `tuiui ps` and `tuiui kill-app <id|all>`
  CLI subcommands that work from any terminal or SSH session.

### Fixed
- **Window dragging stutter**: moving (or resizing) a floating window felt
  sticky and jerky once a few apps were open. The activity monitor's row
  refresh was running every frame even while its panel was closed, and each
  pass cloned every hosted app's full terminal grid just to read its size —
  stalling the render loop mid-drag. The refresh now does nothing unless the
  Activity Monitor window is actually open.
- **Update loop on the main channel**: "Check for updates" compared the
  installed build against the tip of the `main` branch, but the main channel
  installs the latest prebuilt *release*. Any commit landed on `main` after the
  last release showed a permanent "update available" that re-installing the
  same release could never clear. The check now compares the installed version
  against the latest release tag — matching what `install.sh` installs — and
  reports it as versions (`v0.2.0 → v0.2.1`) instead of commit hashes. The dev
  channel still tracks the branch tip and shows short commit hashes.

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
