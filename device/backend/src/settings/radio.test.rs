use super::*;

#[test]
fn off_by_default_with_fcc_ready() {
    let s = RadioRegionSettings::default();
    assert!(!s.enabled);
    assert_eq!(s.region, RadioRegion::Us);
}

#[test]
fn reads_the_region_names() {
    let s: RadioRegionSettings =
        serde_json::from_str(r#"{"enabled": true, "region": "eu"}"#).unwrap();
    assert!(s.enabled);
    assert_eq!(s.region, RadioRegion::Eu);
    let s: RadioRegionSettings = serde_json::from_str(r#"{"region": "id"}"#).unwrap();
    assert!(!s.enabled);
    assert_eq!(s.region, RadioRegion::Id);
}

#[test]
fn refuses_a_region_it_does_not_know() {
    assert!(serde_json::from_str::<RadioRegionSettings>(r#"{"region": "xx"}"#).is_err());
}

#[test]
fn countries() {
    assert_eq!(RadioRegion::Us.country(), Some(*b"US"));
    assert_eq!(RadioRegion::Id.country(), Some(*b"ID"));
    assert_eq!(RadioRegion::Eu.country(), None);
}
