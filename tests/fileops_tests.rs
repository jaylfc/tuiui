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
