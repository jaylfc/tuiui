use tuiui::kittygfx::{GraphicsTap, GraphicsCmd};

/// Build a Kitty graphics APC: ESC _ G <control> ; <payload> ESC \
fn apc(control: &str, payload: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b_G");
    v.extend_from_slice(control.as_bytes());
    v.push(b';');
    v.extend_from_slice(payload.as_bytes());
    v.extend_from_slice(b"\x1b\\");
    v
}

#[test]
fn splits_graphics_from_text() {
    let mut tap = GraphicsTap::new();
    let mut input = b"hello".to_vec();
    input.extend(apc("a=T,f=100,i=1", "AAAA"));
    input.extend_from_slice(b"world");
    let out = tap.feed(&input);
    assert_eq!(out.passthrough, b"helloworld");
    assert_eq!(out.commands.len(), 1);
    let c = &out.commands[0];
    assert_eq!(c.get('a').as_deref(), Some("T"));
    assert_eq!(c.get('i').as_deref(), Some("1"));
    assert_eq!(c.payload, b"AAAA");
}

#[test]
fn reassembles_apc_split_across_feeds() {
    let mut tap = GraphicsTap::new();
    let full = apc("a=t,i=9", "XYZ");
    let (a, b) = full.split_at(5); // split mid-APC
    let o1 = tap.feed(a);
    assert!(o1.commands.is_empty());
    assert!(o1.passthrough.is_empty()); // APC bytes are withheld, not passed through
    let o2 = tap.feed(b);
    assert_eq!(o2.commands.len(), 1);
    assert_eq!(o2.commands[0].get('i').as_deref(), Some("9"));
}

#[test]
fn non_graphics_apc_passes_through() {
    let mut tap = GraphicsTap::new();
    // ESC _ X ... ESC \  (not a 'G' graphics APC)
    let input = b"\x1b_Xsomething\x1b\\rest".to_vec();
    let out = tap.feed(&input);
    assert!(out.commands.is_empty());
    assert_eq!(out.passthrough, input); // passed through untouched
}

use tuiui::kittygfx::GraphicsState;

fn tiny_png() -> Vec<u8> {
    // 1x1 red PNG, generated via the image crate.
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

#[test]
fn direct_png_transmit_decodes() {
    use base64::Engine;
    let png = tiny_png();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let cmd = tuiui::kittygfx::parse_one(&apc("a=t,f=100,t=d,i=7", &b64));
    let mut st = GraphicsState::new();
    st.apply(&cmd, 0, 0);
    assert!(st.png(7).is_some());
    assert!(image::load_from_memory(st.png(7).unwrap()).is_ok());
}

#[test]
fn raw_rgba_transmit_decodes() {
    use base64::Engine;
    // 2x2 RGBA = 16 bytes
    let raw: Vec<u8> = (0..16).map(|i| i as u8).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    let cmd = tuiui::kittygfx::parse_one(&apc("a=t,f=32,t=d,s=2,v=2,i=3", &b64));
    let mut st = GraphicsState::new();
    st.apply(&cmd, 0, 0);
    assert!(st.png(3).is_some());
}

#[test]
fn chunked_transmit_reassembles() {
    use base64::Engine;
    let png = tiny_png();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let (a, b) = b64.split_at(b64.len() / 2);
    let mut st = GraphicsState::new();
    st.apply(&tuiui::kittygfx::parse_one(&apc("a=t,f=100,t=d,i=5,m=1", a)), 0, 0);
    assert!(st.png(5).is_none()); // not complete yet
    st.apply(&tuiui::kittygfx::parse_one(&apc("i=5,m=0", b)), 0, 0);
    assert!(st.png(5).is_some());
}
