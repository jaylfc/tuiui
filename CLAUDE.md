# tuiui — project memory for coding agents

tuiui is a window manager & desktop for the terminal, in Rust. Floating
windows, dock, cascading launcher, app store (600+ curated TUIs), file
manager, desktop icons, menubar tray, mouse — rendered as cells over a
daemon/client socket, like a graphical tmux.

## Build, test, lint

```sh
cargo build           # MSRV: rust 1.95 (rust-version in Cargo.toml)
cargo test            # 300+ tests; keep them green
cargo clippy --all-targets   # warning-clean; keep it that way
```

There is no rustfmt enforcement; match the style of the file you're editing.
Clippy is currently warning-clean — keep it that way.

## Versioning & release channels

`Cargo.toml` carries the semver version (currently 0.2.0); update `CHANGELOG.md`
in the same commit as user-visible changes. The in-app updater (`update_command`
in `session.rs`) has two channels, chosen by `config.update_branch`:
- **main** — downloads the latest prebuilt release via `install.sh` (fast),
  falling back to `cargo install --git`.
- **dev** — `cargo install --git --branch dev` (source build, for testing).
When the feature set is verified, cut a `dev` branch so testers can track it
from Settings → Updates without touching `main`.

## Architecture (the three processes)

```
client (tuiui)  ←socket→  daemon (tuiui --daemon)  ←socket→  apphost (tuiui --apphost)
thin renderer              owns SessionCore (all UI state)    owns the PTY apps
real terminal              composites frames, routes input    survives UI reloads
```

- **client** (`src/client.rs`, `src/main.rs`): sets up the terminal, turns
  `FrameMsg` diffs into ANSI, forwards input as `ClientMsg`. Holds NO session
  state — it routes keys using the `Flags` the daemon sends each frame.
  `main.rs` is also where ssh "system switches" run (the client owns the real
  terminal, so interactive prompts work there and nowhere else).
- **daemon** (`src/daemon.rs`): serves one client at a time; out-of-band
  control socket accepts any `ClientMsg` (the `tuiui launch/tile/theme/msg`
  CLI and the AI assistant use it).
- **session** (`src/session.rs`, the big one): `SessionCore` is the
  `ClientMsg`-in / frame-out boundary. All UI widgets live here as fields.
- **apphost** (`src/apphost/`, `src/ptyhost.rs`): PTY children behind the
  `AppHost` trait; `LocalAppHost` in-process, `RemoteAppHost` over a socket.
  Apps survive frontend reloads because this process never restarts. The
  emulator keeps 10k lines of scrollback; `AppInstance::scroll` moves the
  display offset and `snapshot()` reflects it automatically (the wheel routes
  here via `ClientMsg::ScrollAt`).

## Key seams and conventions

- **Protocol changes**: every new `FrameMsg`/`Flags`/apphost-`Frame` field
  must be `#[serde(default)]` so version skew (old daemon ↔ new client, old
  apphost ↔ new frontend) degrades gracefully. Prefer new fields on existing
  messages over new enum variants (unknown variants fail the whole parse).
  Bump `PROTO_VERSION` (apphost/proto.rs) on any apphost protocol change; if
  a change genuinely breaks older apphosts, ALSO bump `MIN_COMPAT` — that
  arms the post-update safety dialog ("restart the app server, closes your
  apps") instead of letting users' sessions break silently. Never bump
  `MIN_COMPAT` casually: it forces users through an app-closing restart.
- **New windowed widgets** follow the Store/Settings/Logs pattern:
  a `WinContent` variant + `focused_is_X()` flag + `ClientMsg::X*` variants +
  a routing branch in `client.rs`. Widgets are pure state+render; they never
  touch sockets.
- **Shelling out**: only via `crate::system::run_capped` (hard timeout).
  Never block the render loop on the network — see the poller's separate
  thread for slow probes.
- **Logging**: `crate::dbg_log` (always on, `~/tuiui-debug.log`, 4MB cap).
  Log every external effect (ssh, scp, installs, spawns) with enough detail
  to debug from the log alone; users paste it via the in-app Logs viewer.
- **Geometry/hit-testing**: render and hit-test must share the same rect
  functions (see `powermenu.rs`'s "Geometry" section) so they can never drift.
- **Secrets**: passwords ride env vars (`SSHPASS`) for one-shot use, never
  argv, never disk, never the log.

## The AI assistant (✦)

`src/assistant.rs` + the `agent/` folder. `agent/*.md` is the single source
of truth for everything the in-app agent is told — embedded via
`include_str!`, stamped into `~/.local/share/tuiui/assistant/` at launch as
`AGENTS.md`, the context file the assistant reads on startup. tuiui
standardises on the **opencode** CLI (`DEFAULT_AGENT`); `assistant_command`
in config.toml can point the panel at a different binary, but there is no
per-framework branching — opencode is model-agnostic and MCP-extensible, so
broader OS/computer-use control is added via MCP servers in opencode's own
config rather than by adding frameworks here.

## Tests

Integration tests live in `tests/*_tests.rs`, unit tests inline. UI tests
drive widgets through their public state machines (clicks at computed rects,
messages applied to `SessionCore`) — no terminal needed. When you change a
menubar/dock layout, check `tests/chrome_tests.rs` for byte-vs-char column
assumptions (the bar contains multi-byte glyphs).

## Docs

- `README.md` — user-facing; keep "What works today", Controls, Configuration,
  and Roadmap in sync with shipped features.
- `agent/` — what the in-app assistant is told (see its README).
- `docs/superpowers/{specs,plans}` — dated historical design records; append
  new ones, don't retrofit old ones.
