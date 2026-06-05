//! Filesystem listing and mutation behind a testable `FsOps` trait. The real
//! `StdFs` impl talks to disk; the file manager only ever calls through the
//! trait, so its logic is unit-testable and disk I/O is isolated here.

use crate::openwith::{classify, Role};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A single directory entry the file manager displays.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub role: Role,
}

/// All filesystem effects the file manager needs, behind a trait so the UI is
/// testable with a fake and the real impl is the only thing touching disk.
pub trait FsOps {
    /// List `dir`, directories first then files, each group sorted case-insensitively
    /// by name. Hidden (dot) entries are included only when `show_hidden`.
    fn list(&self, dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>>;
    /// Create `name` under `parent`; returns the new path.
    fn mkdir(&self, parent: &Path, name: &str) -> io::Result<PathBuf>;
    /// Rename `path` to `new_name` (same parent); returns the new path.
    fn rename(&self, path: &Path, new_name: &str) -> io::Result<PathBuf>;
    /// Copy `src` into `dst_dir` (recursive for directories), de-duping the name.
    fn copy(&self, src: &Path, dst_dir: &Path) -> io::Result<PathBuf>;
    /// Move `src` into `dst_dir` (rename, falling back to copy+remove across devices).
    fn move_to(&self, src: &Path, dst_dir: &Path) -> io::Result<PathBuf>;
    /// Move `path` to the OS Trash. Never hard-deletes.
    fn trash(&self, path: &Path) -> io::Result<()>;
}

/// The production filesystem implementation.
pub struct StdFs;

impl FsOps for StdFs {
    fn list(&self, dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        for ent in std::fs::read_dir(dir)? {
            let ent = ent?;
            let name = ent.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let meta = ent.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta.as_ref().and_then(|m| m.modified().ok());
            let path = ent.path();
            let role = classify(&path, is_dir);
            out.push(Entry { name, path, is_dir, size, modified, role });
        }
        out.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir) // dirs first (true > false)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    fn mkdir(&self, parent: &Path, name: &str) -> io::Result<PathBuf> {
        let p = parent.join(name);
        std::fs::create_dir(&p)?;
        Ok(p)
    }

    fn rename(&self, path: &Path, new_name: &str) -> io::Result<PathBuf> {
        let parent = path.parent().unwrap_or(Path::new("."));
        let dst = parent.join(new_name);
        std::fs::rename(path, &dst)?;
        Ok(dst)
    }

    fn copy(&self, src: &Path, dst_dir: &Path) -> io::Result<PathBuf> {
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item")
            .to_string();
        let dst = unique_destination(dst_dir, &name);
        copy_recursive(src, &dst)?;
        Ok(dst)
    }

    fn move_to(&self, src: &Path, dst_dir: &Path) -> io::Result<PathBuf> {
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item")
            .to_string();
        let dst = unique_destination(dst_dir, &name);
        match std::fs::rename(src, &dst) {
            Ok(()) => Ok(dst),
            Err(_) => {
                // Cross-device: copy then remove the source.
                copy_recursive(src, &dst)?;
                if src.is_dir() {
                    std::fs::remove_dir_all(src)?;
                } else {
                    std::fs::remove_file(src)?;
                }
                Ok(dst)
            }
        }
    }

    fn trash(&self, path: &Path) -> io::Result<()> {
        let dir = trash_dir().ok_or_else(|| io::Error::other("no trash directory"))?;
        std::fs::create_dir_all(&dir)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item")
            .to_string();
        let dst = unique_destination(&dir, &name);
        match std::fs::rename(path, &dst) {
            Ok(()) => Ok(()),
            Err(_) => {
                copy_recursive(path, &dst)?;
                if path.is_dir() {
                    std::fs::remove_dir_all(path)?;
                } else {
                    std::fs::remove_file(path)?;
                }
                Ok(())
            }
        }
    }
}

/// Recursively copy a file or directory tree from `src` to `dst`.
fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for ent in std::fs::read_dir(src)? {
            let ent = ent?;
            copy_recursive(&ent.path(), &dst.join(ent.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// A non-colliding destination path for `name` inside `dir`: `name`, then
/// `name copy`, `name copy 2`, … (preserving the extension for files).
pub fn unique_destination(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = split_name(name);
    for n in 1..1_000_000 {
        let suffix = if n == 1 { " copy".to_string() } else { format!(" copy {n}") };
        let trial = if ext.is_empty() {
            format!("{stem}{suffix}")
        } else {
            format!("{stem}{suffix}.{ext}")
        };
        let p = dir.join(trial);
        if !p.exists() {
            return p;
        }
    }
    // Astronomically unlikely fallback: append the process id to disambiguate.
    dir.join(format!("{stem}-{}", std::process::id()))
}

/// Split a filename into (stem, extension-without-dot). Dotfiles with no further
/// extension keep their whole name as the stem.
fn split_name(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (name.to_string(), String::new()),
    }
}

/// The OS Trash directory for moved-not-deleted files.
pub fn trash_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    if cfg!(target_os = "macos") {
        Some(home.join(".Trash"))
    } else {
        Some(home.join(".local/share/Trash/files"))
    }
}
