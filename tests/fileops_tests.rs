use std::fs;
use tuiui::fileops::{FsOps, StdFs};

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tuiui-fo-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn list_sorts_dirs_first_then_name_and_hides_dotfiles() {
    let d = tmp("list");
    fs::create_dir(d.join("zeta")).unwrap();
    fs::write(d.join("alpha.txt"), b"hi").unwrap();
    fs::write(d.join(".secret"), b"x").unwrap();
    fs::create_dir(d.join("apps")).unwrap();

    let fs_ops = StdFs;
    let shown = fs_ops.list(&d, false).unwrap();
    let names: Vec<&str> = shown.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["apps", "zeta", "alpha.txt"]); // dirs first (name-sorted), then files; dotfile hidden

    let all = fs_ops.list(&d, true).unwrap();
    assert!(all.iter().any(|e| e.name == ".secret"));

    let alpha = shown.iter().find(|e| e.name == "alpha.txt").unwrap();
    assert!(!alpha.is_dir);
    assert_eq!(alpha.size, 2);
    assert_eq!(alpha.role, tuiui::openwith::Role::Text);

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn list_of_unreadable_or_missing_dir_is_err_not_panic() {
    let fs_ops = StdFs;
    assert!(fs_ops.list(std::path::Path::new("/no/such/dir/here"), false).is_err());
}

#[test]
fn mkdir_rename_copy_move_roundtrip() {
    let d = tmp("mut");
    let fs_ops = StdFs;

    let made = fs_ops.mkdir(&d, "Projects").unwrap();
    assert!(made.is_dir());

    fs::write(d.join("a.txt"), b"hello").unwrap();
    let renamed = fs_ops.rename(&d.join("a.txt"), "b.txt").unwrap();
    assert!(renamed.ends_with("b.txt"));
    assert!(!d.join("a.txt").exists());

    // copy b.txt into Projects, then copy again → de-duped name
    let c1 = fs_ops.copy(&d.join("b.txt"), &made).unwrap();
    assert_eq!(c1.file_name().unwrap(), "b.txt");
    let c2 = fs_ops.copy(&d.join("b.txt"), &made).unwrap();
    assert_eq!(c2.file_name().unwrap(), "b copy.txt");

    // move b.txt into Projects (now original gone from root)
    let moved = fs_ops.move_to(&d.join("b.txt"), &made).unwrap();
    assert!(moved.exists());
    assert!(!d.join("b.txt").exists());

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn copy_is_recursive_for_directories() {
    let d = tmp("rec");
    let fs_ops = StdFs;
    let src = fs_ops.mkdir(&d, "tree").unwrap();
    fs::create_dir(src.join("sub")).unwrap();
    fs::write(src.join("sub/leaf.txt"), b"x").unwrap();
    let into = fs_ops.mkdir(&d, "dest").unwrap();

    let copied = fs_ops.copy(&src, &into).unwrap();
    assert!(copied.join("sub/leaf.txt").exists());

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn unique_destination_suffixes_extension_correctly() {
    use tuiui::fileops::unique_destination;
    let d = tmp("uniq");
    fs::write(d.join("note.md"), b"x").unwrap();
    let p = unique_destination(&d, "note.md");
    assert_eq!(p.file_name().unwrap(), "note copy.md");

    let p2 = unique_destination(&d, "fresh.md");
    assert_eq!(p2.file_name().unwrap(), "fresh.md"); // no collision → unchanged

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn trash_dir_is_os_appropriate() {
    use tuiui::fileops::trash_dir;
    let p = trash_dir().expect("home dir resolvable in test env");
    let s = p.to_string_lossy();
    if cfg!(target_os = "macos") {
        assert!(s.ends_with("/.Trash"), "macOS trash is ~/.Trash, got {s}");
    } else {
        assert!(s.ends_with("Trash/files"), "Linux trash is XDG Trash/files, got {s}");
    }
}

#[test]
fn trash_moves_file_out_of_source_dir() {
    // Verify trash() removes the source without hard-deleting. We point at the
    // real OS trash but immediately clean up our marker file from it.
    let d = tmp("trash");
    let fs_ops = StdFs;
    let marker = format!("tuiui-trash-marker-{}.txt", std::process::id());
    let victim = d.join(&marker);
    fs::write(&victim, b"bye").unwrap();

    fs_ops.trash(&victim).unwrap();
    assert!(!victim.exists(), "source file should be gone after trashing");

    // Clean our marker out of the real trash so we don't litter.
    if let Some(td) = tuiui::fileops::trash_dir() {
        let _ = fs::remove_file(td.join(&marker));
        let _ = fs::remove_file(td.join(format!("{marker} copy")));
    }
    let _ = fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn info_reports_size_and_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("info");
    let f = d.join("data.bin");
    fs::write(&f, b"abcd").unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(0o640)).unwrap();

    let info = tuiui::fileops::info(&f).unwrap();
    assert_eq!(info.size, 4);
    assert!(!info.is_dir);
    assert!(!info.is_symlink);
    assert_eq!(info.mode & 0o777, 0o640);
    assert_eq!(tuiui::fileops::mode_rwx(info.mode), "rw-r-----");

    let _ = fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn info_follows_symlink_reports_target() {
    let d = tmp("link");
    let target = d.join("real.txt");
    fs::write(&target, b"x").unwrap();
    let link = d.join("alias.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let info = tuiui::fileops::info(&link).unwrap();
    assert!(info.is_symlink);
    assert_eq!(info.link_target.as_deref(), Some(target.as_path()));

    let _ = fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn set_permissions_changes_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("chmod");
    let f = d.join("s.sh");
    fs::write(&f, b"#!/bin/sh\n").unwrap();
    tuiui::fileops::StdFs.set_mode(&f, 0o755).unwrap();
    let m = fs::metadata(&f).unwrap().permissions().mode();
    assert_eq!(m & 0o777, 0o755);
    let _ = fs::remove_dir_all(&d);
}
