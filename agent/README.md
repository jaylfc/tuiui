# agent/ — the tuiui desktop assistant's instruction pack

This folder is the **source of truth for everything the AI assistant is told**.
It is embedded into the tuiui binary at build time (`include_str!`) and stamped
into the assistant's working directory (`~/.local/share/tuiui/assistant/`) on
every launch, with live placeholders (host name, saved systems, version)
filled in.

The assistant's working directory is forced regardless of which agent CLI runs
(Claude Code, opencode, smallcode, kilo, hermes, openclaw), and the pack is
written in every context-file convention they read:

| File written at launch        | Read by                          |
|-------------------------------|----------------------------------|
| `CLAUDE.md`                   | Claude Code                      |
| `AGENTS.md`                   | opencode, kilo, codex-style CLIs |
| `knowledge/*.md`              | smallcode                        |

Editing a file here changes what every agent is told, after a rebuild.

## Files

- `BRIEFING.md` — identity and role (who the agent is, where it runs)
- `DESKTOP.md` — driving the desktop via the `tuiui` control CLI
- `SYSTEMS.md` — operating across the user's machines over ssh/scp
- `TROUBLESHOOTING.md` — logs, common failures, fixing tuiui itself
- `RULES.md` — ground rules (safety, output width, confirmations)

Placeholders substituted at launch: `{{HOST}}`, `{{VERSION}}`, `{{SHA}}`,
`{{REPO}}`, `{{SYSTEMS}}` (the saved-systems table from systems.toml).
