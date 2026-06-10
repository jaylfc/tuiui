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
        clear: true,
        switch_to: Some(tuiui::systems::SwitchSpec {
            name: "pi".into(),
            host: "pi@10.0.0.2".into(),
            port: Some(2222),
            theme: Some("nord".into()),
            setup: true,
            password: None,
        }),
    };
    let s = serde_json::to_string(&f).unwrap();
    let back: FrameMsg = serde_json::from_str(&s).unwrap();
    assert_eq!(back.changes.len(), 1);
    assert_eq!(back.changes[0].cell.ch, 'A');
    assert!(back.flags.launcher_open);
    assert!(back.clear);
    let spec = back.switch_to.expect("switch spec survives the round trip");
    assert_eq!(spec.host, "pi@10.0.0.2");
    assert!(spec.setup);
}

#[test]
fn frame_msg_clear_defaults_off_for_older_daemons() {
    // A frame from a daemon that predates `clear` (and `switch_to`) must still
    // parse, with the wipe defaulted off and no switch requested.
    let s = r#"{"changes":[],"cursor":null,"flags":{}}"#;
    let f: FrameMsg = serde_json::from_str(s).unwrap();
    assert!(!f.clear);
    assert!(f.switch_to.is_none());
    assert!(!f.flags.power_editing);
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
