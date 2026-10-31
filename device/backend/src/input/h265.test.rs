use super::*;

const VPS_NAL: &[u8] = &[0, 0, 0, 1, 0x40, 0x01, 0x0C, 0x01, 0xFF];
const SPS_NAL: &[u8] = &[0, 0, 0, 1, 0x42, 0x01, 0x01, 0x01, 0x60];
const PPS_NAL: &[u8] = &[0, 0, 0, 1, 0x44, 0x01, 0xC1, 0x72];
const IDR_NAL: &[u8] = &[0, 0, 0, 1, 0x28, 0x01, 0xAF, 0x11, 0x22];
const P_NAL: &[u8] = &[0, 0, 0, 1, 0x02, 0x01, 0xD0, 0x33];

#[test]
fn reads_the_nal_type() {
    assert_eq!(kind(0x40), VPS);
    assert_eq!(kind(0x42), SPS);
    assert_eq!(kind(0x44), PPS);
    assert!(is_irap(kind(0x28)));
    assert!(is_irap(kind(0x26)));
    assert!(!is_irap(kind(0x02)));
}

#[test]
fn finds_a_picture_to_start_from() {
    assert!(has_irap(&[P_NAL, IDR_NAL].concat()));
    assert!(!has_irap(&[P_NAL, P_NAL].concat()));
}

#[test]
fn keeps_the_parameter_sets_for_later() {
    let mut p = ParamSets::new();
    p.feed(&[VPS_NAL, SPS_NAL, PPS_NAL, IDR_NAL].concat());
    assert_eq!(p.annex_b(), Some([VPS_NAL, SPS_NAL, PPS_NAL].concat()));
    // A long run of slices later they're still there
    for _ in 0..1000 {
        p.feed(P_NAL);
    }
    assert_eq!(p.annex_b(), Some([VPS_NAL, SPS_NAL, PPS_NAL].concat()));
}

#[test]
fn handles_a_parameter_set_split_across_chunks() {
    let stream = [VPS_NAL, SPS_NAL, PPS_NAL, P_NAL].concat();
    let mut p = ParamSets::new();
    for byte in &stream {
        p.feed(std::slice::from_ref(byte));
    }
    assert_eq!(p.annex_b(), Some([VPS_NAL, SPS_NAL, PPS_NAL].concat()));
}

#[test]
fn nothing_until_all_three_arrive() {
    let mut p = ParamSets::new();
    p.feed(&[VPS_NAL, SPS_NAL, P_NAL].concat());
    assert_eq!(p.annex_b(), None);
}

#[test]
fn a_huge_slice_doesnt_grow_the_buffer() {
    let mut p = ParamSets::new();
    p.feed(&[0, 0, 0, 1, 0x02, 0x01]);
    for _ in 0..100 {
        p.feed(&[0x55; 4096]);
    }
    assert!(p.tail.len() <= MAX_TAIL + 4096);
}
