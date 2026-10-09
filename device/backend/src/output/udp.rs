use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use serde::Serialize;

use crate::settings::{UdpFormat, UdpSettings};

const POLL: Duration = Duration::from_secs(1);

#[derive(Serialize, Clone, Default, Debug)]
pub struct UdpStatus {
    pub active: bool,
    pub destinations: usize,
    pub error: Option<String>,
}

struct Running {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
}

pub struct UdpOutput {
    running: Mutex<Option<Running>>,
    status: Mutex<UdpStatus>,
}

impl UdpOutput {
    pub fn new(cfg: &UdpSettings) -> Arc<Self> {
        let this = Arc::new(Self {
            running: Mutex::new(None),
            status: Mutex::new(UdpStatus::default()),
        });
        if let Err(e) = this.apply(cfg) {
            eprintln!("[udp] not started: {e}");
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

    pub fn status(&self) -> UdpStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn apply(&self, cfg: &UdpSettings) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        if let Some(old) = running.take() {
            let _ = old.pipeline.set_state(gst::State::Null);
            eprintln!("[udp] stopped");
        }
        *self.status.lock().unwrap() = UdpStatus::default();
        if !cfg.enabled {
            return Ok(());
        }

        let destinations = cfg.destination_list();
        match start(cfg.format, &destinations) {
            Ok(r) => {
                eprintln!(
                    "[udp] sending {:?} to {}",
                    cfg.format,
                    destinations.join(", ")
                );
                *running = Some(r);
                *self.status.lock().unwrap() = UdpStatus {
                    active: true,
                    destinations: destinations.len(),
                    error: None,
                };
                Ok(())
            }
            Err(e) => {
                self.status.lock().unwrap().error = Some(e.clone());
                Err(e)
            }
        }
    }

    pub fn push(&self, data: &[u8]) {
        let guard = self.running.lock().unwrap();
        if let Some(r) = guard.as_ref() {
            let _ = r.appsrc.push_buffer(gst::Buffer::from_slice(data.to_vec()));
        }
    }

    /// Errors only: a receiver that isn't listening is invisible to UDP
    fn poll(&self) {
        let guard = self.running.lock().unwrap();
        let Some(r) = guard.as_ref() else {
            return;
        };
        let Some(bus) = r.pipeline.bus() else {
            return;
        };
        while let Some(msg) =
            bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Warning])
        {
            let text = match msg.view() {
                gst::MessageView::Error(e) => e.error().to_string(),
                gst::MessageView::Warning(w) => w.error().to_string(),
                _ => continue,
            };
            let mut status = self.status.lock().unwrap();
            if status.error.as_deref() != Some(&text) {
                eprintln!("[udp] {text}");
                status.error = Some(text);
            }
        }
    }
}

/// Hostnames to addresses, IPv4 first: "localhost" is ::1 before 127.0.0.1,
/// and receivers mostly listen on IPv4
fn resolve(destinations: &[String]) -> Result<Vec<String>, String> {
    destinations
        .iter()
        .map(|d| {
            let addrs: Vec<SocketAddr> = d
                .to_socket_addrs()
                .map_err(|e| format!("Can't find {d}: {e}"))?
                .collect();
            addrs
                .iter()
                .find(|a| a.is_ipv4())
                .or_else(|| addrs.first())
                .map(|a| format!("{}:{}", a.ip(), a.port()))
                .ok_or_else(|| format!("Can't find {d}"))
        })
        .collect()
}

fn start(format: UdpFormat, destinations: &[String]) -> Result<Running, String> {
    // Parameter sets before every keyframe, so a receiver can join any time
    let packetize = match format {
        UdpFormat::Rtp => "rtph264pay pt=96 config-interval=-1 mtu=1400",
        // 7 x 188 bytes: the usual payload, fits any network's MTU
        UdpFormat::Mpegts => "mpegtsmux alignment=7",
    };
    let description = format!(
        "appsrc name=src ! h264parse config-interval=-1 ! {packetize} ! \
         multiudpsink name=sink sync=false"
    );
    let pipeline = gst::parse::launch(&description)
        .map_err(|e| format!("UDP pipeline: {e}"))?
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

    let sink = pipeline
        .by_name("sink")
        .expect("Failed to find multiudpsink");
    sink.set_property("clients", resolve(destinations)?.join(","));

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
    Ok(Running { pipeline, appsrc })
}
