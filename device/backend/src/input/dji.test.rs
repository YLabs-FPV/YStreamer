use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn asks_for_keyframes_as_dji_fly_does() {
    // DJI Fly's frames to an RC-N3 + Air 3: right before a keyframe, and
    // right after it
    let on = encode(REMOTE, 56075, 0x00, 0x01, 0x01, &KEYFRAMES_ON);
    assert_eq!(
        hex::encode(on),
        "55cc49571800000055180420020e0bdb0001010000000000240000002400631d"
    );
    let off = encode(REMOTE, 56096, 0x00, 0x01, 0x01, &KEYFRAMES_OFF);
    assert_eq!(
        hex::encode(off),
        "55cc49571800000055180420020e20db0001010000000000040000000400d43a"
    );
}

#[test]
fn asks_once_a_second_until_a_keyframe_comes() {
    let t0 = Instant::now();
    let mut k = Keyframes::default();
    assert_eq!(k.next(t0, false, true), Some(Ask::On { first: true }));
    assert_eq!(k.next(t0 + ms(500), false, true), None);
    assert_eq!(
        k.next(t0 + ms(1000), false, true),
        Some(Ask::On { first: false })
    );
}

#[test]
fn puts_the_drone_back_after_the_keyframe() {
    let t0 = Instant::now();
    let mut k = Keyframes::default();
    k.next(t0, false, true);
    // The keyframe: back to normal, every 50 ms for a second
    assert_eq!(k.next(t0 + ms(30), true, false), Some(Ask::Off));
    assert_eq!(k.next(t0 + ms(60), false, false), None);
    assert_eq!(k.next(t0 + ms(80), false, false), Some(Ask::Off));
    assert_eq!(k.next(t0 + ms(1020), false, false), Some(Ask::Off));
    // Then nothing
    assert_eq!(k.next(t0 + ms(1031), false, false), None);
    assert_eq!(k.next(t0 + ms(5000), true, false), None);
}

#[test]
fn a_keyframe_nobody_asked_for_changes_nothing() {
    let t0 = Instant::now();
    let mut k = Keyframes::default();
    assert_eq!(k.next(t0, true, false), None);
    assert_eq!(k.next(t0 + ms(50), false, false), None);
}

#[test]
fn needing_one_while_putting_it_back_waits_for_that_to_end() {
    let t0 = Instant::now();
    let mut k = Keyframes::default();
    k.next(t0, false, true);
    assert_eq!(k.next(t0 + ms(30), true, true), Some(Ask::Off));
    assert_eq!(
        k.next(t0 + ms(1100), false, true),
        Some(Ask::On { first: true })
    );
}
