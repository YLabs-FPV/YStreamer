use super::*;

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[test]
fn fit_keeps_small_pictures() {
    assert_eq!(fit(1280, 720, 720), (1280, 720));
    assert_eq!(fit(640, 480, 1080), (640, 480));
    // NV12 needs even sizes
    assert_eq!(fit(641, 481, 1080), (640, 480));
}

#[test]
fn fit_scales_down_to_the_same_shape() {
    assert_eq!(fit(1920, 1080, 720), (1280, 720));
    assert_eq!(fit(1440, 1080, 720), (960, 720));
    assert_eq!(fit(1920, 1080, 540), (960, 540));
    assert_eq!(fit(1920, 1440, 480), (640, 480));
    // 853.3 rounds to an even width
    assert_eq!(fit(1280, 720, 480), (854, 480));
}

#[test]
fn pack_drops_row_padding() {
    // 4x2 picture, rows padded to 8 bytes
    let y = [1, 1, 1, 1, 0, 0, 0, 0, 2, 2, 2, 2, 0, 0, 0, 0];
    let uv = [3, 4, 3, 4, 0, 0, 0, 0];
    let picture = Picture {
        width: 4,
        height: 2,
        y: &y,
        y_stride: 8,
        uv: &uv,
        uv_stride: 8,
        frame_rate: None,
    };
    let mut out = Vec::new();
    assert_eq!(picture.pack(&mut out), 4);
    assert_eq!(out, [1, 1, 1, 1, 2, 2, 2, 2, 3, 4, 3, 4]);
}

#[test]
fn rate_prefers_the_stream_within_the_cap() {
    let rate = |frame_rate, max_fps| {
        Picture {
            width: 2,
            height: 2,
            y: &[],
            y_stride: 2,
            uv: &[],
            uv_stride: 2,
            frame_rate,
        }
        .rate(max_fps)
    };
    assert_eq!(rate(Some((30, 1)), 60), (30, 1));
    assert_eq!(rate(Some((30000, 1001)), 30), (30000, 1001));
    assert_eq!(rate(Some((60, 1)), 30), (30, 1));
    assert_eq!(rate(None, 60), (60, 1));
    assert_eq!(rate(Some((0, 1)), 30), (30, 1));
}

/// A few seconds of H.264 from the test source, cut into access units. One
/// slice per picture, like the goggles: the assembler relies on it
fn test_stream(width: u32, height: u32, frames: u32) -> Option<Vec<Vec<u8>>> {
    for element in ["x264enc", "avdec_h264", "videotestsrc"] {
        if gst::ElementFactory::find(element).is_none() {
            eprintln!("skipping: no {element}");
            return None;
        }
    }
    let description = format!(
        "videotestsrc num-buffers={frames} ! \
         video/x-raw,width={width},height={height},framerate=30/1 ! \
         x264enc tune=zerolatency threads=1 key-int-max=15 ! \
         video/x-h264,stream-format=byte-stream,alignment=au ! \
         appsink name=sink sync=false"
    );
    let pipeline = gst::parse::launch(&description)
        .unwrap()
        .dynamic_cast::<gst::Pipeline>()
        .unwrap();
    let sink = pipeline
        .by_name("sink")
        .unwrap()
        .dynamic_cast::<AppSink>()
        .unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let mut aus = Vec::new();
    while let Ok(sample) = sink.pull_sample() {
        let map = sample.buffer().unwrap().map_readable().unwrap();
        aus.push(map.as_slice().to_vec());
    }
    let _ = pipeline.set_state(gst::State::Null);
    Some(aus)
}

/// The size of each picture that came out, and whether its top left corner
/// is the test pattern's white rather than a broken decode's black
fn decode(aus: &[Vec<u8>], limit: Limit) -> Vec<(u32, u32, bool)> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let decoder = {
        let seen = Arc::clone(&seen);
        let mut decoder = Decoder::start(limit, move |p| {
            assert!(p.y.len() >= p.y_stride * (p.height as usize - 1) + p.width as usize);
            assert!(p.uv.len() >= p.uv_stride * (p.height as usize / 2 - 1) + p.width as usize);
            seen.lock().unwrap().push((p.width, p.height, p.y[0] > 200));
        })
        .unwrap();
        for au in aus {
            decoder.push(au);
            // At the source's pace, so the newest-only hand-off keeps up
            thread::sleep(Duration::from_millis(33));
        }
        decoder
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while seen.lock().unwrap().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(decoder.error(), None);
    drop(decoder);
    seen.lock().unwrap().clone()
}

#[test]
fn decodes_and_scales_down() {
    gst::init().unwrap();
    let Some(aus) = test_stream(1920, 1080, 30) else {
        return;
    };
    let seen = decode(
        &aus,
        Limit {
            max_height: 720,
            max_fps: 60,
        },
    );
    assert!(!seen.is_empty(), "no pictures came out");
    assert!(seen.iter().all(|&p| p == (1280, 720, true)), "{seen:?}");
}

#[test]
fn leaves_small_pictures_alone() {
    gst::init().unwrap();
    let Some(aus) = test_stream(640, 360, 15) else {
        return;
    };
    let seen = decode(
        &aus,
        Limit {
            max_height: 720,
            max_fps: 60,
        },
    );
    assert!(!seen.is_empty(), "no pictures came out");
    assert!(seen.iter().all(|&p| p == (640, 360, true)), "{seen:?}");
}
