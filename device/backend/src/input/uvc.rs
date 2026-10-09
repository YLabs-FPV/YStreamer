use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use serde::Serialize;

use super::{Fanout, InputState};
use crate::input::h264;
use crate::output::overlay::{self, Overlay, Rect};
use crate::settings::{InputMode, UvcSettings, parse_uvc_mode};

const BY_ID: &str = "/dev/v4l/by-id";
const POLL: Duration = Duration::from_secs(1);
/// No frame for this long means the device stalled or went away
const STALL: Duration = Duration::from_secs(3);
/// How long an attempt gets to deliver its first picture
const FIRST_FRAME: Duration = Duration::from_secs(4);
const RETRY: Duration = Duration::from_secs(2);
/// What the hardware encoder handles comfortably
const HW_PIXEL_RATE: u64 = 1920 * 1080 * 30;
/// What the software encoder keeps up with on a Pi 4, leaving CPU to spare
const SW_PIXEL_RATE: u64 = 1280 * 720 * 30;
/// Appended to every picture, as the goggles do, so the HDMI and browser
/// paths know a picture is complete without waiting for the next one
const AUD: [u8; 6] = [0, 0, 0, 1, 0x09, 0xf0];

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Passed through as-is
    H264,
    Mjpeg,
    Raw,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Encoder {
    /// The device's own H.264
    Passthrough,
    Hardware,
    /// When the hardware one is unavailable, e.g. too little GPU memory
    Software,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UvcMode {
    /// `WxH@fps`, what settings store
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Cheapest format the device offers this mode in
    pub format: Format,
    /// Every format it's offered in, cheapest first, with the raw pixel
    /// format name where there is one
    #[serde(skip)]
    formats: Vec<(Format, Option<String>)>,
}

impl UvcMode {
    fn pixel_rate(&self) -> u64 {
        self.width as u64 * self.height as u64 * self.fps as u64
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct UvcDevice {
    /// Stable /dev/v4l/by-id path
    pub path: String,
    pub name: String,
    pub modes: Vec<UvcMode>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UvcState {
    /// Another input is selected
    #[default]
    Idle,
    NoDevice,
    Starting,
    Live,
    Error,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct UvcStatus {
    pub state: UvcState,
    pub device: Option<String>,
    pub mode: Option<UvcMode>,
    /// The format actually in use, which may not be the cheapest offered
    pub format: Option<Format>,
    pub encoder: Option<Encoder>,
    pub error: Option<String>,
}

/// One way of running the device
#[derive(Clone, Debug, PartialEq)]
struct Attempt {
    mode: UvcMode,
    format: Format,
    raw_format: Option<String>,
    encoder: Encoder,
}

struct Running {
    pipeline: gst::Pipeline,
}

pub struct UvcSource {
    state: Arc<InputState>,
    overlay: Arc<Overlay>,
    cfg: Mutex<UvcSettings>,
    running: Mutex<Option<Running>>,
    /// Unix millis of the last picture; shared with the appsink callback
    last_frame: Arc<AtomicU64>,
    retry_at: Mutex<Option<Instant>>,
    /// Device path and the attempt that last worked for it
    known_good: Mutex<Option<(String, Attempt)>>,
}

impl UvcSource {
    pub fn start(
        state: Arc<InputState>,
        cfg: &UvcSettings,
        fanout: Arc<Fanout>,
        overlay: Arc<Overlay>,
    ) -> Arc<Self> {
        let this = Arc::new(Self {
            state,
            overlay,
            cfg: Mutex::new(cfg.clone()),
            running: Mutex::new(None),
            last_frame: Arc::new(AtomicU64::new(0)),
            retry_at: Mutex::new(None),
            known_good: Mutex::new(None),
        });
        let supervisor = Arc::clone(&this);
        thread::spawn(move || {
            loop {
                supervisor.supervise(&fanout);
                thread::sleep(POLL);
            }
        });
        this
    }

    /// Takes effect on the next supervisor tick
    pub fn apply(&self, cfg: &UvcSettings) {
        *self.cfg.lock().unwrap() = cfg.clone();
        *self.known_good.lock().unwrap() = None;
        self.stop();
        *self.retry_at.lock().unwrap() = None;
    }

    pub fn status(&self) -> UvcStatus {
        self.state.uvc.lock().unwrap().clone()
    }

    fn set_status(&self, status: UvcStatus) {
        *self.state.uvc.lock().unwrap() = status;
    }

    fn stop(&self) {
        self.overlay.set_burned(false);
        if let Some(r) = self.running.lock().unwrap().take() {
            let _ = r.pipeline.set_state(gst::State::Null);
        }
    }

    fn fail(&self, status: UvcStatus) {
        if let Some(e) = &status.error {
            eprintln!("[uvc] {e}");
        }
        self.set_status(status);
        *self.retry_at.lock().unwrap() = Some(Instant::now() + RETRY);
    }

    fn supervise(&self, fanout: &Arc<Fanout>) {
        if self.state.mode() != InputMode::Uvc {
            self.stop();
            self.set_status(UvcStatus::default());
            return;
        }

        if self.running.lock().unwrap().is_some() {
            if let Some(error) = self.check() {
                self.stop();
                let status = UvcStatus {
                    state: UvcState::Error,
                    error: Some(error),
                    ..self.status()
                };
                self.fail(status);
            }
            return;
        }
        if self
            .retry_at
            .lock()
            .unwrap()
            .is_some_and(|t| Instant::now() < t)
        {
            return;
        }

        let cfg = self.cfg.lock().unwrap().clone();
        let Some(path) = pick_device(&cfg.device) else {
            self.set_status(UvcStatus {
                state: UvcState::NoDevice,
                ..Default::default()
            });
            return;
        };
        let device = match probe(&path) {
            Ok(d) => d,
            Err(e) => {
                return self.fail(UvcStatus {
                    state: UvcState::Error,
                    error: Some(e),
                    ..Default::default()
                });
            }
        };
        self.set_status(UvcStatus {
            state: UvcState::Starting,
            device: Some(device.name.clone()),
            ..Default::default()
        });

        let mut attempts = attempts(&device.modes, &cfg.mode);
        let known = self.known_good.lock().unwrap().clone();
        if let Some((known_path, attempt)) = known
            && known_path == device.path
            && let Some(i) = attempts.iter().position(|a| *a == attempt)
        {
            let a = attempts.remove(i);
            attempts.insert(0, a);
        }

        let mut last_error = "The device offers no usable video mode".to_string();
        for attempt in attempts {
            match self.try_start(&path, &attempt, &cfg, fanout) {
                Ok(pipeline) => {
                    eprintln!(
                        "[uvc] {} at {} ({:?}, {:?})",
                        device.name, attempt.mode.id, attempt.format, attempt.encoder
                    );
                    *self.running.lock().unwrap() = Some(Running { pipeline });
                    *self.known_good.lock().unwrap() = Some((device.path.clone(), attempt.clone()));
                    self.set_status(UvcStatus {
                        state: UvcState::Live,
                        device: Some(device.name),
                        mode: Some(attempt.mode),
                        format: Some(attempt.format),
                        encoder: Some(attempt.encoder),
                        error: None,
                    });
                    return;
                }
                Err(e) => {
                    eprintln!(
                        "[uvc] {} as {:?} with {:?} encoding didn't work: {e}",
                        attempt.mode.id, attempt.format, attempt.encoder
                    );
                    last_error = e;
                }
            }
        }
        self.fail(UvcStatus {
            state: UvcState::Error,
            device: Some(device.name),
            error: Some(last_error),
            ..Default::default()
        });
    }

    /// Built, playing and delivering pictures, or the reason it isn't.
    /// Format and encoder problems only show once data flows
    fn try_start(
        &self,
        path: &Path,
        attempt: &Attempt,
        cfg: &UvcSettings,
        fanout: &Arc<Fanout>,
    ) -> Result<gst::Pipeline, String> {
        self.last_frame.store(0, Ordering::Relaxed);
        let (pipeline, burned) = build(
            path,
            attempt,
            cfg,
            &self.overlay,
            Arc::clone(fanout),
            Arc::clone(&self.last_frame),
        )?;
        let deadline = Instant::now() + FIRST_FRAME;
        while Instant::now() < deadline {
            if let Some(e) = bus_error(&pipeline) {
                let _ = pipeline.set_state(gst::State::Null);
                return Err(e);
            }
            if self.last_frame.load(Ordering::Relaxed) != 0 {
                self.overlay.set_burned(burned);
                return Ok(pipeline);
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = pipeline.set_state(gst::State::Null);
        Err("No video from the device".into())
    }

    /// Why a running pipeline must be rebuilt, if it must
    fn check(&self) -> Option<String> {
        let running = self.running.lock().unwrap();
        let r = running.as_ref()?;
        if let Some(e) = bus_error(&r.pipeline) {
            return Some(e);
        }
        let last = self.last_frame.load(Ordering::Relaxed);
        if Duration::from_millis(now_millis().saturating_sub(last)) > STALL {
            return Some("No video from the device".into());
        }
        None
    }
}

fn bus_error(pipeline: &gst::Pipeline) -> Option<String> {
    let msg = pipeline
        .bus()?
        .pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos])?;
    Some(match msg.view() {
        gst::MessageView::Error(e) => e.error().to_string(),
        _ => "The device stopped sending video".into(),
    })
}

/// Everything worth trying, best first: the chosen mode in each format the
/// device offers it in, then software encoding at the best mode it can keep
/// up with
fn attempts(modes: &[UvcMode], wanted: &str) -> Vec<Attempt> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<Attempt>, m: &UvcMode, encoder: Encoder| {
        for (format, raw) in &m.formats {
            let encoder = match (format, encoder) {
                (Format::H264, Encoder::Hardware) => Encoder::Passthrough,
                // Passthrough was already tried with the hardware round
                (Format::H264, _) => continue,
                (_, e) => e,
            };
            let a = Attempt {
                mode: m.clone(),
                format: *format,
                raw_format: raw.clone(),
                encoder,
            };
            if !out.contains(&a) {
                out.push(a);
            }
        }
    };
    if let Some(m) = choose_mode(modes, wanted, HW_PIXEL_RATE) {
        push(&mut out, &m, Encoder::Hardware);
    }
    if let Some(m) = choose_mode(modes, wanted, SW_PIXEL_RATE) {
        push(&mut out, &m, Encoder::Software);
    }
    out
}

/// The configured mode if it fits `max_rate`, else the biggest picture that
/// does, at its highest frame rate
fn choose_mode(modes: &[UvcMode], wanted: &str, max_rate: u64) -> Option<UvcMode> {
    if let Some((w, h, fps)) = parse_uvc_mode(wanted)
        && let Some(m) = modes
            .iter()
            .find(|m| m.width == w && m.height == h && m.fps == fps && m.pixel_rate() <= max_rate)
    {
        return Some(m.clone());
    }
    modes
        .iter()
        .filter(|m| m.pixel_rate() <= max_rate)
        // Sharpest first, then smoothest; `max_rate` already rules out
        // what the encoder can't keep up with
        .max_by_key(|m| (m.width * m.height, m.fps, std::cmp::Reverse(m.format)))
        .cloned()
}

/// The configured device if it's plugged in, else the first USB one
fn pick_device(wanted: &str) -> Option<PathBuf> {
    let paths = capture_paths();
    if !wanted.is_empty() {
        return paths.into_iter().find(|p| p.to_string_lossy() == wanted);
    }
    paths.into_iter().next()
}

/// Only USB devices get a by-id link, so the Pi's own codec and ISP nodes
/// stay out. Index 0 is the picture; higher indices are metadata
fn capture_paths() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(BY_ID) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with("-video-index0"))
        .collect();
    paths.sort();
    paths
}

pub fn list_devices() -> Vec<UvcDevice> {
    capture_paths()
        .iter()
        .filter_map(|p| probe(p).ok())
        .collect()
}

/// Name and modes, from GStreamer's view of the device
fn probe(path: &Path) -> Result<UvcDevice, String> {
    let node = fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let node_str = node.to_string_lossy().into_owned();
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    monitor
        .start()
        .map_err(|_| "Couldn't look for video devices".to_string())?;
    let device = monitor.devices().into_iter().find(|d| {
        d.properties().is_some_and(|p| {
            p.get::<String>("device.path").ok().as_deref() == Some(node_str.as_str())
                || p.get::<String>("api.v4l2.path").ok().as_deref() == Some(node_str.as_str())
        })
    });
    monitor.stop();
    let device = device.ok_or_else(|| format!("{node_str} isn't a usable video device"))?;
    let caps = device.caps().ok_or("The device reports no formats")?;
    Ok(UvcDevice {
        path: path.to_string_lossy().into_owned(),
        name: tidy_name(&device.display_name()),
        modes: modes_from_caps(&caps),
    })
}

/// UVC names often repeat themselves: "OsmoAction4: OsmoAction4"
fn tidy_name(name: &str) -> String {
    match name.split_once(": ") {
        Some((a, b)) if a.trim() == b.trim() => a.trim().to_string(),
        _ => name.trim().to_string(),
    }
}

fn modes_from_caps(caps: &gst::Caps) -> Vec<UvcMode> {
    let mut modes: Vec<UvcMode> = Vec::new();
    for s in caps.iter() {
        let (format, raw_format) = match s.name().as_str() {
            "video/x-h264" => (Format::H264, None),
            "image/jpeg" => (Format::Mjpeg, None),
            "video/x-raw" => (Format::Raw, s.get::<String>("format").ok()),
            _ => continue,
        };
        let (Ok(width), Ok(height)) = (s.get::<i32>("width"), s.get::<i32>("height")) else {
            continue;
        };
        for fps in framerates(s) {
            let id = format!("{width}x{height}@{fps}");
            let entry = (format, raw_format.clone());
            match modes.iter_mut().find(|m| m.id == id) {
                Some(m) => {
                    if !m.formats.contains(&entry) {
                        m.formats.push(entry);
                        m.formats.sort_by_key(|(f, _)| *f);
                        m.format = m.formats[0].0;
                    }
                }
                None => modes.push(UvcMode {
                    id,
                    width: width as u32,
                    height: height as u32,
                    fps,
                    format,
                    formats: vec![entry],
                }),
            }
        }
    }
    modes.sort_by_key(|m| std::cmp::Reverse((m.width * m.height, m.fps)));
    modes
}

/// Whole frame rates a caps structure allows: a single value, a list, or
/// the ends of a range
fn framerates(s: &gst::StructureRef) -> Vec<u32> {
    let to_fps = |f: gst::Fraction| {
        (f.denom() > 0).then(|| (f.numer() as f64 / f.denom() as f64).round() as u32)
    };
    let mut out: Vec<u32> = if let Ok(f) = s.get::<gst::Fraction>("framerate") {
        to_fps(f).into_iter().collect()
    } else if let Ok(list) = s.get::<gst::List>("framerate") {
        list.iter()
            .filter_map(|v| v.get::<gst::Fraction>().ok())
            .filter_map(to_fps)
            .collect()
    } else if let Ok(range) = s.get::<gst::FractionRange>("framerate") {
        [range.min(), range.max()]
            .into_iter()
            .filter_map(to_fps)
            .collect()
    } else {
        Vec::new()
    };
    out.retain(|&f| f > 0);
    out.sort_unstable();
    out.dedup();
    out
}

fn build(
    path: &Path,
    a: &Attempt,
    cfg: &UvcSettings,
    overlay: &Overlay,
    fanout: Arc<Fanout>,
    last_frame: Arc<AtomicU64>,
) -> Result<(gst::Pipeline, bool), String> {
    let m = &a.mode;
    // Drawn in before the encode that happens anyway, so every output gets
    // it. A device's own H.264 passes through untouched: no logo there
    let logo = match a.encoder {
        Encoder::Passthrough => None,
        _ => overlay.active().map(|(logo, cfg)| {
            let picture = Rect {
                x: 0,
                y: 0,
                w: m.width,
                h: m.height,
            };
            let at = overlay::place(picture, (logo.width, logo.height), &cfg);
            format!(
                "videoconvert ! gdkpixbufoverlay location=\"{}\" offset-x={} offset-y={} \
                 overlay-width={} overlay-height={} alpha={:.2} ! ",
                overlay.path().display(),
                at.x,
                at.y,
                at.w,
                at.h,
                cfg.opacity_percent as f64 / 100.0,
            )
        }),
    };
    let burned = logo.is_some();
    let logo = logo.unwrap_or_default();
    let size = format!(
        "width={},height={},framerate={}/1",
        m.width, m.height, m.fps
    );
    let keyframe = m.fps * cfg.keyframe_secs;
    let encode = match a.encoder {
        Encoder::Passthrough => String::new(),
        Encoder::Hardware => format!(
            "{logo}videoconvert ! video/x-raw,format=I420 ! \
             v4l2h264enc extra-controls=\"controls,video_bitrate={},h264_i_frame_period={keyframe},repeat_sequence_header=1\" ! \
             video/x-h264,level=(string)4,profile=(string)high ! ",
            cfg.bitrate_kbps * 1000,
        ),
        Encoder::Software => format!(
            "{logo}videoconvert ! video/x-raw,format=I420 ! \
             x264enc tune=zerolatency speed-preset=ultrafast bitrate={} key-int-max={keyframe} ! \
             video/x-h264,profile=(string)constrained-baseline ! ",
            cfg.bitrate_kbps,
        ),
    };
    // JPEG is decoded in software: it keeps up with 1080p30 and leaves the
    // GPU memory to the encoder and the HDMI decoder
    let source = match a.format {
        Format::H264 => format!("video/x-h264,{size} ! "),
        Format::Mjpeg => format!("image/jpeg,{size} ! jpegdec ! {encode}"),
        Format::Raw => {
            let format = a
                .raw_format
                .as_deref()
                .map(|f| format!(",format={f}"))
                .unwrap_or_default();
            format!("video/x-raw{format},{size} ! {encode}")
        }
    };
    let description = format!(
        "v4l2src device={} ! {source}h264parse config-interval=-1 ! \
         video/x-h264,stream-format=byte-stream,alignment=au ! \
         appsink name=sink sync=false max-buffers=4 drop=true",
        path.display()
    );
    let pipeline = gst::parse::launch(&description)
        .map_err(|e| format!("USB camera pipeline: {e}"))?
        .dynamic_cast::<gst::Pipeline>()
        .expect("Failed to cast to Pipeline");

    let sink = pipeline
        .by_name("sink")
        .expect("Failed to find appsink")
        .dynamic_cast::<AppSink>()
        .expect("Failed to cast to AppSink");
    sink.set_callbacks(
        gstreamer_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                if let Some(buffer) = sample.buffer()
                    && let Ok(map) = buffer.map_readable()
                {
                    // Encoders put their delimiter first; ours goes last, and
                    // two per picture would read as an empty picture between
                    let picture = h264::strip_leading_aud(&map);
                    let mut au = Vec::with_capacity(picture.len() + AUD.len());
                    au.extend_from_slice(picture);
                    au.extend_from_slice(&AUD);
                    last_frame.store(now_millis(), Ordering::Relaxed);
                    fanout.video(InputMode::Uvc, &au);
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    if let Err(e) = pipeline.set_state(gst::State::Playing) {
        let reason = bus_error(&pipeline).unwrap_or_else(|| e.to_string());
        let _ = pipeline.set_state(gst::State::Null);
        return Err(reason);
    }
    Ok((pipeline, burned))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
