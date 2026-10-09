use super::*;
use std::time::{Duration, Instant};

/// A card and a stick, as two folders
struct Places {
    root: PathBuf,
    card: PathBuf,
    stick: PathBuf,
}

impl Places {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("ystreamer-transfer-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (card, stick) = (root.join("card"), root.join("stick"));
        fs::create_dir_all(&card).unwrap();
        fs::create_dir_all(&stick).unwrap();
        Self { root, card, stick }
    }

    fn to_stick(&self) -> Destination<'_> {
        Destination {
            dir: &self.stick,
            fstype: "exfat",
            free_bytes: 1 << 30,
        }
    }
}

impl Drop for Places {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn put(dir: &Path, name: &str, contents: &[u8]) {
    fs::write(dir.join(name), contents).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn idle(_: &str) -> bool {
    false
}

fn picked(files: &[&str]) -> Vec<String> {
    files.iter().map(|f| f.to_string()).collect()
}

/// Run a plan and wait for it to end
fn run(plan: Plan) -> Progress {
    let transfers = Transfers::new();
    transfers.start(plan).unwrap();
    let started = Instant::now();
    while transfers.running() {
        assert!(started.elapsed() < Duration::from_secs(10), "transfer hung");
        thread::sleep(Duration::from_millis(5));
    }
    transfers.progress().unwrap()
}

#[test]
fn copies_what_was_picked_and_leaves_the_original() {
    let p = Places::new("copy");
    put(&p.card, "a.mp4", b"first");
    put(&p.card, "b.mp4", b"second");

    let plan = plan(
        &p.card,
        &p.to_stick(),
        Some(&picked(&["a.mp4"])),
        false,
        &idle,
    )
    .unwrap();
    let done = run(plan);

    assert_eq!(done.state, State::Done);
    assert_eq!((done.files_done, done.bytes_done), (1, 5));
    assert_eq!(fs::read(p.stick.join("a.mp4")).unwrap(), b"first");
    assert_eq!(names(&p.stick), ["a.mp4"]);
    assert_eq!(names(&p.card), ["a.mp4", "b.mp4"]);
}

#[test]
fn moving_removes_the_original_once_the_copy_is_whole() {
    let p = Places::new("move");
    put(&p.card, "a.mp4", b"first");

    let done = run(plan(
        &p.card,
        &p.to_stick(),
        Some(&picked(&["a.mp4"])),
        true,
        &idle,
    )
    .unwrap());

    assert_eq!(done.state, State::Done);
    assert_eq!(fs::read(p.stick.join("a.mp4")).unwrap(), b"first");
    assert!(names(&p.card).is_empty());
}

#[test]
fn everything_new_skips_what_is_already_there() {
    let p = Places::new("new");
    put(&p.card, "a.mp4", b"first");
    put(&p.card, "b.mp4", b"second");
    put(&p.stick, "a.mp4", b"first");

    let done = run(plan(&p.card, &p.to_stick(), None, false, &idle).unwrap());

    assert_eq!(
        (done.files_total, done.skipped, done.bytes_total),
        (1, 1, 6)
    );
    assert_eq!(names(&p.stick), ["a.mp4", "b.mp4"]);
}

#[test]
fn moving_something_already_there_just_removes_the_original() {
    let p = Places::new("moved-before");
    put(&p.card, "a.mp4", b"first");
    put(&p.stick, "a.mp4", b"first");

    let done = run(plan(&p.card, &p.to_stick(), None, true, &idle).unwrap());

    assert_eq!((done.state, done.bytes_total), (State::Done, 0));
    assert!(names(&p.card).is_empty());
    assert_eq!(names(&p.stick), ["a.mp4"]);
}

#[test]
fn a_different_recording_with_the_same_name_is_not_overwritten() {
    let p = Places::new("clash");
    put(&p.card, "a.mp4", b"from the card");
    put(&p.stick, "a.mp4", b"other");
    put(&p.stick, "a (2).mp4", b"another");

    run(plan(&p.card, &p.to_stick(), None, false, &idle).unwrap());

    assert_eq!(fs::read(p.stick.join("a.mp4")).unwrap(), b"other");
    assert_eq!(fs::read(p.stick.join("a (2).mp4")).unwrap(), b"another");
    assert_eq!(
        fs::read(p.stick.join("a (3).mp4")).unwrap(),
        b"from the card"
    );
}

#[test]
fn the_recording_in_progress_is_left_out() {
    let p = Places::new("busy");
    put(&p.card, "old.mp4", b"done");
    put(&p.card, "now.mp4", b"still growing");
    let recording = |name: &str| name == "now.mp4";

    // Swept up with everything, it's quietly skipped
    let all = plan(&p.card, &p.to_stick(), None, false, &recording).unwrap();
    assert_eq!(all.items.len(), 1);
    assert_eq!(all.items[0].name, "old.mp4");

    // Asked for by name, it's refused
    let asked = plan(
        &p.card,
        &p.to_stick(),
        Some(&picked(&["now.mp4"])),
        false,
        &recording,
    );
    assert_eq!(asked.unwrap_err(), "now.mp4 is still being recorded");
}

#[test]
fn refuses_before_starting_when_it_cannot_fit() {
    let p = Places::new("full");
    put(&p.card, "a.mp4", &[0; 4096]);
    let small = Destination {
        free_bytes: 1024,
        ..p.to_stick()
    };

    let refused = plan(&p.card, &small, None, false, &idle).unwrap_err();

    assert!(refused.starts_with("Not enough room"), "{refused}");
    assert!(names(&p.stick).is_empty());
}

#[test]
fn refuses_a_file_too_big_for_fat32() {
    let p = Places::new("fat");
    // Sparse: 4 GB plus a byte in name only
    let big = File::create(p.card.join("long.mp4")).unwrap();
    big.set_len(FAT_MAX_FILE_BYTES + 1).unwrap();
    let fat = Destination {
        fstype: "vfat",
        free_bytes: u64::MAX,
        ..p.to_stick()
    };

    let refused = plan(&p.card, &fat, None, false, &idle).unwrap_err();
    assert!(refused.contains("larger than 4 GB"), "{refused}");

    let exfat = Destination {
        free_bytes: u64::MAX,
        ..p.to_stick()
    };
    assert!(plan(&p.card, &exfat, None, false, &idle).is_ok());
}

#[test]
fn only_recordings_in_the_folder_can_be_named() {
    let p = Places::new("names");
    put(&p.card, "a.mp4", b"x");
    put(&p.root, "secret.mp4", b"x");
    for name in ["../secret.mp4", "/etc/passwd", "notes.txt", "missing.mp4"] {
        assert!(
            plan(&p.card, &p.to_stick(), Some(&picked(&[name])), false, &idle).is_err(),
            "{name}"
        );
    }
    assert!(
        plan(
            &p.card,
            &Destination {
                dir: &p.card,
                ..p.to_stick()
            },
            None,
            false,
            &idle
        )
        .is_err()
    );
}

#[test]
fn cancelling_leaves_no_half_file_and_keeps_the_original() {
    let p = Places::new("cancel");
    put(&p.card, "a.mp4", &vec![7u8; 64 * CHUNK_BYTES]);
    let transfers = Transfers::new();
    transfers
        .start(plan(&p.card, &p.to_stick(), None, true, &idle).unwrap())
        .unwrap();
    transfers.cancel();
    let started = Instant::now();
    while transfers.running() {
        assert!(started.elapsed() < Duration::from_secs(10), "cancel hung");
        thread::sleep(Duration::from_millis(5));
    }

    // Either it was stopped partway, or the copy was already whole
    let state = transfers.progress().unwrap().state;
    if state == State::Cancelled {
        assert!(names(&p.stick).is_empty());
        assert_eq!(names(&p.card), ["a.mp4"]);
    } else {
        assert_eq!(state, State::Done);
        assert_eq!(names(&p.stick), ["a.mp4"]);
    }
}

#[test]
fn clears_what_an_interrupted_transfer_left() {
    let p = Places::new("leftover");
    put(&p.stick, "a.mp4.part", b"half");
    put(&p.stick, "kept.mp4", b"whole");
    put(&p.card, "b.mp4", b"new");

    run(plan(&p.card, &p.to_stick(), None, false, &idle).unwrap());

    assert_eq!(names(&p.stick), ["b.mp4", "kept.mp4"]);
}

#[test]
fn only_one_transfer_at_a_time() {
    let p = Places::new("one");
    put(&p.card, "a.mp4", &vec![7u8; 64 * CHUNK_BYTES]);
    let transfers = Transfers::new();
    transfers
        .start(plan(&p.card, &p.to_stick(), None, false, &idle).unwrap())
        .unwrap();
    let second = plan(&p.card, &p.to_stick(), None, false, &idle).unwrap();
    // It may already have finished on a fast disk, in which case a second is fine
    if transfers.running() {
        assert!(transfers.start(second).is_err());
    }
    transfers.cancel();
    while transfers.running() {
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn numbers_a_name_that_is_taken() {
    let used = |taken: &'static [&'static str]| move |name: &str| taken.contains(&name);
    assert_eq!(unused_name("a.mp4", used(&[])), "a.mp4");
    assert_eq!(unused_name("a.mp4", used(&["a.mp4"])), "a (2).mp4");
    assert_eq!(
        unused_name("a.mp4", used(&["a.mp4", "a (2).mp4"])),
        "a (3).mp4"
    );
}
