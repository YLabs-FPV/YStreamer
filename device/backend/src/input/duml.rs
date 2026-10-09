pub mod aircraft_recorder;
pub mod camera;
pub mod devices;
pub mod link;
pub mod monitor;
pub mod session;

use std::sync::atomic::{AtomicU16, Ordering};

const VIDEO_PORT: usize = 0x574A;
const VIDEO_PORT_B: usize = 0x574B;

const MAX_BUF: usize = 4 * 1024 * 1024;

/// Our own address on the bus
pub const APP: u8 = 0x02;

pub const FLAGS_REQUEST: u8 = 0x40;
pub const FLAGS_RESPONSE: u8 = 0x80;

static SEQ: AtomicU16 = AtomicU16::new(1);

pub fn next_seq() -> u16 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// A decoded DUML frame from the goggles
#[derive(Debug)]
pub struct Frame<'a> {
    pub src: u8,
    pub dst: u8,
    pub seq: u16,
    pub flags: u8,
    pub cmd_set: u8,
    pub cmd_id: u8,
    pub payload: &'a [u8],
}

impl Frame<'_> {
    pub fn is_response(&self) -> bool {
        self.flags & 0x80 != 0
    }

    pub fn wants_reply(&self) -> bool {
        !self.is_response() && (self.flags >> 5) & 0x03 != 0
    }
}

// ---- CRC8: init 0x77, poly 0x8C (reflected) ----
pub fn calc_crc8(data: &[u8]) -> u8 {
    let mut crc: u16 = 0x77;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 {
                (crc >> 1) ^ 0x8C
            } else {
                crc >> 1
            };
        }
    }
    (crc & 0xFF) as u8
}

// ---- CRC16: init 0x3692, poly 0x8408 (reflected) ----
pub fn calc_crc16(data: &[u8]) -> u16 {
    let mut crc: u32 = 0x3692;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    (crc & 0xFFFF) as u16
}

/// Build a DUML frame from us and wrap it for the LogicLink control channel
pub fn encode(dst: u8, seq: u16, flags: u8, cmd_set: u8, cmd_id: u8, payload: &[u8]) -> Vec<u8> {
    let duml_len = 11 + payload.len() + 2;
    let mut out = Vec::with_capacity(8 + duml_len);

    // LogicLink: 55 CC, channel 0x49, version 0x57, u32 length
    out.extend_from_slice(&[0x55, 0xCC, 0x49, 0x57]);
    out.extend_from_slice(&(duml_len as u32).to_le_bytes());

    let start = out.len();
    out.push(0x55);
    out.push((duml_len & 0xFF) as u8);
    // Length high bits + protocol version 1
    out.push((((duml_len >> 8) & 0x03) as u8) | (1 << 2));
    out.push(calc_crc8(&out[start..start + 3]));
    out.push(APP);
    out.push(dst);
    out.extend_from_slice(&seq.to_le_bytes());
    out.push(flags);
    out.push(cmd_set);
    out.push(cmd_id);
    out.extend_from_slice(payload);
    let crc16 = calc_crc16(&out[start..]);
    out.extend_from_slice(&crc16.to_le_bytes());
    out
}

pub struct DumlReassemblyBuffer {
    buf: Vec<u8>,
}

impl DumlReassemblyBuffer {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(512 * 1024),
        }
    }

    pub fn process(
        &mut self,
        data: &[u8],
        mut on_video: impl FnMut(&[u8]),
        mut on_duml: impl FnMut(&Frame),
    ) {
        self.buf.extend_from_slice(data);
        if self.buf.len() > MAX_BUF {
            eprintln!("[duml] buffer overflow, resetting");
            self.buf.clear();
            return;
        }

        let mut i = 0;
        while i + 8 <= self.buf.len() {
            // LogicLink frames start with 55 CC
            if self.buf[i] != 0x55 || self.buf[i + 1] != 0xCC {
                i += 1;
                continue;
            }

            let port = (self.buf[i + 2] as usize) | ((self.buf[i + 3] as usize) << 8);
            let inner_len = (self.buf[i + 4] as usize) | ((self.buf[i + 5] as usize) << 8);
            let total = 8 + inner_len;

            if i + total > self.buf.len() {
                break; // incomplete frame; wait for more data
            }

            let inner_start = i + 8;
            let inner = &self.buf[inner_start..inner_start + inner_len];

            if port == VIDEO_PORT || port == VIDEO_PORT_B {
                // Video
                if inner_len >= 13 && inner[0] == 0x55 {
                    let duml_len = (inner[1] as usize) | (((inner[2] as usize) & 0x03) << 8);
                    if duml_len == inner_len {
                        let payload_len = inner_len - 13;
                        if payload_len > 0 {
                            on_video(&inner[11..11 + payload_len]);
                        }
                    } else {
                        on_video(inner);
                    }
                } else {
                    on_video(inner);
                }
            } else {
                // DUML command response
                if inner_len >= 13 && inner[0] == 0x55 {
                    let duml_len = (inner[1] as usize) | (((inner[2] as usize) & 0x03) << 8);
                    // Validate: length agrees AND CRC8 over the first 3 bytes matches
                    if duml_len == inner_len && calc_crc8(&inner[0..3]) == inner[3] {
                        let frame = Frame {
                            src: inner[4],
                            dst: inner[5],
                            seq: u16::from_le_bytes([inner[6], inner[7]]),
                            flags: inner[8],
                            cmd_set: inner[9],
                            cmd_id: inner[10],
                            payload: &inner[11..inner_len - 2],
                        };
                        // Ignore our own outgoing packets (sender 2 or 5)
                        if frame.src != 2 && frame.src != 5 {
                            on_duml(&frame);
                        }
                    }
                }
            }

            i += total;
        }

        if i > 0 {
            self.buf.drain(0..i);
        }
    }
}

#[cfg(test)]
#[path = "duml.test.rs"]
mod tests;
