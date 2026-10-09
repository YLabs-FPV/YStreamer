use std::time::{Duration, Instant};

const CMD_SET: u8 = 2;
const CMD_CAMERA_STATE: u8 = 128;
const CMD_STORAGE: u8 = 220;

#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Storage {
    pub total_mb: u32,
    pub free_mb: u32,
    pub secs_left: u32,
}

#[derive(serde::Serialize, Clone, Default, Debug, PartialEq)]
pub struct AircraftRecorder {
    pub recording: bool,
    pub record_secs: u16,
    /// The storage being recorded to; all the O3 sends
    pub storage: Option<Storage>,
    pub sd: Option<Storage>,
    pub internal: Option<Storage>,
    pub to_internal: Option<bool>,
    /// Some units have internal storage only
    pub has_sd_slot: bool,
}

fn u32_at(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

fn storage_at(p: &[u8], total: usize, free: usize, left: usize) -> Storage {
    Storage {
        total_mb: u32_at(p, total),
        free_mb: u32_at(p, free),
        secs_left: u32_at(p, left),
    }
}

/// Follows the two frames and says when a browser should hear about it
pub struct Tracker {
    state: AircraftRecorder,
    detail_at: Option<Instant>,
    sent_at: Option<Instant>,
}

impl Tracker {
    const DETAIL_TIMEOUT: Duration = Duration::from_secs(5);
    const RESEND: Duration = Duration::from_secs(2);

    pub fn new() -> Self {
        Self {
            state: AircraftRecorder::default(),
            detail_at: None,
            sent_at: None,
        }
    }

    pub fn on_frame(
        &mut self,
        cmd_set: u8,
        cmd_id: u8,
        payload: &[u8],
    ) -> Option<AircraftRecorder> {
        if cmd_set != CMD_SET {
            return None;
        }
        let before = self.state.clone();
        match cmd_id {
            CMD_CAMERA_STATE if payload.len() >= 31 => {
                self.state.recording = payload[0] & 0x80 != 0;
                self.state.record_secs = u16::from_le_bytes([payload[29], payload[30]]);
                self.state.storage = Some(storage_at(payload, 5, 9, 17));
                if self
                    .detail_at
                    .is_some_and(|at| at.elapsed() > Self::DETAIL_TIMEOUT)
                {
                    self.detail_at = None;
                    self.state.sd = None;
                    self.state.internal = None;
                    self.state.to_internal = None;
                    self.state.has_sd_slot = false;
                }
            }
            // A list of storages: entry size, count, then per entry its
            // kind, whether it's there, and the same figures as above
            CMD_STORAGE if payload.len() >= 4 => {
                let entry_len = payload[1] as usize;
                let entries = &payload[4..];
                if entry_len < 18 || entries.len() < payload[2] as usize * entry_len {
                    return None;
                }
                self.detail_at = Some(Instant::now());
                self.state.to_internal = Some(payload[0] & 0x10 != 0);
                self.state.sd = None;
                self.state.internal = None;
                self.state.has_sd_slot = false;
                for e in entries.chunks_exact(entry_len).take(payload[2] as usize) {
                    let present = (e[1] == 0x01).then(|| storage_at(e, 2, 6, 14));
                    match e[0] {
                        0 => {
                            self.state.has_sd_slot = true;
                            self.state.sd = present;
                        }
                        1 => self.state.internal = present,
                        _ => {}
                    }
                }
            }
            _ => return None,
        }
        self.changed_since(before)
    }

    fn changed_since(&mut self, before: AircraftRecorder) -> Option<AircraftRecorder> {
        let due = self.sent_at.is_none_or(|at| at.elapsed() > Self::RESEND);
        if self.state == before && !due {
            return None;
        }
        self.sent_at = Some(Instant::now());
        Some(self.state.clone())
    }
}

#[cfg(test)]
#[path = "aircraft_recorder.test.rs"]
mod tests;
