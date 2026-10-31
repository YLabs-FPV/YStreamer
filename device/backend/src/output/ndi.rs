pub mod install;
mod runtime;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::decoded::{Decoder, Limit, Picture, SharedDecoder};
use crate::settings::NetworkSourceSettings;

const POLL: Duration = Duration::from_secs(1);
/// Keep decoding through a receiver reconnecting, e.g. vMix switching inputs
const LINGER: Duration = Duration::from_secs(5);

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct NdiStatus {
    /// The NDI runtime is installed and opens
    pub installed: bool,
    pub active: bool,
    pub receivers: u32,
    /// Of the pictures going out, once there are some
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub error: Option<String>,
}

struct Running {
    sender: Arc<runtime::Sender>,
    limit: Limit,
    /// Width and height packed into one, 0 before the first picture
    size: Arc<AtomicU64>,
    idle_since: Option<Instant>,
}

/// Full-bandwidth NDI: the video is decoded and NDI's runtime compresses
/// each picture again. Sending the H.264 as it is (NDI|HX) needs NDI's
/// separately licensed Advanced SDK, so it isn't offered. Decoding only
/// runs while a receiver is connected
pub struct NdiOutput {
    running: Mutex<Option<Running>>,
    shared: Arc<SharedDecoder>,
    decoder: Mutex<Option<Decoder>>,
    status: Mutex<NdiStatus>,
}

impl NdiOutput {
    pub fn new(cfg: &NetworkSourceSettings, shared: Arc<SharedDecoder>) -> Arc<Self> {
        let this = Arc::new(Self {
            running: Mutex::new(None),
            shared,
            decoder: Mutex::new(None),
            status: Mutex::new(NdiStatus::default()),
        });
        if let Err(e) = this.apply(cfg) {
            eprintln!("[ndi] not started: {e}");
        }
        let watcher = Arc::clone(&this);
        thread::spawn(move || {
            loop {
                thread::sleep(POLL);
                watcher.supervise();
            }
        });
        this
    }

    pub fn status(&self) -> NdiStatus {
        self.status.lock().unwrap().clone()
    }

    /// Looks for the runtime if it isn't open yet, which reads the disk
    pub fn installed(&self) -> bool {
        runtime::load().is_ok()
    }

    pub fn apply(&self, cfg: &NetworkSourceSettings) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        // Taken out first: stopping it waits for its thread, and the video
        // shouldn't wait behind the lock meanwhile
        let old = self.decoder.lock().unwrap().take();
        drop(old);
        if running.take().is_some() {
            eprintln!("[ndi] stopped");
        }
        *self.status.lock().unwrap() = NdiStatus::default();
        if !cfg.enabled {
            return Ok(());
        }

        let result = runtime::load().and_then(|rt| runtime::Sender::new(rt, cfg.name.trim()));
        let mut status = self.status.lock().unwrap();
        match result {
            Ok(sender) => {
                eprintln!("[ndi] source \"{}\" on the network", cfg.name.trim());
                *running = Some(Running {
                    sender: Arc::new(sender),
                    limit: Limit {
                        max_height: cfg.max_height,
                        max_fps: cfg.max_fps,
                    },
                    size: Arc::new(AtomicU64::new(0)),
                    idle_since: None,
                });
                status.active = true;
                status.installed = true;
                Ok(())
            }
            Err(e) => {
                status.error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// Decodes while someone watches, and keeps the status current
    fn supervise(&self) {
        // Dropped last, once the video can get at the decoder slot again
        let mut retired = None;
        let mut running = self.running.lock().unwrap();
        let Some(r) = running.as_mut() else {
            return;
        };
        let receivers = r.sender.connections();
        let mut decoder = self.decoder.lock().unwrap();
        let mut error = decoder.as_ref().and_then(Decoder::error);
        if error.is_some() {
            // Started again on the next round
            retired = decoder.take();
        }

        if receivers > 0 {
            r.idle_since = None;
            if decoder.is_none() && error.is_none() {
                match Decoder::start(&self.shared, r.limit, sending(r)) {
                    Ok(d) => {
                        eprintln!("[ndi] {receivers} receiver(s), decoding");
                        *decoder = Some(d);
                    }
                    Err(e) => error = Some(e),
                }
            }
        } else if decoder.is_some() {
            let since = *r.idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= LINGER {
                retired = decoder.take();
                r.size.store(0, Ordering::Relaxed);
                eprintln!("[ndi] no receivers, decoding stopped");
            }
        }

        let size = r.size.load(Ordering::Relaxed);
        let mut status = self.status.lock().unwrap();
        status.receivers = receivers;
        status.width = (size != 0).then_some((size >> 32) as u32);
        status.height = (size != 0).then_some(size as u32);
        if let Some(e) = error {
            if status.error.as_deref() != Some(&e) {
                eprintln!("[ndi] {e}");
            }
            status.error = Some(e);
        } else if decoder.is_some() {
            status.error = None;
        }
        drop(status);
        drop(decoder);
        drop(running);
        drop(retired);
    }
}

/// What the decoder does with each picture: hand it to NDI
fn sending(r: &Running) -> impl FnMut(&Picture) + Send + 'static {
    let sender = Arc::clone(&r.sender);
    let size = Arc::clone(&r.size);
    let max_fps = r.limit.max_fps;
    let mut packed = Vec::new();
    move |picture| {
        let stride = picture.pack(&mut packed);
        let (n, d) = picture.rate(max_fps);
        let frame = runtime::VideoFrame {
            xres: picture.width as i32,
            yres: picture.height as i32,
            four_cc: runtime::FOURCC_NV12,
            frame_rate_n: n,
            frame_rate_d: d,
            // Square pixels: NDI works the shape out from the size
            picture_aspect_ratio: 0.0,
            frame_format_type: runtime::FRAME_PROGRESSIVE,
            // The time it was decoded, so receivers show it as it arrives
            timecode: now_100ns(),
            p_data: packed.as_ptr(),
            line_stride_in_bytes: stride as i32,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        };
        sender.send(&frame);
        size.store(
            (picture.width as u64) << 32 | picture.height as u64,
            Ordering::Relaxed,
        );
    }
}

fn now_100ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| (d.as_nanos() / 100) as i64)
}

#[cfg(test)]
#[path = "ndi.test.rs"]
mod tests;
