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

mod rtmp {
    use super::*;

    fn rtmp(port: u16, key: &str, allowed_from: &str) -> RtmpInputSettings {
        RtmpInputSettings {
            port,
            key: key.into(),
            allowed_from: allowed_from.into(),
        }
    }

    #[test]
    fn the_defaults_are_valid() {
        assert!(RtmpInputSettings::default().validate().is_ok());
    }

    #[test]
    fn made_up_keys_are_valid_and_differ() {
        let a = RtmpInputSettings::new_key();
        let b = RtmpInputSettings::new_key();
        assert_eq!(a.len(), 10);
        assert_ne!(a, b);
        assert!(rtmp(1935, &a, "").validate().is_ok());
    }

    #[test]
    fn checks_port_and_key() {
        assert!(rtmp(1935, "abcd123456", "").validate().is_ok());
        assert!(rtmp(19350, "my-key_2.0-x", "").validate().is_ok());

        assert!(rtmp(554, "abcd123456", "").validate().is_err());
        assert!(rtmp(8080, "abcd123456", "").validate().is_err());
        assert!(rtmp(1935, "abc", "").validate().is_err());
        assert!(rtmp(1935, "abcdefghij", "").validate().is_ok());
        assert!(rtmp(1935, &"a".repeat(65), "").validate().is_err());
        assert!(rtmp(1935, "my key 123", "").validate().is_err());
        assert!(rtmp(1935, "key?x=1234", "").validate().is_err());
    }

    #[test]
    fn short_keys_are_refused_with_a_clear_reason() {
        let error = rtmp(1935, "abcdefghi", "").validate().unwrap_err();
        assert!(error.contains("10-64 characters"), "{error}");
        // Empty is fine: one is made when RTMP is picked
        assert!(rtmp(1935, "", "").validate().is_ok());
    }

    #[test]
    fn reads_the_allowed_addresses() {
        let r = rtmp(1935, "abcd123456", "192.168.1.20, 10.0.0.5 fe80::1");
        assert!(r.validate().is_ok());
        assert_eq!(r.allowed_list().unwrap().len(), 3);

        assert!(rtmp(1935, "abcd123456", "192.168.1").validate().is_err());
        assert!(rtmp(1935, "abcd123456", "phone.local").validate().is_err());
        assert!(
            rtmp(1935, "abcd123456", "192.168.1.0/24")
                .validate()
                .is_err()
        );
        let many = (1..=9).map(|i| format!("10.0.0.{i}")).collect::<Vec<_>>();
        assert!(
            rtmp(1935, "abcd123456", &many.join(","))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn lets_in_only_the_allowed_addresses() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(rtmp(1935, "abcd123456", "").allows(ip("203.0.113.9")));

        let r = rtmp(1935, "abcd123456", "192.168.1.20");
        assert!(r.allows(ip("192.168.1.20")));
        assert!(r.allows(ip("::ffff:192.168.1.20")));
        assert!(!r.allows(ip("192.168.1.21")));
    }
}
