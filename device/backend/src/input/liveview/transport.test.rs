use super::*;

fn fragment(unit: u8, count: u8, index: u8, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut d = vec![0u8; HEADER_LEN];
    d[4..6].copy_from_slice(&seq.to_le_bytes());
    d[6] = KIND_VIDEO;
    d[16] = unit;
    d[17] = count | ((index & 1) << 7);
    d[18] = index >> 1;
    d.extend_from_slice(payload);
    d
}

#[test]
fn opener_with_recorded_values_is_the_recorded_opener() {
    assert_eq!(opener(0xf32b, 0x9958), OPENER);
}

#[test]
fn checksum_follows_the_session() {
    let d = opener(0x1234, 0x0008);
    assert_eq!(d[7], d[..7].iter().fold(0, |x, b| x ^ b));
    assert_eq!(&d[2..4], &[0x34, 0x12]);
    assert_eq!(&d[8..10], &[0x08, 0x00]);
}

#[test]
fn keep_alive_matches_known_frames() {
    assert_eq!(
        hex::encode(duml::frame(
            0x0e,
            0x1234,
            duml::FLAGS_REQUEST,
            0x00,
            0x01,
            &[]
        )),
        "550d0433020e341240000112de"
    );
    assert_eq!(
        hex::encode(control(0xf32b, 0x9960, 0x9958, 1, 0x1234)),
        "21802bf360990585589960990000000001010000550d0433020e341240000112de"
    );
}

#[test]
fn keep_alive_cycles_through_the_receivers() {
    let receiver = |counter| control(1, 0, 0, counter, 0)[HEADER_LEN + 5];
    let cmd = |counter| control(1, 0, 0, counter, 0)[HEADER_LEN + 10];
    assert_eq!(
        (1..=6).map(receiver).collect::<Vec<_>>(),
        [0x0e, 0x1f, 0xa2, 0x00, 0x28, 0x0e]
    );
    assert_eq!(cmd(4), 0x01);
    assert_eq!(cmd(5), 0xb7);
}

#[test]
fn ack_matches_a_known_frame() {
    let state: [u8; 18] = hex::decode("68ed68ed0000000068c188c1000000000000")
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        hex::encode(ack(0x5dad, 0xf850, &state)),
        "2280ad5d0000045650f850f80000000068ed68ed0000000068c188c1000000000000"
    );
}

#[test]
fn parse_keeps_to_our_session() {
    let ours = control(0x5dad, 8, 0, 1, 0);
    assert!(matches!(
        parse(&ours, 0x5dad),
        Some(Packet::Transport {
            seq: 8,
            kind: KIND_CONTROL,
            ..
        })
    ));
    assert!(parse(&ours, 0x1111).is_none());

    let accepted = [0x09, 0x80, 0xad, 0x5d, 0, 0, 0, 0x55, 0x01];
    assert!(matches!(parse(&accepted, 0x5dad), Some(Packet::OpenerAck)));

    let mut wrong_length = ours.clone();
    wrong_length.push(0);
    assert!(parse(&wrong_length, 0x5dad).is_none());
}

#[test]
fn joins_fragments_and_acks_the_last() {
    let mut a = Assembler::default();
    let first = fragment(7, 2, 0, 0x1008, b"a");
    assert!(a.push(&fragment(7, 2, 1, 0x1010, b"b")).is_empty());
    let ready = a.push(&first);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].data, b"ab");
    assert_eq!(ready[0].last_seq, 0x1010);
    assert!(a.push(&first).is_empty());
    assert_eq!(a.duplicates, 1);
}

#[test]
fn fragment_index_uses_both_bytes() {
    let mut a = Assembler::default();
    for i in (1..5).rev() {
        assert!(
            a.push(&fragment(1, 5, i, u16::from(i), &[b'0' + i]))
                .is_empty()
        );
    }
    assert_eq!(a.push(&fragment(1, 5, 0, 0, b"0"))[0].data, b"01234");
}

#[test]
fn holds_units_until_their_turn_across_the_wrap() {
    let mut a = Assembler::default();
    assert_eq!(a.push(&fragment(0xff, 1, 0, 0xfff8, b"a"))[0].data, b"a");
    assert!(a.push(&fragment(1, 1, 0, 0x0008, b"c")).is_empty());
    let ready = a.push(&fragment(0, 1, 0, 0x0000, b"b"));
    let data: Vec<_> = ready.iter().map(|u| u.data.clone()).collect();
    assert_eq!(data, [b"b".to_vec(), b"c".to_vec()]);
}

#[test]
fn skips_a_unit_that_never_completes() {
    let mut a = Assembler::default();
    a.push(&fragment(0, 1, 0, 0, b"0"));
    // Unit 1 never arrives
    let mut out = Vec::new();
    for n in 2..2 + GIVE_UP_AFTER as u8 {
        out.extend(a.push(&fragment(n, 1, 0, u16::from(n) * 8, &[n])));
    }
    assert_eq!(out.len(), GIVE_UP_AFTER);
    assert_eq!(out[0].data, [2]);
    assert_eq!(a.skipped, 1);
}

#[test]
fn rejects_impossible_fragments() {
    let mut a = Assembler::default();
    assert!(a.push(&fragment(0, 2, 2, 0, b"x")).is_empty());
    assert!(a.push(&fragment(0, 0, 0, 0, b"x")).is_empty());
    assert!(a.push(&[0u8; HEADER_LEN]).is_empty());
    assert_eq!(a.invalid, 3);
}
