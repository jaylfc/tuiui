# agent/ — the tuiui desktop assistant's instruction pack

This folder is the **source of truth for everything the AI assistant is told**.
It is embedded into the tuiui binary at build time (`include_str!`) and stamped
into the assistant's working directory (`~/.local/share/tuiui/assistant/`) on
every launch, with live placeholders (host name, saved systems, version)
filled in.

tuiui standardises on the **opencode** CLI, with **hermes** as a supported
alternative (switch between them in **Settings → Assistant**). Its working
directory is forced as the agent's cwd, and the pack is written there as
`AGENTS.md` — the context file opencode (or hermes) reads on startup:

| File written at launch | Read by                  |
|------------------------|--------------------------|
| `AGENTS.md`            | opencode / hermes (the assistant) |

(`assistant_command` in config.toml stores the switch and can also point the
panel at any other binary by hand, but the briefing is always stamped as
`AGENTS.md`.)

Editing a file here changes what the agent is told, after a rebuild.

## Files

- `BRIEFING.md` — identity and role (who the agent is, where it runs)
- `DESKTOP.md` — driving the desktop via the `tuiui` control CLI
- `SYSTEMS.md` — operating across the user's machines over ssh/scp
- `TROUBLESHOOTING.md` — logs, common failures, fixing tuiui itself
- `RULES.md` — ground rules (safety, output width, confirmations)

Placeholders substituted at launch: `{{HOST}}`, `{{VERSION}}`, `{{SHA}}`,
`{{REPO}}`, `{{SYSTEMS}}` (the saved-systems table from systems.toml).
