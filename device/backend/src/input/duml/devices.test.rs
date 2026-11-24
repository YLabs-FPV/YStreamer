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

#[test]
fn names_the_goggles_3_and_n3() {
    let g3 = from_identity(0xBC, &record("ZV902")).unwrap();
    assert_eq!((g3.name.as_str(), g3.kind), ("Goggles 3", Kind::Goggles));
    let n3 = from_identity(0xBC, &record("ZV300")).unwrap();
    assert_eq!((n3.name.as_str(), n3.kind), ("Goggles N3", Kind::Goggles));
}

#[test]
fn reads_the_rc_n3s_version_reply() {
    // Recorded from an RC-N3 at 0x0e
    let reply =
        hex::decode("0000726331353162207263207665722e61004208014602000900ff0300c0").unwrap();
    let d = from_version(0x0e, &reply).unwrap();
    assert_eq!((d.name.as_str(), d.kind), ("RC-N3", Kind::Remote));
    assert_eq!((d.code.as_str(), d.address), ("rc151b", 0x0e));
}

#[test]
fn reads_the_integras_version_reply() {
    // Recorded from Goggles Integra at 0xbc
    let reply =
        hex::decode("00127a7639303120676c205665722e3032005e51000a5e51000a010000c000").unwrap();
    let d = from_version(0xbc, &reply).unwrap();
    assert_eq!(
        (d.name.as_str(), d.kind),
        ("Goggles Integra", Kind::Goggles)
    );
}

#[test]
fn ignores_version_replies_without_a_name() {
    assert!(from_version(0x0e, &[0x00]).is_none());
    assert!(from_version(0x0e, &[0x00, 0x00, 0x00]).is_none());
    assert!(from_version(0x0e, b"\x00\x00no end").is_none());
}

#[test]
fn on_link_prefers_what_the_device_said() {
    use crate::input::codec::Codec;
    let rc = from_identity(0x06, &record("RC151")).unwrap();
    let air = from_identity(0x48, &record("WA233")).unwrap();
    let link = on_link(&[air.clone(), rc], Some(Codec::H265));
    assert_eq!((link.kind, link.name.as_str()), (Kind::Remote, "RC-N2"));
    assert_eq!(link.code.as_deref(), Some("RC151"));

    let g3 = from_identity(0xBC, &record("ZV902")).unwrap();
    let link = on_link(&[air, g3], Some(Codec::H264));
    assert_eq!(
        (link.kind, link.name.as_str()),
        (Kind::Goggles, "Goggles 3")
    );
}

#[test]
fn on_link_tells_a_remote_by_its_h265() {
    use crate::input::codec::Codec;
    let air = from_identity(0x48, &record("WA233")).unwrap();
    let link = on_link(std::slice::from_ref(&air), Some(Codec::H265));
    assert_eq!(
        (link.kind, link.name.as_str()),
        (Kind::Remote, "RC-N remote")
    );
    assert_eq!(link.code, None);
}

#[test]
fn on_link_calls_anything_else_a_dji_device() {
    use crate::input::codec::Codec;
    let unknown = from_identity(0x06, &record("XX123")).unwrap();
    for codec in [None, Some(Codec::H264)] {
        let link = on_link(std::slice::from_ref(&unknown), codec);
        assert_eq!(
            (link.kind, link.name.as_str()),
            (Kind::Unknown, "DJI device")
        );
    }
}
