pub mod duml;
pub mod gadget;
pub mod goggles;
pub mod h264;
pub mod legacy;
pub mod uvc;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::output::hdmi::GstPlayer;
use crate::output::recording::Recorder;
use crate::output::rtmp::RtmpOutput;
use crate::output::rtsp::RtspServer;
use crate::output::splash::Splash;
use crate::output::srt::SrtOutput;
use crate::output::udp::UdpOutput;
use crate::settings::InputMode;

/// Shared between the sources, the fan-out and the splash screen
#[derive(Default)]
pub struct InputState {
    mode: AtomicU8,
    pub uvc: Mutex<uvc::UvcStatus>,
    pub legacy: Mutex<legacy::LegacyStatus>,
}

impl InputState {
    pub fn new(mode: InputMode) -> Arc<Self> {
        let state = Arc::new(Self::default());
        state.set_mode(mode);
        state
    }

    pub fn mode(&self) -> InputMode {
        match self.mode.load(Ordering::Relaxed) {
            1 => InputMode::Uvc,
            2 => InputMode::DjiFpvLegacy,
            _ => InputMode::DjiFpv,
        }
    }

    pub fn set_mode(&self, mode: InputMode) {
        let v = match mode {
            InputMode::DjiFpv => 0,
            InputMode::Uvc => 1,
            InputMode::DjiFpvLegacy => 2,
        };
        self.mode.store(v, Ordering::Relaxed);
    }
}

/// Hands the selected source's H.264 to every output, through the splash so
/// it can take over when the video stops
pub struct Fanout {
    pub state: Arc<InputState>,
    pub splash: Arc<Splash>,
    pub player: Arc<GstPlayer>,
    pub rtsp: Arc<RtspServer>,
    pub srt: Arc<SrtOutput>,
    pub rtmp: Arc<RtmpOutput>,
    pub udp: Arc<UdpOutput>,
    pub recorder: Arc<Recorder>,
    pub web_tx: mpsc::UnboundedSender<Vec<u8>>,
    pub metrics: Arc<crate::system::metrics::Metrics>,
}

impl Fanout {
    /// Returns whether the chunk went out, i.e. `source` is the one selected
    pub fn video(&self, source: InputMode, chunk: &[u8]) -> bool {
        if source != self.state.mode() {
            return false;
        }
        self.splash.video(chunk, || {
            self.metrics.video(chunk);
            let _ = self.player.push(chunk);
            self.rtsp.push(chunk);
            self.srt.push(chunk);
            self.rtmp.push(chunk);
            self.udp.push(chunk);
            self.recorder.push(chunk);
            let _ = self.web_tx.send(chunk.to_vec());
        });
        true
    }

    /// Splash frames go everywhere live video goes, except the recording
    pub fn start_splash(self: &Arc<Self>) {
        let this = Arc::clone(self);
        self.splash.start(move |au| {
            let _ = this.player.push_au(au.to_vec());
            this.rtsp.push(au);
            this.srt.push(au);
            this.rtmp.push(au);
            this.udp.push(au);
            let _ = this.web_tx.send(au.to_vec());
        });
    }
}
