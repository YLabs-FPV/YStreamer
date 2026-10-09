use std::time::{Duration, Instant};

use super::Frame;

const CMD_SET: u8 = 9;
const CMD_SIGNAL_QUALITY: u8 = 8;
const SENDER: u8 = 14;

pub struct Tracker {
    quality: Option<u8>,
    sent_at: Option<Instant>,
}

impl Tracker {
    const RESEND: Duration = Duration::from_secs(2);

    pub fn new() -> Self {
        Self {
            quality: None,
            sent_at: None,
        }
    }

    pub fn on_frame(&mut self, f: &Frame) -> Option<u8> {
        if f.is_response() || f.cmd_set != CMD_SET || f.cmd_id != CMD_SIGNAL_QUALITY {
            return None;
        }
        if f.src != SENDER {
            return None;
        }
        // The top bit is a flag of its own
        let quality = *f.payload.first()? & 0x7F;
        let due = self.sent_at.is_none_or(|at| at.elapsed() > Self::RESEND);
        if self.quality == Some(quality) && !due {
            return None;
        }
        self.quality = Some(quality);
        self.sent_at = Some(Instant::now());
        Some(quality)
    }
}

#[cfg(test)]
#[path = "link.test.rs"]
mod tests;
