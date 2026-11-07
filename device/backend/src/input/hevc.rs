use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSrc};
use serde::Serialize;

use super::h264;
use super::h265::{self, ParamSets};

/// After a picture a decoder can start from went in, this long without a
/// picture out means this way of converting doesn't work here
const FIRST_PICTURE: Duration = Duration::from_secs(4);
/// What comes out goes to every output, the browser included, so it's
/// sized like a typical goggles stream
const BITRATE_KBPS: u32 = 8000;
const KEYFRAME_FRAMES: u32 = 30;
/// After every converter failed, before trying again
const RETRY: Duration = Duration::from_secs(5);
/// Appended to every picture, as the goggles do; see `uvc.rs`
const AUD: [u8; 6] = [0, 0, 0, 1, 0x09, 0xf0];

/// How H.265 becomes the H.264 every output takes. The Pi 4 decodes
/// H.265 in hardware but only encodes H.264, so this is the one
/// conversion the device makes; outputs stay as they are. Only the
/// hardware codecs: on the CPU an RC-N3 + Neo 2 stream decoded at 27
/// pictures a second and the delay grew by the second
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Converter {
    /// The H.265 decoder's pictures come out tiled, which neither the
    /// encoder nor the converter take from GStreamer, so they're untiled
    /// on the CPU first
    Hardware,
}

const CONVERTERS: [Converter; 1] = [Converter::Hardware];

/// The newest picture only. The decoder keeps up with anything a remote
/// sends (150 pictures a second at 1080p), so nothing queues in front of
/// it; when untiling or the encoder fall behind they skip pictures rather
/// than queue them, which would grow the delay without end
const LATEST: &str = "queue max-size-buffers=1 max-size-bytes=0 max-size-time=0 leaky=downstream";

impl Converter {
    fn elements(self) -> String {
        let hw_encode = format!(
            "v4l2h264enc extra-controls=\"controls,video_bitrate={},h264_i_frame_period={KEYFRAME_FRAMES},repeat_sequence_header=1\" ! \
             video/x-h264,level=(string)4.2,profile=(string)high",
            BITRATE_KBPS * 1000
        );
        match self {
            Self::Hardware => format!(
                "v4l2slh265dec ! video/x-raw,format=NV12_128C8 ! {LATEST} ! \
                 videoconvert n-threads=4 ! video/x-raw,format=NV12 ! {hw_encode}"
            ),
        }
    }
}

struct Running {
    pipeline: gst::Pipeline,
    src: AppSrc,
    converter: Converter,
    /// When a picture a decoder can start from went in
    irap_at: Option<Instant>,
}

/// Gets every converted picture, an H.264 access unit
pub type OnPicture = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Turns the H.265 from an RC-N remote into H.264 for the outputs
pub struct Transcoder {
    on_picture: OnPicture,
    running: Mutex<Option<Running>>,
    /// Index into `CONVERTERS` of the next one to try
    next: Mutex<usize>,
    /// Unix millis of the last picture out; shared with the appsink callback
    last_out: Arc<AtomicU64>,
    retry_at: Mutex<Option<Instant>>,
}

impl Transcoder {
    pub fn new(on_picture: OnPicture) -> Self {
        Self {
            on_picture,
            running: Mutex::new(None),
            next: Mutex::new(0),
            last_out: Arc::new(AtomicU64::new(0)),
            retry_at: Mutex::new(None),
        }
    }

    /// The converter in use, if one is running
    pub fn converter(&self) -> Option<Converter> {
        self.running.lock().unwrap().as_ref().map(|r| r.converter)
    }

    /// Feed the link's bytes; starts a pipeline when there's none, and moves
    /// on to the next converter when one turns out not to work.
    /// Errors are for the status, not fatal
    pub fn push(&self, data: &[u8], params: &ParamSets) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        if let Some(r) = running.as_ref()
            && let Some(reason) = self.failed(r)
        {
            let converter = r.converter;
            stop(running.take());
            eprintln!("[hevc] {converter:?} conversion didn't work: {reason}");
        }
        if running.is_none() {
            let mut retry_at = self.retry_at.lock().unwrap();
            if retry_at.is_some_and(|at| Instant::now() < at) {
                return Err("Couldn't convert the H.265 video".into());
            }
            match self.start(params) {
                Ok(r) => {
                    *retry_at = None;
                    *running = Some(r);
                }
                Err(e) => {
                    *retry_at = Some(Instant::now() + RETRY);
                    return Err(e);
                }
            }
        }
        let r = running.as_mut().expect("started above");
        if r.irap_at.is_none() && h265::has_irap(data) {
            r.irap_at = Some(Instant::now());
        }
        let _ = r.src.push_buffer(gst::Buffer::from_slice(data.to_vec()));
        Ok(())
    }

    /// Back to trying the hardware first, e.g. for a new link
    pub fn stop(&self) {
        stop(self.running.lock().unwrap().take());
        *self.next.lock().unwrap() = 0;
        *self.retry_at.lock().unwrap() = None;
    }

    /// A fresh pipeline on the next push, with the converter that worked:
    /// for another camera, whose pictures mustn't be decoded against the
    /// last one's
    pub fn restart(&self) {
        let mut running = self.running.lock().unwrap();
        if let Some(r) = running.as_ref()
            && let Some(i) = CONVERTERS.iter().position(|&c| c == r.converter)
        {
            *self.next.lock().unwrap() = i;
        }
        stop(running.take());
    }

    fn failed(&self, r: &Running) -> Option<String> {
        if let Some(e) = bus_error(&r.pipeline) {
            return Some(e);
        }
        let waited = r.irap_at.is_some_and(|at| at.elapsed() > FIRST_PICTURE);
        if waited && self.last_out.load(Ordering::Relaxed) == 0 {
            return Some("no pictures came out".into());
        }
        None
    }

    fn start(&self, params: &ParamSets) -> Result<Running, String> {
        let mut next = self.next.lock().unwrap();
        loop {
            let Some(&converter) = CONVERTERS.get(*next) else {
                // Every one failed: start over, the link may have been odd
                *next = 0;
                return Err("Couldn't convert the H.265 video".into());
            };
            *next += 1;
            self.last_out.store(0, Ordering::Relaxed);
            match build(
                converter,
                Arc::clone(&self.on_picture),
                Arc::clone(&self.last_out),
            ) {
                Ok((pipeline, src)) => {
                    println!("[hevc] converting H.265 to H.264 ({converter:?})");
                    // A remote sends its parameter sets only once; a decoder
                    // started after that needs them handed over first
                    if let Some(params) = params.annex_b() {
                        let _ = src.push_buffer(gst::Buffer::from_slice(params));
                    }
                    return Ok(Running {
                        pipeline,
                        src,
                        converter,
                        irap_at: None,
                    });
                }
                Err(e) => eprintln!("[hevc] {converter:?} conversion unavailable: {e}"),
            }
        }
    }
}

impl Drop for Transcoder {
    fn drop(&mut self) {
        stop(self.running.lock().unwrap().take());
    }
}

fn stop(running: Option<Running>) {
    if let Some(r) = running {
        let _ = r.pipeline.set_state(gst::State::Null);
    }
}

fn bus_error(pipeline: &gst::Pipeline) -> Option<String> {
    let msg = pipeline.bus()?.pop_filtered(&[gst::MessageType::Error])?;
    match msg.view() {
        gst::MessageView::Error(e) => Some(e.error().to_string()),
        _ => None,
    }
}

fn build(
    converter: Converter,
    on_picture: OnPicture,
    last_out: Arc<AtomicU64>,
) -> Result<(gst::Pipeline, AppSrc), String> {
    gst::init().map_err(|e| e.to_string())?;
    let description = format!(
        "appsrc name=src is-live=true do-timestamp=true format=time \
         caps=video/x-h265,stream-format=byte-stream ! h265parse ! {} ! \
         h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au ! \
         appsink name=sink sync=false max-buffers=4 drop=true",
        converter.elements()
    );
    let pipeline = gst::parse::launch(&description)
        .map_err(|e| e.to_string())?
        .dynamic_cast::<gst::Pipeline>()
        .map_err(|_| "not a pipeline".to_string())?;
    let src = pipeline
        .by_name("src")
        .and_then(|e| e.dynamic_cast::<AppSrc>().ok())
        .ok_or("no appsrc")?;
    let sink = pipeline
        .by_name("sink")
        .and_then(|e| e.dynamic_cast::<AppSink>().ok())
        .ok_or("no appsink")?;
    sink.set_callbacks(
        gstreamer_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                if let Some(buffer) = sample.buffer()
                    && let Ok(map) = buffer.map_readable()
                {
                    let picture = h264::strip_leading_aud(&map);
                    let mut au = Vec::with_capacity(picture.len() + AUD.len());
                    au.extend_from_slice(picture);
                    au.extend_from_slice(&AUD);
                    last_out.store(now_millis(), Ordering::Relaxed);
                    on_picture(&au);
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
    Ok((pipeline, src))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1)
}

#[cfg(test)]
#[path = "hevc.test.rs"]
mod tests;
