use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use serde::Serialize;

use crate::input::h264::has_keyframe;
use crate::settings::RtmpSettings;

const POLL: Duration = Duration::from_secs(1);
const FIRST_RETRY: Duration = Duration::from_secs(2);
const MAX_RETRY: Duration = Duration::from_secs(15);

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RtmpState {
    #[default]
    Off,
    Connecting,
    Live,
    /// Waiting out the delay before the next attempt
    Retrying,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct RtmpStatus {
    pub state: RtmpState,
    pub error: Option<String>,
    pub uptime_secs: u64,
    pub bitrate_kbps: u32,
}

struct Running {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    sink: gst::Element,
    live_since: Option<Instant>,
    waiting_for_keyframe: bool,
    bytes: u64,
}

struct Inner {
    cfg: RtmpSettings,
    running: Option<Running>,
    retry_at: Option<Instant>,
    backoff: Duration,
    status: RtmpStatus,
}

pub struct RtmpOutput {
    inner: Mutex<Inner>,
}

impl RtmpOutput {
    pub fn new(cfg: &RtmpSettings) -> Arc<Self> {
        let this = Arc::new(Self {
            inner: Mutex::new(Inner {
                cfg: RtmpSettings::default(),
                running: None,
                retry_at: None,
                backoff: FIRST_RETRY,
                status: RtmpStatus::default(),
            }),
        });
        this.apply(cfg);
        let supervisor = Arc::clone(&this);
        thread::spawn(move || {
            loop {
                thread::sleep(POLL);
                supervisor.supervise();
            }
        });
        this
    }

    pub fn status(&self) -> RtmpStatus {
        self.inner.lock().unwrap().status.clone()
    }

    /// Never fails: a server that's down is retried, not refused, since it
    /// may well come up later
    pub fn apply(&self, cfg: &RtmpSettings) {
        let mut inner = self.inner.lock().unwrap();
        stop(&mut inner);
        inner.cfg = cfg.clone();
        inner.backoff = FIRST_RETRY;
        inner.status = RtmpStatus::default();
        if cfg.enabled {
            connect(&mut inner);
        }
    }

    pub fn push(&self, data: &[u8]) {
        let mut inner = self.inner.lock().unwrap();
        let Some(r) = inner.running.as_mut() else {
            return;
        };
        // The platform's first picture must decode
        if r.waiting_for_keyframe {
            if !has_keyframe(data) {
                return;
            }
            r.waiting_for_keyframe = false;
        }
        let _ = r.appsrc.push_buffer(gst::Buffer::from_slice(data.to_vec()));
    }

    fn supervise(&self) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.cfg.enabled {
            return;
        }

        let Some(r) = inner.running.as_mut() else {
            if inner.retry_at.is_some_and(|t| Instant::now() >= t) {
                connect(&mut inner);
            }
            return;
        };

        let error = r.pipeline.bus().and_then(|bus| {
            bus.pop_filtered(&[gst::MessageType::Error])
                .and_then(|m| match m.view() {
                    gst::MessageView::Error(e) => Some(e.error().to_string()),
                    _ => None,
                })
        });
        if let Some(e) = error {
            fail(&mut inner, e);
            return;
        }

        // Bytes going out means the server took the stream
        let sent = r
            .sink
            .property::<Option<gst::Structure>>("stats")
            .and_then(|s| s.get::<u64>("out-bytes-total").ok())
            .unwrap_or(0);
        let rate = sent.saturating_sub(r.bytes) * 8 / 1000 / POLL.as_secs();
        r.bytes = sent;
        if r.live_since.is_none() && rate > 0 {
            r.live_since = Some(Instant::now());
            eprintln!("[rtmp] live");
        }
        let (state, uptime) = match r.live_since {
            Some(t) => (RtmpState::Live, t.elapsed().as_secs()),
            None => (RtmpState::Connecting, 0),
        };
        let backoff_reset = state == RtmpState::Live && uptime > MAX_RETRY.as_secs();
        inner.status = RtmpStatus {
            state,
            error: None,
            uptime_secs: uptime,
            bitrate_kbps: rate as u32,
        };
        // A connection that held for a while earns a fresh start on the delay
        if backoff_reset {
            inner.backoff = FIRST_RETRY;
        }
    }
}

fn connect(inner: &mut Inner) {
    inner.retry_at = None;
    match start(&inner.cfg) {
        Ok(r) => {
            eprintln!("[rtmp] connecting to {}", redact(&inner.cfg));
            inner.running = Some(r);
            inner.status.state = RtmpState::Connecting;
        }
        Err(e) => fail(inner, e),
    }
}

fn fail(inner: &mut Inner, error: String) {
    stop(inner);
    eprintln!("[rtmp] {error}, retrying in {}s", inner.backoff.as_secs());
    inner.status = RtmpStatus {
        state: RtmpState::Retrying,
        error: Some(error),
        ..Default::default()
    };
    inner.retry_at = Some(Instant::now() + inner.backoff);
    inner.backoff = (inner.backoff * 2).min(MAX_RETRY);
}

fn stop(inner: &mut Inner) {
    if let Some(r) = inner.running.take() {
        let _ = r.pipeline.set_state(gst::State::Null);
        if r.live_since.is_some() {
            eprintln!("[rtmp] stopped");
        }
    }
    inner.retry_at = None;
}

/// The key is a password; keep it out of the logs
fn redact(cfg: &RtmpSettings) -> String {
    cfg.url.trim().trim_end_matches('/').to_string()
}

fn start(cfg: &RtmpSettings) -> Result<Running, String> {
    let audio = if cfg.silent_audio {
        let encoder = if gst::ElementFactory::find("voaacenc").is_some() {
            "voaacenc bitrate=64000"
        } else {
            "avenc_aac bitrate=64000"
        };
        format!(
            " audiotestsrc is-live=true wave=silence ! audioconvert ! audioresample ! \
             audio/x-raw,rate=44100,channels=2 ! {encoder} ! aacparse ! queue ! mux."
        )
    } else {
        String::new()
    };
    // Parameter sets before every keyframe: some platforms only look for
    // them there, and it lets a reconnect start cleanly
    let description = format!(
        "appsrc name=src ! h264parse config-interval=-1 ! queue ! \
         flvmux name=mux streamable=true ! rtmp2sink name=sink{audio}"
    );
    let pipeline = gst::parse::launch(&description)
        .map_err(|e| format!("RTMP pipeline: {e}"))?
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
            .field("alignment", "stream")
            .build(),
    ));
    appsrc.set_is_live(true);
    appsrc.set_format(gst::Format::Time);
    appsrc.set_do_timestamp(true);
    appsrc.set_max_bytes(8_000_000);
    appsrc.set_property("block", false);

    let sink = pipeline.by_name("sink").expect("Failed to find rtmp2sink");
    sink.set_property("location", cfg.location());

    if let Err(e) = pipeline.set_state(gst::State::Playing) {
        let reason = pipeline
            .bus()
            .and_then(|b| b.pop_filtered(&[gst::MessageType::Error]))
            .and_then(|m| match m.view() {
                gst::MessageView::Error(e) => Some(e.error().to_string()),
                _ => None,
            })
            .unwrap_or_else(|| e.to_string());
        let _ = pipeline.set_state(gst::State::Null);
        return Err(reason);
    }

    Ok(Running {
        pipeline,
        appsrc,
        sink,
        live_since: None,
        waiting_for_keyframe: true,
        bytes: 0,
    })
}
