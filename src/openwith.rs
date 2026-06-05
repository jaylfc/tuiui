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
