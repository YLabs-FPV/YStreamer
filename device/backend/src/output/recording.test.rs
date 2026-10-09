use super::*;

struct Folder(PathBuf);

impl Folder {
    fn with(files: &[&str]) -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ystreamer-rec-{}-{n}", std::process::id()));
        fs::create_dir_all(dir.join("inner")).unwrap();
        for f in files {
            fs::write(dir.join(f), b"x").unwrap();
        }
        Self(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn finds_recordings_in_the_folder() {
    let dir = Folder::with(&["a.mp4", "b.ts", "C.MP4"]);
    assert_eq!(resolve(&dir.0, "a.mp4"), Ok(dir.0.join("a.mp4")));
    assert_eq!(resolve(&dir.0, "b.ts"), Ok(dir.0.join("b.ts")));
    assert_eq!(resolve(&dir.0, "C.MP4"), Ok(dir.0.join("C.MP4")));
}

#[test]
fn refuses_names_that_leave_the_folder() {
    let dir = Folder::with(&["a.mp4", "inner/b.mp4"]);
    for name in [
        "../a.mp4",
        "inner/b.mp4",
        "inner\\b.mp4",
        "/etc/passwd",
        "..",
        ".",
        "",
    ] {
        assert!(resolve(&dir.0, name).is_err(), "{name:?} got through");
    }
}

#[test]
fn refuses_hidden_and_non_recording_files() {
    let dir = Folder::with(&[".a.mp4", "notes.txt", "mp4", "settings.json"]);
    for name in [".a.mp4", "notes.txt", "mp4", "settings.json"] {
        assert!(resolve(&dir.0, name).is_err(), "{name:?} got through");
    }
}

#[test]
fn refuses_missing_files_and_folders() {
    let dir = Folder::with(&[]);
    fs::create_dir_all(dir.0.join("folder.mp4")).unwrap();
    assert!(resolve(&dir.0, "gone.mp4").is_err());
    assert!(resolve(&dir.0, "folder.mp4").is_err());
}

const MOUNTS: &str = "\
/dev/mmcblk0p2 / ext4 rw,noatime 0 0
proc /proc proc rw 0 0
/dev/mmcblk0p1 /boot/firmware vfat rw 0 0
/dev/sda1 /media/root/USB\\040DISK vfat rw,flush 0 0
/dev/sdb1 /mnt/ssd exfat rw 0 0
";

#[test]
fn finds_the_filesystem_a_folder_is_on() {
    let mounts = parse_mounts(MOUNTS);
    let on =
        |p: &str| mount_of(Path::new(p), &mounts).map(|m| (m.device.as_str(), m.fstype.as_str()));
    assert_eq!(
        on("/media/root/USB DISK/ystreamer"),
        Some(("/dev/sda1", "vfat"))
    );
    assert_eq!(on("/mnt/ssd/clips/today"), Some(("/dev/sdb1", "exfat")));
    assert_eq!(
        on("/var/lib/ystreamer/recordings"),
        Some(("/dev/mmcblk0p2", "ext4"))
    );
    // a longer name is another folder, not this mount
    assert_eq!(on("/mnt/ssd2/clips"), Some(("/dev/mmcblk0p2", "ext4")));
}

#[test]
fn knows_when_the_drive_is_gone() {
    let mounts = parse_mounts(MOUNTS);
    let missing = |p: &str| drive_missing(Path::new(p), &mounts);
    assert!(!missing("/media/root/USB DISK/ystreamer"));
    assert!(!missing("/mnt/ssd/clips"));
    assert!(!missing("/var/lib/ystreamer/recordings"));
    assert!(!missing("/home/pi/clips"));
    // would land on the system card
    assert!(missing("/media/root/OTHER STICK/ystreamer"));
    assert!(missing("/mnt/gone"));
    assert!(drive_missing(
        Path::new("/media/root/USB DISK/ystreamer"),
        &[]
    ));
}
