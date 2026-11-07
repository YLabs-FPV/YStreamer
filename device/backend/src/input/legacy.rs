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
/// Asking again on every empty read would write to the goggles ten times a
/// second for as long as the aircraft is off; this backs off to `ASK_MAX`
const ASK_MIN: Duration = Duration::from_millis(500);
const ASK_MAX: Duration = Duration::from_secs(2);
/// Goggles found and asked, but no video for this long: tell the user the
/// air unit settings both reference players point to
const SILENT_HINT: Duration = Duration::from_secs(3);
const SILENT_HINT_TEXT: &str =
    "No video yet: on the air unit, turn Auto Temp Control off and set 50 Mbps";

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
    /// What to check when the goggles were found but send no video
    pub hint: Option<String>,
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
        if state != LegacyState::Waiting {
            status.hint = None;
        }
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
        let mut asking = Asking::new(Instant::now());
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
                    asking.got_video();
                    self.set(LegacyState::Live, None);
                }
                // Nothing within the timeout. The goggles stop sending when
                // the air unit's link drops, e.g. for a battery swap, and
                // only start again when asked
                Ok(()) | Err(TransferError::Cancelled) => {
                    if last_video.is_some_and(|at| at.elapsed() > VIDEO_GONE) {
                        last_video = None;
                        self.set(LegacyState::Waiting, None);
                    }
                    let now = Instant::now();
                    if asking.should_ask(now) {
                        match ask() {
                            Ok(()) | Err(TransferError::Cancelled) => {}
                            Err(e) => return Err(format!("Lost the goggles: {e}")),
                        }
                    }
                    if asking.should_hint(now) {
                        eprintln!("[fpv] {SILENT_HINT_TEXT}");
                        self.state.legacy.lock().unwrap().hint = Some(SILENT_HINT_TEXT.into());
                    }
                }
                Err(e) => return Err(format!("Lost the goggles: {e}")),
            }
        }
        Ok(())
    }
}

/// When to ask the goggles for video again while none arrives: soon at
/// first, so a battery swap recovers quickly, then less often
struct Asking {
    silent_since: Instant,
    last: Option<Instant>,
    gap: Duration,
    hinted: bool,
}

impl Asking {
    fn new(now: Instant) -> Self {
        Self {
            silent_since: now,
            last: None,
            gap: ASK_MIN,
            hinted: false,
        }
    }

    fn should_ask(&mut self, now: Instant) -> bool {
        match self.last {
            Some(at) if now.duration_since(at) < self.gap => false,
            Some(_) => {
                self.last = Some(now);
                self.gap = (self.gap * 2).min(ASK_MAX);
                true
            }
            None => {
                self.last = Some(now);
                true
            }
        }
    }

    fn should_hint(&mut self, now: Instant) -> bool {
        if self.hinted || now.duration_since(self.silent_since) < SILENT_HINT {
            return false;
        }
        self.hinted = true;
        true
    }

    fn got_video(&mut self) {
        *self = Self::new(Instant::now());
    }
}

#[cfg(test)]
#[path = "legacy.test.rs"]
mod tests;
