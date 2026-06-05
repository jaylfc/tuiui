use std::path::{Path, PathBuf};
use tuiui::dirpicker::{DirLister, DirPicker, PendingLaunch};

struct Fake;
impl DirLister for Fake {
    fn list_dirs(&self, path: &Path, _hidden: bool) -> Vec<(String, PathBuf)> {
        match path.to_str().unwrap() {
            "/root" => vec![("a".into(), "/root/a".into()), ("b".into(), "/root/b".into())],
            "/root/a" => vec![("x".into(), "/root/a/x".into())],
            _ => vec![],
        }
    }
}

fn picker() -> DirPicker {
    DirPicker::with_lister(PathBuf::from("/root"), PendingLaunch::default(), Box::new(Fake))
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
