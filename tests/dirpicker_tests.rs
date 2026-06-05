use std::path::{Path, PathBuf};
use tuiui::dirpicker::{DirLister, DirPicker, PendingLaunch};

use std::cell::RefCell;

#[derive(Default)]
struct Fake {
    /// Extra children created at runtime, keyed by parent path.
    created: RefCell<std::collections::HashMap<String, Vec<(String, PathBuf)>>>,
}
impl DirLister for Fake {
    fn list_dirs(&self, path: &Path, _hidden: bool) -> Vec<(String, PathBuf)> {
        let mut base = match path.to_str().unwrap() {
            "/root" => vec![("a".into(), "/root/a".into()), ("b".into(), "/root/b".into())],
            "/root/a" => vec![("x".into(), "/root/a/x".into())],
            _ => vec![],
        };
        if let Some(extra) = self.created.borrow().get(path.to_str().unwrap()) {
            base.extend(extra.iter().cloned());
        }
        base
    }
    fn create_dir(&self, parent: &Path, name: &str) -> std::io::Result<PathBuf> {
        let p = parent.join(name);
        self.created
            .borrow_mut()
            .entry(parent.to_string_lossy().to_string())
            .or_default()
            .push((name.to_string(), p.clone()));
        Ok(p)
    }
}

fn picker() -> DirPicker {
    DirPicker::with_lister(PathBuf::from("/root"), PendingLaunch::default(), Box::new(Fake::default()))
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
    p.expand(); // selected (a)
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
fn confirm_returns_pending_and_selected_path() {
    let p = picker();
    let (_pending, path) = p.confirm();
    assert_eq!(path, PathBuf::from("/root/a"));
}

#[test]
fn create_folder_inside_selected_and_selects_it() {
    let mut p = picker();
    // selected = a; create a subfolder "proj" inside it.
    p.begin_create();
    assert!(p.is_creating());
    for c in "proj".chars() {
        p.create_type(c);
    }
    p.commit_create();
    assert!(!p.is_creating());
    // "a" is now expanded with the new "proj" selected.
    assert_eq!(p.selected_path(), PathBuf::from("/root/a/proj"));
    assert!(p.visible().iter().any(|r| r.name == "proj"));
}

#[test]
fn cancel_create_discards_input() {
    let mut p = picker();
    p.begin_create();
    for c in "junk".chars() {
        p.create_type(c);
    }
    p.cancel_create();
    assert!(!p.is_creating());
    assert!(!p.visible().iter().any(|r| r.name == "junk"));
}

#[test]
fn render_produces_a_layer() {
    let p = picker();
    assert!(!p.render(80, 24).is_empty());
}

#[test]
fn row_at_round_trips_with_row_rect() {
    let p = picker();
    let r = p.row_rect(1, 80, 24).unwrap();
    assert_eq!(p.row_at(r.center(), 80, 24), Some(1));
}
