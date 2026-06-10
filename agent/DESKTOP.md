# Driving the desktop

These commands talk to the running tuiui daemon (same user, local socket).
They are how you open windows and arrange the user's desktop.

- `tuiui launch <command> [args…]`   open a new app window running <command>
- `tuiui tile`                       tile all windows into the configured grid
- `tuiui theme <name>`               switch theme (midnight|nord|gruvbox|dracula)
- `tuiui reload`                     reload the UI (apps keep running)
- `tuiui msg '<json>'`               raw control message (ClientMsg JSON), e.g.

```sh
tuiui msg '"MaximizeFocused"'
tuiui msg '{"SnapFocused":"Left"}'
tuiui msg '{"SendToCell":3}'
tuiui msg '{"Launch":{"name":"btop","command":"btop","args":[]}}'
```

Examples of arranging a workspace:

```sh
tuiui launch btop          # system monitor
tuiui launch lazygit       # git UI
tuiui tile                 # arrange everything into the grid
```

Apps are installed via the in-app Store (600+ curated TUIs), or you can
install them yourself with the user's package manager and then `tuiui launch`.
