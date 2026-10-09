use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use serde::Serialize;

use crate::settings::{SrtMode, SrtSettings};

const POLL: Duration = Duration::from_secs(1);
const ACK_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct SrtStatus {
    pub active: bool,
    pub clients: u32,
    pub error: Option<String>,
}

struct Running {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    sink: gst::Element,
    mode: SrtMode,
    acks: i32,
    last_ack: Option<Instant>,
}

pub struct SrtOutput {
    running: Mutex<Option<Running>>,
    status: Mutex<SrtStatus>,
}

impl SrtOutput {
    pub fn new(cfg: &SrtSettings) -> Arc<Self> {
        let this = Arc::new(Self {
            running: Mutex::new(None),
            status: Mutex::new(SrtStatus::default()),
        });
        if let Err(e) = this.apply(cfg) {
            eprintln!("[srt] not started: {e}");
        }
        let watcher = Arc::clone(&this);
        thread::spawn(move || {
            loop {
                thread::sleep(POLL);
                watcher.poll();
            }
        });
        this
    }

    pub fn status(&self) -> SrtStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn apply(&self, cfg: &SrtSettings) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        if let Some(old) = running.take() {
            let _ = old.pipeline.set_state(gst::State::Null);
            eprintln!("[srt] stopped");
        }
        *self.status.lock().unwrap() = SrtStatus::default();
        if !cfg.enabled {
            return Ok(());
        }

        let result = start(cfg);
        let mut status = self.status.lock().unwrap();
        match result {
            Ok(r) => {
                match cfg.mode {
                    SrtMode::Listener => eprintln!("[srt] listening on port {}", cfg.port),
                    SrtMode::Caller => {
                        eprintln!("[srt] sending to {}:{}", cfg.host.trim(), cfg.port)
                    }
                }
                status.active = true;
                *running = Some(r);
                Ok(())
            }
            Err(e) => {
                status.error = Some(e.clone());
                Err(e)
            }
        }
    }

    pub fn push(&self, data: &[u8]) {
        let guard = self.running.lock().unwrap();
        let Some(r) = guard.as_ref() else {
            return;
        };
        let _ = r.appsrc.push_buffer(gst::Buffer::from_slice(data.to_vec()));
    }

    fn poll(&self) {
        let mut guard = self.running.lock().unwrap();
        let Some(r) = guard.as_mut() else {
            return;
        };

        let mut error = None;
        if let Some(bus) = r.pipeline.bus() {
            while let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error]) {
                if let gst::MessageView::Error(e) = msg.view() {
                    error = Some(e.error().to_string());
                }
            }
        }

        let stats = r.sink.property::<Option<gst::Structure>>("stats");
        let clients = match r.mode {
            SrtMode::Listener => stats
                .as_ref()
                .and_then(|s| s.get::<gst::glib::ValueArray>("callers").ok())
                .map(|c| c.len() as u32)
                .unwrap_or(0),
            // Data keeps leaving for a while after the receiver is gone, so
            // go by its acknowledgements instead
            SrtMode::Caller => {
                let acks = stats
                    .as_ref()
                    .and_then(|s| s.get::<i32>("packet-ack-received").ok())
                    .unwrap_or(0);
                if acks != r.acks {
                    r.acks = acks;
                    r.last_ack = Some(Instant::now());
                }
                r.last_ack.is_some_and(|t| t.elapsed() < ACK_TIMEOUT) as u32
            }
        };

        let mut status = self.status.lock().unwrap();
        if status.clients != clients {
            eprintln!("[srt] {clients} receiver(s)");
        }
        status.clients = clients;
        if let Some(e) = error {
            eprintln!("[srt] {e}");
            status.error = Some(e);
        }
    }
}

fn start(cfg: &SrtSettings) -> Result<Running, String> {
    let pipeline = gst::Pipeline::new();
    let appsrc = gst::ElementFactory::make("appsrc")
        .property("is-live", true)
        .property("format", gst::Format::Time)
        .property("do-timestamp", true)
        .property(
            "caps",
            gst::Caps::builder("video/x-h264")
                .field("stream-format", "byte-stream")
                .field("alignment", "stream")
                .build(),
        )
        .build()
        .map_err(|e| format!("appsrc: {e}"))?;
    // Parameter sets before every keyframe, so receivers can join any time
    let parse = gst::ElementFactory::make("h264parse")
        .property("config-interval", -1i32)
        .build()
        .map_err(|e| format!("h264parse: {e}"))?;
    let mux = gst::ElementFactory::make("mpegtsmux")
        .build()
        .map_err(|e| format!("mpegtsmux: {e}"))?;

    let uri = match cfg.mode {
        SrtMode::Listener => format!("srt://:{}", cfg.port),
        SrtMode::Caller => format!("srt://{}:{}", cfg.host.trim(), cfg.port),
    };
    let sink = gst::ElementFactory::make("srtsink")
        .property("uri", &uri)
        .property("latency", cfg.latency_ms as i32)
        .property("wait-for-connection", false)
        .property("sync", false)
        .build()
        .map_err(|e| format!("srtsink: {e} (is gstreamer1.0-plugins-bad installed?)"))?;
    sink.set_property_from_str(
        "mode",
        match cfg.mode {
            SrtMode::Listener => "listener",
            SrtMode::Caller => "caller",
        },
    );
    if !cfg.passphrase.is_empty() {
        sink.set_property("passphrase", &cfg.passphrase);
    }
    if cfg.mode == SrtMode::Caller && !cfg.stream_id.is_empty() {
        sink.set_property("streamid", &cfg.stream_id);
    }

    pipeline
        .add_many([&appsrc, &parse, &mux, &sink])
        .map_err(|e| format!("pipeline: {e}"))?;
    gst::Element::link_many([&appsrc, &parse, &mux, &sink]).map_err(|e| format!("link: {e}"))?;

    if pipeline.set_state(gst::State::Playing).is_err() {
        let reason = pipeline
            .bus()
            .and_then(|b| b.pop_filtered(&[gst::MessageType::Error]))
            .and_then(|m| match m.view() {
                gst::MessageView::Error(e) => Some(e.error().to_string()),
                _ => None,
            })
            .unwrap_or_else(|| "couldn't start".into());
        let _ = pipeline.set_state(gst::State::Null);
        return Err(match cfg.mode {
            SrtMode::Listener => format!("Port {}: {reason}", cfg.port),
            SrtMode::Caller => reason,
        });
    }

    let appsrc = appsrc
        .dynamic_cast::<AppSrc>()
        .map_err(|_| "appsrc cast".to_string())?;
    Ok(Running {
        pipeline,
        appsrc,
        sink,
        mode: cfg.mode,
        acks: 0,
        last_ack: None,
    })
}
