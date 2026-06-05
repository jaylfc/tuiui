use tuiui::imagestore::ImageStore;

/// A solid-red PNG of `w × h`, encoded at test time via the `image` crate.
fn red_png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

#[test]
fn load_is_deterministic_and_downscales() {
    let mut s = ImageStore::new();
    let png = red_png(100, 100);
    let id1 = s.load_bytes(&png, 40, 40).unwrap();
    let id2 = s.load_bytes(&png, 40, 40).unwrap();
    assert_eq!(id1, id2, "same input + target → same id");
    let (w, h) = s.dimensions(id1).unwrap();
    assert!(w <= 40 && h <= 40, "downscaled to fit the target box");
    assert!(!s.png_bytes(id1).unwrap().is_empty());
}

#[test]
fn corrupt_bytes_return_none() {
    let mut s = ImageStore::new();
    assert!(s.load_bytes(&[1, 2, 3, 4], 40, 40).is_none());
}
