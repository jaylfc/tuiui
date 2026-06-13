# Troubleshooting & fixing tuiui

## First stop: the log

`~/tuiui-debug.log` — always on, timestamped (ms since epoch), capped at 4MB.
Every subsystem traces here: app launches, installs, ssh/scp transfers,
system switches, notifications, theme changes. `tail -100` it before
theorising. The user can also open it in-app (launcher → tuiui → Logs) and
copy it to their clipboard with `c`.

## Common problems

- **App won't install from the Store**: the install runs in a visible shell
  window — read its output. Usually a missing toolchain (Go/Rust/Node/Python);
  the Store offers to install toolchains, or install via the package manager.
- **App needs models/providers configured** (AI agents like opencode, aider):
  they have their own config/onboarding — run their setup in a
  shell window and follow their docs. The Store's detail pane shows a setup
  tip for these apps.
- **Garbled rendering**: ask the user to resize the terminal once (forces a
  full re-baseline) or run `tuiui reload`. If it persists, collect the log.
- **Remote system unreachable**: check the Systems menu dot, then
  `ssh -o BatchMode=yes <target> true`; "Permission denied" means the key
  isn't installed — re-run Systems → Add Remote.
- **Mouse doesn't work on a bare Linux console**: gpm must be running
  (`sudo systemctl enable --now gpm`).

## Fixing tuiui itself

The source is at {{REPO}}. To fix a bug:

```sh
git clone {{REPO}} && cd tuiui
cargo build          # build
cargo test           # 300+ tests must stay green
```

Make the fix, keep the existing code style, add a regression test, and open a
pull request against that repository. The user installs your merged fix
in-app via Settings → Updates (which runs `cargo install --git {{REPO}}` and
reloads with apps intact).
