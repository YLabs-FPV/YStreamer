use super::super::calc_crc16;
use super::*;
use std::thread;
use std::time::Instant;

const LINK_HEADER: usize = 8;

// DJI Fly on a tablet, through an RC-N3, to our Neo 2: whole USB writes,
// logged by a kprobe on acc_write. rcn-neo2-djifly-rth-2026-08-26.trace.gz,
// sha256 d4b6586c3a66cc6eb4cb671524a09151bcc27752f87a82ee1ad74028688bf0b5
const RTH_2026_08_26: &str = "55 cc 49 57 0e 00 00 00 55 0e 04 66 02 03 b2 ca 40 03 2a 06 d8 7d";
const CANCEL_RTH_2026_08_26: &str =
    "55 cc 49 57 0e 00 00 00 55 0e 04 66 02 03 a2 cd 40 03 2a 0c ee a0";
// Same file, a later RTH press that went out twice with the same sequence
const RTH_RESENT_2026_08_26: &str =
    "55 cc 49 57 0e 00 00 00 55 0e 04 66 02 03 26 db 40 03 2a 06 62 95";
// A labelled RTH press from DJI Fly through a relay that records both
// directions, 2026-09-20: the DUML frame, sent four times
const RTH_2026_09_20: &str = "55 0e 04 66 02 03 64 e4 40 03 2a 06 c8 21";

fn bytes(s: &str) -> Vec<u8> {
    hex::decode(s.replace(' ', "")).unwrap()
}

#[test]
fn builds_the_frames_dji_fly_sends() {
    assert_eq!(frame(Action::ReturnHome, 0xcab2), bytes(RTH_2026_08_26));
    assert_eq!(
        frame(Action::CancelReturnHome, 0xcda2),
        bytes(CANCEL_RTH_2026_08_26)
    );
    assert_eq!(
        frame(Action::ReturnHome, 0xdb26),
        bytes(RTH_RESENT_2026_08_26)
    );
    assert_eq!(
        frame(Action::ReturnHome, 0xe464)[LINK_HEADER..],
        bytes(RTH_2026_09_20)
    );
}

#[test]
fn the_other_actions_differ_only_in_the_command_byte() {
    let recorded = bytes(RTH_2026_08_26);
    for (action, code) in [
        (Action::Takeoff, 0x01),
        (Action::Land, 0x02),
        (Action::CancelLanding, 0x0e),
    ] {
        let f = frame(action, 0xcab2);
        assert_eq!(f.len(), recorded.len());
        assert_eq!(f[..19], recorded[..19], "{action:?}");
        assert_eq!(f[19], code, "{action:?}");
        let crc = calc_crc16(&f[LINK_HEADER..20]);
        assert_eq!(f[20..], crc.to_le_bytes(), "{action:?}");
    }
}

#[test]
fn names_round_trip() {
    for action in Action::ALL {
        assert_eq!(Action::parse(action.id()), Some(action));
    }
    assert_eq!(Action::parse("rth"), None);
    assert_eq!(Action::parse(""), None);
}

#[derive(Clone)]
struct Fake {
    ready: Arc<Mutex<Result<(), Refused>>>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Fake {
    fn new() -> Self {
        Self {
            ready: Arc::new(Mutex::new(Ok(()))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn sent(&self) -> Vec<Vec<u8>> {
        self.sent.lock().unwrap().clone()
    }

    fn wait_for_sends(&self, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.sent.lock().unwrap().len() < n {
            assert!(Instant::now() < deadline, "only {:?}", self.sent());
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Link for Fake {
    fn ready(&self) -> Result<(), Refused> {
        self.ready.lock().unwrap().clone()
    }

    fn send(&self, frame: &[u8]) -> std::io::Result<()> {
        self.sent.lock().unwrap().push(frame.to_vec());
        Ok(())
    }
}

fn flight(link: &Fake, resend_after: Duration) -> Arc<Flight> {
    Arc::new(Flight::with_resend(link.clone(), resend_after))
}

fn seq_of(frame: &[u8]) -> u16 {
    u16::from_le_bytes([frame[LINK_HEADER + 6], frame[LINK_HEADER + 7]])
}

fn answer(src: u8, seq: u16, cmd_id: u8) -> Frame<'static> {
    Frame {
        src,
        dst: 0x02,
        seq,
        flags: 0x80,
        cmd_set: CMD_SET,
        cmd_id,
        payload: &[0x00],
    }
}

#[test]
fn sends_once_when_the_aircraft_answers() {
    let link = Fake::new();
    let f = flight(&link, Duration::from_secs(5));
    let pressed = {
        let f = Arc::clone(&f);
        thread::spawn(move || f.send(Action::Land))
    };
    link.wait_for_sends(1);
    f.on_frame(&answer(FC, seq_of(&link.sent()[0]), CMD_FUNCTION_CONTROL));

    let outcome = pressed.join().unwrap().unwrap();
    assert!(outcome.acknowledged);
    assert_eq!(outcome.sends, 1);
    assert_eq!(outcome.reply.as_deref(), Some("00"));
    assert_eq!(link.sent().len(), 1);
}

#[test]
fn resends_the_same_frame_until_the_last_try() {
    let link = Fake::new();
    let f = flight(&link, Duration::from_millis(5));
    let outcome = f.send(Action::ReturnHome).unwrap();
    assert!(!outcome.acknowledged);
    assert_eq!(outcome.sends, MAX_SENDS);
    let sent = link.sent();
    assert_eq!(sent.len(), MAX_SENDS as usize);
    assert!(sent.iter().all(|s| *s == sent[0]));
}

#[test]
fn a_new_press_is_a_new_sequence_number() {
    let link = Fake::new();
    let f = flight(&link, Duration::from_millis(1));
    f.send(Action::Takeoff).unwrap();
    f.send(Action::Takeoff).unwrap();
    let sent = link.sent();
    assert_ne!(seq_of(&sent[0]), seq_of(&sent[sent.len() - 1]));
}

#[test]
fn ignores_answers_to_anything_else() {
    let link = Fake::new();
    let f = flight(&link, Duration::from_millis(30));
    let pressed = {
        let f = Arc::clone(&f);
        thread::spawn(move || f.send(Action::Land))
    };
    link.wait_for_sends(1);
    let seq = seq_of(&link.sent()[0]);
    f.on_frame(&answer(FC, seq.wrapping_add(1), CMD_FUNCTION_CONTROL));
    f.on_frame(&answer(0x01, seq, CMD_FUNCTION_CONTROL));
    f.on_frame(&answer(FC, seq, 0x43));
    // The command itself, should the goggles echo it
    f.on_frame(&Frame {
        flags: FLAGS_REQUEST,
        ..answer(FC, seq, CMD_FUNCTION_CONTROL)
    });
    assert!(!pressed.join().unwrap().unwrap().acknowledged);
}

#[test]
fn refuses_without_a_live_link_or_an_aircraft() {
    for refusal in [Refused::NoLink, Refused::NoAircraft] {
        let link = Fake::new();
        *link.ready.lock().unwrap() = Err(refusal.clone());
        let f = flight(&link, Duration::from_millis(1));
        assert_eq!(f.send(Action::Takeoff), Err(refusal));
        assert!(link.sent().is_empty());
    }
}

#[test]
fn stops_resending_once_the_aircraft_is_gone() {
    let link = Fake::new();
    let f = flight(&link, Duration::from_millis(50));
    let pressed = {
        let f = Arc::clone(&f);
        thread::spawn(move || f.send(Action::ReturnHome))
    };
    link.wait_for_sends(1);
    *link.ready.lock().unwrap() = Err(Refused::NoAircraft);
    let outcome = pressed.join().unwrap().unwrap();
    assert!(!outcome.acknowledged);
    assert_eq!(outcome.sends, 1);
}

#[test]
fn one_command_at_a_time() {
    let link = Fake::new();
    let f = flight(&link, Duration::from_millis(200));
    let first = {
        let f = Arc::clone(&f);
        thread::spawn(move || f.send(Action::ReturnHome))
    };
    link.wait_for_sends(1);
    assert_eq!(f.send(Action::CancelReturnHome), Err(Refused::Busy));
    f.on_frame(&answer(FC, seq_of(&link.sent()[0]), CMD_FUNCTION_CONTROL));
    assert!(first.join().unwrap().unwrap().acknowledged);
    // Free again once the first one has its answer
    assert!(f.send(Action::CancelReturnHome).is_ok());
}
