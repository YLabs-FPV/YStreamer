use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSrc};
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;

use super::codec::{self, Decoder as Codec};
use crate::input::h264::AccessUnitAssembler;

/// How long the worker waits for a picture before checking it should stop
const PULL_TIMEOUT: gst::ClockTime = gst::ClockTime::from_mseconds(200);
/// A scaler that fails again this soon after starting won't be fixed by
/// starting it once more
const RESTART_WINDOW: Duration = Duration::from_secs(2);
/// How far behind the video NDI and OMT play, in seconds. The goggles send a
/// keyframe about every 0.6 s; the picture after it arrives ~70 ms late and
/// the next few bunched up. Sent as they came, that was a stutter at every
/// keyframe; this long behind, they go out evenly
const PLAYOUT_DELAY: f64 = 0.1;
/// How quickly the measured frame rate follows a change
const SMOOTHING: f64 = 0.05;
/// Pictures waiting to be played out: PLAYOUT_DELAY at 60 fps and a bunch
const PLAYOUT_BUFFERS: u32 = 10;

/// What a decoded output asks for: larger pictures are scaled down, faster
/// ones thinned out
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit {
    pub max_height: u32,
    pub max_fps: u32,
}

/// One decoded picture in NV12: a full-size Y plane, then U and V
/// interleaved at half the width and height
pub struct Picture<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u8],
    pub y_stride: usize,
    pub uv: &'a [u8],
    pub uv_stride: usize,
    /// From the stream, when it says; DJI's often doesn't
    pub frame_rate: Option<(i32, i32)>,
}

impl Picture<'_> {
    /// The stream's own rate when it has one within the cap, else the cap
    pub fn rate(&self, max_fps: u32) -> (i32, i32) {
        match self.frame_rate {
            Some((n, d)) if n > 0 && d > 0 && n as i64 <= max_fps as i64 * d as i64 => (n, d),
            _ => (max_fps as i32, 1),
        }
    }

    /// Both planes back to back with no padding between rows, the layout
    /// NDI takes. Returns the stride, which is the width
    pub fn pack(&self, out: &mut Vec<u8>) -> usize {
        let w = self.width as usize;
        let h = self.height as usize;
        out.clear();
        out.reserve(w * h * 3 / 2);
        for row in 0..h {
            let at = row * self.y_stride;
            out.extend_from_slice(&self.y[at..at + w]);
        }
        for row in 0..h / 2 {
            let at = row * self.uv_stride;
            out.extend_from_slice(&self.uv[at..at + w]);
        }
        w
    }
}

/// The size a picture is scaled to: no taller than `max_height`, the same
/// shape, and even, which NV12 needs
pub fn fit(width: u32, height: u32, max_height: u32) -> (u32, u32) {
    if height <= max_height || height == 0 {
        return (width & !1, height & !1);
    }
    let w = (width as u64 * max_height as u64 + height as u64 / 2) / height as u64;
    ((w as u32 + 1) & !1, max_height & !1)
}

/// Called with each decoded picture, on the decoder's thread
type Consumer = Box<dyn FnMut(&gst::Sample) + Send>;

struct Decoding {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
}

/// Decodes the goggles' H.264 once for every output that shows pictures:
/// HDMI, NDI and OMT. The Pi has one hardware decoder, which manages about
/// 70 fps of the goggles' 1080p60; two decodes of it at once got under 40
/// each and fell further behind every second. Runs while anything
/// subscribes; each subscriber gets every picture, as the decoder's own
/// buffers, and drops what it can't keep up with
pub struct SharedDecoder {
    codec: Codec,
    assembler: Mutex<AccessUnitAssembler>,
    decoding: Mutex<Option<Decoding>>,
    consumers: Arc<Mutex<Vec<(u64, Consumer)>>>,
    next_id: AtomicU64,
}

impl SharedDecoder {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            codec: codec::decoder(),
            assembler: Mutex::new(AccessUnitAssembler::new()),
            decoding: Mutex::new(None),
            consumers: Arc::new(Mutex::new(Vec::new())),
            next_id: AtomicU64::new(0),
        })
    }

    /// Hands `consumer` every picture until the subscription is dropped
    pub fn subscribe(
        self: &Arc<Self>,
        consumer: impl FnMut(&gst::Sample) + Send + 'static,
    ) -> Result<Subscription, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.consumers
            .lock()
            .unwrap()
            .push((id, Box::new(consumer)));
        let subscription = Subscription {
            decoder: Arc::clone(self),
            id,
        };
        let mut decoding = self.decoding.lock().unwrap();
        if decoding.is_none() {
            *decoding = Some(self.start()?);
        }
        Ok(subscription)
    }

    fn start(&self) -> Result<Decoding, String> {
        // Held to NV12: left to choose, the Pi's decoder switched to I420
        // when the video changed size, which its image processor refused
        let decoder = match self.codec {
            Codec::Software => "avdec_h264".to_string(),
            hw => format!("{} ! video/x-raw,format=NV12", hw.element()),
        };
        // Parameter sets before every keyframe, so a restarted decoder picks
        // up at the next one. What can't be decoded in time is dropped as
        // H.264 rather than queueing up as delay
        let description = format!(
            "appsrc name=src leaky-type=downstream ! h264parse config-interval=-1 ! {decoder} ! \
             appsink name=sink sync=false max-buffers=1 drop=true"
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("decode pipeline: {e}"))?
            .dynamic_cast::<gst::Pipeline>()
            .expect("Failed to cast to Pipeline");

        let appsrc = pipeline
            .by_name("src")
            .expect("Failed to find appsrc")
            .dynamic_cast::<AppSrc>()
            .expect("Failed to cast to AppSrc");
        appsrc.set_caps(Some(
            &gst::Caps::builder("video/x-h264")
                .field("stream-format", "byte-stream")
                .field("alignment", "au")
                .build(),
        ));
        appsrc.set_is_live(true);
        appsrc.set_format(gst::Format::Time);
        appsrc.set_do_timestamp(true);
        // Under a second of the goggles' video. What's dropped here smears
        // the picture until the next keyframe, so it's a last resort for
        // when the decoder itself can't keep up
        appsrc.set_max_bytes(1_500_000);
        appsrc.set_block(false);

        // Handed on from the decoder's thread: every subscriber's own
        // pipeline takes it without waiting
        let consumers = Arc::clone(&self.consumers);
        let sink = pipeline
            .by_name("sink")
            .expect("Failed to find appsink")
            .dynamic_cast::<AppSink>()
            .expect("Failed to cast to AppSink");
        // The Pi's decoder pads 1080 rows to 1088. Every subscriber reads
        // the layout from the buffer, but unless appsink says so the decoder
        // copied each picture into an unpadded one first: three quarters of
        // a core at 1080p60
        sink.static_pad("sink")
            .expect("Failed to find appsink's pad")
            .add_probe(gst::PadProbeType::QUERY_DOWNSTREAM, |_, info| {
                if let Some(gst::PadProbeData::Query(query)) = &mut info.data
                    && let gst::QueryViewMut::Allocation(allocation) = query.view_mut()
                {
                    allocation.add_allocation_meta::<gst_video::VideoMeta>(None);
                }
                gst::PadProbeReturn::Ok
            });
        sink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    for (_, consumer) in consumers.lock().unwrap().iter_mut() {
                        consumer(&sample);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            let reason = take_error(&pipeline).unwrap_or_else(|| e.to_string());
            let _ = pipeline.set_state(gst::State::Null);
            return Err(reason);
        }
        Ok(Decoding { pipeline, appsrc })
    }

    /// Raw chunks from the source, cut into pictures here
    pub fn push(&self, chunk: &[u8]) {
        if self.decoding.lock().unwrap().is_none() {
            return;
        }
        let mut assembler = self.assembler.lock().unwrap();
        assembler.push(chunk);
        while let Some(au) = assembler.next_au() {
            self.push_owned(au);
        }
    }

    /// One complete picture, e.g. from the splash screen
    pub fn push_au(&self, au: &[u8]) {
        self.push_owned(au.to_vec());
    }

    fn push_owned(&self, mut au: Vec<u8>) {
        if au.len() < 10 {
            return;
        }
        let mut decoding = self.decoding.lock().unwrap();
        let Some(d) = decoding.as_ref() else {
            return;
        };
        // Started again, to pick up at the next keyframe
        if let Some(e) = take_error(&d.pipeline) {
            eprintln!("[decode] {e}, restarting");
            let _ = d.pipeline.set_state(gst::State::Null);
            *decoding = None;
            match self.start() {
                Ok(d) => *decoding = Some(d),
                Err(e) => {
                    eprintln!("[decode] {e}");
                    return;
                }
            }
        }
        if self.codec.needs_level_patch() {
            super::hdmi::patch_sps_level(&mut au);
        }
        if let Some(d) = decoding.as_ref() {
            let _ = d.appsrc.push_buffer(gst::Buffer::from_mut_slice(au));
        }
    }

    #[cfg(test)]
    pub fn pipeline(&self) -> Option<gst::Pipeline> {
        self.decoding
            .lock()
            .unwrap()
            .as_ref()
            .map(|d| d.pipeline.clone())
    }

    fn unsubscribe(&self, id: u64) {
        let idle = {
            let mut consumers = self.consumers.lock().unwrap();
            consumers.retain(|(other, _)| *other != id);
            consumers.is_empty()
        };
        // Stopping waits for the decoder's thread, which takes the
        // consumers' lock: not held here
        if idle && let Some(d) = self.decoding.lock().unwrap().take() {
            let _ = d.pipeline.set_state(gst::State::Null);
            *self.assembler.lock().unwrap() = AccessUnitAssembler::new();
        }
    }
}

/// Pictures keep coming while this is held
pub struct Subscription {
    decoder: Arc<SharedDecoder>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.decoder.unsubscribe(self.id);
    }
}

/// Keeps an even share of the pictures to bring their rate down to the
/// limit, e.g. 3 of every 5 from 50 fps to 30. videorate went by arrival
/// times, and as the goggles' jitter, it dropped two in a row now and then
struct Thinner {
    max_fps: f64,
    /// Seconds between pictures, smoothed
    interval: f64,
    last: Option<f64>,
    share: f64,
}

impl Thinner {
    fn new(max_fps: u32) -> Self {
        Self {
            max_fps: max_fps as f64,
            interval: 0.0,
            last: None,
            share: 0.0,
        }
    }

    /// Whether to keep the picture that arrived `at` seconds
    fn keep(&mut self, at: f64) -> bool {
        if let Some(last) = self.last {
            let gap = at - last;
            self.interval = if self.interval > 0.0 {
                self.interval + (gap - self.interval) * SMOOTHING
            } else {
                gap
            };
        }
        self.last = Some(at);
        self.share += if self.interval > 0.0 {
            (self.max_fps * self.interval).min(1.0)
        } else {
            1.0
        };
        if self.share >= 1.0 {
            self.share -= 1.0;
            true
        } else {
            false
        }
    }
}

/// When to send each picture: evenly spaced, PLAYOUT_DELAY behind their
/// arrival, and slowly following the video's actual rate
#[derive(Default)]
struct Pacer {
    /// Seconds between pictures, smoothed
    period: f64,
    last_arrival: Option<f64>,
    last_release: Option<f64>,
}

impl Pacer {
    fn release(&mut self, arrival: f64) -> f64 {
        if let Some(last) = self.last_arrival {
            let gap = arrival - last;
            self.period = if self.period > 0.0 {
                self.period + (gap - self.period) * SMOOTHING
            } else {
                gap
            };
        }
        self.last_arrival = Some(arrival);
        let target = arrival + PLAYOUT_DELAY;
        let at = match self.last_release {
            Some(last) => {
                let even = last + self.period;
                even + (target - even) * SMOOTHING
            }
            None => target,
        };
        // Never before it's there, nor so late that delay builds up
        let at = at.clamp(arrival, arrival + 2.0 * PLAYOUT_DELAY);
        self.last_release = Some(at);
        at
    }
}

/// Scales the shared decoder's pictures for one output
struct Scaler {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    sink: AppSink,
}

impl Scaler {
    fn start(hardware: bool, limit: Limit) -> Result<Self, String> {
        // The Pi's image processor scales, NV12 to NV12; videoscale did it on
        // one CPU core. It takes the decoder's buffers as they are: copying
        // them in from the decoder's uncached memory took a third of a core
        let scaler = if hardware {
            "v4l2convert name=scale output-io-mode=dmabuf-import"
        } else {
            "videoscale name=scale ! videoconvert"
        };
        // Its own thread, so the decoder never waits on the scaler. Room for
        // a bunch of pictures in, and for those waiting to be played out
        let description = format!(
            "appsrc name=src leaky-type=downstream max-buffers=4 ! \
             {scaler} ! \
             capsfilter name=size caps=video/x-raw,format=NV12 ! \
             appsink name=sink sync=false max-buffers={PLAYOUT_BUFFERS} drop=true"
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("decode pipeline: {e}"))?
            .dynamic_cast::<gst::Pipeline>()
            .expect("Failed to cast to Pipeline");

        let appsrc = pipeline
            .by_name("src")
            .expect("Failed to find appsrc")
            .dynamic_cast::<AppSrc>()
            .expect("Failed to cast to AppSrc");
        appsrc.set_is_live(true);
        appsrc.set_format(gst::Format::Time);
        appsrc.set_block(false);

        // The target size depends on the video's, which is only known once
        // it's decoded; it can also change, e.g. 16:9 to 4:3
        let size = pipeline.by_name("size").expect("Failed to find capsfilter");
        let scale_sink = pipeline
            .by_name("scale")
            .and_then(|s| s.static_pad("sink"))
            .expect("Failed to find videoscale");
        scale_sink.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
            if let Some(gst::PadProbeData::Event(event)) = &info.data
                && let gst::EventView::Caps(caps) = event.view()
                && let Some(s) = caps.caps().structure(0)
                && let (Ok(w), Ok(h)) = (s.get::<i32>("width"), s.get::<i32>("height"))
            {
                let (w, h) = fit(w.max(0) as u32, h.max(0) as u32, limit.max_height);
                size.set_property(
                    "caps",
                    gst::Caps::builder("video/x-raw")
                        .field("format", "NV12")
                        .field("width", w as i32)
                        .field("height", h as i32)
                        .build(),
                );
            }
            gst::PadProbeReturn::Ok
        });

        let sink = pipeline
            .by_name("sink")
            .expect("Failed to find appsink")
            .dynamic_cast::<AppSink>()
            .expect("Failed to cast to AppSink");
        // The image processor writes into as many buffers as are asked for;
        // with only its default few, pictures held for playing out would
        // stall it
        sink.static_pad("sink")
            .expect("Failed to find appsink's pad")
            .add_probe(gst::PadProbeType::QUERY_DOWNSTREAM, |_, info| {
                if let Some(gst::PadProbeData::Query(query)) = &mut info.data
                    && let gst::QueryViewMut::Allocation(allocation) = query.view_mut()
                {
                    allocation.add_allocation_meta::<gst_video::VideoMeta>(None);
                    allocation.add_allocation_pool(
                        None::<&gst::BufferPool>,
                        0,
                        PLAYOUT_BUFFERS + 2,
                        0,
                    );
                }
                gst::PadProbeReturn::Ok
            });

        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            let reason = take_error(&pipeline).unwrap_or_else(|| e.to_string());
            let _ = pipeline.set_state(gst::State::Null);
            return Err(reason);
        }
        Ok(Self {
            pipeline,
            appsrc,
            sink,
        })
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Hands decoded pictures to a callback on its own thread: scaled down,
/// thinned out to the limit and played out evenly. When the callback is
/// slower than the video, pictures are skipped instead of queueing up as
/// delay
pub struct Decoder {
    /// Why the scaler couldn't be started again
    failed: Arc<Mutex<Option<String>>>,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// Dropped first, so nothing is handed to a pipeline being stopped
    subscription: Option<Subscription>,
}

impl Decoder {
    pub fn start(
        shared: &Arc<SharedDecoder>,
        limit: Limit,
        mut on_picture: impl FnMut(&Picture) + Send + 'static,
    ) -> Result<Self, String> {
        // The Pi's image processor; nothing else here has one that's known
        let hardware = shared.codec == Codec::V4l2;
        // What arrival times count from
        let started = Instant::now();
        let mut scaler = Scaler::start(hardware, limit)?;
        // Where the decoder hands pictures in; swapped when the scaler starts
        // again
        let feed = Arc::new(Mutex::new(scaler.appsrc.clone()));
        let failed = Arc::new(Mutex::new(None));
        let running = Arc::new(AtomicBool::new(true));
        let worker = {
            let feed = Arc::clone(&feed);
            let failed = Arc::clone(&failed);
            let running = Arc::clone(&running);
            thread::spawn(move || {
                let mut pacer = Pacer::default();
                let mut restarted: Option<Instant> = None;
                while running.load(Ordering::Relaxed) {
                    if let Some(sample) = scaler.sink.try_pull_sample(PULL_TIMEOUT)
                        && play_out(started, &sample, &mut pacer)
                    {
                        deliver(&sample, &mut on_picture);
                    }
                    let Some(e) = take_error(&scaler.pipeline) else {
                        continue;
                    };
                    // Taking the decoder's buffers as they are, the image
                    // processor refuses some changes of size mid-stream; a
                    // fresh one starts at the new size. Failing again at
                    // once is something else, left to the output to report
                    if restarted.is_some_and(|t| t.elapsed() < RESTART_WINDOW) {
                        *failed.lock().unwrap() = Some(e);
                        break;
                    }
                    if crate::settings::verbose() {
                        println!("[decode] scaler: {e}, starting again");
                    }
                    restarted = Some(Instant::now());
                    match Scaler::start(hardware, limit) {
                        Ok(fresh) => {
                            *feed.lock().unwrap() = fresh.appsrc.clone();
                            scaler = fresh;
                            pacer = Pacer::default();
                        }
                        Err(e) => {
                            *failed.lock().unwrap() = Some(e);
                            break;
                        }
                    }
                }
            })
        };
        let mut this = Self {
            failed,
            running,
            worker: Some(worker),
            subscription: None,
        };
        let mut thinner = Thinner::new(limit.max_fps);
        // Skipped while the scaler starts again, so the decoder doesn't wait
        this.subscription = Some(shared.subscribe(move |sample| {
            let arrived = started.elapsed();
            if !thinner.keep(arrived.as_secs_f64()) {
                return;
            }
            let (Ok(appsrc), Some(mut buffer), Some(caps)) =
                (feed.try_lock(), sample.buffer_owned(), sample.caps_owned())
            else {
                return;
            };
            // Stamped with when it arrived, which playing out goes by. The
            // decoder's own times count from when it started, perhaps an
            // hour before this output did. The picture itself isn't copied
            let stamped = buffer.make_mut();
            stamped.set_pts(gst::ClockTime::from_nseconds(arrived.as_nanos() as u64));
            stamped.set_dts(gst::ClockTime::NONE);
            // Carries the caps along, so a change of size reaches the scaler
            let _ = appsrc.push_sample(&gst::Sample::builder().buffer(&buffer).caps(&caps).build());
        })?);
        Ok(this)
    }

    /// Why the output stopped getting pictures, if it has
    pub fn error(&self) -> Option<String> {
        self.failed.lock().unwrap().clone()
    }
}

/// Waits for the picture's turn; false when it's already too late for it,
/// as the callback fell behind, and should be skipped
fn play_out(started: Instant, sample: &gst::Sample, pacer: &mut Pacer) -> bool {
    let Some(pts) = sample.buffer().and_then(|b| b.pts()) else {
        return true;
    };
    let now = started.elapsed().as_secs_f64();
    let at = pacer.release(pts.nseconds() as f64 / 1e9);
    if now > at + pacer.period {
        return false;
    }
    if at > now {
        thread::sleep(Duration::from_secs_f64(at - now));
    }
    true
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.subscription.take();
        self.running.store(false, Ordering::Relaxed);
        // Stops the scaler as it ends
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn take_error(pipeline: &gst::Pipeline) -> Option<String> {
    let bus = pipeline.bus()?;
    let mut last = None;
    while let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error]) {
        if let gst::MessageView::Error(e) = msg.view() {
            last = Some(e.error().to_string());
        }
    }
    last
}

fn deliver(sample: &gst::Sample, on_picture: &mut impl FnMut(&Picture)) {
    let (Some(caps), Some(buffer)) = (sample.caps(), sample.buffer()) else {
        return;
    };
    let Ok(info) = gst_video::VideoInfo::from_caps(caps) else {
        return;
    };
    if info.format() != gst_video::VideoFormat::Nv12 {
        return;
    }
    let Ok(frame) = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info) else {
        return;
    };
    let (Ok(y), Ok(uv)) = (frame.plane_data(0), frame.plane_data(1)) else {
        return;
    };
    let fps = info.fps();
    let picture = Picture {
        width: info.width(),
        height: info.height(),
        y,
        y_stride: frame.plane_stride()[0] as usize,
        uv,
        uv_stride: frame.plane_stride()[1] as usize,
        frame_rate: (fps.numer() > 0 && fps.denom() > 0).then(|| (fps.numer(), fps.denom())),
    };
    on_picture(&picture);
}

#[cfg(test)]
#[path = "decoded.test.rs"]
mod tests;
