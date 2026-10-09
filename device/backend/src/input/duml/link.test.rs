use super::*;

fn frame(src: u8, flags: u8, cmd_set: u8, cmd_id: u8, payload: &[u8]) -> Frame<'_> {
    Frame {
        src,
        dst: 2,
        seq: 1,
        flags,
        cmd_set,
        cmd_id,
        payload,
    }
}

#[test]
fn reads_the_quality_without_the_flag_bit() {
    assert_eq!(
        Tracker::new().on_frame(&frame(14, 0, 9, 8, &[0x5A])),
        Some(90)
    );
    assert_eq!(
        Tracker::new().on_frame(&frame(14, 0, 9, 8, &[0xC6])),
        Some(70)
    );
}

#[test]
fn ignores_the_other_sender_and_other_frames() {
    let mut t = Tracker::new();
    assert_eq!(t.on_frame(&frame(9, 0, 9, 8, &[0x80])), None);
    assert_eq!(t.on_frame(&frame(14, 0, 9, 17, &[0x00])), None);
    assert_eq!(t.on_frame(&frame(14, 0, 2, 8, &[0x5A])), None);
    assert_eq!(t.on_frame(&frame(14, 0x80, 9, 8, &[0x5A])), None);
    assert_eq!(t.on_frame(&frame(14, 0, 9, 8, &[])), None);
}

#[test]
fn stays_quiet_while_nothing_changes() {
    let mut t = Tracker::new();
    assert_eq!(t.on_frame(&frame(14, 0, 9, 8, &[0x5A])), Some(90));
    assert_eq!(t.on_frame(&frame(14, 0, 9, 8, &[0x5A])), None);
    assert_eq!(t.on_frame(&frame(14, 0, 9, 8, &[0x46])), Some(70));
}
