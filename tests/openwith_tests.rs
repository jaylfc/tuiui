use std::collections::BTreeMap;
use std::path::Path;
use tuiui::openwith::{classify, resolve, OpenAction, Role};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn resolve_routes_by_handler() {
    let m = map(&[("image", "@image"), ("text", "vi"), ("directory", "@navigate")]);
    assert_eq!(resolve(Path::new("/x/cat.png"), false, &m), OpenAction::Builtin("@image"));
    assert_eq!(resolve(Path::new("/x/folder"), true, &m), OpenAction::Navigate);
    assert_eq!(
        resolve(Path::new("/x/notes.md"), false, &m),
        OpenAction::RunApp { command: "vi".into(), args: vec!["/x/notes.md".into()] }
    );
}

#[test]
fn unknown_or_unset_handler_is_menu() {
    let m = map(&[]); // nothing configured
    assert_eq!(resolve(Path::new("/x/mystery"), false, &m), OpenAction::OpenWithMenu);
}

#[test]
fn classifies_by_extension() {
    assert_eq!(classify(Path::new("/x/cat.png"), false), Role::Image);
    assert_eq!(classify(Path::new("/x/a.JPG"), false), Role::Image); // case-insensitive
    assert_eq!(classify(Path::new("/x/notes.md"), false), Role::Text);
    assert_eq!(classify(Path::new("/x/main.rs"), false), Role::Code);
    assert_eq!(classify(Path::new("/x/song.mp3"), false), Role::Audio);
    assert_eq!(classify(Path::new("/x/clip.mp4"), false), Role::Video);
    assert_eq!(classify(Path::new("/x/pack.zip"), false), Role::Archive);
    assert_eq!(classify(Path::new("/x/doc.pdf"), false), Role::Pdf);
}

#[test]
fn directory_and_unknown() {
    assert_eq!(classify(Path::new("/x/folder"), true), Role::Directory);
    assert_eq!(classify(Path::new("/x/mystery"), false), Role::Other);
}
