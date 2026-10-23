use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::Fanout;
use super::codec::{Codec, Sniffer};
use super::h265::ParamSets;
use super::hevc::{Converter, Transcoder};
use crate::settings::InputMode;

/// Held while the codec is still unknown. An RC-N remote's parameter sets
/// come in its first few chunks, and are never repeated
const MAX_PENDING: usize = 8 * 1024 * 1024;
/// H.265 this long without parameter sets means the remote sent them before
/// we were listening, e.g. YStreamer restarted with the cable in
const MISSED_START: Duration = Duration::from_secs(3);
const REPLUG_PROMPT: &str = "The remote started sending before YStreamer was listening: unplug its cable and plug it back in";

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DjiStatus {
    /// What the link carries, once worked out
    pub codec: Option<Codec>,
    /// How H.265 is turned into H.264, while it is
    pub converter: Option<Converter>,
    pub error: Option<String>,
}

/// The video from the USB-C link, whatever its codec: goggles send H.264,
/// which goes straight out; an RC-N remote sends H.265, which is converted
pub struct DjiVideo {
    fanout: Arc<Fanout>,
    sniffer: Sniffer,
    codec: Option<Codec>,
    pending: Vec<u8>,
    params: ParamSets,
    transcoder: Transcoder,
    generation: u64,
    missed: MissedStart,
    /// Drops the link and brings it back, as unplugging would
    replug: Box<dyn Fn() + Send>,
}

impl DjiVideo {
    pub fn new(fanout: Arc<Fanout>, replug: Box<dyn Fn() + Send>) -> Self {
        Self {
            transcoder: {
                let fanout = Arc::clone(&fanout);
                Transcoder::new(Arc::new(move |au: &[u8]| {
                    fanout.video(InputMode::DjiFpv, au);
                }))
            },
            fanout,
            sniffer: Sniffer::new(),
            codec: None,
            pending: Vec::new(),
            params: ParamSets::new(),
            generation: 0,
            missed: MissedStart::default(),
            replug,
        }
    }

    /// `generation` is the link's: a new one may be a different device.
    /// Returns whether the chunk went out
    pub fn push(&mut self, generation: u64, chunk: &[u8]) -> bool {
        if generation != self.generation {
            self.reset(generation);
        }
        if self.codec.is_some() {
            return self.route(chunk);
        }
        if self.pending.len() + chunk.len() <= MAX_PENDING {
            self.pending.extend_from_slice(chunk);
        }
        let Some(codec) = self.sniffer.feed(chunk) else {
            return false;
        };
        println!("[video] the link carries {codec:?}");
        self.codec = Some(codec);
        self.publish(None);
        let pending = std::mem::take(&mut self.pending);
        self.route(&pending)
    }

    fn route(&mut self, data: &[u8]) -> bool {
        match self.codec {
            Some(Codec::H264) => self.fanout.video(InputMode::DjiFpv, data),
            Some(Codec::H265) => {
                // Kept even while another input is selected: they're sent once
                self.params.feed(data);
                if self.fanout.state.mode() != InputMode::DjiFpv {
                    if self.transcoder.converter().is_some() {
                        self.transcoder.stop();
                        self.publish(None);
                    }
                    return false;
                }
                let have_params = self.params.annex_b().is_some();
                match self.missed.check(Instant::now(), have_params) {
                    Recovery::Wait => {}
                    Recovery::Replug => {
                        println!(
                            "[video] no stream settings from the remote, reconnecting the link"
                        );
                        (self.replug)();
                        return false;
                    }
                    Recovery::AskUser => {
                        self.publish(Some(REPLUG_PROMPT.into()));
                        return false;
                    }
                }
                if !have_params {
                    // Nothing can decode it yet
                    return false;
                }
                let result = self.transcoder.push(data, &self.params);
                self.publish(result.as_ref().err().cloned());
                result.is_ok()
            }
            None => false,
        }
    }

    fn reset(&mut self, generation: u64) {
        self.generation = generation;
        self.sniffer = Sniffer::new();
        self.codec = None;
        self.pending.clear();
        self.params.clear();
        self.transcoder.stop();
        self.missed.new_link();
        self.publish(None);
    }

    fn publish(&self, error: Option<String>) {
        let status = DjiStatus {
            codec: self.codec,
            converter: self.transcoder.converter(),
            error,
        };
        let mut current = self.fanout.state.dji.lock().unwrap();
        if *current != status {
            *current = status;
        }
    }
}

#[derive(Debug, PartialEq)]
enum Recovery {
    Wait,
    Replug,
    AskUser,
}

/// An RC-N remote sends its parameter sets once, when the cable goes in.
/// Joining later, nothing can decode the stream until it starts over, which
/// a replug makes it do. One is tried on our side; if that doesn't help,
/// the user is asked to do it by hand
#[derive(Default)]
struct MissedStart {
    since: Option<Instant>,
    replugs: u32,
}

impl MissedStart {
    fn check(&mut self, now: Instant, have_params: bool) -> Recovery {
        if have_params {
            *self = Self::default();
            return Recovery::Wait;
        }
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) < MISSED_START {
            return Recovery::Wait;
        }
        if self.replugs == 0 {
            self.replugs += 1;
            self.since = None;
            return Recovery::Replug;
        }
        Recovery::AskUser
    }

    /// The count of replugs carries over: the one we made starts a new link
    fn new_link(&mut self) {
        self.since = None;
    }
}

#[cfg(test)]
#[path = "dji.test.rs"]
mod tests;
