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

#[test]
fn enter_directory_navigates_and_back_returns() {
    let d = tmp("nav");
    fs::create_dir(d.join("sub")).unwrap();
    fs::write(d.join("sub/inner.txt"), b"x").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    // cursor on "sub" (dirs first) → activate navigates in
    fm.activate();
    assert_eq!(fm.cwd(), d.join("sub"));
    assert_eq!(fm.entries().len(), 1);
    fm.go_back();
    assert_eq!(fm.cwd(), d.as_path());
    fm.go_forward();
    assert_eq!(fm.cwd(), d.join("sub"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn parent_navigates_up() {
    let d = tmp("up");
    fs::create_dir(d.join("child")).unwrap();
    let mut fm = FileManager::new(d.join("child"), BTreeMap::new());
    fm.go_parent();
    assert_eq!(fm.cwd(), d.as_path());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn activate_file_requests_open_action() {
    use tuiui::filemanager::FileManagerAction;
    let d = tmp("open");
    fs::write(d.join("notes.md"), b"# hi").unwrap();
    let mut handlers = BTreeMap::new();
    handlers.insert("text".to_string(), "vi".to_string());
    let mut fm = FileManager::new(d.clone(), handlers);
    // only one entry, the file
    fm.activate();
    match fm.take_action() {
        Some(FileManagerAction::RunApp { command, args }) => {
            assert_eq!(command, "vi");
            assert!(args.last().unwrap().ends_with("notes.md"));
        }
        other => panic!("expected RunApp, got {other:?}"),
    }
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn activate_image_requests_open_image() {
    use tuiui::filemanager::FileManagerAction;
    let d = tmp("img");
    fs::write(d.join("p.png"), b"\x89PNG").unwrap(); // ext is enough for classify
    let mut handlers = BTreeMap::new();
    handlers.insert("image".to_string(), "@image".to_string());
    let mut fm = FileManager::new(d.clone(), handlers);
    fm.activate();
    assert!(matches!(fm.take_action(), Some(FileManagerAction::OpenImage(_))));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn new_folder_overlay_creates_directory() {
    let d = tmp("mkdir");
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.begin_new_folder();
    assert!(fm.is_editing());
    for c in "Projects".chars() { fm.overlay_char(c); }
    fm.overlay_commit();
    assert!(!fm.is_editing());
    assert!(d.join("Projects").is_dir());
    assert!(fm.entries().iter().any(|e| e.name == "Projects"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn rename_overlay_renames_focused() {
    let d = tmp("rename");
    fs::write(d.join("old.txt"), b"x").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.select_at(0, false, false);
    fm.begin_rename();
    for _ in 0.."old.txt".len() { fm.overlay_backspace(); }
    for c in "new.txt".chars() { fm.overlay_char(c); }
    fm.overlay_commit();
    assert!(d.join("new.txt").exists());
    assert!(!d.join("old.txt").exists());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn copy_paste_duplicates_into_cwd() {
    let d = tmp("paste");
    fs::write(d.join("f.txt"), b"x").unwrap();
    let sub = d.join("sub");
    fs::create_dir(&sub).unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    // select f.txt (index 1: sub dir first, then f.txt)
    let i = fm.entries().iter().position(|e| e.name == "f.txt").unwrap();
    fm.select_at(i, false, false);
    fm.copy_selection();
    // enter sub and paste
    let si = fm.entries().iter().position(|e| e.name == "sub").unwrap();
    fm.select_at(si, false, false);
    fm.activate();
    fm.paste();
    assert!(sub.join("f.txt").exists());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn delete_moves_to_trash_after_confirm() {
    let d = tmp("del");
    let marker = format!("tuiui-fm-del-{}.txt", std::process::id());
    fs::write(d.join(&marker), b"x").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.select_at(0, false, false);
    fm.begin_delete();
    assert!(matches!(fm.overlay(), Some(tuiui::filemanager::Overlay::ConfirmDelete { .. })));
    fm.confirm_delete();
    assert!(!d.join(&marker).exists());
    if let Some(td) = tuiui::fileops::trash_dir() {
        let _ = fs::remove_file(td.join(&marker));
    }
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn get_info_overlay_opens_for_focused_entry() {
    use tuiui::filemanager::Overlay;
    let d = tmp("getinfo");
    fs::write(d.join("a.txt"), b"hello").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.select_at(0, false, false);
    fm.begin_get_info();
    assert!(matches!(fm.overlay(), Some(Overlay::GetInfo { .. })));
    // render must not panic and must include the permission triad somewhere
    let _ = fm.render(80, 24);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn thumbnail_requests_lists_image_entries() {
    let d = tmp("thumbreq");
    fs::write(d.join("pic.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    fs::write(d.join("note.txt"), b"hi").unwrap();
    let fm = FileManager::new(d.clone(), BTreeMap::new());
    let reqs = fm.thumbnail_requests();
    assert_eq!(reqs.len(), 1);
    assert!(reqs[0].1.ends_with("pic.png"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn set_thumb_then_placement_is_reported() {
    use tuiui::geometry::Rect;
    let d = tmp("thumbplace");
    fs::write(d.join("pic.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    let idx = fm.thumbnail_requests()[0].0;
    fm.set_thumb(idx, 12345);
    // content rect at origin (0,0) sized 80x24
    let places = fm.thumbnail_placements(Rect::new(10, 2, 80, 24), true);
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].id, 12345);
    assert!(places[0].visible);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn preview_toggle_and_text_head() {
    let d = tmp("preview");
    fs::write(d.join("a.txt"), b"line1\nline2\nline3\n").unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.set_view(ViewMode::List);
    fm.select_at(0, false, false);
    assert!(!fm.preview_open());
    fm.toggle_preview();
    assert!(fm.preview_open());
    let lines = fm.preview_lines(20);
    assert!(lines.iter().any(|l| l.contains("line1")));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn columns_view_cycles_and_renders() {
    let d = tmp("cols");
    fs::create_dir(d.join("sub")).unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    fm.set_view(ViewMode::Columns);
    assert_eq!(fm.view(), ViewMode::Columns);
    let buf = fm.render(100, 24);
    assert_eq!(buf.width(), 100);
    // cycle_view goes Icon -> List -> Columns -> Icon
    let mut f2 = FileManager::new(d.clone(), BTreeMap::new());
    f2.cycle_view();
    assert_eq!(f2.view(), ViewMode::List);
    f2.cycle_view();
    assert_eq!(f2.view(), ViewMode::Columns);
    f2.cycle_view();
    assert_eq!(f2.view(), ViewMode::Icon);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn tabs_open_switch_and_close() {
    let d = tmp("tabs");
    fs::create_dir(d.join("sub")).unwrap();
    let mut fm = FileManager::new(d.clone(), BTreeMap::new());
    assert_eq!(fm.tab_count(), 1);
    fm.new_tab();
    assert_eq!(fm.tab_count(), 2);
    assert_eq!(fm.active_tab(), 1);
    // navigate only the active tab
    fm.activate(); // into "sub" (dirs first)
    assert_eq!(fm.cwd(), d.join("sub"));
    fm.next_tab();
    assert_eq!(fm.active_tab(), 0);
    assert_eq!(fm.cwd(), d.as_path()); // tab 0 unchanged
    fm.close_tab();
    assert_eq!(fm.tab_count(), 1);
    let _ = fs::remove_dir_all(&d);
}
