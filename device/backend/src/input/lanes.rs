use std::time::{Duration, Instant};

use serde::Serialize;

use super::codec::Codec;
use super::{h264, h265};
use crate::settings::CameraChoice;

/// A video stream on the USB-C link. A two-camera aircraft, such as the
/// Avata 360 over an RC-N3, sends a second one next to the usual
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Main,
    Second,
}

impl Lane {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Lane::Main => "main",
            Lane::Second => "second",
        }
    }

    pub fn other(self) -> Lane {
        match self {
            Lane::Main => Lane::Second,
            Lane::Second => Lane::Main,
        }
    }
}

/// Rates are measured in these steps, over the last `BUCKETS` of them
const TICK: Duration = Duration::from_millis(250);
const BUCKETS: usize = 4;
/// How far ahead the other camera must be to take over. The live view
/// runs about ten times the other camera's rate, so this stays clear of
/// noise without missing a real change
const RATIO: f32 = 1.6;
/// Below this a stream isn't a live view, e.g. both in the middle of
/// DJI Fly changing the picture
const MIN_KBPS: u32 = 800;
/// Ticks in a row the other camera must stay ahead: a burst of keyframes
/// shouldn't take the outputs
const CONFIRM_TICKS: u8 = 6;
/// Between two switches, and after a failed one
const DWELL: Duration = Duration::from_secs(4);
/// The two streams' parameter sets can come in either order at the start
const WARMUP: Duration = Duration::from_secs(2);
/// The camera on the outputs sending nothing this long, while the other
/// one is live, switches at once: waiting would only freeze the picture
const SILENT: Duration = Duration::from_millis(750);
/// A stream is live while it delivers bytes and pictures
const LIVE_BYTES: Duration = Duration::from_secs(1);
const LIVE_PICTURES: Duration = Duration::from_secs(2);
/// A switch waits this long for the other camera's own parameter sets,
/// which come at once when DJI Fly changes its picture. Without one by
/// then the camera hasn't changed, and the ones kept from earlier, with a
/// keyframe, still describe it
const SWITCH_SETTLE: Duration = Duration::from_millis(400);
/// A switch with nothing to start from by then is given up, and the
/// outputs stay on the camera they have
const SWITCH_DEADLINE: Duration = Duration::from_secs(3);
const SWITCH_WARNING_EVERY: Duration = Duration::from_secs(10);

/// How one camera's stream is doing
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct LaneStatus {
    pub kbps: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub live: bool,
}

#[derive(Default)]
struct Meter {
    buckets: [u64; BUCKETS],
    current: usize,
    stepped: Option<Instant>,
    last_bytes: Option<Instant>,
    last_picture: Option<Instant>,
    size: Option<(u32, u32)>,
}

impl Meter {
    /// Moves on with the clock, so a stream that stops drops to zero
    fn step(&mut self, now: Instant) {
        let Some(at) = self.stepped else {
            self.stepped = Some(now);
            return;
        };
        let steps = (now.saturating_duration_since(at).as_millis() / TICK.as_millis()) as usize;
        if steps == 0 {
            return;
        }
        for _ in 0..steps.min(BUCKETS) {
            self.current = (self.current + 1) % BUCKETS;
            self.buckets[self.current] = 0;
        }
        self.stepped = Some(at + TICK * steps as u32);
    }

    fn add(&mut self, bytes: usize, picture: bool, now: Instant) {
        self.step(now);
        self.buckets[self.current] += bytes as u64;
        self.last_bytes = Some(now);
        if picture {
            self.last_picture = Some(now);
        }
    }

    fn kbps(&self) -> u32 {
        let bytes: u64 = self.buckets.iter().sum();
        let window = (TICK * BUCKETS as u32).as_millis() as u64;
        (bytes * 8 / window) as u32
    }

    fn live(&self, now: Instant) -> bool {
        let within = |at: Option<Instant>, d| at.is_some_and(|at| now.duration_since(at) <= d);
        within(self.last_bytes, LIVE_BYTES) && within(self.last_picture, LIVE_PICTURES)
    }

    fn silent(&self, now: Instant) -> bool {
        self.last_bytes
            .is_none_or(|at| now.duration_since(at) >= SILENT)
    }

    fn pixels(&self) -> Option<u32> {
        self.size.map(|(w, h)| w * h)
    }
}

/// Picks the camera that goes to the outputs. DJI Fly's picture-in-picture
/// changes the encoder on a stream instead of swapping the streams, so the
/// port says nothing about which camera is which: the big stream is the
/// live view. Rate decides, since it's there all the time; the picture
/// size, known only from parameter sets, can only stop a switch to a
/// smaller picture
pub struct Selector {
    meters: [Meter; 2],
    program: Lane,
    confirm: u8,
    started: Instant,
    last_switch: Option<Instant>,
    blocked_until: Option<Instant>,
    last_tick: Option<Instant>,
}

impl Selector {
    pub fn new(now: Instant) -> Self {
        Self {
            meters: Default::default(),
            program: Lane::Main,
            confirm: 0,
            started: now,
            last_switch: None,
            blocked_until: None,
            last_tick: None,
        }
    }

    /// The camera on the outputs
    pub fn program(&self) -> Lane {
        self.program
    }

    pub fn on_data(&mut self, lane: Lane, bytes: usize, picture: bool, now: Instant) {
        self.meters[lane.index()].add(bytes, picture, now);
    }

    pub fn set_size(&mut self, lane: Lane, size: Option<(u32, u32)>) {
        self.meters[lane.index()].size = size;
    }

    /// The camera to switch to, if any. Rates are judged once a tick
    pub fn tick(&mut self, now: Instant, choice: CameraChoice) -> Option<Lane> {
        if self
            .last_tick
            .is_some_and(|at| now.duration_since(at) < TICK)
        {
            return None;
        }
        self.last_tick = Some(now);
        for m in &mut self.meters {
            m.step(now);
        }
        if self.blocked_until.is_some_and(|at| now < at) {
            return None;
        }
        let other = self.program.other();
        let challenger = &self.meters[other.index()];
        let pinned = match choice {
            CameraChoice::Auto => None,
            CameraChoice::Main => Some(Lane::Main),
            CameraChoice::Second => Some(Lane::Second),
        };
        if let Some(lane) = pinned {
            // Only to a camera that's there; a single-camera aircraft
            // stays on the one it has
            return (lane == other && challenger.live(now)).then_some(lane);
        }
        if !challenger.live(now) {
            self.confirm = 0;
            return None;
        }
        if self.meters[self.program.index()].silent(now) {
            return Some(other);
        }
        if now.duration_since(self.started) < WARMUP
            || self
                .last_switch
                .is_some_and(|at| now.duration_since(at) < DWELL)
        {
            return None;
        }
        let (ours, theirs) = (self.meters[self.program.index()].kbps(), challenger.kbps());
        let ahead = theirs >= MIN_KBPS && theirs as f32 > ours as f32 * RATIO;
        let smaller = matches!(
            (challenger.pixels(), self.meters[self.program.index()].pixels()),
            (Some(theirs), Some(ours)) if theirs < ours
        );
        if ahead && !smaller {
            self.confirm += 1;
        } else {
            self.confirm = 0;
        }
        (self.confirm >= CONFIRM_TICKS).then_some(other)
    }

    /// Sending nothing, e.g. a camera that stopped
    pub fn silent(&self, lane: Lane, now: Instant) -> bool {
        self.meters[lane.index()].silent(now)
    }

    /// The outputs now get `lane`
    pub fn switched(&mut self, lane: Lane, now: Instant) {
        self.program = lane;
        self.confirm = 0;
        self.last_switch = Some(now);
    }

    /// A switch found nothing to start from; not tried again for a while
    pub fn failed(&mut self, now: Instant) {
        self.confirm = 0;
        self.blocked_until = Some(now + DWELL);
    }

    pub fn status(&self, lane: Lane, now: Instant) -> LaneStatus {
        let m = &self.meters[lane.index()];
        LaneStatus {
            kbps: m.kbps(),
            width: m.size.map(|(w, _)| w),
            height: m.size.map(|(_, h)| h),
            live: m.live(now),
        }
    }
}

/// What to do with a chunk of one camera's stream
#[derive(Debug, PartialEq)]
pub enum Route {
    /// Not for the outputs
    Drop,
    /// To the outputs
    Send,
    /// From this offset on, the outputs get this camera instead of the
    /// other one. Whatever decodes it starts over
    Switch(usize),
}

/// Which camera's stream goes to the outputs, and the switches between
/// them. The outputs keep the old camera until the new one reaches its
/// parameter sets or a keyframe, so they never get pictures that refer to
/// the other camera's
pub struct Program {
    selector: Selector,
    switch: Option<(Lane, Instant)>,
    warned: Option<Instant>,
}

impl Program {
    pub fn new(now: Instant) -> Self {
        Self {
            selector: Selector::new(now),
            switch: None,
            warned: None,
        }
    }

    /// The camera on the outputs
    pub fn lane(&self) -> Lane {
        self.selector.program()
    }

    pub fn set_size(&mut self, lane: Lane, size: Option<(u32, u32)>) {
        self.selector.set_size(lane, size);
    }

    pub fn status(&self, lane: Lane, now: Instant) -> LaneStatus {
        self.selector.status(lane, now)
    }

    /// `kept`: the parameter sets this camera sent earlier are all there
    pub fn route(
        &mut self,
        lane: Lane,
        codec: Codec,
        marks: &Marks,
        kept: bool,
        choice: CameraChoice,
        now: Instant,
    ) -> Route {
        self.selector.on_data(lane, marks.bytes, marks.picture, now);
        // Before routing, so a switch can start on this very chunk
        self.choose(choice, now);
        if let Some((to, since)) = self.switch
            && to == lane
        {
            let settled = now.duration_since(since) >= SWITCH_SETTLE
                || self.selector.silent(self.selector.program(), now);
            let Some(at) = join_at(codec, marks, settled, kept) else {
                // The outputs stay on the other camera until this one can start
                return Route::Drop;
            };
            self.switch = None;
            self.selector.switched(lane, now);
            return Route::Switch(at);
        }
        if lane == self.selector.program() {
            Route::Send
        } else {
            Route::Drop
        }
    }

    fn choose(&mut self, choice: CameraChoice, now: Instant) {
        if let Some((to, since)) = self.switch {
            let unwanted = matches!(
                (choice, to),
                (CameraChoice::Main, Lane::Second) | (CameraChoice::Second, Lane::Main)
            );
            if unwanted {
                self.switch = None;
            } else if now.duration_since(since) >= SWITCH_DEADLINE {
                self.switch = None;
                self.selector.failed(now);
                if self
                    .warned
                    .is_none_or(|at| now.duration_since(at) >= SWITCH_WARNING_EVERY)
                {
                    self.warned = Some(now);
                    eprintln!(
                        "[video] couldn't switch to the {} camera: no picture to start from",
                        to.name()
                    );
                }
            }
            return;
        }
        if let Some(to) = self.selector.tick(now, choice) {
            println!("[video] switching the outputs to the {} camera", to.name());
            self.switch = Some((to, now));
        }
    }
}

/// Where a camera being switched to can start in a chunk: at its own
/// parameter sets, or, once `settled`, at a keyframe after the ones `kept`.
/// H.264 has its SPS with every keyframe, so it always starts there
pub fn join_at(codec: Codec, marks: &Marks, settled: bool, kept: bool) -> Option<usize> {
    if marks.params.is_some() {
        return marks.params;
    }
    match codec {
        Codec::H265 if settled && kept => marks.keyframe,
        _ => None,
    }
}

/// Where in a chunk a camera's stream could be joined
#[derive(Debug, Default, PartialEq)]
pub struct Marks {
    pub bytes: usize,
    /// Holds part of a picture
    pub picture: bool,
    /// The first parameter set: what a stream starts over with, e.g. when
    /// DJI Fly changes its picture
    pub params: Option<usize>,
    /// The first keyframe, for joining with parameter sets kept earlier
    pub keyframe: Option<usize>,
}

pub fn marks(codec: Codec, data: &[u8]) -> Marks {
    let mut marks = Marks {
        bytes: data.len(),
        ..Default::default()
    };
    // A slice starting a picture has the first bit after the NAL header set:
    // first_slice_segment_in_pic_flag, or first_mb_in_slice of 0
    let first_slice = |at: usize| data.get(at).is_some_and(|b| b & 0x80 != 0);
    for nal in h264::nals(data) {
        let header = data[nal.header];
        let (picture, params, keyframe) = match codec {
            Codec::H265 => {
                let kind = h265::kind(header);
                (
                    kind < h265::VPS,
                    kind == h265::VPS || kind == h265::SPS,
                    h265::is_irap(kind) && first_slice(nal.header + 2),
                )
            }
            Codec::H264 => (
                nal.is_vcl(),
                nal.kind == h264::SPS,
                nal.kind == h264::IDR && first_slice(nal.header + 1),
            ),
        };
        marks.picture |= picture;
        if params && marks.params.is_none() {
            marks.params = Some(nal.start);
        }
        if keyframe && marks.keyframe.is_none() {
            marks.keyframe = Some(nal.start);
        }
    }
    marks
}

#[cfg(test)]
#[path = "lanes.test.rs"]
mod tests;
