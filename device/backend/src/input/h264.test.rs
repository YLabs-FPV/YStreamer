use super::*;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSrc};

const AUD_NAL: &[u8] = &[0, 0, 0, 1, 0x09, 0xf0];
const SPS_NAL: &[u8] = &[0, 0, 0, 1, 0x67, 1, 2, 3];
const IDR_NAL: &[u8] = &[0, 0, 0, 1, 0x65, 9, 9, 9];
const P_NAL: &[u8] = &[0, 0, 0, 1, 0x41, 7, 7];

fn pictures(stream: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut a = AccessUnitAssembler::new();
    let mut out = Vec::new();
    for part in stream {
        a.push(part);
        while let Some(au) = a.next_au() {
            out.push(au);
        }
    }
    out
}

fn has_picture(au: &[u8]) -> bool {
    nals(au).any(|n| n.is_vcl())
}

#[test]
fn goggles_layout_delimiter_at_the_end() {
    let out = pictures(&[SPS_NAL, IDR_NAL, AUD_NAL, P_NAL, AUD_NAL, P_NAL, AUD_NAL]);
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|au| has_picture(au)));
}

#[test]
fn doubled_delimiters_yield_no_empty_pictures() {
    // An encoder's leading delimiter plus one appended at the end
    let frame = |slice: &[u8]| [AUD_NAL, slice, AUD_NAL].concat();
    let (a, b, c) = (frame(IDR_NAL), frame(P_NAL), frame(P_NAL));
    let out = pictures(&[&a, &b, &c]);
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|au| has_picture(au)));
}

#[test]
fn no_delimiters_cuts_at_the_next_slice() {
    let out = pictures(&[SPS_NAL, IDR_NAL, P_NAL, P_NAL]);
    assert_eq!(out.len(), 2);
}

#[test]
fn strips_only_a_leading_delimiter() {
    let au = [AUD_NAL, SPS_NAL, IDR_NAL].concat();
    assert_eq!(strip_leading_aud(&au), [SPS_NAL, IDR_NAL].concat());
    let plain = [SPS_NAL, IDR_NAL].concat();
    assert_eq!(strip_leading_aud(&plain), plain);
    assert_eq!(strip_leading_aud(AUD_NAL), AUD_NAL);
}

const SEI_NAL: &[u8] = &[0, 0, 0, 1, 0x06, 0x05, 1, 0, 0x80];
const PPS_NAL: &[u8] = &[0, 0, 0, 1, 0x68, 0xee, 0x3c, 0xb0];

fn kinds(stream: &[u8]) -> Vec<u8> {
    nals(stream).map(|n| n.kind).collect()
}

#[test]
fn recovery_point_goes_before_the_picture_after_an_sps() {
    let mut r = RecoveryPoints::default();
    let out = r.mark(&[AUD_NAL, SPS_NAL, PPS_NAL, SEI_NAL, P_NAL, AUD_NAL, P_NAL].concat());
    assert_eq!(kinds(&out), [AUD, SPS, 8, SEI, SEI, 1, AUD, 1]);
    assert!(
        out.windows(RECOVERY_POINT.len())
            .any(|w| w == RECOVERY_POINT)
    );
    // Pictures without an SPS before them stay as they are
    assert_eq!(r.mark(P_NAL), P_NAL);
}

#[test]
fn recovery_point_waits_for_the_picture_in_a_later_chunk() {
    let mut r = RecoveryPoints::default();
    let params = [SPS_NAL, PPS_NAL].concat();
    assert_eq!(r.mark(&params), params);
    assert_eq!(r.mark(P_NAL), [RECOVERY_POINT, P_NAL].concat());
}

#[test]
fn recovery_point_isnt_doubled() {
    let mut r = RecoveryPoints::default();
    let own = [SPS_NAL, PPS_NAL, RECOVERY_POINT, P_NAL].concat();
    assert_eq!(r.mark(&own), own);
}

/// Whether the GStreamer elements a test needs are installed; tests skip
/// themselves without them
pub(crate) fn have(elements: &[&str]) -> bool {
    if gst::init().is_err() {
        return false;
    }
    let missing: Vec<_> = elements
        .iter()
        .filter(|e| gst::ElementFactory::find(e).is_none())
        .collect();
    if !missing.is_empty() {
        eprintln!("skipped: no {missing:?}");
    }
    missing.is_empty()
}

/// Every buffer `desc` produces, from its appsink `sink`. With an appsrc
/// `src` that gets `input` as H.264 first
pub(crate) fn run(desc: &str, input: Option<&[u8]>) -> Vec<gst::Buffer> {
    gst::init().unwrap();
    let pipeline = gst::parse::launch(desc)
        .unwrap()
        .dynamic_cast::<gst::Pipeline>()
        .unwrap();
    let sink = pipeline
        .by_name("sink")
        .unwrap()
        .dynamic_cast::<AppSink>()
        .unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    if let Some(input) = input {
        let src = pipeline
            .by_name("src")
            .unwrap()
            .dynamic_cast::<AppSrc>()
            .unwrap();
        src.set_caps(Some(
            &gst::Caps::builder("video/x-h264")
                .field("stream-format", "byte-stream")
                .build(),
        ));
        src.push_buffer(gst::Buffer::from_slice(input.to_vec()))
            .unwrap();
        src.end_of_stream().unwrap();
    }
    let mut out = Vec::new();
    while let Ok(sample) = sink.pull_sample() {
        out.push(sample.buffer_owned().unwrap());
    }
    pipeline.set_state(gst::State::Null).unwrap();
    out
}

/// A stream like the goggles' liveview sharing: SPS and PPS every refresh
/// cycle, no IDR pictures and no recovery points. x264's intra refresh, cut
/// at its second SPS, without its SEI
pub(crate) fn gradual_refresh_stream() -> Option<Vec<u8>> {
    if !have(&["videotestsrc", "x264enc"]) {
        return None;
    }
    let encoded: Vec<u8> = run(
        "videotestsrc num-buffers=180 pattern=ball ! \
         video/x-raw,width=320,height=180,framerate=60/1 ! \
         x264enc intra-refresh=true key-int-max=30 option-string=repeat-headers=1 ! \
         video/x-h264,stream-format=byte-stream ! appsink name=sink sync=false",
        None,
    )
    .iter()
    .flat_map(|b| b.map_readable().unwrap().to_vec())
    .collect();

    let starts: Vec<_> = nals(&encoded).collect();
    let second_sps = starts.iter().filter(|n| n.kind == SPS).nth(1)?.start;
    let mut out = Vec::new();
    for (i, nal) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(encoded.len(), |n| n.start);
        if nal.start >= second_sps && nal.kind != SEI {
            out.extend_from_slice(&encoded[nal.start..end]);
        }
    }
    assert!(!has_idr(&out));
    Some(out)
}

fn has_idr(stream: &[u8]) -> bool {
    nals(stream).any(|n| n.kind == IDR)
}

/// FFmpeg's decoder with its default of not showing broken pictures, as
/// OBS and VLC use it
fn pictures_decoded(stream: &[u8]) -> usize {
    run(
        "appsrc name=src ! h264parse ! avdec_h264 output-corrupt=false ! \
         appsink name=sink sync=false",
        Some(stream),
    )
    .len()
}

#[test]
fn players_start_on_a_stream_without_idr_once_it_has_recovery_points() {
    if !have(&["h264parse", "avdec_h264"]) {
        return;
    }
    let Some(stream) = gradual_refresh_stream() else {
        return;
    };
    assert_eq!(pictures_decoded(&stream), 0, "starts without any help");
    let marked = RecoveryPoints::default().mark(&stream);
    assert!(pictures_decoded(&marked) > 100);
}

#[test]
fn h264parse_takes_recovery_points_for_keyframes() {
    if !have(&["h264parse"]) {
        return;
    }
    let Some(stream) = gradual_refresh_stream() else {
        return;
    };
    let keyframes = |stream: &[u8]| {
        run(
            "appsrc name=src ! h264parse ! video/x-h264,alignment=au ! \
             appsink name=sink sync=false",
            Some(stream),
        )
        .iter()
        .filter(|b| !b.flags().contains(gst::BufferFlags::DELTA_UNIT))
        .count()
    };
    assert_eq!(keyframes(&stream), 0);
    // Recording, RTMP and the rest start and split files at these
    assert!(keyframes(&RecoveryPoints::default().mark(&stream)) >= 4);
}
