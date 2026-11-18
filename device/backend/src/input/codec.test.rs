use super::*;

fn nal(header: &[u8], body: &[u8]) -> Vec<u8> {
    [&[0, 0, 0, 1][..], header, body].concat()
}

#[test]
fn an_h264_sps_settles_it() {
    // High profile, level 4.1
    let chunk = nal(&[0x67], &[100, 0, 41, 0xAC]);
    assert_eq!(Sniffer::new().feed(&chunk), Some(Codec::H264));
}

#[test]
fn an_h265_vps_settles_it() {
    let chunk = nal(&[0x40, 0x01], &[0x0C, 0x01]);
    assert_eq!(Sniffer::new().feed(&chunk), Some(Codec::H265));
}

#[test]
fn h265_slices_alone_settle_it_after_a_run() {
    // What a late start on an RC-N link looks like: P-slices, no parameter set
    let slice = nal(&[0x02, 0x01], &[0xD0, 0x12, 0x34]);
    let mut s = Sniffer::new();
    for _ in 0..SLICES_NEEDED - 1 {
        assert_eq!(s.feed(&slice), None);
    }
    assert_eq!(s.feed(&slice), Some(Codec::H265));
}

#[test]
fn h264_slices_alone_settle_it_after_a_run() {
    let slice = nal(&[0x41], &[0x9A, 0x12, 0x34, 0x56]);
    let mut s = Sniffer::new();
    let run: Vec<u8> = (0..SLICES_NEEDED).flat_map(|_| slice.clone()).collect();
    assert_eq!(s.feed(&run), Some(Codec::H264));
}

#[test]
fn a_fake_sps_inside_h265_data_is_not_h264() {
    // 0x67 followed by bytes that are no profile and level
    let chunk = nal(&[0x67], &[0x01, 0xFF, 0x00]);
    assert_eq!(Sniffer::new().feed(&chunk), None);
}

#[test]
fn h265_slices_never_count_as_h264() {
    // 0x26 0x01 is an H.265 CRA; read as H.264 it'd be a slice partition
    let chunk = nal(&[0x26, 0x01], &[0xAF, 0x00, 0x11]);
    let mut s = Sniffer::new();
    let run: Vec<u8> = (0..SLICES_NEEDED).flat_map(|_| chunk.clone()).collect();
    assert_eq!(s.feed(&run), Some(Codec::H265));
}

#[test]
fn noise_settles_nothing() {
    let noise: Vec<u8> = (0..4096u32).map(|i| (i * 37 % 251) as u8).collect();
    assert_eq!(Sniffer::new().feed(&noise), None);
}
