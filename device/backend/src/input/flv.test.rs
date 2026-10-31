use super::*;

const SPS: &[u8] = &[0x67, 0x42, 0x00, 0x1f, 0xda];
const PPS: &[u8] = &[0x68, 0xce, 0x06, 0xe2];
const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00];
const P: &[u8] = &[0x41, 0x9a, 0x02];
const AUD_NAL: &[u8] = &[0x09, 0xf0];

/// An AVC sequence header tag, with 4-byte NAL lengths
fn config_tag() -> Vec<u8> {
    let mut t = vec![0x17, 0, 0, 0, 0];
    t.extend([1, 0x42, 0x00, 0x1f, 0xff, 0xe1]);
    t.extend((SPS.len() as u16).to_be_bytes());
    t.extend(SPS);
    t.push(1);
    t.extend((PPS.len() as u16).to_be_bytes());
    t.extend(PPS);
    t
}

/// A picture tag holding `nals`, each with a 4-byte length
fn frame_tag(keyframe: bool, cts: u32, nals: &[&[u8]]) -> Vec<u8> {
    let mut t = vec![if keyframe { 0x17 } else { 0x27 }, 1];
    t.extend(&cts.to_be_bytes()[1..]);
    for nal in nals {
        t.extend((nal.len() as u32).to_be_bytes());
        t.extend(*nal);
    }
    t
}

fn annexb(nals: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nals {
        out.extend(START_CODE);
        out.extend(*nal);
    }
    out
}

#[test]
fn keyframes_get_the_parameter_sets_and_every_picture_a_delimiter() {
    let mut tags = AvcTags::new();
    assert_eq!(tags.push(&config_tag()), Ok(None));

    let key = tags.push(&frame_tag(true, 0, &[IDR])).unwrap().unwrap();
    assert_eq!(key, annexb(&[SPS, PPS, IDR, AUD_NAL]));

    let p = tags.push(&frame_tag(false, 0, &[P])).unwrap().unwrap();
    assert_eq!(p, annexb(&[P, AUD_NAL]));
    assert!(!tags.reorders());
}

#[test]
fn what_the_encoder_already_sends_isnt_doubled() {
    let mut tags = AvcTags::new();
    tags.push(&config_tag()).unwrap();

    // Encoders that repeat the SPS themselves, and put a delimiter first
    let key = tags
        .push(&frame_tag(true, 0, &[AUD_NAL, SPS, PPS, IDR]))
        .unwrap()
        .unwrap();
    assert_eq!(key, annexb(&[SPS, PPS, IDR, AUD_NAL]));
}

#[test]
fn pictures_before_the_configuration_are_skipped() {
    let mut tags = AvcTags::new();
    assert_eq!(tags.push(&frame_tag(true, 0, &[IDR])), Ok(None));
    tags.push(&config_tag()).unwrap();
    assert!(tags.push(&frame_tag(true, 0, &[IDR])).unwrap().is_some());
}

#[test]
fn notices_b_frames() {
    let mut tags = AvcTags::new();
    tags.push(&config_tag()).unwrap();
    tags.push(&frame_tag(false, 66, &[P])).unwrap();
    assert!(tags.reorders());
}

#[test]
fn reads_shorter_nal_lengths() {
    let mut config = config_tag();
    config[9] = 0xfd; // two-byte lengths
    let mut tags = AvcTags::new();
    tags.push(&config).unwrap();

    let mut t = vec![0x27, 1, 0, 0, 0];
    t.extend((P.len() as u16).to_be_bytes());
    t.extend(P);
    assert_eq!(tags.push(&t).unwrap().unwrap(), annexb(&[P, AUD_NAL]));
}

#[test]
fn reads_enhanced_rtmp_h264() {
    let mut tags = AvcTags::new();
    let mut config = vec![0x90 | EX_SEQUENCE_START];
    config.extend(b"avc1");
    config.extend(&config_tag()[5..]);
    tags.push(&config).unwrap();

    let mut frame = vec![0x90 | EX_CODED_FRAMES_X];
    frame.extend(b"avc1");
    frame.extend((IDR.len() as u32).to_be_bytes());
    frame.extend(IDR);
    let key = tags.push(&frame).unwrap().unwrap();
    assert_eq!(key, annexb(&[SPS, PPS, IDR, AUD_NAL]));
}

#[test]
fn turns_away_other_codecs_by_name() {
    let mut tags = AvcTags::new();
    // H.265 the old Chinese way, and the Enhanced RTMP way OBS uses
    assert_eq!(
        tags.push(&[0x1c, 0, 0, 0, 0]),
        Err(TagError::Unsupported("H.265"))
    );
    let mut enhanced = vec![0x90];
    enhanced.extend(b"hvc1");
    assert_eq!(tags.push(&enhanced), Err(TagError::Unsupported("H.265")));
    assert_eq!(
        tags.push(&[0x12, 0, 0]),
        Err(TagError::Unsupported("Sorenson H.263"))
    );
    assert!(
        TagError::Unsupported("H.265")
            .to_string()
            .contains("set the app or encoder to H.264")
    );
}

#[test]
fn survives_garbage() {
    let mut tags = AvcTags::new();
    assert!(tags.push(&[]).is_err());
    assert!(tags.push(&[0x17, 0, 0, 0, 0, 1, 2]).is_err());
    tags.push(&config_tag()).unwrap();
    // A length running past the end of the tag
    assert!(tags.push(&[0x27, 1, 0, 0, 0, 0, 0, 0, 9, 0x41]).is_err());
    // Command frames carry no picture
    assert_eq!(tags.push(&[0x57, 0]), Ok(None));
}
