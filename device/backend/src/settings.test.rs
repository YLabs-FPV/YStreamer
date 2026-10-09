use super::*;

#[test]
fn saved_file_carries_the_version() {
    let json: serde_json::Value = serde_json::from_str(&serialize(&Settings::default())).unwrap();
    assert_eq!(json["version"], VERSION);
}

#[test]
fn saved_file_reads_back_the_same() {
    let mut settings = Settings::default();
    settings.splash.freeze_last_frame = true;
    assert_eq!(parse(&serialize(&settings)).unwrap(), (settings, VERSION));
}

#[test]
fn file_without_a_version_is_the_first_format() {
    let (settings, version) = parse(r#"{"splash": {"freeze_last_frame": true}}"#).unwrap();
    assert_eq!(version, 1);
    assert!(settings.splash.freeze_last_frame);
}

#[test]
fn file_from_a_newer_release_still_loads() {
    let text =
        r#"{"version": 99, "not_invented_yet": {"a": 1}, "splash": {"freeze_last_frame": true}}"#;
    let (settings, version) = parse(text).unwrap();
    assert_eq!(version, 99);
    assert!(settings.splash.freeze_last_frame);
}

#[test]
fn broken_file_is_an_error() {
    assert!(parse("{").is_err());
}
