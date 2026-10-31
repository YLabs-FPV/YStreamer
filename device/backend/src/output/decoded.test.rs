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
         video/x-raw,format=I420,width={width},height={height},framerate=30/1 ! \
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

/// Whether the picture shows the test pattern's colour bars: white, yellow,
/// cyan, green, magenta, red, blue. Read at each bar's centre through the
/// strides, so padded rows or swapped chroma show up as wrong colours
fn shows_the_bars(p: &Picture) -> bool {
    let (w, row) = (p.width as usize, p.height as usize / 3);
    let at = |bar: usize| {
        let x = ((2 * bar + 1) * w / 14) & !1;
        let uv = row / 2 * p.uv_stride + x;
        (p.y[row * p.y_stride + x], p.uv[uv], p.uv[uv + 1])
    };
    let (white, green, red, blue) = (at(0), at(3), at(5), at(6));
    white.0 > 200
        && (white.1 as i32 - 128).abs() < 16
        && green.1 < 100
        && green.2 < 100
        && red.2 > 180
        && blue.1 > 180
}

/// The size of each picture that came out, and whether it shows the test
/// pattern rather than a broken decode
fn decode(aus: &[Vec<u8>], limit: Limit) -> Vec<(u32, u32, bool)> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let shared = SharedDecoder::new();
    let decoder = {
        let seen = Arc::clone(&seen);
        let decoder = Decoder::start(&shared, limit, move |p| {
            assert!(p.y.len() >= p.y_stride * (p.height as usize - 1) + p.width as usize);
            assert!(p.uv.len() >= p.uv_stride * (p.height as usize / 2 - 1) + p.width as usize);
            seen.lock()
                .unwrap()
                .push((p.width, p.height, shows_the_bars(p)));
        })
        .unwrap();
        for au in aus {
            shared.push(au);
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
    assert_eq!(shared.pipeline().as_ref().and_then(take_error), None);
    drop(decoder);
    // Nothing left to decode for
    assert!(shared.pipeline().is_none());
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

#[test]
fn follows_the_video_changing_size() {
    gst::init().unwrap();
    let mut aus = Vec::new();
    // 16:9, then 4:3, then 16:9 again, as when the goggles switch modes
    for (width, height) in [(1920, 1080), (1440, 1080), (1280, 720)] {
        let Some(stream) = test_stream(width, height, 30) else {
            return;
        };
        aus.extend(stream);
    }
    let seen = decode(
        &aus,
        Limit {
            max_height: 480,
            max_fps: 60,
        },
    );
    assert!(seen.iter().all(|p| p.2), "{seen:?}");
    let mut sizes: Vec<_> = seen.iter().map(|p| (p.0, p.1)).collect();
    sizes.dedup();
    assert_eq!(sizes, [(854, 480), (640, 480), (854, 480)]);
}

#[test]
fn one_decode_feeds_every_output() {
    gst::init().unwrap();
    let Some(aus) = test_stream(1920, 1080, 60) else {
        return;
    };
    let shared = SharedDecoder::new();
    // What HDMI takes: every picture, at the decoded size
    let full = Arc::new(Mutex::new(Vec::new()));
    let screen = {
        let full = Arc::clone(&full);
        shared
            .subscribe(move |sample| {
                let s = sample.caps().unwrap().structure(0).unwrap().to_owned();
                full.lock().unwrap().push((
                    s.get::<i32>("width").unwrap(),
                    s.get::<i32>("height").unwrap(),
                ));
            })
            .unwrap()
    };
    let outputs: Vec<_> = [720, 480]
        .into_iter()
        .map(|max_height| {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let limit = Limit {
                max_height,
                max_fps: 30,
            };
            let decoder = {
                let seen = Arc::clone(&seen);
                Decoder::start(&shared, limit, move |p| {
                    seen.lock()
                        .unwrap()
                        .push((p.width, p.height, shows_the_bars(p)));
                })
                .unwrap()
            };
            (decoder, seen)
        })
        .collect();
    for au in &aus {
        shared.push(au);
        thread::sleep(Duration::from_millis(33));
    }
    thread::sleep(Duration::from_millis(500));

    // The decoder's own pictures, all of them
    let full = full.lock().unwrap().clone();
    assert!(
        full.len() >= aus.len() - 2,
        "{} of {}",
        full.len(),
        aus.len()
    );
    assert!(full.iter().all(|&s| s == (1920, 1080)));
    for ((decoder, seen), size) in outputs.iter().zip([(1280, 720), (854, 480)]) {
        assert_eq!(decoder.error(), None);
        let seen = seen.lock().unwrap();
        assert!(
            seen.len() > aus.len() / 2,
            "{} of {}",
            seen.len(),
            aus.len()
        );
        assert!(
            seen.iter().all(|&p| p == (size.0, size.1, true)),
            "{seen:?}"
        );
    }

    // Decodes until the last one leaves
    drop(outputs);
    assert!(shared.pipeline().is_some());
    drop(screen);
    assert!(shared.pipeline().is_none());
}

#[test]
fn joins_a_decoder_thats_been_running() {
    gst::init().unwrap();
    let Some(aus) = test_stream(1280, 720, 150) else {
        return;
    };
    let shared = SharedDecoder::new();
    // HDMI, say, decoding since long before the output started
    let _screen = shared.subscribe(|_| {}).unwrap();
    let mut aus = aus.iter();
    for au in aus.by_ref().take(90) {
        shared.push(au);
        thread::sleep(Duration::from_millis(33));
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let decoder = {
        let seen = Arc::clone(&seen);
        Decoder::start(
            &shared,
            Limit {
                max_height: 720,
                max_fps: 30,
            },
            move |p| seen.lock().unwrap().push(shows_the_bars(p)),
        )
        .unwrap()
    };
    let rest: Vec<_> = aus.collect();
    for au in &rest {
        shared.push(au);
        thread::sleep(Duration::from_millis(33));
    }
    thread::sleep(Duration::from_millis(300));
    assert_eq!(decoder.error(), None);
    let seen = seen.lock().unwrap();
    // Every picture after it joined, bar the first few before a keyframe
    assert!(
        seen.len() >= rest.len() - 20,
        "{} of {}",
        seen.len(),
        rest.len()
    );
    assert!(seen.iter().all(|&ok| ok));
}

/// Arrival times like the goggles': `fps` with a little jitter, and every
/// `gop` pictures one held up ~50 ms, the next few bunched behind it
fn goggles_timing(fps: f64, gop: usize, n: usize) -> Vec<f64> {
    let period = 1.0 / fps;
    (0..n)
        .map(|i| {
            let jitter = [0.0, 0.003, -0.002, 0.004, -0.003][i % 5];
            let after_key = i % gop;
            let late = match after_key {
                1 => 0.05,
                2 => 0.04,
                3 => 0.025,
                4 => 0.01,
                _ => 0.0,
            };
            i as f64 * period + jitter + late
        })
        .collect()
}

#[test]
fn thinning_keeps_an_even_share() {
    for (fps, max_fps, kept_of_10) in [(60.0, 30, 5), (50.0, 30, 6), (30.0, 30, 10), (25.0, 60, 10)]
    {
        let mut thinner = Thinner::new(max_fps);
        let kept: Vec<bool> = goggles_timing(fps, 30, 600)
            .into_iter()
            .map(|at| thinner.keep(at))
            .collect();
        // Past the first second, which the rate is measured over
        let settled = &kept[100..];
        let share = settled.iter().filter(|&&k| k).count() as f64 / settled.len() as f64;
        assert!(
            (share * 10.0 - kept_of_10 as f64).abs() < 0.2,
            "{fps} to {max_fps}: kept {share}"
        );
        // Never two dropped in a row while keeping at least half
        if kept_of_10 >= 5 {
            assert!(
                settled.windows(2).all(|w| w[0] || w[1]),
                "{fps} to {max_fps}"
            );
        }
    }
}

#[test]
fn pacing_evens_out_late_and_bunched_pictures() {
    let mut pacer = Pacer::default();
    let arrivals = goggles_timing(30.0, 18, 600);
    let released: Vec<f64> = arrivals.iter().map(|&a| pacer.release(a)).collect();
    for (a, r) in arrivals.iter().zip(&released) {
        assert!(r >= a, "released before it arrived");
        assert!(r - a <= 2.0 * PLAYOUT_DELAY + 1e-9);
    }
    // Once settled, every gap is close to the frame time; as they arrived,
    // some were 50 ms longer
    for w in released[120..].windows(2) {
        let gap = w[1] - w[0];
        assert!((gap - 1.0 / 30.0).abs() < 0.006, "gap {gap}");
    }
}

#[test]
fn pacing_follows_a_change_of_rate() {
    let mut pacer = Pacer::default();
    let mut released = Vec::new();
    let mut at = 0.0;
    for i in 0..600 {
        at += if i < 300 { 1.0 / 30.0 } else { 1.0 / 25.0 };
        released.push((at, pacer.release(at)));
    }
    // Still about the same delay behind, not drifting away
    let (a, r) = released[599];
    assert!((r - a - PLAYOUT_DELAY).abs() < 0.03, "{}", r - a);
}
