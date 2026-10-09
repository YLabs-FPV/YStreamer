use super::*;

fn uvc(mode: &str, bitrate_kbps: u32, keyframe_secs: u32) -> UvcSettings {
    UvcSettings {
        mode: mode.into(),
        bitrate_kbps,
        keyframe_secs,
        ..Default::default()
    }
}

#[test]
fn reads_a_camera_mode() {
    assert_eq!(parse_uvc_mode("1280x720@30"), Some((1280, 720, 30)));
    assert_eq!(parse_uvc_mode("1920x1080@60"), Some((1920, 1080, 60)));
}

#[test]
fn rejects_malformed_camera_modes() {
    for mode in [
        "",
        "1280x720",
        "1280@30",
        "1280x@30",
        "x720@30",
        "1280x720@",
        "1280X720@30",
        "1280x720@30fps",
        "-1x720@30",
    ] {
        assert_eq!(parse_uvc_mode(mode), None, "{mode:?} got through");
    }
}

#[test]
fn the_defaults_are_valid() {
    assert!(UvcSettings::default().validate().is_ok());
}

#[test]
fn checks_mode_bitrate_and_keyframe_interval() {
    assert!(uvc("", 6000, 2).validate().is_ok());
    assert!(uvc("1280x720@30", 500, 1).validate().is_ok());
    assert!(uvc("1280x720@30", 20_000, 10).validate().is_ok());

    assert!(uvc("720p", 6000, 2).validate().is_err());
    assert!(uvc("", 499, 2).validate().is_err());
    assert!(uvc("", 20_001, 2).validate().is_err());
    assert!(uvc("", 6000, 0).validate().is_err());
    assert!(uvc("", 6000, 11).validate().is_err());
}
