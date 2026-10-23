//! Moving the aircraft's camera from the browser.
//!
//! Avata 2: the gimbal (cmd set 0x04) follows an angle only after a Resume,
//! and the Resume also takes it away from the remote, so the hold is handed
//! back with a Suspend once the browser goes quiet. Resume, Suspend and the
//! angle command are the ones dji-firmware-tools documents for cmd set 0x04;
//! the 7-byte angle layout is the one that moved our own Avata 2.
//!
//! Avata 360: the view heading is the 13-byte angle command DJI Fly sends
//! when the view is turned, recorded from our own aircraft. DJI Fly sends no
//! Resume for it, and only ever moves the heading.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::{FLAGS_REQUEST, encode, next_seq};
use crate::input::goggles::Goggles;

const GIMBAL: u8 = 0x04;
const CMD_SET: u8 = 0x04;
/// "Ext Ctrl Degree" in dji-firmware-tools: angles in tenths of a degree
const CMD_ANGLE: u8 = 0x0A;
const CMD_SUSPEND_RESUME: u8 = 0x0D;
const RESUME: u16 = 0x2AB5;
const SUSPEND: u16 = 0x7EF2;
/// The gimbal doesn't answer these, so none is asked for
const NO_ACK: u8 = 0x00;

/// What the Avata 2's camera can be asked for, in degrees; up stops short of
/// this on the airframe
pub const TILT_MIN: f32 = -90.0;
pub const TILT_MAX: f32 = 50.0;

/// Without a new angle for this long the remote gets the gimbal back
const IDLE: Duration = Duration::from_millis(1500);
const TICK: Duration = Duration::from_millis(250);

fn tenths(deg: f32) -> [u8; 2] {
    ((deg * 10.0).round() as i16).to_le_bytes()
}

pub fn resume(seq: u16) -> Vec<u8> {
    encode(
        GIMBAL,
        seq,
        NO_ACK,
        CMD_SET,
        CMD_SUSPEND_RESUME,
        &RESUME.to_le_bytes(),
    )
}

pub fn suspend(seq: u16) -> Vec<u8> {
    encode(
        GIMBAL,
        seq,
        NO_ACK,
        CMD_SET,
        CMD_SUSPEND_RESUME,
        &SUSPEND.to_le_bytes(),
    )
}

/// Tilt first, then the two axes the Avata 2 doesn't have, then the flag
/// that says which ones apply
pub fn tilt(deg: f32, seq: u16) -> Vec<u8> {
    let t = tenths(deg.clamp(TILT_MIN, TILT_MAX));
    let payload = [t[0], t[1], 0, 0, 0, 0, 0x01];
    encode(GIMBAL, seq, NO_ACK, CMD_SET, CMD_ANGLE, &payload)
}

/// The Avata 360's view heading, wrapped onto -180..=180: the sphere has no
/// end stop. Pitch and roll stay 0, as DJI Fly sends them
pub fn avata360_heading(deg: f32, seq: u16) -> Vec<u8> {
    let wrapped = match deg {
        // DJI Fly sends both ends, so they're kept as given
        d if (-180.0..=180.0).contains(&d) => d,
        d if d.is_finite() => {
            let d = d.rem_euclid(360.0);
            if d > 180.0 { d - 360.0 } else { d }
        }
        _ => 0.0,
    };
    let h = tenths(wrapped);
    let payload = [
        0x00, 0x00, h[0], h[1], 0, 0, 0, 0, 0xF4, 0x04, 0x0A, 0x00, 0x08,
    ];
    encode(GIMBAL, seq, FLAGS_REQUEST, CMD_SET, CMD_ANGLE, &payload)
}

/// Whether the browser has the gimbal, on which goggles session, and since
/// when it last said where to point it
#[derive(Default)]
struct Hold {
    held: Option<(u64, Instant)>,
}

impl Hold {
    /// A new angle: true when the gimbal has to be taken first
    fn moved(&mut self, link: u64, now: Instant) -> bool {
        let take = self.held.is_none_or(|(l, _)| l != link);
        self.held = Some((link, now));
        take
    }

    /// Let go: true when there's a hold on this link to hand back. A hold
    /// from an earlier link went with it
    fn release(&mut self, link: u64) -> bool {
        self.held.take().is_some_and(|(l, _)| l == link)
    }

    fn expired(&self, now: Instant) -> bool {
        self.held
            .is_some_and(|(_, at)| now.saturating_duration_since(at) > IDLE)
    }
}

pub struct Gimbal {
    goggles: Goggles,
    hold: Mutex<Hold>,
}

impl Gimbal {
    pub fn start(goggles: Goggles) -> Arc<Self> {
        let gimbal = Arc::new(Self {
            goggles,
            hold: Mutex::new(Hold::default()),
        });
        let this = Arc::clone(&gimbal);
        thread::spawn(move || {
            loop {
                thread::sleep(TICK);
                if this.hold.lock().unwrap().expired(Instant::now()) {
                    this.release();
                }
            }
        });
        gimbal
    }

    /// The browser keeps sending its angle while it holds the gimbal, so
    /// each one also keeps the hold alive
    pub fn tilt(&self, deg: f32) {
        let mut hold = self.hold.lock().unwrap();
        if hold.moved(self.goggles.generation(), Instant::now()) {
            self.send(&resume(next_seq()));
        }
        self.send(&tilt(deg, next_seq()));
    }

    pub fn release(&self) {
        let mut hold = self.hold.lock().unwrap();
        if hold.release(self.goggles.generation()) {
            self.send(&suspend(next_seq()));
        }
    }

    pub fn avata360_heading(&self, deg: f32) {
        self.send(&avata360_heading(deg, next_seq()));
    }

    fn send(&self, bytes: &[u8]) {
        if let Err(e) = self.goggles.send(bytes) {
            eprintln!("[gimbal] send failed: {e}");
        }
    }
}

#[cfg(test)]
#[path = "gimbal.test.rs"]
mod tests;
