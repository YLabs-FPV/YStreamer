use super::*;

use std::sync::Mutex as StdMutex;

fn have(elements: &[&str]) -> bool {
    gst::init().is_ok()
        && elements
            .iter()
            .all(|e| gst::ElementFactory::find(e).is_some())
}

/// A short H.265 clip, as one byte stream
fn h265_clip() -> Vec<u8> {
    h265_clip_sized(640, 360)
}

fn h265_clip_sized(width: u32, height: u32) -> Vec<u8> {
    let pipeline = gst::parse::launch(&format!(
        "videotestsrc num-buffers=45 ! video/x-raw,width={width},height={height},framerate=30/1 ! \
         x265enc speed-preset=ultrafast key-int-max=15 ! h265parse ! \
         video/x-h265,stream-format=byte-stream ! appsink name=sink sync=false",
    ))
    .unwrap()
    .dynamic_cast::<gst::Pipeline>()
    .unwrap();
    let sink = pipeline
        .by_name("sink")
        .unwrap()
        .dynamic_cast::<AppSink>()
        .unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let mut out = Vec::new();
    while let Ok(sample) = sink.pull_sample() {
        out.extend_from_slice(&sample.buffer().unwrap().map_readable().unwrap());
    }
    pipeline.set_state(gst::State::Null).unwrap();
    out
}

#[test]
fn converts_h265_to_h264_pictures() {
    if !have(&["x265enc", "avdec_h265", "x264enc", "h265parse", "h264parse"]) {
        eprintln!("skipped: GStreamer plugins for H.265 aren't installed");
        return;
    }
    let clip = h265_clip();
    assert!(!clip.is_empty());

    let pictures: Arc<StdMutex<Vec<Vec<u8>>>> = Arc::default();
    let seen = Arc::clone(&pictures);
    let t = Transcoder::new(Arc::new(move |au: &[u8]| {
        seen.lock().unwrap().push(au.to_vec());
    }));

    let mut params = ParamSets::new();
    // In link-sized chunks, as the goggles' parser hands them over
    for chunk in clip.chunks(4080) {
        params.feed(chunk);
        // On a machine without the Pi's codecs the first converters fail
        // and the software one takes over
        let _ = t.push(chunk, &params);
        std::thread::sleep(Duration::from_millis(5));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while pictures.lock().unwrap().len() < 10 && Instant::now() < deadline {
        let _ = t.push(&[], &params);
        std::thread::sleep(Duration::from_millis(100));
    }

    let pictures = pictures.lock().unwrap();
    assert!(pictures.len() >= 10, "only {} pictures", pictures.len());
    // Every output can start from it: an H.264 SPS, and our delimiter last
    assert!(
        pictures
            .iter()
            .any(|au| h264::nals(au).any(|n| n.kind == h264::SPS))
    );
    assert!(pictures.iter().all(|au| au.ends_with(&AUD)));
    assert!(t.converter().is_some());
}

#[test]
fn a_restarted_converter_gets_the_parameter_sets_it_missed() {
    if !have(&["x265enc", "avdec_h265", "x264enc", "h265parse", "h264parse"]) {
        eprintln!("skipped: GStreamer plugins for H.265 aren't installed");
        return;
    }
    // The encoder sends its parameter sets once, at the start, as a remote does
    let clip = h265_clip();
    let mut params = ParamSets::new();
    let first = 4080 * 2;
    params.feed(&clip[..first]);
    assert!(params.annex_b().is_some());
    assert!(!clip[first..].windows(5).any(|w| w == [0, 0, 1, 0x40, 0x01]));

    let pictures: Arc<StdMutex<usize>> = Arc::default();
    let seen = Arc::clone(&pictures);
    let t = Transcoder::new(Arc::new(move |_: &[u8]| *seen.lock().unwrap() += 1));
    // Joins mid-stream, as after a converter restart
    for chunk in clip[first..].chunks(4080) {
        params.feed(chunk);
        let _ = t.push(chunk, &params);
        std::thread::sleep(Duration::from_millis(5));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while *pictures.lock().unwrap() < 5 && Instant::now() < deadline {
        let _ = t.push(&[], &params);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(*pictures.lock().unwrap() >= 5);
}

#[test]
fn another_camera_gets_a_fresh_converter_of_the_kind_that_worked() {
    if !have(&["x265enc", "avdec_h265", "x264enc", "h265parse", "h264parse"]) {
        eprintln!("skipped: GStreamer plugins for H.265 aren't installed");
        return;
    }
    let pictures: Arc<StdMutex<Vec<Vec<u8>>>> = Arc::default();
    let seen = Arc::clone(&pictures);
    let t = Transcoder::new(Arc::new(move |au: &[u8]| {
        seen.lock().unwrap().push(au.to_vec());
    }));
    let play = |clip: &[u8], wanted: usize| {
        let mut params = ParamSets::new();
        for chunk in clip.chunks(4080) {
            params.feed(chunk);
            let _ = t.push(chunk, &params);
            std::thread::sleep(Duration::from_millis(5));
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while pictures.lock().unwrap().len() < wanted && Instant::now() < deadline {
            let _ = t.push(&[], &params);
            std::thread::sleep(Duration::from_millis(100));
        }
    };

    // The live view, then the other camera at its own size
    play(&h265_clip_sized(640, 360), 5);
    let before = pictures.lock().unwrap().len();
    assert!(before >= 5, "only {before} pictures");
    let converter = t.converter();
    assert!(converter.is_some());

    t.restart();
    assert_eq!(t.converter(), None);
    play(&h265_clip_sized(320, 180), before + 5);
    let pictures = pictures.lock().unwrap();
    assert!(
        pictures.len() >= before + 5,
        "only {} after",
        pictures.len() - before
    );
    // Not back through the ones that failed here
    assert_eq!(t.converter(), converter);
    // The outputs can pick it up from its first picture
    assert!(h264::nals(&pictures[before]).any(|n| n.kind == h264::SPS));
}
