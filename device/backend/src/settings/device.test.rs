use super::*;

#[test]
fn accepts_usable_device_names() {
    for name in ["ystreamer", "pi-5", "A1", "x", &"a".repeat(63)] {
        assert!(validate_hostname(name).is_ok(), "{name:?} refused");
    }
}

#[test]
fn rejects_names_the_network_would_not_take() {
    for name in [
        "",
        "-pi",
        "pi-",
        "my pi",
        "pi.local",
        "pi_5",
        "päron",
        &"a".repeat(64),
    ] {
        assert!(validate_hostname(name).is_err(), "{name:?} got through");
    }
}

#[test]
fn accepts_timezone_names() {
    for tz in [
        "Etc/UTC",
        "UTC",
        "Europe/Madrid",
        "America/Argentina/Buenos_Aires",
        "America/Port-au-Prince",
        "Etc/GMT+3",
    ] {
        assert!(validate_timezone(tz).is_ok(), "{tz:?} refused");
    }
}

#[test]
fn rejects_what_is_not_a_zone_name() {
    for tz in [
        "",
        "/etc/passwd",
        "../../etc/passwd",
        "Europe/../../etc",
        "Europe//Madrid",
        "Europe/Madrid/",
        "Europe/.hidden",
        "Europe/Madrid --adjust-system-clock",
        "Europe/Mädrid",
        "--help",
        "Europe/-x",
        &"a".repeat(65),
    ] {
        assert!(validate_timezone(tz).is_err(), "{tz:?} got through");
    }
}

#[test]
fn reads_the_zone_from_the_localtime_link() {
    assert_eq!(
        zone_from_path("/usr/share/zoneinfo/Europe/Madrid").as_deref(),
        Some("Europe/Madrid")
    );
    assert_eq!(
        zone_from_path("../usr/share/zoneinfo/Etc/UTC").as_deref(),
        Some("Etc/UTC")
    );
    // macOS, for development
    assert_eq!(
        zone_from_path("/var/db/timezone/zoneinfo/Europe/Kyiv").as_deref(),
        Some("Europe/Kyiv")
    );
    assert_eq!(zone_from_path("/etc/somewhere-else"), None);
}
