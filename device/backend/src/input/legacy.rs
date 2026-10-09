use std::fs::File;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nusb::MaybeFuture;
use nusb::descriptors::TransferType;
use nusb::transfer::{Bulk, Direction, In, Out, TransferError};
use serde::Serialize;

use super::{Fanout, InputState};
use crate::settings::InputMode;

const VENDOR: u16 = 0x2CA3;
const PRODUCT: u16 = 0x001F;
const INTERFACE: u8 = 3;
const START_VIDEO: &[u8] = b"RMVT";

const READ_LEN: usize = 128 * 1024;
const READ_TIMEOUT: Duration = Duration::from_millis(100);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const VIDEO_GONE: Duration = Duration::from_secs(2);
const RETRY: Duration = Duration::from_secs(1);

pub const CAPTURE_PATH: &str = "/tmp/ystreamer-fpv-goggles.h264";
const CAPTURE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LegacyState {
    /// Another source is selected
    #[default]
    Idle,
    NoDevice,
    /// Goggles found; no video, e.g. the aircraft is off
    Waiting,
    Live,
    Error,
}

#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct LegacyStatus {
    pub state: LegacyState,
    pub error: Option<String>,
    /// Bytes of the raw stream saved so far, while or after capturing
    pub captured: Option<usize>,
}

/// The stream as the goggles send it, for working out what's wrong with it
/// from a file someone else recorded
struct Capture {
    file: File,
    left: usize,
}

pub struct LegacySource {
    state: Arc<InputState>,
    capture: Mutex<Option<Capture>>,
}

impl LegacySource {
    pub fn start(state: Arc<InputState>, fanout: Arc<Fanout>) -> Arc<Self> {
        let source = Arc::new(Self {
            state,
            capture: Mutex::new(None),
        });
        let this = Arc::clone(&source);
        thread::spawn(move || {
            loop {
                if this.selected() {
                    match this.session(&fanout) {
                        Ok(()) => {}
                        Err(e) => {
                            eprintln!("[fpv] {e}");
                            this.set(LegacyState::Error, Some(e));
                        }
                    }
                } else {
                    this.set(LegacyState::Idle, None);
                }
                thread::sleep(RETRY);
            }
        });
        source
    }

    pub fn status(&self) -> LegacyStatus {
        self.state.legacy.lock().unwrap().clone()
    }

    /// Saves the next few megabytes of the stream, replacing an earlier file
    pub fn start_capture(&self) -> Result<(), String> {
        let file = File::create(CAPTURE_PATH).map_err(|e| format!("{CAPTURE_PATH}: {e}"))?;
        *self.capture.lock().unwrap() = Some(Capture {
            file,
            left: CAPTURE_BYTES,
        });
        self.state.legacy.lock().unwrap().captured = Some(0);
        Ok(())
    }

    fn selected(&self) -> bool {
        self.state.mode() == InputMode::DjiFpvLegacy
    }

    fn set(&self, state: LegacyState, error: Option<String>) {
        let mut status = self.state.legacy.lock().unwrap();
        status.state = state;
        status.error = error;
    }

    fn save(&self, data: &[u8]) {
        let mut capture = self.capture.lock().unwrap();
        let Some(c) = capture.as_mut() else {
            return;
        };
        let part = &data[..data.len().min(c.left)];
        let ok = c.file.write_all(part).is_ok();
        c.left -= part.len();
        let saved = CAPTURE_BYTES - c.left;
        self.state.legacy.lock().unwrap().captured = Some(saved);
        if !ok || c.left == 0 {
            *capture = None;
        }
    }

    /// Runs until the goggles go away or another source is selected
    fn session(&self, fanout: &Fanout) -> Result<(), String> {
        let found = nusb::list_devices()
            .wait()
            .map_err(|e| format!("Couldn't list USB devices: {e}"))?
            .find(|d| d.vendor_id() == VENDOR && d.product_id() == PRODUCT);
        let Some(info) = found else {
            self.set(LegacyState::NoDevice, None);
            return Ok(());
        };

        let device = info
            .open()
            .wait()
            .map_err(|e| format!("Couldn't open the goggles: {e}"))?;
        let interface = device
            .detach_and_claim_interface(INTERFACE)
            .wait()
            .map_err(|e| format!("Couldn't claim the goggles' video interface: {e}"))?;

        let endpoint = |direction| {
            interface
                .descriptor()
                .into_iter()
                .flat_map(|d| d.endpoints())
                .find(|e| e.transfer_type() == TransferType::Bulk && e.direction() == direction)
                .map(|e| e.address())
        };
        let (Some(out), Some(inp)) = (endpoint(Direction::Out), endpoint(Direction::In)) else {
            return Err("The goggles' video interface has no bulk endpoints".into());
        };
        let mut out = interface
            .endpoint::<Bulk, Out>(out)
            .map_err(|e| format!("Bulk out: {e}"))?;
        let mut inp = interface
            .endpoint::<Bulk, In>(inp)
            .map_err(|e| format!("Bulk in: {e}"))?;

        let mut ask = || {
            out.transfer_blocking(START_VIDEO.to_vec().into(), WRITE_TIMEOUT)
                .status
        };
        ask().map_err(|e| format!("Couldn't ask the goggles for video: {e}"))?;
        println!("[fpv] goggles found, asked for video");
        self.set(LegacyState::Waiting, None);

        let mut last_video: Option<Instant> = None;
        while self.selected() {
            let done = inp.transfer_blocking(inp.allocate(READ_LEN), READ_TIMEOUT);
            match done.status {
                Ok(()) if done.actual_len > 0 => {
                    let data = &done.buffer[..done.actual_len];
                    self.save(data);
                    fanout.video(InputMode::DjiFpvLegacy, data);
                    if last_video.is_none() {
                        println!("[fpv] video started");
                    }
                    last_video = Some(Instant::now());
                    self.set(LegacyState::Live, None);
                }
                // Nothing within the timeout. The goggles stop sending now
                // and then, and want to be asked again
                Ok(()) | Err(TransferError::Cancelled) => {
                    if last_video.is_some_and(|at| at.elapsed() > VIDEO_GONE) {
                        last_video = None;
                        self.set(LegacyState::Waiting, None);
                    }
                    match ask() {
                        Ok(()) | Err(TransferError::Cancelled) => {}
                        Err(e) => return Err(format!("Lost the goggles: {e}")),
                    }
                }
                Err(e) => return Err(format!("Lost the goggles: {e}")),
            }
        }
        Ok(())
    }
}
