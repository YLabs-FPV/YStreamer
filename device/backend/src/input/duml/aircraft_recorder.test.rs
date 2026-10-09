use super::*;

fn bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex.replace(' ', "")).unwrap()
}

// Captured from an O4 Air Unit Pro, a few seconds into a recording
const O4_RECORDING: &str = "81 02 80 00 01 ff dc 01 00 a2 88 01 00 00 00 00 \
     00 86 17 00 00 00 00 00 00 00 00 00 00 1c 00 00 \
     00 6b 00 00 01 00 00 00 00 00 00 00 00 00 00 00 \
     00 00 00 00 00 00 00 00 00 01 00 00";
const O4_STORAGE_SD: &str = "00 12 02 00 00 01 ff dc 01 00 ae 87 01 00 00 00 \
     00 00 78 17 00 00 01 01 1c 10 00 00 1a 10 00 00 \
     00 00 00 00 f7 00 00 00";
const O4_STORAGE_NO_CARD: &str = "10 12 02 00 00 02 00 00 00 00 00 00 00 00 00 00 \
     00 00 00 00 00 00 01 01 1c 10 00 00 1a 10 00 00 \
     00 00 00 00 f7 00 00 00";
// Captured from an O3 Air Unit, idle, storage nearly full
const O3_IDLE: &str = "01 02 80 00 01 7a 52 00 00 08 00 00 00 00 00 00 \
     00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 \
     00 4d 00 00 01 00 00 00 00 00 00 00 00 00 00 00 \
     00 00 00 00 00 00 00 00 00 01 00 00";

#[test]
fn reads_a_recording_in_progress() {
    let s = Tracker::new()
        .on_frame(2, 128, &bytes(O4_RECORDING))
        .unwrap();
    assert!(s.recording);
    assert_eq!(s.record_secs, 28);
    assert_eq!(
        s.storage,
        Some(Storage {
            total_mb: 122_111,
            free_mb: 100_514,
            secs_left: 6022,
        })
    );
    assert_eq!(s.sd, None);
}

#[test]
fn reads_an_idle_o3() {
    let s = Tracker::new().on_frame(2, 128, &bytes(O3_IDLE)).unwrap();
    assert!(!s.recording);
    assert_eq!(s.record_secs, 0);
    assert_eq!(
        s.storage,
        Some(Storage {
            total_mb: 21_114,
            free_mb: 8,
            secs_left: 1,
        })
    );
}

#[test]
fn tells_the_sd_card_from_internal_storage() {
    let s = Tracker::new()
        .on_frame(2, 220, &bytes(O4_STORAGE_SD))
        .unwrap();
    assert_eq!(s.to_internal, Some(false));
    assert_eq!(
        s.sd,
        Some(Storage {
            total_mb: 122_111,
            free_mb: 100_270,
            secs_left: 6008,
        })
    );
    assert_eq!(
        s.internal,
        Some(Storage {
            total_mb: 4124,
            free_mb: 4122,
            secs_left: 247,
        })
    );
}

#[test]
fn falls_back_to_internal_without_a_card() {
    let s = Tracker::new()
        .on_frame(2, 220, &bytes(O4_STORAGE_NO_CARD))
        .unwrap();
    assert_eq!(s.to_internal, Some(true));
    assert_eq!(s.sd, None);
    assert!(s.internal.is_some());

    let mut chosen = bytes(O4_STORAGE_SD);
    chosen[0] = 0x11;
    let s = Tracker::new().on_frame(2, 220, &chosen).unwrap();
    assert_eq!(s.to_internal, Some(true));
    assert!(s.sd.is_some());
}

#[test]
fn stays_quiet_while_nothing_changes() {
    let mut t = Tracker::new();
    let frame = bytes(O4_RECORDING);
    assert!(t.on_frame(2, 128, &frame).is_some());
    assert!(t.on_frame(2, 128, &frame).is_none());

    let mut later = frame.clone();
    later[29] = 0x1d;
    assert_eq!(t.on_frame(2, 128, &later).unwrap().record_secs, 29);
    // the two frames build up one picture
    let both = t.on_frame(2, 220, &bytes(O4_STORAGE_SD)).unwrap();
    assert_eq!(both.record_secs, 29);
    assert!(both.sd.is_some());
}

#[test]
fn ignores_other_and_short_frames() {
    let mut t = Tracker::new();
    assert!(t.on_frame(2, 129, &[0; 104]).is_none());
    assert!(t.on_frame(9, 128, &bytes(O4_RECORDING)).is_none());
    assert!(t.on_frame(2, 128, &[0x81; 30]).is_none());
    // announces two storages, holds one
    assert!(t.on_frame(2, 220, &bytes(O4_STORAGE_SD)[..39]).is_none());
    assert!(t.on_frame(2, 220, &[0; 3]).is_none());
}

// Captured from an O4 Air Unit (the Lite one), which has no card slot
const O4_LITE_STORAGE: &str = "11 12 01 00 01 01 76 5c 00 00 0b 4d 00 00 00 00 \
     00 00 e0 05 00 00";

#[test]
fn reads_a_unit_with_internal_storage_only() {
    let s = Tracker::new()
        .on_frame(2, 220, &bytes(O4_LITE_STORAGE))
        .unwrap();
    assert!(!s.has_sd_slot);
    assert_eq!(s.sd, None);
    assert_eq!(s.to_internal, Some(true));
    assert_eq!(
        s.internal,
        Some(Storage {
            total_mb: 23_670,
            free_mb: 19_723,
            secs_left: 1504,
        })
    );
}

#[test]
fn tells_an_empty_slot_from_no_slot() {
    let s = Tracker::new()
        .on_frame(2, 220, &bytes(O4_STORAGE_NO_CARD))
        .unwrap();
    assert!(s.has_sd_slot);
    assert_eq!(s.sd, None);

    let s = Tracker::new()
        .on_frame(2, 220, &bytes(O4_STORAGE_SD))
        .unwrap();
    assert!(s.has_sd_slot);
    assert!(s.sd.is_some());
}
