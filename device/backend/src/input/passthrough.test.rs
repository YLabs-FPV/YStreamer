use super::*;
use crate::input::duml::{APP, FLAGS_REQUEST, encode};

fn frames(framer: &mut Framer, data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    framer.push(data, |f| out.push(f.to_vec()));
    out
}

#[test]
fn passes_whole_frames_through_unchanged() {
    let a = encode(0x28, 1, FLAGS_REQUEST, 0, 0x99, &[1, 2, 3]);
    let b = encode(0x3C, 2, FLAGS_REQUEST, 0, 0x88, &[4; 40]);
    let mut framer = Framer::default();
    assert_eq!(
        frames(&mut framer, &[a.clone(), b.clone()].concat()),
        vec![a, b]
    );
}

#[test]
fn holds_a_frame_split_across_reads() {
    let f = encode(0x28, 1, FLAGS_REQUEST, 0, 0x99, &[7; 100]);
    let mut framer = Framer::default();
    assert!(frames(&mut framer, &f[..5]).is_empty());
    assert!(frames(&mut framer, &f[5..60]).is_empty());
    assert_eq!(frames(&mut framer, &f[60..]), vec![f]);
}

#[test]
fn skips_noise_between_frames() {
    let f = encode(0x28, 1, FLAGS_REQUEST, 0, 0x99, &[1]);
    let mut framer = Framer::default();
    let data = [&[0x00, 0x55, 0x12][..], &f, &[0xFF; 3], &f].concat();
    assert_eq!(frames(&mut framer, &data), vec![f.clone(), f]);
}

#[test]
fn reads_the_frames_duml() {
    let f = encode(0x28, 9, FLAGS_REQUEST, 0, 0x99, &[1, 2]);
    let d = Frame::decode(&f[LINK_HEADER..]).unwrap();
    assert_eq!((d.src, d.dst, d.seq), (APP, 0x28, 9));
    assert_eq!(d.payload, &[1, 2]);
}

#[test]
fn queue_hands_over_everything_in_order() {
    let q = Queue::new(1024);
    q.push(&[1, 2]);
    q.push(&[3]);
    assert_eq!(q.take(1024, Duration::ZERO), Some(vec![1, 2, 3]));
    assert_eq!(q.take(1024, Duration::ZERO), None);
}

#[test]
fn queue_batches_up_to_the_limit() {
    let q = Queue::new(1024);
    q.push(&[1; 10]);
    q.push(&[2; 10]);
    q.push(&[3; 10]);
    assert_eq!(q.take(25, Duration::ZERO).unwrap().len(), 20);
    assert_eq!(q.take(25, Duration::ZERO), Some(vec![3; 10]));
}

#[test]
fn full_queue_drops_the_oldest() {
    let q = Queue::new(20);
    assert_eq!(q.push(&[1; 10]), 0);
    assert_eq!(q.push(&[2; 10]), 0);
    assert_eq!(q.push(&[3; 10]), 10);
    assert_eq!(
        q.take(100, Duration::ZERO),
        Some([[2; 10], [3; 10]].concat())
    );
}

#[test]
fn queue_counts_what_it_holds() {
    let q = Queue::new(20);
    q.push(&[1; 15]);
    q.take(100, Duration::ZERO);
    // Room for a whole 20 again, nothing to drop
    assert_eq!(q.push(&[2; 20]), 0);
}

#[test]
fn asks_only_devices_that_could_be_phones() {
    // A phone: MTP, or its vendor's own interfaces (adb)
    assert!(might_be_phone(0x04E8, 0x00, &[0x06]));
    // A Pixel shares its vendor ID with the accessory mode it switches to
    assert!(might_be_phone(0x18D1, 0x00, &[0x06, 0xFF]));
    assert!(might_be_phone(0x22B8, 0x00, &[0xFF]));

    assert!(!might_be_phone(DJI_VENDOR, 0x00, &[0xFF]));
    assert!(!might_be_phone(APPLE_VENDOR, 0x00, &[0x06]));
    assert!(!might_be_phone(0x1D6B, HUB, &[0x09]));
    // Webcam, USB stick, keyboard, USB tethering
    assert!(!might_be_phone(0x046D, 0xEF, &[0x0E, 0x0E, 0x01]));
    assert!(!might_be_phone(0x0781, 0x00, &[0x08]));
    assert!(!might_be_phone(0x046D, 0x00, &[0x03]));
    assert!(!might_be_phone(0x04E8, 0x00, &[0xE0, 0x0A]));
}

#[test]
fn identity_is_what_the_goggles_announce() {
    // DJI Fly's accessory filter matches these two
    assert_eq!(&IDENTITY[..2], &["DJI", "com.dji.logiclink"]);
    assert_eq!(nul_terminated("DJI"), b"DJI\0");
}
