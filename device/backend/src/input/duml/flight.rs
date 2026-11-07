use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::devices::Kind;
use super::session::Session;
use super::{FLAGS_REQUEST, Frame, encode, next_seq};
use crate::input::goggles::{Goggles, LinkState};

/// The flight controller's Function Control command: one byte saying what
/// to do, the same command DJI Fly sends for these buttons
const FC: u8 = 0x03;
const CMD_SET: u8 = 0x03;
const CMD_FUNCTION_CONTROL: u8 = 0x2a;

/// DJI Fly sends the same frame again, same sequence number, about every
/// half second until the aircraft answers; four sends at most in our
/// recordings
const RESEND_AFTER: Duration = Duration::from_millis(500);
const MAX_SENDS: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Takeoff,
    Land,
    CancelLanding,
    ReturnHome,
    CancelReturnHome,
}

impl Action {
    pub const ALL: [Action; 5] = [
        Action::Takeoff,
        Action::Land,
        Action::CancelLanding,
        Action::ReturnHome,
        Action::CancelReturnHome,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == name)
    }

    /// As it appears in the URL
    pub fn id(self) -> &'static str {
        match self {
            Action::Takeoff => "takeoff",
            Action::Land => "land",
            Action::CancelLanding => "cancel_landing",
            Action::ReturnHome => "return_home",
            Action::CancelReturnHome => "cancel_return_home",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Takeoff => "Take off",
            Action::Land => "Land",
            Action::CancelLanding => "Cancel landing",
            Action::ReturnHome => "Return to home",
            Action::CancelReturnHome => "Cancel return to home",
        }
    }

    /// The flight controller's command numbers, as listed in the public
    /// DUML dissector (dji-firmware-tools)
    fn code(self) -> u8 {
        match self {
            Action::Takeoff => 0x01,
            Action::Land => 0x02,
            Action::ReturnHome => 0x06,
            Action::CancelReturnHome => 0x0c,
            Action::CancelLanding => 0x0e,
        }
    }
}

pub fn frame(action: Action, seq: u16) -> Vec<u8> {
    encode(
        FC,
        seq,
        FLAGS_REQUEST,
        CMD_SET,
        CMD_FUNCTION_CONTROL,
        &[action.code()],
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    NoLink,
    NoAircraft,
    Busy,
    SendFailed(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NoLink => write!(f, "The goggles link isn't live"),
            Refused::NoAircraft => write!(f, "No aircraft is linked to the goggles"),
            Refused::Busy => write!(
                f,
                "The last aircraft command is still waiting for an answer"
            ),
            Refused::SendFailed(e) => write!(f, "Couldn't send to the goggles: {e}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct Outcome {
    pub action: Action,
    /// The aircraft answered the command
    pub acknowledged: bool,
    /// How often the frame went out before the answer, or before giving up
    pub sends: u32,
    /// The answer's payload, as hex
    pub reply: Option<String>,
}

/// Where the commands go: the goggles on the device, a fake in tests
pub trait Link: Send + Sync {
    fn ready(&self) -> Result<(), Refused>;
    fn send(&self, frame: &[u8]) -> std::io::Result<()>;
}

pub struct GogglesLink {
    pub goggles: Goggles,
    pub session: Arc<Session>,
}

impl Link for GogglesLink {
    fn ready(&self) -> Result<(), Refused> {
        if self.goggles.state() != LinkState::Live {
            return Err(Refused::NoLink);
        }
        if !self
            .session
            .devices()
            .iter()
            .any(|d| d.kind == Kind::Aircraft)
        {
            return Err(Refused::NoAircraft);
        }
        Ok(())
    }

    fn send(&self, frame: &[u8]) -> std::io::Result<()> {
        self.goggles.send(frame)
    }
}

#[derive(Default)]
struct Waiting {
    seq: Option<u16>,
    reply: Option<Vec<u8>>,
}

/// Sends one aircraft command at a time and waits for its answer
pub struct Flight {
    link: Box<dyn Link>,
    waiting: Mutex<Waiting>,
    answered: Condvar,
    resend_after: Duration,
}

impl Flight {
    pub fn new(link: impl Link + 'static) -> Arc<Self> {
        Arc::new(Self::with_resend(link, RESEND_AFTER))
    }

    fn with_resend(link: impl Link + 'static, resend_after: Duration) -> Self {
        Self {
            link: Box::new(link),
            waiting: Mutex::new(Waiting::default()),
            answered: Condvar::new(),
            resend_after,
        }
    }

    /// One press, one command. It only goes out again, unchanged, while the
    /// aircraft hasn't answered, like DJI Fly does. Blocks until the answer
    /// or the last resend
    pub fn send(&self, action: Action) -> Result<Outcome, Refused> {
        self.link.ready()?;
        let seq = next_seq();
        {
            let mut w = self.waiting.lock().unwrap();
            if w.seq.is_some() {
                return Err(Refused::Busy);
            }
            *w = Waiting {
                seq: Some(seq),
                reply: None,
            };
        }
        let result = self.exchange(action, &frame(action, seq));
        *self.waiting.lock().unwrap() = Waiting::default();
        result
    }

    fn exchange(&self, action: Action, frame: &[u8]) -> Result<Outcome, Refused> {
        let mut sends = 0;
        while sends < MAX_SENDS {
            // A resend only while the aircraft is still there to hear it
            if sends > 0 && self.link.ready().is_err() {
                break;
            }
            if let Err(e) = self.link.send(frame) {
                if sends == 0 {
                    return Err(Refused::SendFailed(e.to_string()));
                }
                break;
            }
            sends += 1;
            let w = self.waiting.lock().unwrap();
            let (w, _) = self
                .answered
                .wait_timeout_while(w, self.resend_after, |w| w.reply.is_none())
                .unwrap();
            if let Some(reply) = &w.reply {
                return Ok(Outcome {
                    action,
                    acknowledged: true,
                    sends,
                    reply: Some(hex::encode(reply)),
                });
            }
        }
        Ok(Outcome {
            action,
            acknowledged: false,
            sends,
            reply: None,
        })
    }

    /// Called for every DUML frame from the goggles; picks out the answer
    /// to the command in flight
    pub fn on_frame(&self, f: &Frame) {
        if !f.is_response()
            || f.src & 0x1f != FC
            || f.cmd_set != CMD_SET
            || f.cmd_id != CMD_FUNCTION_CONTROL
        {
            return;
        }
        let mut w = self.waiting.lock().unwrap();
        if w.seq == Some(f.seq) && w.reply.is_none() {
            w.reply = Some(f.payload.to_vec());
            self.answered.notify_all();
        }
    }
}

#[cfg(test)]
#[path = "flight.test.rs"]
mod tests;
