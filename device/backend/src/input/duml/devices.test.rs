use super::*;

fn record(code: &str) -> Vec<u8> {
    let mut r = code.as_bytes().to_vec();
    r.resize(64, 0);
    r
}

#[test]
fn names_the_integra_and_o4_pro() {
    let g = from_identity(0xBC, &record("ZV901")).unwrap();
    assert_eq!(
        (g.name.as_str(), g.kind),
        ("Goggles Integra", Kind::Goggles)
    );
    let a = from_identity(0x48, &record("ZA5305")).unwrap();
    assert_eq!(
        (a.name.as_str(), a.kind),
        ("O4 Air Unit Pro", Kind::Aircraft)
    );
}

#[test]
fn keeps_unknown_codes() {
    let d = from_identity(0x48, &record("XX123")).unwrap();
    assert_eq!((d.name.as_str(), d.kind), ("XX123", Kind::Unknown));
    assert!(from_identity(0x48, &[0; 64]).is_none());
}
