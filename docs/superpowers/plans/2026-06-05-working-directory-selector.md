# Working-Directory Selector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Apps flagged `requires_cwd` (the AI coding CLIs) open a browsable file-tree picker on launch; the chosen directory becomes the app's PTY working directory.

**Architecture:** A daemon-side overlay widget (`dirpicker.rs`) mirroring the launcher: an expandable directory tree rendered to layers, lazy-loaded via an injectable `DirLister` (so the tree logic is unit-testable without disk). The PTY host gains a `cwd`. The session intercepts flagged launches, opens the picker, and launches on confirm.

**Tech Stack:** Existing Rust plumbing; std::fs behind `DirLister`.

**Reference spec:** `docs/superpowers/specs/2026-06-04-working-directory-selector-design.md`

---

## File Structure

- **Create `src/dirpicker.rs`** — `DirLister` trait, `DirPicker` tree model + render + hit-test.
- **Modify `src/catalog.rs`** — `Recipe.requires_cwd`.
- **Modify `src/config.rs`** — `AppEntry.requires_cwd`/`cwd`; `default_project_dir`/`recent_dirs`/`show_hidden_dirs`.
- **Modify `assets/recipes.json`** — `requires_cwd: true` on the 16 AI tools.
- **Modify `src/ptyhost.rs`** — `spawn` gains `cwd: Option<&Path>`.
- **Modify `src/session.rs`** — `PendingLaunch`, picker open/confirm, `launch_in`, `ClientMsg::DirPicker*`.
- **Modify `src/protocol.rs`** — `Flags.dirpicker_open`.
- **Modify `src/daemon.rs`** — populate the flag.
- **Modify `src/client.rs`** — route keys when `dirpicker_open`.
- **Modify `src/lib.rs`** — `pub mod dirpicker;`.

---

### Task 1: Manifest flag (`requires_cwd`)

**Files:** `src/catalog.rs`, `src/config.rs`, `assets/recipes.json`; Test `tests/catalog_tests.rs` (or inline).

- [ ] **Step 1:** Add to `Recipe` (catalog.rs):

```rust
    /// Whether launching this app should first prompt for a working directory.
    #[serde(default)]
    pub requires_cwd: bool,
```

- [ ] **Step 2:** Add to `AppEntry` (config.rs):

```rust
    /// Prompt for a working directory on launch (overrides the catalog flag).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_cwd: Option<bool>,
    /// Fixed working directory; when set, launches there and skips the picker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
```

Update the four `AppEntry { … }` literals in the codebase (config defaults, session `build_launcher_apps` Store/Settings entries, settings `commit_edit`, catalog `detect_installed`) to add `requires_cwd: None, cwd: None`.

- [ ] **Step 3:** Flag the AI tools in `assets/recipes.json` via a script:

```bash
python3 - <<'PY'
import json
cat = {a['name']: a for a in json.load(open('assets/catalog.json'))}
r = json.load(open('assets/recipes.json'))
for name, app in cat.items():
    if app.get('category') == 'AI' and name in r:
        r[name]['requires_cwd'] = True
json.dump(r, open('assets/recipes.json','w'), ensure_ascii=False, indent=1, sort_keys=True)
open('assets/recipes.json','a').write('\n')
print('flagged', sum(1 for v in r.values() if v.get('requires_cwd')))
PY
```

- [ ] **Step 4:** Test (append to `tests/catalog_tests.rs`):

```rust
#[test]
fn ai_tools_require_cwd() {
    assert!(tuiui::catalog::recipe("Claude Code").unwrap().requires_cwd);
    assert!(!tuiui::catalog::recipe("btop").map(|r| r.requires_cwd).unwrap_or(false));
}
```

Run: `cargo test --offline --test catalog_tests` → PASS.

- [ ] **Step 5:** Commit: `tiling`→ `git commit -m "cwd: requires_cwd manifest flag on AI tools + AppEntry fields"`

---

### Task 2: PTY host `cwd`

**Files:** `src/ptyhost.rs`, all `AppInstance::spawn` call sites.

- [ ] **Step 1:** Change the signature:

```rust
    pub fn spawn(cmd: &str, args: &[String], cols: i32, rows: i32, cwd: Option<&std::path::Path>) -> std::io::Result<AppInstance> {
```

Replace the `if let Some(home) = dirs::home_dir() { builder.cwd(home); }` block with:

```rust
        match cwd {
            Some(d) => builder.cwd(d),
            None => { if let Some(home) = dirs::home_dir() { builder.cwd(home); } }
        }
```

- [ ] **Step 2:** Update the single call site in `src/session.rs::launch` to pass `None` for now (Task 5 adds the real path). Build: `cargo build --offline` → compiles.

- [ ] **Step 3:** Commit: `git commit -m "cwd: AppInstance::spawn accepts an optional working directory"`

---

### Task 3: `DirPicker` tree model (lazy, testable)

**Files:** Create `src/dirpicker.rs`; Modify `src/lib.rs`; Test `tests/dirpicker_tests.rs`.

- [ ] **Step 1: Write the failing test** (`tests/dirpicker_tests.rs`):

```rust
use std::path::{Path, PathBuf};
use tuiui::dirpicker::{DirLister, DirPicker, PendingLaunch};

struct Fake;
impl DirLister for Fake {
    fn list_dirs(&self, path: &Path, _hidden: bool) -> Vec<(String, PathBuf)> {
        match path.to_str().unwrap() {
            "/root" => vec![("a".into(), "/root/a".into()), ("b".into(), "/root/b".into())],
            "/root/a" => vec![("x".into(), "/root/a/x".into())],
            _ => vec![],
        }
    }
}

fn picker() -> DirPicker {
    DirPicker::with_lister(PathBuf::from("/root"), PendingLaunch::default(), Box::new(Fake))
}

#[test]
fn lists_root_children() {
    let p = picker();
    let rows = p.visible();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "a");
}

#[test]
fn expand_reveals_children_inline() {
    let mut p = picker();
    p.expand(); // expand selected (a)
    let rows = p.visible();
    assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["a", "x", "b"]);
    assert_eq!(rows[1].depth, 1);
}

#[test]
fn collapse_hides_children() {
    let mut p = picker();
    p.expand();
    p.collapse();
    assert_eq!(p.visible().len(), 2);
}

#[test]
fn selected_path_is_the_highlighted_dir() {
    let mut p = picker();
    p.move_down(); // select b
    assert_eq!(p.selected_path(), Path::new("/root/b"));
}

#[test]
fn confirm_returns_pending_and_path() {
    let mut p = picker();
    let (pending, path) = p.confirm();
    assert_eq!(path, PathBuf::from("/root/a"));
    let _ = pending;
}
```

- [ ] **Step 2:** Run → FAIL (types missing).

- [ ] **Step 3: Implement `src/dirpicker.rs`** — the arena/tree, lazy expand, flatten to `visible()`, `move_up/down`, `expand/collapse`, `selected_path`, `confirm`, plus the real `FsLister`. Register `pub mod dirpicker;` in `lib.rs`. (Full model code; directories only; unreadable dirs → empty children.)

- [ ] **Step 4:** Run → PASS.

- [ ] **Step 5:** Commit: `git commit -m "cwd: DirPicker tree model (lazy expand/collapse/confirm) + tests"`

---

### Task 4: `DirPicker` render + hit-test

**Files:** `src/dirpicker.rs`; Test append.

- [ ] **Step 1:** Test that `render(w,h)` returns a non-empty layer and that `row_at(p)` maps a click to a visible-row index. Implement `render` (bordered box, breadcrumb, indented `▸/▾ 📁 name` rows, footer hint) + `row_at`. Mouse: click row selects; click ▸/▾ toggles.

- [ ] **Step 2:** Run → PASS. Commit: `git commit -m "cwd: DirPicker rendering + click hit-testing"`

---

### Task 5: Session flow + protocol + client routing

**Files:** `src/session.rs`, `src/protocol.rs`, `src/daemon.rs`, `src/client.rs`.

- [ ] **Step 1:** `Flags.dirpicker_open: bool` (protocol.rs) + populate in daemon.rs (`core.dirpicker_open()`).

- [ ] **Step 2:** `ClientMsg`: `DirPickerUp/Down/Expand/Collapse/Confirm/Cancel/Char(char)/Backspace/ToggleHidden`.

- [ ] **Step 3:** Session: hold `Option<DirPicker>`; `launch_in(name,cmd,args,cwd)`; on a flagged launch open the picker with a `PendingLaunch`; on `DirPickerConfirm` → `launch_in(.., Some(path))` + push to `recent_dirs`; render its layers in `build_frame`; hit-test on `MouseDown`. `dirpicker_open()` accessor.

- [ ] **Step 4:** client.rs: when `f.dirpicker_open`, route arrows/Enter/Esc/Backspace/Char to the `DirPicker*` messages.

- [ ] **Step 5:** Build + test → green. Commit: `git commit -m "cwd: session picker flow, dirpicker_open flag, client routing"`

---

### Task 6: Config roots + recent dirs

**Files:** `src/config.rs`, `src/session.rs`.

- [ ] **Step 1:** `default_project_dir: Option<String>` (default `~`), `recent_dirs: Vec<String>` (cap 10), `show_hidden_dirs: bool`. Picker root resolves `default_project_dir` (tilde-expanded) else `~`. On confirm, prepend to `recent_dirs` (dedup, cap) and `cfg.save()`. Show a "Recent" group pinned at the top of the tree.

- [ ] **Step 2:** Build + test → green. Commit: `git commit -m "cwd: configurable picker root + recent dirs MRU"`

---

### Task 7: Final verification

- [ ] `cargo build --offline && cargo clippy --offline --all-targets && cargo test --offline` → builds, 0 warnings, all pass.
- [ ] Manual (mini): launch `claude` from the store → file-tree picker opens at `~`; arrow/expand to a project, Enter → claude starts in that dir.
- [ ] Commit fixups.

## Notes
- Directories only; no file selection, no dir creation (YAGNI).
- Listing is synchronous local-FS; no async/timeout in v1.
