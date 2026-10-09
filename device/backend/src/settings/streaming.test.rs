use super::*;

mod rtmp {
    use super::*;

    fn rtmp(url: &str, key: &str) -> RtmpSettings {
        RtmpSettings {
            enabled: true,
            url: url.into(),
            key: key.into(),
            silent_audio: true,
        }
    }

    #[test]
    fn joins_url_and_key() {
        assert_eq!(
            rtmp("rtmp://a.rtmp.youtube.com/live2", "abcd-1234").location(),
            "rtmp://a.rtmp.youtube.com/live2/abcd-1234"
        );
        assert_eq!(
            rtmp("rtmps://live-api-s.facebook.com:443/rtmp/", " FB-key ").location(),
            "rtmps://live-api-s.facebook.com:443/rtmp/FB-key"
        );
    }

    #[test]
    fn validates_the_url() {
        assert!(rtmp("rtmp://live.twitch.tv/app", "k").validate().is_ok());
        assert!(rtmp("http://live.twitch.tv/app", "k").validate().is_err());
        assert!(rtmp("rtmp:///app", "k").validate().is_err());
        assert!(rtmp("rtmp://live.twitch.tv", "k").validate().is_err());
        assert!(
            rtmp("rtmp://live.twitch.tv/app", "has space")
                .validate()
                .is_err()
        );
        let mut off = rtmp("nonsense", "");
        off.enabled = false;
        assert!(off.validate().is_ok());
    }
}

mod udp {
    use super::*;

    fn udp(destinations: &str) -> UdpSettings {
        UdpSettings {
            enabled: true,
            format: UdpFormat::Rtp,
            destinations: destinations.into(),
        }
    }

    #[test]
    fn splits_destinations() {
        assert_eq!(
            udp("192.168.1.20:5600, gcs.local:5600\n239.0.0.1:1234").destination_list(),
            ["192.168.1.20:5600", "gcs.local:5600", "239.0.0.1:1234"]
        );
    }

    #[test]
    fn validates_destinations() {
        assert!(udp("192.168.1.20:5600, 239.0.0.1:1234").validate().is_ok());
        assert!(udp("").validate().is_err());
        assert!(udp("192.168.1.20").validate().is_err());
        assert!(udp("192.168.1.20:0").validate().is_err());
        assert!(udp(":5600").validate().is_err());
        assert!(udp(&["10.0.0.1:5600"; 9].join(",")).validate().is_err());
    }
}

mod srt {
    use super::*;

    fn srt(change: impl FnOnce(&mut SrtSettings)) -> Result<(), String> {
        let mut s = SrtSettings::default();
        change(&mut s);
        s.validate()
    }

    fn caller(host: &str) -> Result<(), String> {
        srt(|s| {
            s.mode = SrtMode::Caller;
            s.host = host.into();
        })
    }

    #[test]
    fn the_defaults_are_valid() {
        assert!(SrtSettings::default().validate().is_ok());
    }

    #[test]
    fn a_listener_needs_an_unprivileged_port() {
        assert!(srt(|s| s.port = 1024).is_ok());
        assert!(srt(|s| s.port = 1023).is_err());
        assert!(srt(|s| s.port = 0).is_err());
    }

    #[test]
    fn a_caller_needs_a_bare_address() {
        assert!(caller("192.168.1.20").is_ok());
        assert!(caller("obs.example.com").is_ok());
        assert!(caller("").is_err());
        assert!(caller("   ").is_err());
        assert!(caller("srt://192.168.1.20").is_err());
        assert!(caller("192.168.1.20?streamid=x").is_err());
        assert!(caller("user@host").is_err());
        assert!(caller("two words").is_err());
    }

    #[test]
    fn a_caller_may_use_a_low_port() {
        assert!(
            srt(|s| {
                s.mode = SrtMode::Caller;
                s.host = "10.0.0.1".into();
                s.port = 443;
            })
            .is_ok()
        );
    }

    #[test]
    fn checks_latency() {
        assert!(srt(|s| s.latency_ms = 20).is_ok());
        assert!(srt(|s| s.latency_ms = 8000).is_ok());
        assert!(srt(|s| s.latency_ms = 19).is_err());
        assert!(srt(|s| s.latency_ms = 8001).is_err());
    }

    #[test]
    fn checks_the_passphrase_length() {
        assert!(srt(|s| s.passphrase = String::new()).is_ok());
        assert!(srt(|s| s.passphrase = "a".repeat(10)).is_ok());
        assert!(srt(|s| s.passphrase = "a".repeat(79)).is_ok());
        assert!(srt(|s| s.passphrase = "a".repeat(9)).is_err());
        assert!(srt(|s| s.passphrase = "a".repeat(80)).is_err());
        // counted in characters, not bytes
        assert!(srt(|s| s.passphrase = "é".repeat(10)).is_ok());
    }

    #[test]
    fn limits_the_stream_id() {
        assert!(srt(|s| s.stream_id = "a".repeat(512)).is_ok());
        assert!(srt(|s| s.stream_id = "a".repeat(513)).is_err());
    }
}

mod rtsp {
    use super::*;

    fn rtsp(change: impl FnOnce(&mut RtspSettings)) -> Result<(), String> {
        let mut s = RtspSettings::default();
        change(&mut s);
        s.validate()
    }

    #[test]
    fn the_defaults_are_valid() {
        assert!(RtspSettings::default().validate().is_ok());
    }

    #[test]
    fn checks_the_port() {
        assert!(rtsp(|s| s.port = 554).is_ok());
        assert!(rtsp(|s| s.port = 1024).is_ok());
        assert!(rtsp(|s| s.port = 80).is_err());
        assert!(rtsp(|s| s.port = 0).is_err());
        assert!(rtsp(|s| s.port = 8080).is_err());
    }

    #[test]
    fn checks_the_path() {
        assert!(rtsp(|s| s.path = "/live/cam-1_a.b".into()).is_ok());
        assert!(rtsp(|s| s.path = "stream".into()).is_err());
        assert!(rtsp(|s| s.path = "/".into()).is_err());
        assert!(rtsp(|s| s.path = String::new()).is_err());
        assert!(rtsp(|s| s.path = "/my stream".into()).is_err());
        assert!(rtsp(|s| s.path = "/a?b".into()).is_err());
    }

    #[test]
    fn a_password_needs_both_halves() {
        let auth = |user: &str, pass: &str| {
            rtsp(|s| {
                s.auth = true;
                s.username = user.into();
                s.password = pass.into();
            })
        };
        assert!(auth("admin", "secret").is_ok());
        assert!(auth("admin", "").is_err());
        assert!(auth("", "secret").is_err());
        assert!(auth("ad:min", "secret").is_err());
    }
}
