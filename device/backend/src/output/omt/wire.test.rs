use super::*;

#[test]
fn video_frame_layout() {
    let frame = video(&[0xaa, 0xbb], 1280, 720, (30, 1), 0x0102_0304_0506_0708);
    assert_eq!(frame.len(), HEADER_LEN + 32 + 2);
    assert_eq!(&frame[..2], &[1, VIDEO]);
    assert_eq!(&frame[2..10], &0x0102_0304_0506_0708i64.to_le_bytes());
    assert_eq!(&frame[10..12], &[0, 0]);
    assert_eq!(&frame[12..16], &34i32.to_le_bytes());
    assert_eq!(&frame[16..20], b"VMX1");
    assert_eq!(&frame[20..24], &1280i32.to_le_bytes());
    assert_eq!(&frame[24..28], &720i32.to_le_bytes());
    assert_eq!(&frame[28..32], &30i32.to_le_bytes());
    assert_eq!(&frame[32..36], &1i32.to_le_bytes());
    assert_eq!(&frame[36..40], &(1280.0f32 / 720.0).to_le_bytes());
    assert_eq!(&frame[40..44], &0i32.to_le_bytes());
    assert_eq!(&frame[44..48], &709i32.to_le_bytes());
    assert_eq!(&frame[48..], &[0xaa, 0xbb]);
}

#[test]
fn metadata_reads_back() {
    let frame = metadata(r#"<OMTSubscribe Video="true" />"#);
    let header = parse_header(frame[..HEADER_LEN].try_into().unwrap()).unwrap();
    assert_eq!(
        header,
        Header {
            kind: METADATA,
            timestamp: 0,
            data_len: frame.len() - HEADER_LEN,
        }
    );
    assert_eq!(
        subscription(&frame[HEADER_LEN..]),
        Some(Subscription::Video)
    );
}

#[test]
fn refuses_other_versions_and_negative_lengths() {
    let mut frame = metadata("<x />");
    frame[0] = 2;
    assert_eq!(parse_header(frame[..HEADER_LEN].try_into().unwrap()), None);
    let mut frame = metadata("<x />");
    frame[12..16].copy_from_slice(&(-1i32).to_le_bytes());
    assert_eq!(parse_header(frame[..HEADER_LEN].try_into().unwrap()), None);
}

#[test]
fn reads_each_kind_of_subscription() {
    assert_eq!(
        subscription(b"<OMTSubscribe Video=\"true\" />\0"),
        Some(Subscription::Video)
    );
    assert_eq!(
        subscription(b" <OMTSubscribe Video=\"true\" />\n"),
        Some(Subscription::Video)
    );
    assert_eq!(
        subscription(b"<OMTSubscribe Audio=\"true\" />"),
        Some(Subscription::Audio)
    );
    assert_eq!(
        subscription(b"<OMTSubscribe Metadata=\"true\" />"),
        Some(Subscription::Metadata)
    );
}

#[test]
fn nothing_else_is_a_subscription() {
    for xml in [
        &b"<OMTSettings Preview=\"true\" />"[..],
        b"<OMTTally Preview=\"true\" Program=\"false\" />",
        b"<OMTSubscribe Video=\"false\" />",
        b"<OMTSubscribe Video=\"true\" Audio=\"true\" />",
        b"",
    ] {
        assert_eq!(subscription(xml), None, "{}", String::from_utf8_lossy(xml));
    }
}

#[test]
fn sender_info_names_the_version() {
    assert_eq!(
        sender_info("0.1.0"),
        r#"<OMTInfo ProductName="YStreamer" Manufacturer="YStreamer" Version="0.1.0" />"#
    );
}
