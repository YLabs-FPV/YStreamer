use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSrc};
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;

use crate::input::h264::AccessUnitAssembler;

/// How long the worker waits for a picture before checking it should stop
const PULL_TIMEOUT: gst::ClockTime = gst::ClockTime::from_mseconds(200);

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

/// Decodes the H.264 into pictures and hands each to a callback on its own
/// thread. Only the newest picture waits there: when the callback is slower
/// than the video, the ones in between are dropped instead of queueing up
/// as delay
pub struct Decoder {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    assembler: AccessUnitAssembler,
    /// The Pi's decoder only takes up to level 4.2
    hardware: bool,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Decoder {
    pub fn start(
        limit: Limit,
        mut on_picture: impl FnMut(&Picture) + Send + 'static,
    ) -> Result<Self, String> {
        // The Pi's hardware decoder; the software one is for other machines
        let hardware = gst::ElementFactory::find("v4l2h264dec").is_some();
        let decoder = if hardware {
            "v4l2h264dec"
        } else {
            "avdec_h264"
        };
        let description = format!(
            "appsrc name=src ! h264parse ! {decoder} ! \
             videorate drop-only=true max-rate={} ! \
             videoscale name=scale ! videoconvert ! \
             capsfilter name=size caps=video/x-raw,format=NV12 ! \
             appsink name=sink sync=false max-buffers=1 drop=true",
            limit.max_fps
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
        appsrc.set_max_bytes(4_000_000);
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

        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            let reason = take_error(&pipeline).unwrap_or_else(|| e.to_string());
            let _ = pipeline.set_state(gst::State::Null);
            return Err(reason);
        }

        let running = Arc::new(AtomicBool::new(true));
        let worker = {
            let running = Arc::clone(&running);
            thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    if let Some(sample) = sink.try_pull_sample(PULL_TIMEOUT) {
                        deliver(&sample, &mut on_picture);
                    }
                }
            })
        };

        Ok(Self {
            pipeline,
            appsrc,
            assembler: AccessUnitAssembler::new(),
            hardware,
            running,
            worker: Some(worker),
        })
    }

    /// Raw chunks from the source, cut into pictures here
    pub fn push(&mut self, chunk: &[u8]) {
        self.assembler.push(chunk);
        while let Some(au) = self.assembler.next_au() {
            self.push_owned(au);
        }
    }

    /// One complete picture, e.g. from the splash screen
    pub fn push_au(&self, au: &[u8]) {
        self.push_owned(au.to_vec());
    }

    fn push_owned(&self, mut au: Vec<u8>) {
        if self.hardware {
            super::hdmi::patch_sps_level(&mut au);
        }
        let _ = self.appsrc.push_buffer(gst::Buffer::from_mut_slice(au));
    }

    /// The pipeline's latest error, if it has failed since the last look
    pub fn error(&self) -> Option<String> {
        take_error(&self.pipeline)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.pipeline.set_state(gst::State::Null);
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
