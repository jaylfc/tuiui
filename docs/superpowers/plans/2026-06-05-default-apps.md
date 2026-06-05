# Default Apps (B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A configurable file-type → app resolver ("open this `.png` with…") plus a Settings → Default Apps panel, so the file manager (built next) can open files like a real OS.

**Architecture:** A standalone `openwith` module classifies a path into a `Role` (extension table first, `infer`/`mime_guess` fallback) and resolves the role to an `OpenAction` using a config-backed handler map with OS-aware defaults. A new Settings section edits the map.

**Tech Stack:** Rust; `infer` (magic-byte sniffing) + `mime_guess` (extension→MIME); existing config/settings.

**Reference spec:** `docs/superpowers/specs/2026-06-05-file-manager-default-apps-design.md` (sections "B — Default Apps engine" and "Settings → Default Apps").

---

## File Structure

- **Create `src/openwith.rs`** — `Role`, `classify`, `OpenAction`, `resolve`, `candidates`, default handler map.
- **Modify `src/config.rs`** — `default_apps: BTreeMap<String, String>` field + OS-aware defaults.
- **Modify `src/settings.rs`** — a "Default Apps" section editing the map.
- **Modify `src/lib.rs`** — `pub mod openwith;`.
- **Modify `Cargo.toml`** — `infer`, `mime_guess`.
- **Tests:** `tests/openwith_tests.rs`.

This plan produces a working, testable resolver + Settings panel. The file manager (Plan 2) consumes `openwith::resolve`.

---

### Task 1: Deps + `Role` classification

**Files:** `Cargo.toml`, Create `src/openwith.rs`, `src/lib.rs`; Test `tests/openwith_tests.rs`.

- [ ] **Step 1: Add deps**

`Cargo.toml` `[dependencies]`: `infer = "0.16"` and `mime_guess = "2"`.

- [ ] **Step 2: Write the failing test** (`tests/openwith_tests.rs`):

```rust
use std::path::Path;
use tuiui::openwith::{classify, Role};

#[test]
fn classifies_by_extension() {
    assert_eq!(classify(Path::new("/x/cat.png"), false), Role::Image);
    assert_eq!(classify(Path::new("/x/a.JPG"), false), Role::Image); // case-insensitive
    assert_eq!(classify(Path::new("/x/notes.md"), false), Role::Text);
    assert_eq!(classify(Path::new("/x/main.rs"), false), Role::Code);
    assert_eq!(classify(Path::new("/x/song.mp3"), false), Role::Audio);
    assert_eq!(classify(Path::new("/x/clip.mp4"), false), Role::Video);
    assert_eq!(classify(Path::new("/x/pack.zip"), false), Role::Archive);
    assert_eq!(classify(Path::new("/x/doc.pdf"), false), Role::Pdf);
}

#[test]
fn directory_and_unknown() {
    assert_eq!(classify(Path::new("/x/folder"), true), Role::Directory);
    assert_eq!(classify(Path::new("/x/mystery"), false), Role::Other);
}
```

- [ ] **Step 3: Run → FAIL** (`cargo test --offline --test openwith_tests`; one network fetch for the crates, then offline).

- [ ] **Step 4: Implement `src/openwith.rs`**

```rust
//! The Default Apps engine: classify a file into a `Role` and resolve the role to
//! an action (open with a builtin viewer, run a configured app, or show a menu).

use std::path::Path;

/// A coarse file category used to pick a default application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Image,
    Video,
    Audio,
    Text,
    Code,
    Archive,
    Pdf,
    Directory,
    Executable,
    Other,
}

impl Role {
    /// The config key for this role's handler.
    pub fn key(self) -> &'static str {
        match self {
            Role::Image => "image",
            Role::Video => "video",
            Role::Audio => "audio",
            Role::Text => "text",
            Role::Code => "code",
            Role::Archive => "archive",
            Role::Pdf => "pdf",
            Role::Directory => "directory",
            Role::Executable => "executable",
            Role::Other => "other",
        }
    }
}

/// Classify `path` into a `Role`. `is_dir` is passed in (the caller already
/// stat'd the entry). Uses the extension first, then magic-byte sniffing.
pub fn classify(path: &Path, is_dir: bool) -> Role {
    if is_dir {
        return Role::Directory;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if let Some(r) = role_for_ext(&ext) {
        return r;
    }
    // Magic-byte fallback for extension-less / mislabeled files.
    if let Ok(Some(kind)) = infer::get_from_path(path) {
        if let Some(r) = role_for_mime(kind.mime_type()) {
            return r;
        }
    }
    Role::Other
}

fn role_for_ext(ext: &str) -> Option<Role> {
    Some(match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "ico" => Role::Image,
        "mp4" | "mkv" | "mov" | "webm" | "avi" => Role::Video,
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "opus" => Role::Audio,
        "md" | "txt" | "log" | "json" | "toml" | "yaml" | "yml" | "csv" | "ini" | "conf" => Role::Text,
        "rs" | "py" | "js" | "ts" | "go" | "c" | "h" | "cpp" | "hpp" | "java" | "rb" | "sh" | "lua" | "html" | "css" => Role::Code,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" => Role::Archive,
        "pdf" => Role::Pdf,
        _ => return None,
    })
}

fn role_for_mime(mime: &str) -> Option<Role> {
    let top = mime.split('/').next().unwrap_or("");
    Some(match top {
        "image" => Role::Image,
        "video" => Role::Video,
        "audio" => Role::Audio,
        "text" => Role::Text,
        _ => match mime {
            "application/pdf" => Role::Pdf,
            "application/zip" | "application/gzip" | "application/x-tar" => Role::Archive,
            _ => return None,
        },
    })
}
```

Register in `src/lib.rs`: `pub mod openwith;`.

- [ ] **Step 5: Run → PASS. Commit:**

```bash
git add Cargo.toml Cargo.lock src/openwith.rs src/lib.rs tests/openwith_tests.rs
git commit -m "openwith: Role classification (extension + MIME fallback)"
```

---

### Task 2: `OpenAction` + handler map + `resolve`

**Files:** `src/openwith.rs`; Test `tests/openwith_tests.rs` (append).

- [ ] **Step 1: Write the failing test** (append):

```rust
use std::collections::BTreeMap;
use tuiui::openwith::{resolve, OpenAction};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn resolve_routes_by_handler() {
    let m = map(&[("image", "@image"), ("text", "vi"), ("directory", "@navigate")]);
    assert_eq!(resolve(Path::new("/x/cat.png"), false, &m), OpenAction::Builtin("@image"));
    assert_eq!(resolve(Path::new("/x/folder"), true, &m),
               OpenAction::Builtin("@navigate"));
    assert_eq!(
        resolve(Path::new("/x/notes.md"), false, &m),
        OpenAction::RunApp { command: "vi".into(), args: vec!["/x/notes.md".into()] }
    );
}

#[test]
fn unknown_or_unset_handler_is_menu() {
    let m = map(&[]); // nothing configured
    assert_eq!(resolve(Path::new("/x/mystery"), false, &m), OpenAction::OpenWithMenu);
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement** — append to `src/openwith.rs`:

```rust
use std::collections::BTreeMap;

/// What to do when opening a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenAction {
    /// A directory the file manager should navigate into.
    Navigate,
    /// A built-in tuiui viewer, e.g. "@image".
    Builtin(&'static str),
    /// Launch a terminal app with the file path appended.
    RunApp { command: String, args: Vec<String> },
    /// No default — let the user pick.
    OpenWithMenu,
}

/// Resolve how to open `path`, given the configured `[default_apps]` map.
pub fn resolve(path: &Path, is_dir: bool, handlers: &BTreeMap<String, String>) -> OpenAction {
    let role = classify(path, is_dir);
    if role == Role::Directory {
        // Directories always navigate (the handler may still say "@navigate").
        return OpenAction::Navigate;
    }
    match handlers.get(role.key()).map(String::as_str) {
        Some("@image") => OpenAction::Builtin("@image"),
        Some("@navigate") => OpenAction::Navigate,
        Some(cmd) if !cmd.is_empty() => {
            let mut parts = cmd.split_whitespace().map(String::from);
            let program = parts.next().unwrap_or_default();
            let mut args: Vec<String> = parts.collect();
            args.push(path.to_string_lossy().to_string());
            OpenAction::RunApp { command: program, args }
        }
        _ => OpenAction::OpenWithMenu,
    }
}

/// The default handler map for a fresh config (OS-aware where it matters).
pub fn default_handlers() -> BTreeMap<String, String> {
    let editor = std::env::var("EDITOR").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| "vi".into());
    let mut m = BTreeMap::new();
    m.insert("image".into(), "@image".into());
    m.insert("directory".into(), "@navigate".into());
    m.insert("text".into(), editor.clone());
    m.insert("code".into(), editor.clone());
    // audio/video/archive/pdf/other left unset → Open-with menu until the user picks.
    // OS roles used by shortcuts:
    m.insert("editor".into(), editor);
    m.insert("terminal".into(), std::env::var("SHELL").unwrap_or_else(|_| "bash".into()));
    m
}

/// Candidate handlers offered in the Settings → Default Apps chooser for a role.
pub fn candidates(role: Role) -> Vec<String> {
    let mut v = vec![String::new()]; // "" = Open-with menu / unset
    match role {
        Role::Image => v.push("@image".into()),
        Role::Directory => v.push("@navigate".into()),
        _ => {}
    }
    // Common terminal apps detected on PATH.
    for app in ["vi", "vim", "nvim", "nano", "micro", "hx", "emacs", "less", "bat", "mpv", "feh"] {
        if crate::catalog::is_installed(app) {
            v.push(app.to_string());
        }
    }
    v
}
```

- [ ] **Step 4: Run → PASS. Commit:**

```bash
git add src/openwith.rs tests/openwith_tests.rs
git commit -m "openwith: OpenAction + resolve + default/candidate handlers"
```

---

### Task 3: Config `default_apps`

**Files:** `src/config.rs`; Test `tests/config_tests.rs` (append).

- [ ] **Step 1: Write the failing test** (append):

```rust
#[test]
fn default_apps_has_builtin_image_handler() {
    let c = Config::default();
    assert_eq!(c.default_apps.get("image").map(String::as_str), Some("@image"));
    assert_eq!(c.default_apps.get("directory").map(String::as_str), Some("@navigate"));
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --test config_tests`).

- [ ] **Step 3: Implement** — in `src/config.rs`:

Add the field to `Config`:

```rust
    /// File-role → handler map for opening files (see `openwith`).
    #[serde(default)]
    pub default_apps: std::collections::BTreeMap<String, String>,
```

In `Default for Config`, set it: `default_apps: crate::openwith::default_handlers(),`.

- [ ] **Step 4: Run → PASS. Commit:**

```bash
git add src/config.rs tests/config_tests.rs
git commit -m "openwith: config [default_apps] map with builtin defaults"
```

---

### Task 4: Settings → Default Apps panel

**Files:** `src/settings.rs`; inline test in `src/settings.rs`.

- [ ] **Step 1: Write the failing inline test** (append to the `mod tests` in `src/settings.rs`):

```rust
    #[test]
    fn default_apps_section_cycles_handler() {
        let mut s = Settings::new(Config::default());
        while SECTIONS[s.section] != "Default Apps" {
            s.next_section();
        }
        // Row 0 is the first role; cycling changes its handler in the config.
        s.sel = 0;
        let before = s.config().default_apps.clone();
        s.right();
        assert_ne!(s.config().default_apps, before, "cycling changed a handler");
    }
```

- [ ] **Step 2: Run → FAIL** (`cargo test --offline --lib settings`).

- [ ] **Step 3: Implement** — in `src/settings.rs`:

Add `"Default Apps"` to `SECTIONS` (before `"About"`):

```rust
const SECTIONS: &[&str] = &["Windows", "Appearance", "Updates", "Apps", "Default Apps", "About"];
```

Define the roles shown, as a const:

```rust
/// Roles listed in the Default Apps section (config key, label).
const DEFAULT_APP_ROLES: &[(&str, &str)] = &[
    ("image", "Images"),
    ("text", "Text"),
    ("code", "Code"),
    ("audio", "Audio"),
    ("video", "Video"),
    ("pdf", "PDF"),
    ("archive", "Archives"),
    ("editor", "Editor"),
    ("terminal", "Terminal"),
];
```

In `item_count`, add the new section (it is index 4; "About" becomes the `_` arm):

```rust
            4 => DEFAULT_APP_ROLES.len(), // Default Apps
```

In `adjust`, handle the Default Apps section (cycle the selected role's handler through `openwith::candidates`):

```rust
            (4, i) => {
                if let Some((key, _)) = DEFAULT_APP_ROLES.get(i) {
                    let role = role_from_key(key);
                    let cands = crate::openwith::candidates(role);
                    let cur = self.cfg.default_apps.get(*key).cloned().unwrap_or_default();
                    let idx = cands.iter().position(|c| c == &cur).unwrap_or(0);
                    let next = match dir {
                        -1 => (idx + cands.len() - 1) % cands.len(),
                        _ => (idx + 1) % cands.len(),
                    };
                    let val = cands[next].clone();
                    if val.is_empty() {
                        self.cfg.default_apps.remove(*key);
                    } else {
                        self.cfg.default_apps.insert((*key).to_string(), val);
                    }
                }
            }
```

Render the section (add a `4 =>` arm in `render`):

```rust
            4 => {
                for (i, (key, label)) in DEFAULT_APP_ROLES.iter().enumerate() {
                    let val = self.cfg.default_apps.get(*key).cloned().unwrap_or_else(|| "(ask)".into());
                    let shown = match val.as_str() { "@image" => "image viewer".into(), "@navigate" => "open folder".into(), v => v.to_string() };
                    self.row(&mut buf, cx, 3 + i as i32, i, label, format!("\u{25C2} {} \u{25B8}", shown));
                }
            }
```

Add the helper near the bottom of `settings.rs`:

```rust
fn role_from_key(key: &str) -> crate::openwith::Role {
    use crate::openwith::Role::*;
    match key {
        "image" => Image, "video" => Video, "audio" => Audio, "text" => Text,
        "code" => Code, "archive" => Archive, "pdf" => Pdf, _ => Other,
    }
}
```

(The `editor`/`terminal` rows resolve to `Other` candidates — generic app list — which is fine.)

- [ ] **Step 4: Run → PASS; build + full tests + clippy.**

Run: `cargo build --offline && cargo test --offline && cargo clippy --offline --all-targets`
Expected: green, 0 warnings.

- [ ] **Step 5: Commit:**

```bash
git add src/settings.rs
git commit -m "openwith: Settings > Default Apps panel (cycle role handlers)"
```

---

### Task 5: Final verification

- [ ] `cargo build --offline && cargo clippy --offline --all-targets && cargo test --offline` → green, 0 warnings.
- [ ] Confirm `openwith::resolve` is public and returns the four `OpenAction` variants (the file manager plan depends on it).
- [ ] Reinstall to the mini: `cargo install --path . --root ~/.local --force`; restart and check **Settings → Default Apps** shows the roles and cycling persists to `config.toml`.

## Notes for the implementer
- `resolve` is the public seam Plan 2 (File Manager) calls; keep its signature `(path, is_dir, &BTreeMap<String,String>) -> OpenAction`.
- Directories always return `Navigate` regardless of handler, so the FM never tries to "run" a folder.
- Unset handlers → `OpenWithMenu`, so the FM shows a chooser rather than failing.
