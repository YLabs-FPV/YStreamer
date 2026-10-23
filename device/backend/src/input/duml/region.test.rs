use super::*;

const FC: u8 = 0x03;
const FC_CMD_SET: u8 = 0x03;
const LINK_HEADER: usize = 8;

fn counter() -> impl FnMut() -> u16 {
    let mut n = 0;
    move || {
        n += 1;
        n - 1
    }
}

fn hex_all(frames: &[Vec<u8>]) -> Vec<String> {
    frames.iter().map(hex::encode).collect()
}

// The same frames, byte for byte, as tw-streamer's `libs/dji-core/src/fcc.rs`
// builds them (sequence numbers counted from 0)

#[test]
fn builds_the_fcc_set_like_tw_streamer() {
    assert_eq!(
        hex_all(&set_frames(*b"US", counter())),
        [
            "55cc49571000000055100456021200002010580301004b9a",
            "55cc495719000000551904e40209010020073055530000555300000100aa719aaf",
            "55cc495719000000551904e40209020020073055530000555300000200c25b8b9f",
            "55cc495719000000551904e4020e030020073055530000555300000100aa711375",
            "55cc495719000000551904e4020e040020073055530000555300000200c25b3e05",
            "55cc4957140000005514046d02060500200672000000000001ffa6b7",
            "55cc4957100000005510045602120600201058030100fa87",
        ]
    );
}

#[test]
fn builds_the_keepalive_like_tw_streamer() {
    assert_eq!(
        hex_all(&keepalive_frames(*b"US", counter())),
        [
            "55cc495719000000551904e40209000020073055530000555300000100aa7195bf",
            "55cc495719000000551904e4020e010020073055530000555300000100aa710d55",
            "55cc4957140000005514046d02060200200672000000000001ff3e35",
        ]
    );
    assert_eq!(
        hex_all(&keepalive_frames(*b"ID", counter())),
        [
            "55cc495719000000551904e40209000020073049440000494400000100e39b95bf",
            "55cc495719000000551904e4020e010020073049440000494400000100e39b0d55",
            "55cc4957140000005514046d02060200200672000000000001ff3e35",
        ]
    );
}

#[test]
fn builds_the_ce_restore_like_tw_streamer() {
    assert_eq!(
        hex_all(&restore_frames(counter())),
        [
            "55cc495719000000551904e40209000020073000000000000000000000588495bf",
            "55cc495719000000551904e4020e01002007300000000000000000000058840d55",
            "55cc4957140000005514046d02060200200672000000000000009e23",
        ]
    );
}

#[test]
fn never_addresses_the_flight_controller() {
    let all = [
        set_frames(*b"US", counter()),
        set_frames(*b"ID", counter()),
        keepalive_frames(*b"US", counter()),
        restore_frames(counter()),
    ]
    .concat();
    for f in &all {
        let d = &f[LINK_HEADER..];
        assert_ne!(d[5] & 0x1F, FC, "{}", hex::encode(f));
        assert_ne!(d[9], FC_CMD_SET, "{}", hex::encode(f));
    }
}

#[test]
fn region_payload_carries_its_crc() {
    let p = region_payload(*b"US", BAND_5G8);
    assert_eq!(&p[..10], b"US\0\0US\0\0\x02\0");
    assert_eq!(u16::from_le_bytes([p[10], p[11]]), calc_crc16(&p[..10]));
}

#[test]
fn ce_applies_as_the_factory_setting() {
    assert_eq!(
        apply_frames(RadioRegion::Eu, counter()),
        restore_frames(counter())
    );
    assert_eq!(
        apply_frames(RadioRegion::Us, counter()),
        set_frames(*b"US", counter())
    );
}

fn up(generation: u64, aircraft: bool) -> Link {
    Link {
        up: true,
        generation,
        aircraft,
    }
}

fn on(region: RadioRegion) -> RadioRegionSettings {
    RadioRegionSettings {
        enabled: true,
        region,
    }
}

const OFF: RadioRegionSettings = RadioRegionSettings {
    enabled: false,
    region: RadioRegion::Us,
};

/// What the executor records after a successful send
fn sent(st: &mut State, action: &Action) {
    st.sent_at = Some(Instant::now());
    match action {
        Action::Apply(r) => {
            st.applied = Some(*r);
            st.restored = false;
        }
        Action::Restore => st.restored = true,
        _ => {}
    }
}

#[test]
fn off_and_never_applied_sends_nothing() {
    let mut st = State::default();
    assert_eq!(decide(&mut st, up(1, true), OFF, false), Action::Wait);
}

#[test]
fn waits_for_the_goggles() {
    let mut st = State::default();
    let down = Link {
        up: false,
        generation: 1,
        aircraft: false,
    };
    assert_eq!(
        decide(&mut st, down, on(RadioRegion::Us), false),
        Action::Wait
    );
    assert_eq!(decide(&mut st, down, OFF, true), Action::Wait);
}

#[test]
fn applies_once_per_link_then_keeps_it_alive() {
    let mut st = State::default();
    let a = decide(&mut st, up(1, true), on(RadioRegion::Us), false);
    assert_eq!(a, Action::Apply(RadioRegion::Us));
    sent(&mut st, &a);

    // Just sent: nothing until the keep-alive is due
    assert_eq!(
        decide(&mut st, up(1, true), on(RadioRegion::Us), true),
        Action::Wait
    );
    st.sent_at = Some(Instant::now() - KEEPALIVE);
    assert_eq!(
        decide(&mut st, up(1, true), on(RadioRegion::Us), true),
        Action::Keepalive(*b"US")
    );

    // A new connection gets the whole set again
    assert_eq!(
        decide(&mut st, up(2, true), on(RadioRegion::Us), true),
        Action::Apply(RadioRegion::Us)
    );
}

#[test]
fn a_new_region_or_aircraft_gets_the_whole_set() {
    let mut st = State::default();
    let a = decide(&mut st, up(1, false), on(RadioRegion::Us), false);
    sent(&mut st, &a);
    assert_eq!(
        decide(&mut st, up(1, true), on(RadioRegion::Us), true),
        Action::Apply(RadioRegion::Us)
    );
    sent(&mut st, &Action::Apply(RadioRegion::Us));
    assert_eq!(
        decide(&mut st, up(1, true), on(RadioRegion::Id), true),
        Action::Apply(RadioRegion::Id)
    );
}

#[test]
fn ce_needs_no_keepalive() {
    let mut st = State::default();
    let a = decide(&mut st, up(1, true), on(RadioRegion::Eu), false);
    assert_eq!(a, Action::Apply(RadioRegion::Eu));
    sent(&mut st, &a);
    st.sent_at = Some(Instant::now() - KEEPALIVE * 10);
    assert_eq!(
        decide(&mut st, up(1, true), on(RadioRegion::Eu), true),
        Action::Wait
    );
}

#[test]
fn reapply_sends_the_whole_set() {
    let mut st = State::default();
    let a = decide(&mut st, up(1, true), on(RadioRegion::Us), false);
    sent(&mut st, &a);
    st.reapply = true;
    assert_eq!(
        decide(&mut st, up(1, true), on(RadioRegion::Us), true),
        Action::Apply(RadioRegion::Us)
    );
    assert!(!st.reapply);
}

#[test]
fn turning_off_restores_ce_once() {
    let mut st = State::default();
    let a = decide(&mut st, up(1, true), on(RadioRegion::Us), false);
    sent(&mut st, &a);

    let a = decide(&mut st, up(1, true), OFF, true);
    assert_eq!(a, Action::Restore);
    sent(&mut st, &a);
    assert_eq!(decide(&mut st, up(1, true), OFF, true), Action::Wait);
}

#[test]
fn restore_reaches_an_aircraft_that_links_later() {
    let mut st = State::default();
    let a = decide(&mut st, up(1, false), OFF, true);
    assert_eq!(a, Action::Restore);
    sent(&mut st, &a);
    assert_eq!(decide(&mut st, up(1, false), OFF, true), Action::Wait);
    assert_eq!(decide(&mut st, up(1, true), OFF, true), Action::Restore);
}

#[test]
fn restore_waits_for_the_next_connection_after_a_restart() {
    // A fresh process still finds the marker the last one left
    let mut st = State::default();
    assert_eq!(decide(&mut st, up(5, true), OFF, true), Action::Restore);
}
