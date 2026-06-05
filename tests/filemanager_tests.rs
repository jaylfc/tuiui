use std::collections::BTreeMap;
use std::fs;
use tuiui::filemanager::{FileManager, ViewMode};

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tuiui-fm-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn new_lists_root_dirs_first() {
    let d = tmp("new");
    fs::create_dir(d.join("sub")).unwrap();
    fs::write(d.join("a.txt"), b"x").unwrap();

    let fm = FileManager::new(d.clone(), BTreeMap::new());
    assert_eq!(fm.cwd(), d.as_path());
    let names: Vec<&str> = fm.entries().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["sub", "a.txt"]);
    assert_eq!(fm.cursor(), 0);
    assert_eq!(fm.view(), ViewMode::Icon);

    let _ = fs::remove_dir_all(&d);
}

#[test]
fn cursor_moves_and_clamps() {
    let d = tmp("cursor");
    fs::write(d.join("a"), b"").unwrap();
    fs::write(d.join("b"), b"").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    assert_eq!(fm.cursor(), 0);
    fm.move_cursor(1, 0); // down one (dx, dy) — see note below
    assert_eq!(fm.cursor(), 1);
    fm.move_cursor(5, 0); // clamps
    assert_eq!(fm.cursor(), 1);
    fm.move_cursor(-9, 0);
    assert_eq!(fm.cursor(), 0);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn selection_single_ctrl_and_clear() {
    let d = tmp("sel");
    for n in ["a", "b", "c"] { fs::write(d.join(n), b"").unwrap(); }
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.select_at(0, false, false); // click a → {0}
    assert_eq!(fm.selection_indices(), vec![0]);
    fm.select_at(2, true, false);  // ctrl-click c → {0,2}
    assert_eq!(fm.selection_indices(), vec![0, 2]);
    fm.select_at(1, false, false); // plain click b → {1}
    assert_eq!(fm.selection_indices(), vec![1]);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn toggle_hidden_reloads() {
    let d = tmp("hidden");
    fs::write(d.join(".dot"), b"").unwrap();
    fs::write(d.join("v"), b"").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    assert_eq!(fm.entries().len(), 1);
    fm.toggle_hidden();
    assert_eq!(fm.entries().len(), 2);
    let _ = fs::remove_dir_all(&d);
}
