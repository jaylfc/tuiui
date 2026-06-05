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
