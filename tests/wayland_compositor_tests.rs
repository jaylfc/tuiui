#[cfg(feature = "wayland-compositor")]
mod tests {
    use tuiui::wayland::{WaylandCompositor, LayerType, Anchor, OutputId, SeatId, CompositorState, SeatState, OutputInfo};
    use tuiui::geometry::Point;

    #[test]
    fn compositor_creates_successfully() {
        let comp = WaylandCompositor::new().expect("compositor should create");
        let _ = comp;
    }

    #[test]
    fn compositor_state_manages_outputs() {
        let state = CompositorState::default();
        let info = OutputInfo {
            name: "HDMI-A-1".to_string(),
            width: 1920,
            height: 1080,
            frame_buffer: None,
        };
        state.update_output(OutputId(0), info);
        assert_eq!(state.screen_size(), (1920, 1080));
    }

    #[test]
    fn compositor_state_manages_seats() {
        let state = CompositorState::default();
        let seat = SeatState {
            name: "seat0".to_string(),
            pointer_position: Some(Point::new(100, 100)),
            keyboard_focus: Some(1),
        };
        state.update_seat(SeatId(0), seat);
        assert_eq!(state.seats.lock().unwrap().len(), 1);
    }

    #[test]
    fn layer_types_defined() {
        let _ = LayerType::Background;
        let _ = LayerType::Bottom;
        let _ = LayerType::Top;
        let _ = LayerType::Overlay;
    }

    #[test]
    fn anchor_flags_work() {
        let anchor = Anchor {
            top: true,
            bottom: false,
            left: true,
            right: false,
        };
        assert!(anchor.top);
        assert!(anchor.left);
        assert!(!anchor.bottom);
    }
}