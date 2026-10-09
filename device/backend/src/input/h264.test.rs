use super::*;

const AUD_NAL: &[u8] = &[0, 0, 0, 1, 0x09, 0xf0];
const SPS_NAL: &[u8] = &[0, 0, 0, 1, 0x67, 1, 2, 3];
const IDR_NAL: &[u8] = &[0, 0, 0, 1, 0x65, 9, 9, 9];
const P_NAL: &[u8] = &[0, 0, 0, 1, 0x41, 7, 7];

fn pictures(stream: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut a = AccessUnitAssembler::new();
    let mut out = Vec::new();
    for part in stream {
        a.push(part);
        while let Some(au) = a.next_au() {
            out.push(au);
        }
    }
    out
}

fn has_picture(au: &[u8]) -> bool {
    nals(au).any(|n| n.is_vcl())
}

#[test]
fn goggles_layout_delimiter_at_the_end() {
    let out = pictures(&[SPS_NAL, IDR_NAL, AUD_NAL, P_NAL, AUD_NAL, P_NAL, AUD_NAL]);
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|au| has_picture(au)));
}

#[test]
fn doubled_delimiters_yield_no_empty_pictures() {
    // An encoder's leading delimiter plus one appended at the end
    let frame = |slice: &[u8]| [AUD_NAL, slice, AUD_NAL].concat();
    let (a, b, c) = (frame(IDR_NAL), frame(P_NAL), frame(P_NAL));
    let out = pictures(&[&a, &b, &c]);
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|au| has_picture(au)));
}

#[test]
fn no_delimiters_cuts_at_the_next_slice() {
    let out = pictures(&[SPS_NAL, IDR_NAL, P_NAL, P_NAL]);
    assert_eq!(out.len(), 2);
}

#[test]
fn strips_only_a_leading_delimiter() {
    let au = [AUD_NAL, SPS_NAL, IDR_NAL].concat();
    assert_eq!(strip_leading_aud(&au), [SPS_NAL, IDR_NAL].concat());
    let plain = [SPS_NAL, IDR_NAL].concat();
    assert_eq!(strip_leading_aud(&plain), plain);
    assert_eq!(strip_leading_aud(AUD_NAL), AUD_NAL);
}
