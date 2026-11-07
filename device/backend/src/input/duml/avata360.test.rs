use super::*;

const LINK_HEADER: usize = 8;

fn duml(f: &[u8]) -> &[u8] {
    assert_eq!(&f[..4], &[0x55, 0xCC, 0x49, 0x57]);
    assert_eq!(&f[4..8], &((f.len() - LINK_HEADER) as u32).to_le_bytes());
    &f[LINK_HEADER..]
}

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

// Frames DJI Fly sent to our Avata 360, recorded on the relay, unless noted

#[test]
fn builds_the_lens_fov_at_both_ends() {
    assert_eq!(
        duml(&lens_fov(272.0, 0xd20b)),
        hex(
            "55 20 04 7b 02 01 0b d2 40 02 ff 00 39 00 01 0d 00 00 00 00 00 00 00 00 00 00 00 40 6a 00 a9 a2"
        )
    );
    assert_eq!(
        duml(&lens_fov(104.0, 0xe3e4)),
        hex(
            "55 20 04 7b 02 01 e4 e3 40 02 ff 00 39 00 01 0d 00 00 00 00 00 00 00 00 00 00 00 a0 28 00 40 e9"
        )
    );
}

#[test]
fn keeps_the_lens_fov_in_range() {
    assert_eq!(lens_fov(10.0, 1), lens_fov(FOV_MIN, 1));
    assert_eq!(lens_fov(400.0, 1), lens_fov(FOV_MAX, 1));
}

#[test]
fn builds_the_vision_assist_setting() {
    assert_eq!(
        duml(&vision_assist_param(true, 0xa484)),
        hex("55 12 04 c7 02 92 84 a4 40 03 f9 1e 36 d3 6c 01 53 38")
    );
}

#[test]
fn builds_the_vision_assist_stream_start() {
    // The payload from the relay notes, framed by comm_mkdupc.py
    assert_eq!(
        duml(&vision_assist_stream(true, 0xa485)),
        hex("55 16 04 fc 02 08 85 a4 40 08 69 00 00 02 00 5a 00 01 0a 00 03 c8")
    );
}

#[test]
fn turning_vision_assist_off_clears_both_values() {
    let on = vision_assist_param(true, 1);
    let off = vision_assist_param(false, 1);
    let at = LINK_HEADER + 11 + 4;
    assert_eq!((on[at], off[at]), (1, 0));

    let on = vision_assist_stream(true, 1);
    let off = vision_assist_stream(false, 1);
    let at = LINK_HEADER + 11 + 6;
    assert_eq!((on[at], off[at]), (1, 0));
}
