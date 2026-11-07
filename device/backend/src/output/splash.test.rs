use super::*;
use crate::input::h264::tests::{gradual_refresh_stream, have};

const SPS_NAL: &[u8] = &[0, 0, 0, 1, 0x67, 1, 2, 3];
const IDR_NAL: &[u8] = &[0, 0, 0, 1, 0x65, 9, 9, 9];
const P_NAL: &[u8] = &[0, 0, 0, 1, 0x41, 7, 7];

fn cycle(first: &[u8]) -> Vec<u8> {
    [SPS_NAL, first, P_NAL].concat()
}

#[test]
fn gop_starts_over_at_each_sps_when_there_are_idrs() {
    let mut gop = Gop::default();
    gop.push(P_NAL);
    assert!(gop.bytes.is_empty(), "nothing before the first SPS");
    gop.push(&cycle(IDR_NAL));
    gop.push(&cycle(IDR_NAL));
    assert_eq!(gop.take(), cycle(IDR_NAL));
}

#[test]
fn gop_keeps_the_cycle_before_without_idrs() {
    let mut gop = Gop::default();
    for _ in 0..3 {
        gop.push(&cycle(P_NAL));
    }
    assert_eq!(gop.take(), [cycle(P_NAL), cycle(P_NAL)].concat());
}

#[test]
fn freezes_on_a_stream_without_idr() {
    if !have(&["h264parse", "avdec_h264", "x264enc", "imagefreeze"]) {
        return;
    }
    let Some(stream) = gradual_refresh_stream() else {
        return;
    };
    let mut gop = Gop::default();
    for nal in h264::nals(&stream).collect::<Vec<_>>().windows(2) {
        gop.push(&stream[nal[0].start..nal[1].start]);
    }
    assert!(!freeze(&gop.take(), false).unwrap().is_empty());
}
