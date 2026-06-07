# Apphost / Frontend Split — Design

**Status:** Approved direction (process split, decided 2026-06-06). This spec details
it and phases the build. Goal: **live updates** — update tuiui and reload the whole
UI while running apps stay alive with their full terminal state.

## Problem

Today one process (the daemon) owns *both* the running apps (PTYs + `alacritty`
terminals + child processes) *and* the frontend (window manager, compositor,
launcher, desktop, settings, the client-facing socket). Any update requires
`tuiui kill`, which destroys the apps. We want to replace the frontend without
killing apps.

## Architecture: two processes

1. **apphost** — small, stable, long-lived. Owns every `AppInstance` (PTY master fd,
   `alacritty_terminal::Term`, reader thread, the `kittygfx` graphics tap). Holds an
   opaque per-app "frontend metadata" blob (window geometry/title/z — apphost never
   interprets it). Exposes a Unix socket. It rarely changes, so it rarely needs
   restarting; restarting it (`tuiui kill`) is the only thing that drops apps.
2. **frontend** — everything else: `SessionCore` (wm, layout/tiling, launcher,
   desktop icons, settings, tray, compositor) + the existing client-facing socket.
   Connects to apphost as a client. **Replaceable**: kill + relaunch to update; it
   reconnects to apphost and rebuilds its windows from the per-app metadata, so apps
   reappear in place with intact screen state (apphost kept the `Term` grids).

Process tree: `apphost` ⇄ (socket) ⇄ `frontend` ⇄ (existing socket) ⇄ `client`.
The thin client is unchanged.

## The frontend ⇄ apphost protocol (`src/apphost/proto.rs`)

Mirrors the existing client/frontend pattern (newline-JSON or length-prefixed).

**frontend → apphost:**
- `Spawn { req_id: u64, cmd, args, cwd: Option<String>, cols, rows }`
- `Input { app_id, bytes }`
- `Resize { app_id, cols, rows }`
- `SetMeta { app_id, meta: Vec<u8> }` — store opaque window state for restore.
- `Kill { app_id }`
- (on connect, apphost auto-sends the current roster — no explicit Subscribe needed)

**apphost → frontend:**
- `Spawned { req_id, app_id }` (or `SpawnFailed { req_id, error }`)
- `AppFrame { app_id, generation, grid: GridSnapshot, placements: Vec<ImagePlacement>, images: Vec<ImageBlob> }`
  — pushed when the app's `Term` changes (debounced to the ~16ms frame tick) or on
  reconnect. `GridSnapshot` is the cells (reuse `CellBuffer` serialization, or a
  per-app cell diff in a later optimization). `images` are blob-once per app id.
- `AppGone { app_id }` (the child exited).
- `Roster { apps: Vec<RosterEntry { app_id, meta: Vec<u8> }> }` — sent on connect so a
  restarted frontend can rebuild windows.

The frontend caches the latest `AppFrame` per `app_id`; `build_frame` composites from
the cache instead of calling `AppInstance::snapshot()`. Input/resize/kill/spawn become
message sends. PTY device-status replies (DSR/DA) stay entirely inside apphost
(`PtyResponder` already writes them back to the PTY) — the frontend never sees them.

## Window ⇄ app mapping & restore

`WinContent::App` carries an `AppId` (u64) instead of an `AppInstance`. The frontend
keeps `WindowId → AppId`. Each frame (or on change) it serializes each app window's
state (`rect`, `z`, `title`, `minimized`) and sends `SetMeta`. On a fresh frontend
start, the `Roster` gives `(app_id, meta)` for every live app → the frontend recreates
a window per app from `meta`, re-binds `WindowId → AppId`, and requests/receives the
current `AppFrame` (full grid) so the window paints immediately with intact state.
Non-app windows (Store/Settings/FileManager/desktop) are frontend-only and simply
rebuilt empty/default on restart.

## Update flow

- **`tuiui`** (launcher binary, `main.rs`): ensure apphost running (spawn detached if
  the apphost socket is absent) → ensure frontend running (spawn if the frontend
  socket is absent, told where the apphost socket is) → attach the client to the
  frontend (unchanged client path).
- **`tuiui reload`** (new): tell the frontend to exit, then the launcher restarts it
  (new binary) against the same apphost → windows + apps restored. apphost untouched.
- **`tuiui kill`**: stop both (drops apps) — the full reset, as today.
- **In-app:** Settings → Updates → "Update & Reload" = install new binary, then
  trigger `reload`. Replaces today's "kill & restart everything".

## Phasing (each phase ships working software)

- **Phase 1 — In-process AppHost boundary (no behavior change).** Introduce
  `src/apphost/host.rs` with a `LocalAppHost` that owns the `AppInstance` map behind a
  clean API: `spawn(cmd,args,cwd,cols,rows)->AppId`, `input(id,bytes)`, `resize(id,
  cols,rows)`, `kill(id)`, `is_alive(id)->bool`, `snapshot(id)->Option<CellBuffer>`,
  `graphics(id)->{placements, new images}`, `list()->Vec<AppId>`, `set_meta`/`meta`.
  `WinContent::App(AppId)`; `SessionCore` owns a `LocalAppHost` and calls it via the
  API (replacing every direct `AppInstance`/`a.snapshot()`/etc. touch point mapped in
  the design). Fully in one process; all 203 tests still pass. **This carves the seam
  with zero user-visible change — the safe foundation.**
- **Phase 2 — Separate process + IPC.** Define `proto.rs`; implement the apphost
  binary path (`tuiui --apphost`) running `LocalAppHost` behind the socket; implement
  a `RemoteAppHost` (same API as `LocalAppHost`) that talks over the socket and caches
  `AppFrame`s. `SessionCore` uses `RemoteAppHost`. `main.rs` spawns/connects apphost
  before the frontend. Apps now survive a frontend restart.
- **Phase 3 — Update UX.** `tuiui reload` + the reconnect/restore-from-`meta` path +
  the in-app "Update & Reload" button; wire `SetMeta` each frame and `Roster` rebuild.

## Risks

- **Snapshot volume:** full grid per app per frame over the socket. Local IPC is fast
  and grids are small (~84×30); start simple, add per-app cell diffing if needed.
- **Latency:** apphost pushes `AppFrame`s asynchronously; the frontend renders from
  cache, so input→echo is one extra hop. Acceptable locally; measure.
- **Graphics:** the `kittygfx` tap stays with the PTY in apphost; placements + blobs
  ride `AppFrame`. The frontend's `ImageStore`/client path is unchanged downstream.
- **apphost stability:** since the goal is "rarely restart apphost", keep its code
  minimal (no feature logic) so updates almost always touch only the frontend.

## Testing

- Phase 1: the existing suite must pass unchanged (behavior-preserving). Add unit
  tests for `LocalAppHost` (spawn/list/kill/alive against a real short-lived child;
  snapshot of a known echo). `WinContent::App(AppId)` keeps `SessionCore` tests green.
- Phase 2: `proto` serde round-trips; a `RemoteAppHost`↔`LocalAppHost` loopback test
  over an in-memory/socket pair (spawn → AppFrame received → input echoed).
- Phase 3: restart-restore test (spawn app, drop+recreate frontend handle, Roster
  rebuilds the window). Manual: `tuiui reload` keeps a running btop alive.

## Out of scope (later)

Per-app cell-diff compression; multiple simultaneous frontends (web PWA + SSH on one
apphost — the architecture allows it, wire later); apphost self-update without
dropping apps (would need its own fd handoff — separate effort).
