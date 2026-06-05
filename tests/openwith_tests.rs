use std::path::Path;
use tuiui::openwith::{classify, Role};

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
