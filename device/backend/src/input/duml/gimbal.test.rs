use super::*;

const LINK_HEADER: usize = 8;

/// The DUML frame inside the LogicLink wrapper, checking the wrapper too
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

// Resume, Suspend and tilt: as dji-firmware-tools' comm_mkdupc.py builds them
// (cmd set 4, sender App, receiver Gimbal, no ACK)

#[test]
fn builds_resume_and_suspend() {
    assert_eq!(
        duml(&resume(1)),
        hex("55 0f 04 a2 02 04 01 00 00 04 0d b5 2a d5 51")
    );
    assert_eq!(
        duml(&suspend(2)),
        hex("55 0f 04 a2 02 04 02 00 00 04 0d f2 7e 74 e6")
    );
}

#[test]
fn builds_the_tilt_that_moved_our_avata_2() {
    assert_eq!(
        duml(&tilt(45.0, 3)),
        hex("55 14 04 6d 02 04 03 00 00 04 0a c2 01 00 00 00 00 01 19 5c")
    );
    assert_eq!(
        duml(&tilt(-90.0, 4)),
        hex("55 14 04 6d 02 04 04 00 00 04 0a 7c fc 00 00 00 00 01 8b fd")
    );
}

#[test]
fn keeps_the_tilt_in_range() {
    assert_eq!(tilt(-120.0, 9), tilt(TILT_MIN, 9));
    assert_eq!(tilt(80.0, 9), tilt(TILT_MAX, 9));
}

// Heading: frames DJI Fly sent to our Avata 360, recorded on the relay

#[test]
fn builds_the_heading_dji_fly_sends() {
    for (deg, seq, frame) in [
        (
            90.0,
            0xf208,
            "55 1a 04 b1 02 04 08 f2 40 04 0a 00 00 84 03 00 00 00 00 f4 04 0a 00 08 fe fd",
        ),
        (
            180.0,
            0xf21f,
            "55 1a 04 b1 02 04 1f f2 40 04 0a 00 00 08 07 00 00 00 00 f4 04 0a 00 08 9e 31",
        ),
        (
            -180.0,
            0xf32b,
            "55 1a 04 b1 02 04 2b f3 40 04 0a 00 00 f8 f8 00 00 00 00 f4 04 0a 00 08 d9 25",
        ),
        (
            0.0,
            0xf472,
            "55 1a 04 b1 02 04 72 f4 40 04 0a 00 00 00 00 00 00 00 00 f4 04 0a 00 08 e4 f2",
        ),
    ] {
        assert_eq!(duml(&avata360_heading(deg, seq)), hex(frame), "{deg}°");
    }
}

#[test]
fn wraps_the_heading_around_the_sphere() {
    assert_eq!(avata360_heading(270.0, 1), avata360_heading(-90.0, 1));
    assert_eq!(avata360_heading(-450.0, 1), avata360_heading(-90.0, 1));
    assert_eq!(avata360_heading(540.0, 1), avata360_heading(180.0, 1));
    assert_eq!(avata360_heading(f32::NAN, 1), avata360_heading(0.0, 1));
}

#[test]
fn takes_the_gimbal_once_per_link() {
    let mut hold = Hold::default();
    let t = Instant::now();
    assert!(hold.moved(1, t));
    assert!(!hold.moved(1, t));
    // New goggles session: the old hold went with the old link
    assert!(hold.moved(2, t));
}

#[test]
fn hands_the_gimbal_back_once() {
    let mut hold = Hold::default();
    assert!(!hold.release(1));
    hold.moved(1, Instant::now());
    assert!(hold.release(1));
    assert!(!hold.release(1));
}

#[test]
fn a_hold_from_an_earlier_link_isnt_handed_back() {
    let mut hold = Hold::default();
    hold.moved(1, Instant::now());
    assert!(!hold.release(2));
    // and it's gone, so the next move takes the gimbal again
    assert!(hold.moved(2, Instant::now()));
}

#[test]
fn a_quiet_hold_expires() {
    let mut hold = Hold::default();
    let t = Instant::now();
    assert!(!hold.expired(t));
    hold.moved(1, t);
    assert!(!hold.expired(t + IDLE));
    assert!(hold.expired(t + IDLE + Duration::from_millis(1)));
}
