use tuiui::imageview::ImageView;

#[test]
fn placeholder_shows_filename_and_reports_id() {
    let v = ImageView::new("/x/cat.png".into(), Some(7), (64, 48));
    let buf = v.render(30, 8);
    let mut text = String::new();
    for y in 0..buf.height() {
        for x in 0..buf.width() {
            if let Some(c) = buf.get(x, y) {
                text.push(c.ch);
            }
        }
    }
    assert!(text.contains("cat.png"));
    assert_eq!(v.image_id(), Some(7));
}
