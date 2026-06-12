# Activity Monitor + Blank-Shell Recovery

## What I'm fixing

You have a wedged `tuiui` stack and you want a built-in way to see and kill the apps `tuiui` is running.

### Root cause of the blank shell (no code change required)

From `ps`/`lsof`/socket ownership:

| PID     | Role                       | Started          | Socket it owns                       | Notes                                                                                  |
|---------|----------------------------|------------------|--------------------------------------|----------------------------------------------------------------------------------------|
| `3194`  | `tuiui` front-end client   | Thu 18:17 (1d+)  | none (fds point at dead daemon peer) | **Stale** — orphan client attached to a long-gone daemon.                              |
| `1633`  | `tuiui --apphost`          | Thu 18:17 (1d+)  | `apphost.sock`                       | Owns the live apps from yesterday.                                                     |
| `86161` | `tuiui --daemon` (fresh)   | 7:53 pm (4 min)  | `daemon.sock`, `daemon-ctl.sock`     | Rebound the daemon socket; re-used the apphost already running.                        |
| `90291` | `.kilo`                    | 7:54 pm (3 min)  | unix-pair to nothing                 | **R+ 100% CPU**, three orphaned socket fds → Kilo is spinning on broken pipes.         |

The daemon's `serve_client` loop is **single-client-serial** (`src/daemon.rs:97` `for stream in listener.incoming()`). When a second `tuiui` client attached, the first one's stream was severed; the orphan is PID `3194`. The new Kilo window is blank because the apphost's Kilo child is wedged on dead sockets (100% CPU, no readable peer).

**Claude Code sessions (PIDs `12013`, `10103`) are untouched.**

### Recovery (no code, do this first)

```bash
kill 3194 1633 86161 90291 2>/dev/null
rm -f /var/folders/gh/52p2m9rs61d04xmmbb4mxsnh0000gn/T/tuiui-jay/{daemon,daemon-ctl,apphost}.sock
tuiui
```

This unblocks you immediately. Everything below is the new feature.

---

## What I'm building

Two surfaces, both driving the same data:

1. **CLI** — `tuiui ps` (list hosted apps) and `tuiui kill-app <id>` (kill one). Lives next to the existing `tuiui kill` / `tuiui reload` in `src/main.rs`. Uses the apphost socket directly, so it works from any TTY or over SSH.
2. **In-app panel** — a new built-in window (`Ctrl+Space m`, plus a dock/menubar entry) showing a live, auto-refreshing table of hosted apps with `k` / `K` to kill, mirroring the Settings/Store/Filter pattern. No new process: it runs inside the existing daemon, which already has the `AppHost` trait handle.

### CLI scope

```
$ tuiui ps
APPID  PID     CMD                                 COLSxROWS  AGE     STATE
1      58312   /opt/homebrew/bin/kilo              120x40     2m      alive
2      —       /bin/zsh                            80x24      1h12m   alive
3      —       /opt/homebrew/bin/lazygit           200x50     18s     dead (exit 0)

$ tuiui kill-app 1
tuiui: sent kill to app 1 (kilo, pid 58312)
$ tuiui kill-app 999
tuiui: no such app (have: 1, 2, 3)
$ tuiui kill-app all
tuiui: sent kill to 3 app(s)
```

- `tuiui ps` and `tuiui kill-app` are subcommands of the **front-end `tuiui` binary** in `src/main.rs` (not `--daemon` / `--apphost`). They connect to the apphost socket directly using the same path resolution as the daemon (`src/protocol.rs:118` `apphost_socket_path()`).
- Both send a new apphost request and read a structured response, then exit. They do **not** require a client to be attached — they work even with `tuiui` running headless.
- "age" comes from a spawn timestamp the apphost tracks (new field on `AppInstance`).
- "pid" comes from `portable_pty::Child::process_id()` — already on the trait, just not surfaced.

### In-app panel scope

A new window "Activity Monitor", opened by:
- `Ctrl+Space` → `m` (new leader binding, alongside `s`=Store, `,`=Settings).
- An entry in the menubar (next to Store/Settings), and a launcher entry `@activity` for symmetry with `@store`/`@settings`/`@files`.

Contents (a single screen, fixed-width table):

```
Activity Monitor                                3 apps   refresh: 1s
─────────────────────────────────────────────────────────────────────
  ID    PID      CMD                          COLS×ROWS  AGE    STATE
▶  1   58312    /opt/homebrew/bin/kilo         120×40   2m     alive
   2     —      /bin/zsh                        80×24   1h12m  alive
   3     —      /opt/homebrew/bin/lazygit       200×50   18s    dead

k: kill selected   K: kill all dead   r: refresh now   Esc: close
```

- **Auto-refresh**: re-pulls the list every 1s while the window is open and focused (timer in `daemon.rs`'s `serve_client` loop; only ticks when panel is focused so it costs nothing when closed).
- **Keys**: `↑/↓` move, `k` kills the selected (with a confirm overlay like the titlebar ✕ close), `K` kills all `dead` rows (no confirm — these are already gone), `r` forces refresh, `Esc` closes.
- **Click-to-kill**: clicking a row's `kill` glyph does the same as `k`. A click anywhere on the row selects it.
- **No new process**: the panel reads the existing `AppHost` directly in-process. No apphost protocol change is needed for the panel itself.

### Why I'm NOT touching the apphost protocol for the panel

`SessionCore` already calls `host.list()`, `host.meta(id)`, and `host.is_alive(id)` every frame. I'll add two cheap reads:
- `host.spawn_time(id) -> Option<Instant>` — new method on `AppHost`, default returns `None`. `LocalAppHost` returns `Some(self.spawned_at[&id])` (new field, set in `spawn`).
- `host.pid(id) -> Option<u32>` — new method. `LocalAppHost` calls `self.apps[&id].child.process_id()` via a new `AppInstance::process_id()` (which `portable_pty::Child` already exposes).

The CLI side does need a protocol change — see below.

---

## Files I'll touch

| File                                       | What                                                                                                                                  |
|--------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------|
| `src/main.rs`                              | New subcommands `ps`, `kill-app`. Dispatch table in the `match` at `main.rs:20`. Two new free functions.                              |
| `src/launcher.rs`                          | New `@activity` special-case alongside `@store`/`@settings`/`@files` at `session.rs:1755-1765` (actually that lives in session).       |
| `src/session.rs`                           | `open_activity()` mirroring `open_settings()`/`open_store()`; new `ClientMsg::{OpenActivity, ActivityUp, ActivityDown, ActivityKill, ActivityKillDead, ActivityRefresh, ActivityClose}`; new `WinContent::Activity(Activity)`; `focused_is_activity()`, `activity_editing()` flags. New `Launcher::activity()` keyword. |
| `src/activity.rs` *(new)*                  | The `Activity` widget — list of rows, selection, kill-confirm overlay. ~250 lines, patterned on `settings.rs` + `store.rs`.            |
| `src/apphost/api.rs`                       | Add `spawn_time(id) -> Option<Instant>` and `pid(id) -> Option<u32>` to the trait (with defaults for back-compat).                   |
| `src/apphost/host.rs`                      | Implement both on `LocalAppHost`. Store `Instant::now()` in a new `HashMap<AppId, Instant>`.                                          |
| `src/ptyhost.rs`                           | Add `AppInstance::process_id() -> Option<u32>` (forwarding to `child.process_id()`); add a `spawned_at: Instant` field.                |
| `src/protocol.rs`                          | Add the new `Flags` field `activity_focused: bool`.                                                                                  |
| `src/client.rs`                            | In the key-routing `if/else` chain around line 169–235, add the `activity_focused` branch with the keys listed above.                 |
| `src/daemon.rs`                            | Populate the new `Flags` field; if `activity_focused`, kick a refresh every 1s by calling `core.refresh_activity()` (a new method).   |
| `src/apphost/proto.rs`                     | Add `HostReq::ListApps` (id+cmd+pid+cols+rows+age+alive) and `HostEvt::AppList { apps: Vec<AppInfo> }` for the CLI path.               |
| `src/apphost/server.rs`                    | Handle `ListApps`: walk `local.list()`, build `AppInfo`s, send `AppList` back.                                                       |
| `src/apphost/remote.rs`                    | Implement the new `spawn_time` and `pid` methods on `RemoteAppHost` (just return `None` for now — these are panel-only, in-process).  |
| `src/main.rs` (CLI helper)                 | `fn cmd_ps()` and `fn cmd_kill_app(id: &str)`: connect to apphost, send `ListApps`, print, exit. Use `serde_json` for the wire format. |
| `Cargo.toml`                               | No new deps. `portable_pty` and `serde_json` already in.                                                                             |
| `tests/`                                   | One new test: spawn 3 apps via `RemoteAppHost::spawn` over a loopback socket, send `ListApps`, assert 3 rows. Reuse the pattern at `src/apphost/remote.rs:209-251`. |
| `README.md`                                | Add `Ctrl+Space m` to the shortcuts table; add a one-liner for `tuiui ps`/`tuiui kill-app` next to `tuiui kill`/`reload` in the "Persistent daemon" paragraph. |

## Things I'm explicitly NOT doing

- **Not** changing the apphost `Kill` semantics (it already kills a single app by id). The CLI just dispatches the same `HostReq::Kill { app }`.
- **Not** adding a force-kill / SIGKILL path. The portable-pty `Child::kill()` is the right level — process group, SIGTERM-equivalent on macOS/Linux.
- **Not** showing the daemon/apphost processes themselves. The user's spec was "apps tuiui is managing"; those don't change per-launch and we already have `tuiui service status` for them.
- **Not** adding a "kill all" key to the in-app panel (only `K` for "kill all dead", which is a no-cost cleanup). Closing the daemon is what `tuiui kill` is for.
- **Not** touching the Claude Code sessions.

## Verification

```bash
# Unit + integration tests (new + existing)
cargo test --quiet
# Lint
cargo clippy --all-targets --quiet
# Build
cargo build --release

# Manual: in one terminal
tuiui &
# launch a few apps from inside (e.g. via the launcher), then in a different TTY:
tuiui ps                       # should show them
tuiui kill-app 1               # kills app 1
tuiui ps                       # app 1 gone
# In the running tuiui, Ctrl+Space m should show the same list, with r/k working
```

The recovery commands at the top are independent of this work — you can run them now to get unstuck before any of the code is written.
