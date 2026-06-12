#[cfg(feature = "wayland-compositor")]
mod tests {
    use tuiui::wayland::{
        WaylandCompositor, LayerType, Anchor, OutputId, SeatId,
        CompositorState, SeatState, OutputInfo,
        InputManager, InputConfig, KeyboardLayout, ModifierState,
    };
    use tuiui::geometry::{Point, Rect};
    use tuiui::input::Action as InputAction;
    use tuiui::window::{Window, WindowId, WindowState};

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
        assert_eq!(state.seat_count(), 1);
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

    fn test_window(id: u64, z: i32) -> Window {
        let rect = Rect::new(0, 0, 20, 10);
        Window {
            id: WindowId(id),
            title: "shell".to_string(),
            rect,
            z,
            state: WindowState::Floating,
            restore_rect: rect,
            minimized: false,
        }
    }

    #[test]
    fn input_manager_creates() {
        let mgr = InputManager::new(InputConfig::default());
        let _ = mgr;
    }

    #[test]
    fn input_manager_enumerates_devices() {
        let mgr = InputManager::new(InputConfig::default());
        let _devices = mgr.devices();
    }

    #[test]
    fn keyboard_layout_conversion() {
        assert_eq!(KeyboardLayout::from_str("us"), KeyboardLayout::Us);
        assert_eq!(KeyboardLayout::from_str("de"), KeyboardLayout::De);
        assert_eq!(KeyboardLayout::from_str("uk"), KeyboardLayout::Uk);
        assert_eq!(KeyboardLayout::Us.as_str(), "us");
        assert_eq!(KeyboardLayout::De.display_name(), "German");
    }

    #[test]
    fn modifier_state_conversion() {
        let mods = ModifierState {
            shift: true,
            ctrl: false,
            alt: true,
            super_key: false,
            caps_lock: false,
        };
        let bits: u32 = (&mods).into();
        assert_eq!(ModifierState::from(bits).shift, true);
        assert_eq!(ModifierState::from(bits).alt, true);
        assert!(!ModifierState::from(bits).ctrl);
    }

    #[test]
    fn seat_data_capabilities() {
        let mut seat = tuiui::wayland::SeatData::new("seat0");
        seat.has_pointer = true;
        seat.has_keyboard = true;
        seat.has_touch = false;
        seat.refresh_capabilities();
        assert_eq!(seat.capabilities, 0b11); // pointer | keyboard
    }

    #[test]
    fn pointer_click_sets_keyboard_focus_for_shortcuts() {
        let mgr = InputManager::new(InputConfig {
            shortcuts: true,
            ..Default::default()
        });
        let windows = vec![test_window(1, 0), test_window(2, 1)];
        let action = mgr.handle_pointer_button(Point::new(5, 5), 0x01, &windows);
        assert!(matches!(action, InputAction::FocusAndForward { id, .. } if id == WindowId(2)));

        let mods = ModifierState { alt: true, ..Default::default() };
        let action = mgr.handle_key(0x71, 0x01, mods);
        assert!(matches!(action, Some(InputAction::Close(WindowId(2)))));
    }

    #[test]
    fn keyboard_shortcuts_trigger() {
        let mgr = InputManager::new(InputConfig {
            shortcuts: true,
            ..Default::default()
        });
        let mods = ModifierState { alt: true, ..Default::default() };
        let act = mgr.handle_key(0x09, 0x01, mods);
        assert!(matches!(act, Some(tuiui::input::Action::BeginFocusCycle)));
    }

    #[test]
    fn keyboard_shortcuts_do_not_trigger_when_disabled() {
        let mgr = InputManager::new(InputConfig {
            shortcuts: false,
            ..Default::default()
        });
        let mods = ModifierState { alt: true, ..Default::default() };
        let act = mgr.handle_key(0x09, 0x01, mods);
        assert!(act.is_none());
    }

    #[test]
    fn vt_switch_handler() {
        let mut h = tuiui::wayland::VtSwitchHandler::new();
        assert!(h.is_active());
        h.vt_changed(1);
        assert!(!h.is_active());
        h.vt_changed(7);
        assert!(h.is_active());
    }
}
