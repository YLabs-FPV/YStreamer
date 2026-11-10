use super::*;

#[test]
fn matches_the_kernel_structs() {
    assert_eq!(std::mem::size_of::<Iocb>(), 64);
    assert_eq!(std::mem::size_of::<IoEvent>(), 32);
}

#[test]
fn hands_back_every_queued_read_in_order() {
    let path = std::env::temp_dir().join(format!("ystreamer-aio-{}", std::process::id()));
    std::fs::write(&path, b"abcdef").unwrap();
    let file = File::open(&path).unwrap();
    // A file reads from offset 0 every time, so each read brings the same
    let mut reader = QueuedReader::new(file, 4, 3).unwrap();
    for _ in 0..10 {
        assert_eq!(reader.next().unwrap(), b"abc");
    }
    drop(reader);
    std::fs::remove_file(path).unwrap();
}
