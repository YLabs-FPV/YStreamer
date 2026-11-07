use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::Fanout;
use super::codec::{Codec, Sniffer};
use super::h265::{self, ParamSets};
use super::hevc::{Converter, Transcoder};
use super::lanes::{self, Lane, LaneStatus, Program, Route};
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
    /// Set once a second camera is seen on the link
    pub cameras: Option<Cameras>,
}

/// A two-camera aircraft's streams
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Cameras {
    /// The one the outputs get
    pub program: Lane,
    pub main: LaneStatus,
    pub second: LaneStatus,
}

/// One camera's stream, as it comes in
struct Stream {
    sniffer: Sniffer,
    codec: Option<Codec>,
    pending: Vec<u8>,
    params: ParamSets,
    /// The SPS the picture size was last read from
    sps: Vec<u8>,
}

impl Stream {
    fn new() -> Self {
        Self {
            sniffer: Sniffer::new(),
            codec: None,
            pending: Vec::new(),
            params: ParamSets::new(),
            sps: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.sniffer = Sniffer::new();
        self.codec = None;
        self.pending.clear();
        self.params.clear();
        self.sps.clear();
    }

    /// What to go on with once the codec is known: the chunk, or all that
    /// was held until then
    fn take<'a>(&mut self, chunk: &'a [u8]) -> Option<Cow<'a, [u8]>> {
        if self.codec.is_some() {
            return Some(Cow::Borrowed(chunk));
        }
        if self.pending.len() + chunk.len() <= MAX_PENDING {
            self.pending.extend_from_slice(chunk);
        }
        self.codec = Some(self.sniffer.feed(chunk)?);
        Some(Cow::Owned(std::mem::take(&mut self.pending)))
    }

    /// Keeps the parameter sets, even while another camera or input is on
    /// the outputs: they're sent once. Returns the picture size when the
    /// SPS changed
    fn keep_params(&mut self, data: &[u8]) -> Option<Option<(u32, u32)>> {
        if self.codec != Some(Codec::H265) {
            return None;
        }
        self.params.feed(data);
        let sps = self.params.sps()?;
        if sps == self.sps.as_slice() {
            return None;
        }
        self.sps = sps.to_vec();
        Some(h265::picture_size(sps))
    }
}

/// The video from the USB-C link, whatever its codec: goggles send H.264,
/// which goes straight out; an RC-N remote sends H.265, which is converted.
/// A two-camera aircraft sends two streams, and one of them goes out
pub struct DjiVideo {
    fanout: Arc<Fanout>,
    streams: [Stream; 2],
    program: Program,
    /// Shared by both streams: a switch starts it over on the new one
    transcoder: Transcoder,
    generation: u64,
    missed: MissedStart,
    error: Option<String>,
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
            streams: [Stream::new(), Stream::new()],
            program: Program::new(Instant::now()),
            generation: 0,
            missed: MissedStart::default(),
            error: None,
            replug,
        }
    }

    /// `generation` is the link's: a new one may be a different device.
    /// Returns whether the chunk went out
    pub fn push(&mut self, generation: u64, lane: Lane, chunk: &[u8]) -> bool {
        if generation != self.generation {
            self.reset(generation);
        }
        let now = Instant::now();
        let stream = &mut self.streams[lane.index()];
        let known = stream.codec.is_some();
        let Some(data) = stream.take(chunk) else {
            return false;
        };
        let codec = stream.codec.expect("known once taken");
        if !known {
            match lane {
                Lane::Main => println!("[video] the link carries {codec:?}"),
                Lane::Second => println!("[video] a second camera on the link, {codec:?}"),
            }
        }
        if let Some(size) = stream.keep_params(&data) {
            self.program.set_size(lane, size);
        }
        let kept = stream.params.annex_b().is_some();
        let marks = lanes::marks(codec, &data);
        let choice = self.fanout.state.camera();
        let sent = match self.program.route(lane, codec, &marks, kept, choice, now) {
            Route::Drop => false,
            Route::Send => self.send(lane, &data, now),
            Route::Switch(at) => {
                // The converter starts over too, so no picture of the other
                // camera is ever decoded against this one's
                self.transcoder.restart();
                let size = match self.program.status(lane, now) {
                    LaneStatus {
                        width: Some(w),
                        height: Some(h),
                        ..
                    } => format!(", {w}x{h}"),
                    _ => String::new(),
                };
                println!("[video] the outputs show the {} camera{size}", lane.name());
                self.send(lane, &data[at..], now)
            }
        };
        self.publish(now);
        sent
    }

    fn send(&mut self, lane: Lane, data: &[u8], now: Instant) -> bool {
        let stream = &self.streams[lane.index()];
        match stream.codec {
            Some(Codec::H264) => self.fanout.video(InputMode::DjiFpv, data),
            Some(Codec::H265) => {
                if self.fanout.state.mode() != InputMode::DjiFpv {
                    if self.transcoder.converter().is_some() {
                        self.transcoder.stop();
                        self.error = None;
                    }
                    return false;
                }
                let have_params = stream.params.annex_b().is_some();
                match self.missed.check(now, have_params) {
                    Recovery::Wait => {}
                    Recovery::Replug => {
                        println!(
                            "[video] no stream settings from the remote, reconnecting the link"
                        );
                        (self.replug)();
                        return false;
                    }
                    Recovery::AskUser => {
                        self.error = Some(REPLUG_PROMPT.into());
                        return false;
                    }
                }
                if !have_params {
                    // Nothing can decode it yet
                    return false;
                }
                let result = self.transcoder.push(data, &stream.params);
                self.error = result.as_ref().err().cloned();
                result.is_ok()
            }
            None => false,
        }
    }

    fn reset(&mut self, generation: u64) {
        self.generation = generation;
        for stream in &mut self.streams {
            stream.clear();
        }
        self.program = Program::new(Instant::now());
        self.error = None;
        self.transcoder.stop();
        self.missed.new_link();
        self.publish(Instant::now());
    }

    fn publish(&self, now: Instant) {
        let program = self.program.lane();
        let status = DjiStatus {
            codec: self.streams[program.index()].codec,
            converter: self.transcoder.converter(),
            error: self.error.clone(),
            cameras: self.streams[Lane::Second.index()].codec.map(|_| Cameras {
                program,
                main: self.program.status(Lane::Main, now),
                second: self.program.status(Lane::Second, now),
            }),
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
