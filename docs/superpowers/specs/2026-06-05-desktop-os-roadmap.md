# tuiui "Real Desktop OS" Roadmap

**Status:** Planning roadmap (2026-06-05). Sequences six subsystems toward a
file-manager + browser + default-apps + desktop-icons/widgets desktop. Each item
gets its own `spec → plan → build` cycle; this doc only orders them, sizes them,
and flags the cross-cutting risks.

**Effort key:** S ≈ a day-ish, M ≈ a few focused sessions, L ≈ a major effort /
multiple sessions, R = has a genuine research/feasibility risk.

---

## The critical fork: native image rendering vs. graphics passthrough

Everything image-related splits into two very different problems. Naming them now
prevents a nasty surprise later.

- **A1 — Native image rendering (feasible).** tuiui's *own* compositor places
  images: the daemon includes image placements in a frame, and the thin client
  emits Kitty graphics escapes at the right screen cells, clipped to the owning
  window. This powers our **own** surfaces — an image viewer, file-manager
  thumbnails, desktop icons, richer widgets. Self-contained; Ghostty supports the
  protocol.

- **A2 — Graphics passthrough from embedded apps (hard, R).** Apps we host run in
  a PTY rendered by our embedded emulator (`alacritty_terminal`), which is
  **cell-based and does not understand the Kitty graphics protocol** — it would
  swallow the escapes. So a graphics-emitting TUI inside a tuiui window (yazi
  previews, `timg`, Carbonyl) will **not** show images today. Fixing this means
  teaching the embedded emulator to capture graphics commands and threading them
  out through our frame to the client — a real emulator-level change, possibly
  requiring a different/augmented terminal backend.

**Consequence:** build image features **natively (A1)** wherever possible
(file-manager previews render *our* thumbnails, not a hosted app's). Treat A2 as
its own research spike, needed mainly for the web browser.

---

## The six subsystems

### A1 — Native image layer (Kitty graphics)  ·  Effort: M
- **What:** an `Image` layer type in the compositor; frame protocol carries image
  placements (id, screen rect, source hash); client emits/streams Kitty graphics,
  re-positions on move/resize, deletes on close; capability-detects the terminal
  and falls back to a placeholder block when unsupported.
- **Deliverable demo:** a native **image viewer** window (open a PNG/JPG).
- **Depends on:** nothing. **Unblocks:** C previews, D icons, E rich widgets.
- **Risks:** image lifecycle vs. the diff-based renderer; clipping under
  overlapping windows; throughput over SSH (cache by hash, send once).

### B — Default Applications + file associations  ·  Effort: M
- **What:** `Settings → Default Apps` mapping **roles** (browser, file-manager,
  editor, terminal, image-viewer) and **file-type → app** (by extension/MIME);
  a resolver `open(path) → app`; persisted to `config.toml`. Falls out for free:
  dock/menubar/keyboard **shortcuts to the default browser & file-manager**.
- **Depends on:** nothing (graphics-free). **Unblocks:** C double-click-open, D
  icon launching, the default-app shortcuts.
- **Risks:** sensible defaults per OS; the picker UX for choosing an app (reuse
  the launcher list); keeping it data-driven so new roles are cheap.

### C — File manager (native TUI)  ·  Effort: L
- **What:** a native window: dual-pane or Miller columns, keyboard + mouse nav,
  selection, and operations (open, copy/move/delete/rename, mkdir). **Image
  thumbnails via A1**; **double-click → open with the default app via B**.
- **Decision to make in its spec:** *build native* vs *integrate an existing TUI
  manager* (yazi/lf/nnn). Integration looks cheaper but inherits the **A2**
  problem (their previews won't render in our window) and bypasses our default-app
  system — so **native is likely the right call** despite being more work.
- **Depends on:** A1 + B. **The centerpiece.**
- **Risks:** scope creep (file ops are a long tail); doing previews natively means
  generating thumbnails ourselves (decode + downscale).

### D — Desktop icons  ·  Effort: M
- **What:** clickable icons on the wallpaper (apps / files / folders); drag to
  arrange; positions persisted; double-click opens via **B**; optional thumbnails
  via **A1**. New desktop-level mouse layer beneath the windows.
- **Depends on:** B (open), optionally A1 (thumbnails).
- **Risks:** desktop vs. window mouse routing; icon-position persistence; grid
  snapping.

### E — Desktop widgets  ·  Effort: M
- **What:** a small widget framework + built-ins (clock, system stats reusing the
  tray's `SystemState`, calendar, sticky note). Positioned on the wallpaper,
  non-interactive or lightly interactive. Cell-based first; richer ones can use A1.
- **Depends on:** mostly independent (the tray's `SystemState` already exists).
- **Risks:** keeping the widget API small; refresh cadence (reuse the poller).

### F — Web browser with images  ·  Effort: L · R
- **What:** integrate an existing terminal browser — **Carbonyl** (headless
  Chromium → terminal) or **Browsh** (Firefox) — packaged in the store, launched
  in a window. "With images" depends on either **A2** (passthrough of its
  graphics) or a Carbonyl-specific output path.
- **Depends on:** A2 (the hard one) for real image rendering; usable in text/box
  mode without it.
- **Risks:** the A2 research risk; Carbonyl/Browsh resource weight; input mapping.

---

## Recommended sequence

1. **A1 — Native image layer** (visual enabler; demoable image viewer).
2. **B — Default Applications** (the "real OS" backbone; frees the shortcuts).
3. **C — File manager** (centerpiece; combines A1 + B).
4. **D — Desktop icons** (uses B, optionally A1).
5. **E — Desktop widgets** (largely independent; nice polish).
6. **F — Web browser** (gated behind the A2 research spike).

A and B are independent, so their order can swap; A1 is first here because it's
the piece you anchored on and unblocks the most visual surface. F is last because
of the A2 passthrough risk — we'll spike A2 separately before committing to it.

## What each cycle produces
Per subsystem: a `docs/superpowers/specs/…-design.md`, then a
`docs/superpowers/plans/…md`, then TDD implementation with commits + push, exactly
as A1→F. This roadmap is the umbrella; it is not itself a buildable spec.

## Open questions to resolve when each spec starts
- **A1:** image cache/eviction policy; SSH bandwidth budget; placeholder style.
- **B:** default role assignments per OS; how the app-picker UI looks.
- **C:** native vs. integrate (leaning native); which file ops are in v1.
- **F:** Carbonyl vs. Browsh; is A2 in scope or do we ship text-mode first.
