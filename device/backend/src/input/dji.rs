use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::Fanout;
use super::codec::{Codec, Sniffer};
use super::duml::{encode, next_seq};
use super::h265::{self, ParamSets};
use super::hevc::{Converter, Transcoder, WAITING_FOR_KEYFRAME};
use crate::settings::InputMode;

/// Held while the codec is still unknown
const MAX_PENDING: usize = 8 * 1024 * 1024;

/// The drone behind an RC-N remote sends VPS/SPS/PPS and a keyframe only
/// when it links to the remote, then refreshes a strip of every picture
/// instead (intra refresh). A decoder that joins later asks, as DJI Fly
/// does: `0x01/0x01` to the remote, no reply wanted. `24` makes the drone
/// send a keyframe, with its parameter sets, within ~25-75 ms, and keep
/// sending one every 8 pictures; `04` puts it back to normal. DJI Fly sends
/// the second at ~20 Hz for a second after the keyframe (captured with an
/// RC-N3 + Air 3; tw-streamer's RC-N + Neo 2 capture has the same bytes)
const KEYFRAMES_ON: [u8; 11] = [0, 0, 0, 0, 0, 0x24, 0, 0, 0, 0x24, 0];
const KEYFRAMES_OFF: [u8; 11] = [0, 0, 0, 0, 0, 0x04, 0, 0, 0, 0x04, 0];
const REMOTE: u8 = 0x0e;
/// Between requests while no keyframe comes
const ASK_AGAIN: Duration = Duration::from_secs(1);
/// How long, and how often, `KEYFRAMES_OFF` follows a keyframe
const RELEASE_FOR: Duration = Duration::from_secs(1);
const RELEASE_EVERY: Duration = Duration::from_millis(50);

fn keyframes_frame(on: bool) -> Vec<u8> {
    let payload = if on { &KEYFRAMES_ON } else { &KEYFRAMES_OFF };
    encode(REMOTE, next_seq(), 0x00, 0x01, 0x01, payload)
}

/// Sends a frame to the remote
pub type SendToRemote = Box<dyn Fn(&[u8]) + Send>;

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
    keyframes: Keyframes,
    send: SendToRemote,
}

impl DjiVideo {
    pub fn new(fanout: Arc<Fanout>, send: SendToRemote) -> Self {
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
            keyframes: Keyframes::default(),
            send,
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
                // Kept even while another input is selected
                self.params.feed(data);
                let keyframe = h265::irap_start(data).is_some();
                if self.fanout.state.mode() != InputMode::DjiFpv {
                    if self.transcoder.converter().is_some() {
                        self.transcoder.stop();
                        self.publish(None);
                    }
                    self.keyframes(keyframe, false);
                    return false;
                }
                if self.params.annex_b().is_none() {
                    // Nothing can decode it yet
                    self.keyframes(keyframe, true);
                    self.publish(Some(WAITING_FOR_KEYFRAME.into()));
                    return false;
                }
                let result = self.transcoder.push(data, &self.params);
                let waiting = result.as_ref().is_err_and(|e| e == WAITING_FOR_KEYFRAME);
                self.keyframes(keyframe, waiting);
                self.publish(result.as_ref().err().cloned());
                result.is_ok()
            }
            None => false,
        }
    }

    fn keyframes(&mut self, arrived: bool, needed: bool) {
        match self.keyframes.next(Instant::now(), arrived, needed) {
            Some(Ask::On { first }) => {
                if first {
                    println!("[video] asking the drone for a keyframe");
                }
                (self.send)(&keyframes_frame(true));
            }
            Some(Ask::Off) => (self.send)(&keyframes_frame(false)),
            None => {}
        }
    }

    fn reset(&mut self, generation: u64) {
        self.generation = generation;
        self.sniffer = Sniffer::new();
        self.codec = None;
        self.pending.clear();
        self.params.clear();
        self.transcoder.stop();
        self.keyframes = Keyframes::default();
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
enum Ask {
    /// `KEYFRAMES_ON`; `first` of this wait, for the log
    On {
        first: bool,
    },
    Off,
}

/// When to ask the drone for a keyframe, and to put it back afterwards
#[derive(Default)]
struct Keyframes {
    /// The last request, while no keyframe has come
    asked: Option<Instant>,
    /// Until when `KEYFRAMES_OFF` is sent, and when it last was
    releasing: Option<(Instant, Option<Instant>)>,
}

impl Keyframes {
    /// `arrived`: the stream just brought a keyframe; `needed`: nothing
    /// can decode it without one
    fn next(&mut self, now: Instant, arrived: bool, needed: bool) -> Option<Ask> {
        if arrived && self.asked.take().is_some() {
            println!("[video] got a keyframe from the drone");
            self.releasing = Some((now + RELEASE_FOR, None));
        }
        if let Some((until, last)) = &mut self.releasing {
            if now >= *until {
                self.releasing = None;
            } else {
                if last.is_none_or(|at| now.duration_since(at) >= RELEASE_EVERY) {
                    *last = Some(now);
                    return Some(Ask::Off);
                }
                return None;
            }
        }
        if !needed {
            return None;
        }
        match self.asked {
            Some(at) if now.duration_since(at) < ASK_AGAIN => None,
            asked => {
                self.asked = Some(now);
                Some(Ask::On {
                    first: asked.is_none(),
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "dji.test.rs"]
mod tests;
