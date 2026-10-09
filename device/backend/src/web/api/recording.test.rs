use super::*;

#[test]
fn reads_closed_and_open_ranges() {
    assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
    assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
    assert_eq!(parse_range("bytes=999-999", 1000), Some((999, 999)));
    assert_eq!(parse_range("bytes= 0-0 ", 1000), Some((0, 0)));
}

#[test]
fn reads_a_suffix_as_the_last_bytes() {
    assert_eq!(parse_range("bytes=-200", 1000), Some((800, 999)));
    assert_eq!(parse_range("bytes=-5000", 1000), Some((0, 999)));
    assert_eq!(parse_range("bytes=-0", 1000), None);
}

#[test]
fn clamps_the_end_to_the_file() {
    assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 999)));
    assert_eq!(parse_range("bytes=990-1000", 1000), Some((990, 999)));
}

#[test]
fn rejects_ranges_outside_the_file() {
    assert_eq!(parse_range("bytes=1000-", 1000), None);
    assert_eq!(parse_range("bytes=1000-2000", 1000), None);
    assert_eq!(parse_range("bytes=5-2", 1000), None);
    assert_eq!(parse_range("bytes=0-", 0), None);
}

#[test]
fn rejects_what_it_does_not_understand() {
    assert_eq!(parse_range("items=0-5", 1000), None);
    assert_eq!(parse_range("bytes=0-1,5-6", 1000), None);
    assert_eq!(parse_range("bytes=a-b", 1000), None);
    assert_eq!(parse_range("bytes=5", 1000), None);
    assert_eq!(parse_range("bytes=-", 1000), None);
    assert_eq!(parse_range("", 1000), None);
}
