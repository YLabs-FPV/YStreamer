use super::*;

fn ap(mode: WifiMode, address: &str) -> WifiSettings {
    WifiSettings {
        mode,
        fallback_ap: true,
        ap: WifiApSettings {
            address: address.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn checks_the_fallback_address_in_system_mode() {
    assert!(ap(WifiMode::Unmanaged, "10.0.0").validate().is_err());
    assert!(ap(WifiMode::Unmanaged, "garbage").validate().is_err());
    assert!(ap(WifiMode::Client, "garbage").validate().is_err());
    assert!(ap(WifiMode::Unmanaged, "10.0.0.1").validate().is_ok());
    // Neither on nor a fallback: unused, so not in the way
    let mut off = ap(WifiMode::Unmanaged, "garbage");
    off.fallback_ap = false;
    assert!(off.validate().is_ok());
}

#[test]
fn rejects_unusable_addresses() {
    for bad in [
        "8.8.8.8",
        "10.0.0.0",
        "192.168.4.255",
        "10.0.0.1.5",
        "10.0.0.256",
        "",
        " 10.0.0.1",
    ] {
        assert!(ap(WifiMode::Ap, bad).validate().is_err(), "{bad}");
    }
    for good in ["10.0.0.1", "192.168.4.1", "172.20.1.254"] {
        assert!(ap(WifiMode::Ap, good).validate().is_ok(), "{good}");
    }
}

#[test]
fn country_is_empty_or_a_two_letter_code() {
    let with = |country: &str| WifiSettings {
        country: country.into(),
        ..Default::default()
    };
    assert!(with("").validate().is_ok());
    assert!(with("DE").validate().is_ok());
    assert!(with("de").validate().is_err());
    assert!(with("DEU").validate().is_err());
    assert!(with("D1").validate().is_err());
}
