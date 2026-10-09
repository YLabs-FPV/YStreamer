use super::*;

fn network(wifi: InterfaceStatus, ethernet: Option<InterfaceStatus>) -> NetworkStatus {
    NetworkStatus {
        wifi,
        ethernet,
        tether: None,
        internet_via: None,
        iphone_support: false,
        image: false,
        fallback_active: false,
        fallback_reason: None,
        fallback_resolved: false,
        applying: false,
        last_error: None,
    }
}

fn access_point() -> (WifiSettings, NetworkStatus) {
    let mut wifi = WifiSettings {
        mode: WifiMode::Ap,
        ..Default::default()
    };
    wifi.ap.ssid = "YStreamer".into();
    wifi.ap.password = "flyfast123".into();
    let status = network(
        InterfaceStatus {
            addresses: vec!["10.42.0.1/24".into()],
            role: Some("ap".into()),
            ssid: Some("YStreamer".into()),
            ..Default::default()
        },
        Some(InterfaceStatus {
            addresses: vec!["192.168.1.20/24".into()],
            ..Default::default()
        }),
    );
    (wifi, status)
}

#[test]
fn band_over_an_image_keeps_what_is_needed_to_get_in() {
    let (wifi, status) = access_point();
    let svg = describe(80, LinkState::Unplugged, &wifi, &status, true)
        .over_image("data:image/png;base64,AAAA", Scaling::Fit);

    assert!(svg.contains(r#"href="data:image/png;base64,AAAA""#));
    assert!(svg.contains("Connect the goggles to the USB-C port"));
    assert!(svg.contains("10.42.0.1"));
    assert!(svg.contains("192.168.1.20"));
    assert!(svg.contains("flyfast123"));
}

#[test]
fn band_over_an_image_leaves_out_the_branding() {
    let (wifi, status) = access_point();
    let svg = describe(80, LinkState::Unplugged, &wifi, &status, true)
        .over_image("data:image/png;base64,AAAA", Scaling::Fit);

    assert!(!svg.contains("SCAN TO JOIN"));
    assert!(!svg.contains(">YStreamer</text>"));
}

#[test]
fn image_under_the_band_is_fitted_or_fills_the_screen() {
    let (wifi, status) = access_point();
    let screen = describe(80, LinkState::Unplugged, &wifi, &status, true);
    assert!(
        screen
            .over_image("data:,", Scaling::Fit)
            .contains("xMidYMid meet")
    );
    assert!(
        screen
            .over_image("data:,", Scaling::Fill)
            .contains("xMidYMid slice")
    );
}

#[test]
fn access_point_password_can_be_kept_off_the_screen() {
    let (wifi, status) = access_point();
    let screen = describe(80, LinkState::Unplugged, &wifi, &status, false);
    for svg in [screen.svg(), screen.over_image("data:,", Scaling::Fit)] {
        assert!(!svg.contains("flyfast123"));
        assert!(!svg.contains("SCAN TO JOIN"));
        // still says which network to join and where to go
        assert!(svg.contains("YStreamer"));
        assert!(svg.contains("10.42.0.1"));
    }
    assert!(
        describe(80, LinkState::Unplugged, &wifi, &status, true)
            .svg()
            .contains("SCAN TO JOIN")
    );
}
