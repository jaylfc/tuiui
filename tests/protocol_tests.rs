use tuiui::session::ClientMsg;
use tuiui::protocol::{FrameMsg, Flags};
use tuiui::compositor::CellChange;
use tuiui::cell::{Cell, Rgba};
use tuiui::geometry::Point;

#[test]
fn client_msg_roundtrips_json() {
    for msg in [
        ClientMsg::MouseDown(Point::new(3, 4)),
        ClientMsg::Key(vec![1, 2, 3]),
        ClientMsg::Resize { w: 80, h: 24 },
        ClientMsg::StoreChar('z'),
        ClientMsg::Shutdown,
    ] {
        let s = serde_json::to_string(&msg).unwrap();
        let _back: ClientMsg = serde_json::from_str(&s).unwrap();
    }
}

#[test]
fn frame_msg_roundtrips_json() {
    let f = FrameMsg {
        changes: vec![CellChange { x: 1, y: 2, cell: Cell { ch: 'A', fg: Rgba::rgb(1,2,3), bg: Rgba::rgb(4,5,6), attrs: Default::default() } }],
        cursor: Some(Point::new(5, 6)),
        flags: Flags { launcher_open: true, ..Default::default() },
        images: Vec::new(),
        image_data: Vec::new(),
    };
    let s = serde_json::to_string(&f).unwrap();
    let back: FrameMsg = serde_json::from_str(&s).unwrap();
    assert_eq!(back.changes.len(), 1);
    assert_eq!(back.changes[0].cell.ch, 'A');
    assert!(back.flags.launcher_open);
}

#[test]
fn new_mouse_messages_roundtrip() {
    use tuiui::session::ClientMsg;
    use tuiui::geometry::Point;
    for msg in [ClientMsg::MouseDouble(Point::new(3, 4)), ClientMsg::MouseRightDown(Point::new(5, 6))] {
        let s = serde_json::to_string(&msg).unwrap();
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), s);
    }
}

#[test]
fn launcher_left_right_roundtrip() {
    use tuiui::session::ClientMsg;
    for msg in [ClientMsg::LauncherLeft, ClientMsg::LauncherRight] {
        let s = serde_json::to_string(&msg).unwrap();
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), s);
    }
}
