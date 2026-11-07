use super::*;

#[test]
fn everything_is_off_by_default() {
    let s = AircraftSettings::default();
    assert!(!s.gimbal_control);
    assert!(!s.avata360);
}

#[test]
fn a_file_from_before_these_settings_has_them_off() {
    let s: crate::settings::Settings = serde_json::from_str(r#"{"input": {}}"#).unwrap();
    assert_eq!(s.aircraft, AircraftSettings::default());

    let s: AircraftSettings = serde_json::from_str(r#"{"gimbal_control": true}"#).unwrap();
    assert!(s.gimbal_control);
    assert!(!s.avata360);
}
