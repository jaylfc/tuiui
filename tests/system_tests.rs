use tuiui::system::{bars_glyph, mem_pct, volume_glyph, VolumeInfo};

#[test]
fn signal_bars_fill_left_to_right() {
    assert_eq!(bars_glyph(0), "····");
    assert_eq!(bars_glyph(1), "▮···");
    assert_eq!(bars_glyph(2), "▮▮··");
    assert_eq!(bars_glyph(4), "▮▮▮▮");
    assert_eq!(bars_glyph(9), "▮▮▮▮"); // clamps
}

#[test]
fn volume_glyph_reflects_mute_and_level() {
    assert_eq!(volume_glyph(&VolumeInfo { level: 0, muted: false }), "🔇");
    assert_eq!(volume_glyph(&VolumeInfo { level: 50, muted: false }), "🔉");
    assert_eq!(volume_glyph(&VolumeInfo { level: 90, muted: false }), "🔊");
    assert_eq!(volume_glyph(&VolumeInfo { level: 50, muted: true }), "🔇");
}

#[test]
fn mem_pct_rounds_and_guards_zero() {
    assert_eq!(mem_pct(6, 10), 60);
    assert_eq!(mem_pct(0, 0), 0);
}

#[cfg(target_os = "macos")]
mod macos_argv {
    use tuiui::system::macos::*;

    #[test]
    fn set_volume_builds_osascript_and_clamps() {
        assert_eq!(set_volume_argv(40), vec!["-e".to_string(), "set volume output volume 40".to_string()]);
        assert_eq!(set_volume_argv(250)[1], "set volume output volume 100");
    }
    #[test]
    fn wifi_power_argv_on_off() {
        assert_eq!(wifi_power_argv("en0", true), vec!["-setairportpower", "en0", "on"]);
        assert_eq!(wifi_power_argv("en0", false), vec!["-setairportpower", "en0", "off"]);
    }
    #[test]
    fn wifi_connect_known_argv() {
        assert_eq!(wifi_connect_argv("en0", "HomeNet"), vec!["-setairportnetwork", "en0", "HomeNet"]);
    }
}

#[cfg(target_os = "linux")]
mod linux_argv {
    use tuiui::system::linux::*;

    #[test]
    fn set_volume_builds_wpctl_and_clamps() {
        assert_eq!(set_volume_argv(40), vec!["set-volume", "@DEFAULT_AUDIO_SINK@", "0.40"]);
        assert_eq!(set_volume_argv(250)[2], "1.00");
    }
    #[test]
    fn wifi_radio_argv_on_off() {
        assert_eq!(wifi_radio_argv(true), vec!["radio", "wifi", "on"]);
        assert_eq!(wifi_radio_argv(false), vec!["radio", "wifi", "off"]);
    }
    #[test]
    fn wifi_connect_argv_known() {
        assert_eq!(wifi_connect_argv("HomeNet"), vec!["dev", "wifi", "connect", "HomeNet"]);
    }
}
