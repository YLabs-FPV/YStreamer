use super::*;

use std::sync::Mutex as StdMutex;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSrc};

/// The GStreamer elements that make a test clip, and ffmpeg and the Pi's
/// H.264 encoder to convert it
fn have(elements: &[&str]) -> bool {
    let pi_encoder = std::fs::read_dir("/sys/class/video4linux")
        .into_iter()
        .flatten()
        .flatten()
        .any(|dev| {
            std::fs::read_to_string(dev.path().join("name"))
                .is_ok_and(|name| name.trim() == "bcm2835-codec-encode")
        });
    pi_encoder
        && gst::init().is_ok()
        && elements
            .iter()
            .all(|e| gst::ElementFactory::find(e).is_some())
        && Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

/// A short H.265 clip, as one byte stream
fn h265_clip() -> Vec<u8> {
    let pipeline = gst::parse::launch(
        "videotestsrc num-buffers=45 ! video/x-raw,width=640,height=360,framerate=30/1 ! \
         x265enc speed-preset=ultrafast key-int-max=15 ! h265parse ! \
         video/x-h265,stream-format=byte-stream ! appsink name=sink sync=false",
    )
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
    if !have(&["x265enc", "h265parse"]) {
        eprintln!("skipped: needs x265enc for a clip, ffmpeg and the Pi's encoder");
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
    // In link-sized chunks, as the goggles' parser hands them over, over and
    // over as a live stream goes on: a converter started again (once the
    // hardware turned the clip down) waits for the next keyframe
    let deadline = Instant::now() + Duration::from_secs(20);
    while pictures.lock().unwrap().len() < 10 && Instant::now() < deadline {
        for chunk in clip.chunks(4080) {
            params.feed(chunk);
            let _ = t.push(chunk, &params);
            std::thread::sleep(Duration::from_millis(5));
        }
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
    if !have(&["x265enc", "h265parse"]) {
        eprintln!("skipped: needs x265enc for a clip, ffmpeg and the Pi's encoder");
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
    // Joins mid-stream, as after a converter restart, and the stream goes
    // on without them
    let deadline = Instant::now() + Duration::from_secs(20);
    while *pictures.lock().unwrap() < 5 && Instant::now() < deadline {
        for chunk in clip[first..].chunks(4080) {
            params.feed(chunk);
            let _ = t.push(chunk, &params);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(*pictures.lock().unwrap() >= 5);
}

/// A real recording through the converter at its own pace, for measuring:
/// `YSTREAMER_H265_CLIP=clip.h265 cargo test --release converter_keeps_up -- --ignored --nocapture`
#[test]
#[ignore]
fn converter_keeps_up() {
    let Ok(path) = std::env::var("YSTREAMER_H265_CLIP") else {
        return;
    };
    let clip = std::fs::read(path).unwrap();
    let fps: u64 = std::env::var("YSTREAMER_H265_FPS")
        .ok()
        .and_then(|f| f.parse().ok())
        .unwrap_or(60);
    let secs: u64 = std::env::var("YSTREAMER_H265_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);
    // One access unit at a time, split at the delimiters (NAL 35)
    let mut starts: Vec<usize> = clip
        .windows(4)
        .enumerate()
        .filter(|(_, w)| w[..3] == [0, 0, 1] && w[3] == 0x46)
        .map(|(i, _)| if i > 0 && clip[i - 1] == 0 { i - 1 } else { i })
        .collect();
    // What comes before the first delimiter (parameter sets, a keyframe)
    // goes in first
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    starts.push(clip.len());

    let out = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counted = Arc::clone(&out);
    // YSTREAMER_BENCH_HDMI=1: also decode what comes out, as the HDMI
    // output does, on the same codec block as the encoder
    gst::init().unwrap();
    let hdmi = std::env::var_os("YSTREAMER_BENCH_HDMI").map(|_| {
        let p = gst::parse::launch(
            "appsrc name=src is-live=true format=time do-timestamp=true \
             caps=video/x-h264,stream-format=byte-stream,alignment=au ! h264parse ! \
             v4l2h264dec capture-io-mode=dmabuf ! fakesink sync=false",
        )
        .unwrap()
        .dynamic_cast::<gst::Pipeline>()
        .unwrap();
        p.set_state(gst::State::Playing).unwrap();
        p
    });
    let hdmi_src = hdmi
        .as_ref()
        .map(|p| p.by_name("src").unwrap().dynamic_cast::<AppSrc>().unwrap());
    let out_times: Arc<StdMutex<Vec<Instant>>> = Arc::default();
    let out_seen = Arc::clone(&out_times);
    let t = Transcoder::new(Arc::new(move |au: &[u8]| {
        counted.fetch_add(1, Ordering::Relaxed);
        out_seen.lock().unwrap().push(Instant::now());
        if let Some(src) = &hdmi_src {
            let _ = src.push_buffer(gst::Buffer::from_slice(au.to_vec()));
        }
    }));
    let mut params = ParamSets::new();
    let cpu = |pid: u32| -> f64 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let f: Vec<&str> = stat
            .rsplit(')')
            .next()
            .unwrap()
            .split_whitespace()
            .collect();
        (f[11].parse::<f64>().unwrap() + f[12].parse::<f64>().unwrap()) / 100.0
    };
    let pid = std::process::id();
    let frame = Duration::from_micros(1_000_000 / fps);
    let start = Instant::now();
    let mut measured_from = None;
    let mut in_times = Vec::new();
    for (n, w) in starts.windows(2).enumerate() {
        let au = &clip[w[0]..w[1]];
        in_times.push(Instant::now());
        params.feed(au);
        let _ = t.push(au, &params);
        // The first seconds start the pipeline; measure from 5 s in
        if n as u64 == fps * 5 {
            measured_from = Some((Instant::now(), out.load(Ordering::Relaxed), cpu(pid)));
        }
        if n as u64 >= fps * secs {
            break;
        }
        let due = start + frame * (n as u32 + 1);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    let (at, before, cpu_before) = measured_from.expect("clip shorter than 5 s");
    let secs = at.elapsed().as_secs_f64();
    let pictures = out.load(Ordering::Relaxed) - before;
    // Out k is roughly in k * (in / out): the cap passes every other
    let outs = out_times.lock().unwrap().clone();
    if outs.len() > 60 {
        let ratio = in_times.len() as f64 / outs.len() as f64;
        let mut delays: Vec<f64> = (30..outs.len() - 10)
            .map(|k| {
                let i = ((k as f64 * ratio) as usize).min(in_times.len() - 1);
                outs[k].saturating_duration_since(in_times[i]).as_secs_f64() * 1000.0
            })
            .collect();
        delays.sort_by(f64::total_cmp);
        eprintln!(
            "converter: delay from picture in to picture out: median {:.0} ms, 90% {:.0} ms",
            delays[delays.len() / 2],
            delays[delays.len() * 9 / 10]
        );
    }
    eprintln!(
        "converter: {:.1} pictures/s out of {fps} in, CPU {:.0}% of one core",
        pictures as f64 / secs,
        (cpu(pid) - cpu_before) / secs * 100.0
    );
}

#[test]
fn splits_ffmpeg_output_into_pictures() {
    let aud = [0, 0, 0, 1, 0x09, 0x10];
    let sps_pps_idr = [
        0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x68, 2, 0, 0, 0, 1, 0x65, 3,
    ];
    let p = [0, 0, 0, 1, 0x41, 4];
    let first = [&aud[..], &sps_pps_idr].concat();
    let second = [&aud[..], &p].concat();
    let both = [&first[..], &second].concat();
    assert_eq!(pictures(&both), vec![&first[..], &second[..]]);
    assert_eq!(pictures(&first), vec![&first[..]]);
    assert!(pictures(&[]).is_empty());
}

#[test]
fn asks_ffmpeg_for_what_the_outputs_need() {
    let args = ffmpeg_args(true).join(" ");
    // Hardware both ways, at most 30 a second, joinable at every keyframe
    assert!(args.contains("-hwaccel drm"));
    // Picked before they're copied out of the decoder
    assert!(args.contains("-hwaccel_output_format drm_prime"));
    assert!(args.contains("fps=30,hwdownload"));
    assert!(!ffmpeg_args(false).join(" ").contains("hwdownload"));
    assert!(args.contains("-c:v h264_v4l2m2m"));
    assert!(args.contains("-fpsmax 30"));
    assert!(args.contains("dump_extra=freq=keyframe"));
    assert!(args.contains("aud=insert"));
}

/// A converter for the stopping tests: `program` stands in for ffmpeg, fed
/// by a thread as ffmpeg is, so closing `input` closes its stdin
fn running(program: &str, args: &[&str]) -> Running {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let (input, chunks) = mpsc::sync_channel::<Vec<u8>>(1);
    thread::spawn(move || {
        for chunk in chunks {
            if stdin.write_all(&chunk).is_err() {
                break;
            }
        }
    });
    Running {
        child,
        hardware_refused: Arc::new(AtomicBool::new(false)),
        input,
        converter: Converter::Hardware,
        irap_at: None,
        dropped: 0,
        dropped_logged: None,
    }
}

#[test]
fn stops_ffmpeg_by_closing_its_input() {
    // `cat` ends at the end of its input, as ffmpeg does
    let at = Instant::now();
    stop_gently(running("cat", &[]));
    assert!(at.elapsed() < STOP_AFTER_INPUT, "{:?}", at.elapsed());
}

#[test]
fn tells_ffmpeg_to_stop_when_closing_its_input_isnt_enough() {
    // `sleep` doesn't read its input: SIGTERM ends it
    let at = Instant::now();
    stop_gently(running("sleep", &["30"]));
    let took = at.elapsed();
    assert!(
        took >= STOP_AFTER_INPUT && took < STOP_AFTER_INPUT + STOP_AFTER_TERM,
        "{took:?}"
    );
}

#[test]
fn waits_a_moment_between_one_ffmpeg_and_the_next() {
    let t = Transcoder::new(Arc::new(|_: &[u8]| {}));
    assert!(t.ready_to_start());
    t.retire(Some(running("cat", &[])));
    assert!(!t.ready_to_start());
    std::thread::sleep(RESTART_PAUSE + Duration::from_millis(300));
    assert!(t.ready_to_start());
}

#[test]
fn says_when_the_hardware_turned_the_stream_down() {
    let line = "Impossible to convert between the formats supported by the filter 'Parsed_fps_0' and the filter 'auto_scale_0'";
    assert!(HARDWARE_REFUSED.iter().any(|m| line.contains(m)));
    let damaged = "[hevc @ 0x55] Could not find ref with POC 917";
    assert!(!HARDWARE_REFUSED.iter().any(|m| damaged.contains(m)));
}

#[test]
fn describes_the_colours_for_the_decoders_after_it() {
    assert!(bitstream_filters().contains("colour_primaries=1"));
    assert!(ffmpeg_args(true).join(" ").contains(&bitstream_filters()));
}
