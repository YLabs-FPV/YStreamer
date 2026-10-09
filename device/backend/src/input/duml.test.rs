use super::*;

const GOGGLES: u8 = 0x1C;
const LINK_HEADER: usize = 8;

#[derive(Debug, PartialEq)]
struct Seen {
    src: u8,
    dst: u8,
    seq: u16,
    flags: u8,
    cmd_set: u8,
    cmd_id: u8,
    payload: Vec<u8>,
}

/// A frame as the goggles would send it: ours, with the sender swapped
fn from(src: u8, seq: u16, cmd_set: u8, cmd_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = encode(APP, seq, FLAGS_RESPONSE, cmd_set, cmd_id, payload);
    f[LINK_HEADER + 4] = src;
    let end = f.len() - 2;
    let crc = calc_crc16(&f[LINK_HEADER..end]);
    f[end..].copy_from_slice(&crc.to_le_bytes());
    f
}

fn feed(buf: &mut DumlReassemblyBuffer, data: &[u8]) -> (Vec<Seen>, Vec<Vec<u8>>) {
    let mut frames = Vec::new();
    let mut video = Vec::new();
    buf.process(
        data,
        |v| video.push(v.to_vec()),
        |f| {
            frames.push(Seen {
                src: f.src,
                dst: f.dst,
                seq: f.seq,
                flags: f.flags,
                cmd_set: f.cmd_set,
                cmd_id: f.cmd_id,
                payload: f.payload.to_vec(),
            })
        },
    );
    (frames, video)
}

#[test]
fn encodes_the_link_and_duml_headers() {
    let f = encode(0x0A, 0x1234, FLAGS_REQUEST, 2, 129, &[1, 2, 3]);
    let duml_len = 11 + 3 + 2;
    assert_eq!(&f[..4], &[0x55, 0xCC, 0x49, 0x57]);
    assert_eq!(&f[4..8], &(duml_len as u32).to_le_bytes());
    assert_eq!(f.len(), LINK_HEADER + duml_len);

    let d = &f[LINK_HEADER..];
    assert_eq!(d[0], 0x55);
    assert_eq!(d[1] as usize, duml_len);
    assert_eq!(d[2], 1 << 2);
    assert_eq!(d[3], calc_crc8(&d[..3]));
    assert_eq!(&d[4..11], &[APP, 0x0A, 0x34, 0x12, FLAGS_REQUEST, 2, 129]);
    assert_eq!(&d[11..14], &[1, 2, 3]);
    assert_eq!(&d[14..], &calc_crc16(&d[..14]).to_le_bytes());
}

#[test]
fn encodes_lengths_past_one_byte() {
    let f = encode(0x0A, 1, FLAGS_REQUEST, 2, 1, &[0; 300]);
    let d = &f[LINK_HEADER..];
    let len = (d[1] as usize) | (((d[2] as usize) & 0x03) << 8);
    assert_eq!(len, 313);
    assert_eq!(d[2] >> 2, 1);
}

#[test]
fn decodes_a_frame_from_the_goggles() {
    let mut buf = DumlReassemblyBuffer::new();
    let (frames, video) = feed(&mut buf, &from(GOGGLES, 7, 6, 30, &[9, 8, 7, 6, 55]));
    assert!(video.is_empty());
    assert_eq!(
        frames,
        [Seen {
            src: GOGGLES,
            dst: APP,
            seq: 7,
            flags: FLAGS_RESPONSE,
            cmd_set: 6,
            cmd_id: 30,
            payload: vec![9, 8, 7, 6, 55],
        }]
    );
}

#[test]
fn waits_for_a_frame_split_across_reads() {
    let frame = from(GOGGLES, 1, 2, 129, &[0xAB; 40]);
    let mut buf = DumlReassemblyBuffer::new();
    for cut in [1, 5, LINK_HEADER, LINK_HEADER + 3, frame.len() - 1] {
        let (first, _) = feed(&mut buf, &frame[..cut]);
        assert!(first.is_empty(), "decoded early at {cut}");
        let (rest, _) = feed(&mut buf, &frame[cut..]);
        assert_eq!(rest.len(), 1, "lost the frame cut at {cut}");
        assert_eq!(rest[0].payload, vec![0xAB; 40]);
    }
}

#[test]
fn decodes_several_frames_from_one_read() {
    let mut data = from(GOGGLES, 1, 2, 129, &[1]);
    data.extend(from(GOGGLES, 2, 6, 30, &[2, 2]));
    data.extend(from(GOGGLES, 3, 0, 1, &[]));
    let (frames, _) = feed(&mut DumlReassemblyBuffer::new(), &data);
    assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(frames[2].payload.is_empty());
}

#[test]
fn skips_garbage_before_a_frame() {
    let mut data = vec![0x00, 0x55, 0x01, 0xCC, 0xFF];
    data.extend(from(GOGGLES, 9, 2, 129, &[4, 2]));
    let (frames, _) = feed(&mut DumlReassemblyBuffer::new(), &data);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].seq, 9);
}

#[test]
fn ignores_our_own_frames() {
    let mut buf = DumlReassemblyBuffer::new();
    let ours = encode(0x0A, 1, FLAGS_REQUEST, 2, 129, &[1]);
    assert!(feed(&mut buf, &ours).0.is_empty());
    assert!(feed(&mut buf, &from(5, 1, 2, 129, &[1])).0.is_empty());
}

#[test]
fn drops_a_frame_with_a_bad_header_checksum() {
    let mut frame = from(GOGGLES, 1, 2, 129, &[1, 2, 3]);
    frame[LINK_HEADER + 3] ^= 0xFF;
    let mut buf = DumlReassemblyBuffer::new();
    assert!(feed(&mut buf, &frame).0.is_empty());
    // and carries on with the next one
    assert_eq!(feed(&mut buf, &from(GOGGLES, 2, 2, 129, &[1])).0.len(), 1);
}

#[test]
fn drops_a_frame_whose_lengths_disagree() {
    let mut frame = from(GOGGLES, 1, 2, 129, &[1, 2, 3]);
    frame[LINK_HEADER + 1] += 1;
    frame[LINK_HEADER + 3] = calc_crc8(&frame[LINK_HEADER..LINK_HEADER + 3]);
    assert!(feed(&mut DumlReassemblyBuffer::new(), &frame).0.is_empty());
}

#[test]
fn passes_video_through_untouched() {
    let picture = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42];
    for port in [[0x4A, 0x57], [0x4B, 0x57]] {
        let mut data = vec![0x55, 0xCC, port[0], port[1]];
        data.extend((picture.len() as u32).to_le_bytes());
        data.extend(picture);
        let (frames, video) = feed(&mut DumlReassemblyBuffer::new(), &data);
        assert!(frames.is_empty());
        assert_eq!(video, [picture.to_vec()]);
    }
}

#[test]
fn tells_requests_that_want_a_reply() {
    let frame = |flags| Frame {
        src: GOGGLES,
        dst: APP,
        seq: 1,
        flags,
        cmd_set: 0,
        cmd_id: 1,
        payload: &[],
    };
    assert!(frame(FLAGS_REQUEST).wants_reply());
    assert!(!frame(0x00).wants_reply());
    assert!(frame(FLAGS_RESPONSE).is_response());
    assert!(!frame(FLAGS_RESPONSE | 0x40).wants_reply());
}
