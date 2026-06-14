# Session handoff — in-app update "stuck" + release work (2026-06-14)

Status at pause: **0.2.9 shipped.** One open bug under active investigation
(in-app "update from Settings" still gets stuck) plus one tracked feature
(Wayland compositor). This doc is the handoff so a fresh session can resume
without re-deriving anything.

---

## ▶ START HERE NEXT SESSION — ask the user these again

The in-app updater investigation is **blocked on a log from the user.** 0.2.9
added the instrumentation needed to pinpoint it. Re-ask:

1. **Are you on 0.2.9 yet?** If not, get there reliably first (manual install
   bypasses the broken in-app path):
   ```sh
   curl -fsSL https://raw.githubusercontent.com/jaylfc/tuiui/main/install.sh | sh
   tuiui reload
   ```
   Confirm Settings → Updates shows `v0.2.9`.
2. **Trigger the update from Settings → Updates → Install**, let it run, then
   paste the tail:
   ```sh
   tail -40 ~/tuiui-debug.log
   ```
3. **When it's "stuck", what exactly happens** — update window hangs open?
   screen freezes? drops back to the desktop unchanged?

### How to read that log (the missing/anomalous line IS the bug)

The 0.2.9 log is persistent across reloads and records version+binary per
start. Expected healthy chain:
```
update: install -> '…/.local/bin'
update: install.sh ok; reloading via '…/.local/bin/tuiui'
daemon: reload — exiting to restart, apphost preserved
client: daemon reload — …respawning
daemon: spawning '…/.local/bin/tuiui' --daemon
=== tuiui session start (v0.2.9 → vNEW, git …, exe …) ===
```
| Symptom in log | Diagnosis |
|---|---|
| no `update: install -> …` | Install button not firing the action |
| `update: FAILED …` | install.sh/cargo failed (line says which) |
| `…install.sh ok` but no `daemon: reload — exiting` | `tuiui reload` didn't reach the daemon |
| reload happens but no new `=== … session start …` banner | client didn't respawn the daemon |
| new banner shows the **same** version/exe | **respawn ran a stale binary** ← prime suspect |
| new banner shows a **newer** version | update worked → "stuck" is a repaint/UI issue (different fix) |

### Leading hypotheses (ranked) if the log points at stale-binary respawn
- macOS `current_exe()` after `tar` overwrites the running binary: confirm the
  client respawns the **new inode**. `spawn_daemon()` (`src/main.rs`) uses
  `std::env::current_exe()`. If the user launched via a **symlink** (e.g.
  `~/.local/bin/tuiui` → `~/.cargo/bin/tuiui`), `current_exe()` canonicalizes
  and `exe_dir` may differ from where they think the binary lives.
- The reload tears down the daemon but the client doesn't detect the socket
  drop / doesn't return `ClientExit::Reload` (frozen screen case).

---

## What shipped this session (all via the auto-release pipeline)

| ver | what |
|---|---|
| **0.2.5** | Fixed the update **loop**: v0.2.4 was tagged on the 0.2.3 commit, so its binaries reported 0.2.3 → perpetual "update available". Re-released from the right commit. |
| **0.2.6** | **Dock right-click context menu** (#32, port from dev #11/#20) + **activity-monitor/apphost follow-ups** (#33, re-landed from stale #16). |
| **0.2.7** | **In-app update fix**: reload via the installed binary's **absolute path** (bare `tuiui reload` could miss `$PATH` in `sh -lc`) + `update:` logging. |
| **0.2.8** | **Switchable assistant agent** (#37): Settings → Assistant flips opencode ⇄ hermes (stored in `assistant_command`). |
| **0.2.9** | **Debug log persists across reloads** (was truncating on every daemon start, wiping the update trace) + reload→respawn seam logging. Diagnostic build for the stuck-update bug. |

Also built this session: the **auto-release-on-version-bump pipeline** +
tag/version guard in `release.yml` (see CLAUDE.md → Versioning).

## Release process reminder
Bump `Cargo.toml`+`Cargo.lock`, roll CHANGELOG `[Unreleased]`→`[x.y.z]`, PR,
merge → `release.yml` auto-tags/builds/publishes. Can't push tags or dispatch
workflows from the session (403) — merging the bump is the only path. Dev
working branch: `claude/clever-turing-xbe9mg` (force-push it freely; it carries
already-merged squash commits between PRs).

## Other open item
- **Wayland compositor** — tracked in issue **#34** (stale PR #15, ~4k lines,
  conflicts with `release.yml`; needs a deliberate rebase preserving the
  release automation). PRs #16/#17 closed (superseded/moot).

## Key code pointers (for the update bug)
- `update_command()` / `drain_settings_action()` `InstallUpdate` arm — `src/session.rs`
- reload flow: `ClientMsg::Reload` → `core.reload` flag; daemon exits on reload
  (`src/daemon.rs` ~85–133), client respawns (`src/main.rs` `attach()` +
  `spawn_daemon()`).
- `dbg_init()` / `dbg_log()` — `src/lib.rs` (now appends; 4 MB cap).

## Environment facts about the user (macOS)
- Apple Silicon, **ghostty** terminal, truecolor + kitty-graphics.
- Binary at `/Volumes/NVMe/Users/jay/.local/bin/tuiui` (single, on `$PATH`);
  prebuilt arm64 binary runs fine (no signing/`Killed: 9` issue).
- Config: `~/.config/tuiui/config.toml` had a stale `assistant_command =
  "hermes"` (why the ✦ panel launched hermes, not opencode) — the new agent
  switch reflects/flips it.
- Manual `install.sh` works perfectly; only the **in-app** update is stuck.
- Qodo reviews are paused (trial ended) — ignore Qodo/CodeRabbit/Kilo bot
  comments on PRs; they're noise. gitar-bot gives real reviews.
