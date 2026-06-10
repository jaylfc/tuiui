# agent/ — the tuiui desktop assistant's instruction pack

This folder is the **source of truth for everything the AI assistant is told**.
It is embedded into the tuiui binary at build time (`include_str!`) and stamped
into the assistant's working directory (`~/.local/share/tuiui/assistant/`) on
every launch, with live placeholders (host name, saved systems, version)
filled in.

The assistant's working directory is forced regardless of which agent CLI runs
(Claude Code, opencode, smallcode, kilo, hermes, openclaw), and the pack is
written in every context-file convention they read:

| File written at launch          | Read by                          |
|---------------------------------|----------------------------------|
| `CLAUDE.md`                     | Claude Code (and hermes, fallback) |
| `AGENTS.md`                     | opencode, kilo, codex-style CLIs |
| `HERMES.md`                     | hermes (its highest-priority file) |
| `knowledge/*.md`                | smallcode                        |
| `.env` (template, written once) | smallcode model/endpoint config  |

OpenClaw is the exception: it assembles its prompt from its own workspace
(`~/.openclaw/workspace/`), so launch also appends a marked, idempotent
pointer to this pack in that workspace's `AGENTS.md`.

Editing a file here changes what every agent is told, after a rebuild.

## Files

- `BRIEFING.md` — identity and role (who the agent is, where it runs)
- `DESKTOP.md` — driving the desktop via the `tuiui` control CLI
- `SYSTEMS.md` — operating across the user's machines over ssh/scp
- `TROUBLESHOOTING.md` — logs, common failures, fixing tuiui itself
- `RULES.md` — ground rules (safety, output width, confirmations)

Placeholders substituted at launch: `{{HOST}}`, `{{VERSION}}`, `{{SHA}}`,
`{{REPO}}`, `{{SYSTEMS}}` (the saved-systems table from systems.toml).
