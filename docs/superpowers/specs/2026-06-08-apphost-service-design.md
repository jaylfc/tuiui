# Apphost as a Per-User Service — Design

**Status:** Approved (2026-06-08). Run the durable **apphost** as a managed per-user service so
it auto-starts on login and restarts on crash, on macOS, Linux, and WSL.

## Goal

`tuiui service install` registers `tuiui --apphost` (the process that owns running apps) as a
per-user background service. It auto-starts on login and restarts on failure, so apps stay alive
in the background and the daemon/client always find a running apphost. `tuiui service uninstall`
removes it; `tuiui service status` reports state. The frontend daemon is unchanged (still spawned
on attach; it's the replaceable UI).

## Backends (chosen at runtime by platform + capability)

1. **macOS → launchd LaunchAgent.** Write `~/Library/LaunchAgents/co.uk.janlabs.tuiui-apphost.plist`,
   then `launchctl bootstrap gui/$UID <plist>` (fallback `launchctl load -w`). `RunAtLoad=true`;
   `KeepAlive = { SuccessfulExit = false }` (restart on crash, NOT on a clean exit — so a clean
   apphost exit from `tuiui kill` / "already running" does not loop).
2. **Linux/WSL with a usable systemd `--user` instance → systemd user service.** Write
   `~/.config/systemd/user/tuiui-apphost.service`, then `systemctl --user daemon-reload` +
   `systemctl --user enable --now tuiui-apphost.service`. `Restart=on-failure`,
   `WantedBy=default.target`. (Note linger: without `loginctl enable-linger $USER` a user service
   stops at logout — acceptable; mention it in `status`.)
3. **No usable systemd (old WSL / minimal distros) → `~/.profile` hook fallback.** Append a guarded
   block (between `# >>> tuiui apphost >>>` / `# <<< tuiui apphost <<<` markers) that, on a login
   shell, starts `tuiui --apphost` in the background if its socket isn't already up. Best-effort:
   no supervision (no restart-on-crash), starts when a login shell runs. `uninstall` strips the block.

**systemd detection (`has_user_systemd`):** run `systemctl --user show-environment`; treat success
(exit 0, no "Failed to connect to bus" on stderr) as usable. Else fall back to the `~/.profile` hook.

## Environment baked into the service

A service's environment is sparse, so the apphost couldn't find/launch apps. At install time we
capture and bake the current `PATH`, `HOME`, `SHELL`, and `LANG` into the unit:
- launchd: `EnvironmentVariables` dict.
- systemd: `Environment=` lines.
The user installs from their shell, so the captured values are correct. (`USER`/`XDG_RUNTIME_DIR`
are provided by launchd/systemd; the socket dir logic already falls back to the temp dir on macOS.)

`ExecStart` / `ProgramArguments` use `std::env::current_exe()` resolved at install time (the
installed `tuiui` binary path).

## Apphost idempotency (avoid service ↔ daemon races)

The daemon still spawns `tuiui --apphost` on demand if the socket is absent. To avoid two apphosts
fighting over the socket, `apphost::server::run()` first tries to connect to the existing socket;
if a live apphost answers, it logs and **exits cleanly (Ok)** instead of rebinding. Combined with
"restart on failure only", a redundant service start exits without a restart loop, and the daemon's
on-demand spawn and the service coexist (whichever binds first wins; the other exits).

## CLI (`src/main.rs` + `src/service.rs`)

- `tuiui service install` — set up + start the service for this platform.
- `tuiui service uninstall` — stop + remove it (and strip the profile block / plist / unit).
- `tuiui service status` — print which backend is in use, whether it's installed/running, and any
  follow-up tips (e.g. enable systemd in WSL, or `loginctl enable-linger`).
- `tuiui service` (no subcommand) → print usage.

`src/service.rs` separates **pure generators** (plist string, systemd unit string, profile snippet
— given exe path + env map) from the **effectful** install/uninstall (write files, run
`launchctl`/`systemctl`, edit `~/.profile`). Generators are unit-tested; effectful parts run the
platform tools.

## Testing

- **Unit (pure, any OS):** `launchd_plist()` contains the Label, `--apphost` arg, `RunAtLoad`,
  `KeepAlive/SuccessfulExit=false`, and the baked PATH; is valid (round-trips/lints). `systemd_unit()`
  contains `ExecStart=… --apphost`, `Restart=on-failure`, `Environment=PATH=…`,
  `WantedBy=default.target`. `profile_block()` is idempotent (the guarded markers) and references
  `--apphost` + a socket check.
- **macOS:** generated plist passes `plutil -lint` (validated in dev without loading it).
- **Apphost idempotency:** a unit/loopback check that a second `run()` against a live socket returns
  promptly without error (or at least that the connect-probe path is exercised).
- **Manual (user, per platform):** `tuiui service install` then reboot/relogin → `pgrep -fa
  'tuiui --apphost'` shows it; kill it → it restarts (systemd/launchd) within seconds; `tuiui`
  attaches instantly; `tuiui service uninstall` removes it. On WSL, both the systemd and the
  `~/.profile`-fallback paths.

## Out of scope

System-wide (boot, root) services; running the frontend daemon as a service; `loginctl enable-linger`
automation (just advise it); Windows-native (non-WSL) services.
