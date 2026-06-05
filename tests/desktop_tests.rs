use std::collections::BTreeMap;
use std::fs;
use tuiui::config::AppEntry;
use tuiui::desktop::{DesktopAction, DesktopIcons, IconSource};
use tuiui::geometry::Point;

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tuiui-dt-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn pins() -> Vec<AppEntry> {
    vec![AppEntry { name: "Files".into(), command: "@files".into(), args: vec![], category: None, requires_cwd: None, cwd: None }]
}

#[test]
fn merges_folder_entries_and_pins() {
    let d = tmp("merge");
    fs::write(d.join("notes.md"), b"x").unwrap();
    fs::create_dir(d.join("proj")).unwrap();
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&pins(), &BTreeMap::new());
    dt.layout(100, 30); // assign cells
    // 2 folder items + 1 pin = 3 icons
    assert_eq!(dt.icons().len(), 3);
    assert!(dt.icons().iter().any(|i| i.label == "Files" && matches!(i.source, IconSource::Pinned)));
    assert!(dt.icons().iter().any(|i| i.label == "proj" && matches!(i.source, IconSource::Folder)));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn saved_position_wins_else_first_free_cell() {
    let d = tmp("pos");
    fs::write(d.join("a"), b"").unwrap();
    fs::write(d.join("b"), b"").unwrap();
    let mut pos = BTreeMap::new();
    pos.insert(d.join("b").to_string_lossy().to_string(), (2u16, 1u16));
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&[], &pos);
    dt.layout(100, 30);
    let b = dt.icons().iter().find(|i| i.label == "b").unwrap();
    assert_eq!(b.cell, (2, 1));
    // a has no saved position → first free cell (0,0)
    let a = dt.icons().iter().find(|i| i.label == "a").unwrap();
    assert_eq!(a.cell, (0, 0));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn hit_test_and_select_then_double_click_opens() {
    let d = tmp("hit");
    fs::create_dir(d.join("proj")).unwrap();
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&[], &BTreeMap::new());
    dt.layout(100, 30);
    // proj is at cell (0,0): tile origin (0*14, 1+0*3) = (0,1); glyph row y=1
    let p = Point::new(2, 1);
    assert_eq!(dt.icon_at(p), Some(0));
    assert!(dt.icon_at(Point::new(60, 20)).is_none()); // empty desktop
    dt.click(p, false);
    assert_eq!(dt.selection(), vec![0]);
    dt.double_click(p);
    assert_eq!(dt.take_action(), Some(DesktopAction::Open(d.join("proj"))));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn double_click_pin_runs_command() {
    let d = tmp("pin");
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&pins(), &BTreeMap::new());
    dt.layout(100, 30);
    let idx = dt.icons().iter().position(|i| i.label == "Files").unwrap();
    // place a click on that icon's cell
    let (col, row) = dt.icons()[idx].cell;
    let p = Point::new((col as i32) * 14 + 1, 1 + (row as i32) * 3);
    dt.double_click(p);
    assert_eq!(dt.take_action(), Some(DesktopAction::Run { command: "@files".into(), args: vec![] }));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn drag_snaps_to_target_cell_and_reports_position() {
    let d = tmp("drag");
    fs::create_dir(d.join("proj")).unwrap();
    let mut dt = DesktopIcons::new(d.clone());
    dt.reload(&[], &BTreeMap::new());
    dt.layout(100, 30);
    // proj starts at (0,0); grab it and drop at a point inside cell (2,1)
    dt.begin_drag(Point::new(2, 1));
    let drop = Point::new(2 * 14 + 3, 1 + 3 + 1); // inside cell (2,1): GRID_TOP + 1*ICON_H + 1
    let moved = dt.end_drag(drop);
    assert!(moved); // a move happened
    let key = dt.icon_key(0).unwrap();
    assert_eq!(dt.icons()[0].cell, (2, 1));
    // the model exposes the position to persist
    assert_eq!(dt.position_of(&key), Some((2, 1)));
    let _ = fs::remove_dir_all(&d);
}
