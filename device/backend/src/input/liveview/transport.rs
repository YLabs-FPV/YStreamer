use std::collections::HashMap;

use crate::input::duml;

/// Every datagram starts with a header this long:
///
/// ```text
///  0  1   0x8000 | datagram length, little-endian
///  2  3   session id, chosen by us and echoed by the goggles
///  4  5   sender sequence
///  6      kind
///  7      XOR of bytes 0-6
///  8  9   the peer's sequence being acknowledged
/// 10 11   sender sequence again
/// 16 19   kind-specific; for video the unit number and fragment index
/// ```
pub const HEADER_LEN: usize = 20;

const KIND_OPENER: u8 = 0x00;
/// The goggles' transport state, echoed back in our acks
pub const KIND_STATE: u8 = 0x01;
pub const KIND_VIDEO: u8 = 0x02;
const KIND_ACK: u8 = 0x04;
const KIND_CONTROL: u8 = 0x05;

/// What the goggles answer an accepted opener with
const OPENER_ACK_LEN: usize = 9;
const ACK_LEN: usize = 34;

/// DJI Fly's opener, as recorded from it connecting to our goggles. The
/// session id, the first sequence and the checksum are filled in per session
const OPENER: [u8; 48] = [
    0x30, 0x80, 0x2b, 0xf3, 0x00, 0x00, 0x00, 0x68, 0x58, 0x99, 0x64, 0x00, 0x64, 0x00, 0xc0, 0x05,
    0x14, 0x00, 0x00, 0x64, 0x00, 0x00, 0x01, 0x90, 0x01, 0xc0, 0x05, 0x14, 0x00, 0x00, 0x64, 0x00,
    0x14, 0x00, 0x64, 0x00, 0xc0, 0x05, 0x14, 0x00, 0x00, 0x64, 0x00, 0x01, 0x01, 0x04, 0x01, 0x02,
];

/// DJI Fly keeps the session alive with a DUML request to each of these in
/// turn: an empty 00/01 to the first four, 00/b7 to the last
const RECEIVERS: [u8; 5] = [0x0e, 0x1f, 0xa2, 0x00, 0x28];
const B7_PAYLOAD: [u8; 5] = [0x01, 0x01, 0x00, 0x0c, 0x00];

fn checksum(datagram: &[u8]) -> u8 {
    datagram[..7].iter().fold(0, |x, b| x ^ b)
}

fn length_field(len: usize) -> [u8; 2] {
    (0x8000 | len as u16).to_le_bytes()
}

/// `seq` is the first sender sequence; DJI Fly keeps its low three bits clear
pub fn opener(session: u16, seq: u16) -> [u8; 48] {
    let mut d = OPENER;
    d[2..4].copy_from_slice(&session.to_le_bytes());
    d[6] = KIND_OPENER;
    d[8..10].copy_from_slice(&seq.to_le_bytes());
    d[7] = checksum(&d);
    d
}

/// The keep-alive, sent every few milliseconds. `counter` picks the DUML
/// request in the cycle and starts at 1
pub fn control(session: u16, seq: u16, peer_seq: u16, counter: u8, duml_seq: u16) -> Vec<u8> {
    let step = usize::from(counter.wrapping_sub(1)) % RECEIVERS.len();
    let (cmd_id, payload): (u8, &[u8]) = if step == RECEIVERS.len() - 1 {
        (0xb7, &B7_PAYLOAD)
    } else {
        (0x01, &[])
    };
    let frame = duml::frame(
        RECEIVERS[step],
        duml_seq,
        duml::FLAGS_REQUEST,
        0x00,
        cmd_id,
        payload,
    );
    let len = HEADER_LEN + frame.len();
    let mut d = vec![0u8; len];
    d[0..2].copy_from_slice(&length_field(len));
    d[2..4].copy_from_slice(&session.to_le_bytes());
    d[4..6].copy_from_slice(&seq.to_le_bytes());
    d[6] = KIND_CONTROL;
    d[8..10].copy_from_slice(&peer_seq.to_le_bytes());
    d[10..12].copy_from_slice(&seq.to_le_bytes());
    d[16] = counter;
    d[17] = 0x01;
    d[HEADER_LEN..].copy_from_slice(&frame);
    d[7] = checksum(&d);
    d
}

/// Acknowledges every video datagram up to `video_seq`. Only ever the last
/// fragment of a complete unit: acking past a gap stops the goggles from
/// sending it again
pub fn ack(session: u16, video_seq: u16, state: &[u8; 18]) -> [u8; ACK_LEN] {
    let mut d = [0u8; ACK_LEN];
    d[0..2].copy_from_slice(&length_field(ACK_LEN));
    d[2..4].copy_from_slice(&session.to_le_bytes());
    d[6] = KIND_ACK;
    d[8..10].copy_from_slice(&video_seq.to_le_bytes());
    d[10..12].copy_from_slice(&video_seq.to_le_bytes());
    d[16..].copy_from_slice(state);
    d[7] = checksum(&d);
    d
}

pub enum Packet<'a> {
    /// The session was accepted
    OpenerAck,
    Transport {
        seq: u16,
        kind: u8,
        datagram: &'a [u8],
    },
}

/// None for anything not ours: another session, or a malformed datagram
pub fn parse(datagram: &[u8], session: u16) -> Option<Packet<'_>> {
    if datagram.len() < 4 || datagram[2..4] != session.to_le_bytes() {
        return None;
    }
    if datagram.len() == OPENER_ACK_LEN {
        return Some(Packet::OpenerAck);
    }
    if datagram.len() < HEADER_LEN || datagram[0..2] != length_field(datagram.len()) {
        return None;
    }
    Some(Packet::Transport {
        seq: u16::from_le_bytes([datagram[4], datagram[5]]),
        kind: datagram[6],
        datagram,
    })
}

/// What a state datagram says, for echoing in acks
pub fn state(datagram: &[u8]) -> Option<[u8; 18]> {
    datagram.get(16..ACK_LEN)?.try_into().ok()
}

/// A whole piece of the H.264 stream, put back together
pub struct Unit {
    pub data: Vec<u8>,
    /// Sequence of its last fragment: what to ack
    pub last_seq: u16,
}

struct Partial {
    count: usize,
    fragments: HashMap<usize, (Vec<u8>, u16)>,
}

/// Units this many ahead completed while an earlier one never did: that one
/// is lost for good, so skip it rather than stall. The next refresh repairs
/// the picture
const GIVE_UP_AFTER: usize = 8;

/// Joins video fragments into units and hands them on in order. Byte 16 is
/// the unit number, wrapping; the low 7 bits of byte 17 the fragment count.
/// The fragment index is bits 0-4 of byte 18 times two, plus bit 7 of byte 17
#[derive(Default)]
pub struct Assembler {
    partial: HashMap<u8, Partial>,
    complete: HashMap<u8, Unit>,
    last: Option<u8>,
    pub duplicates: u64,
    pub invalid: u64,
    pub skipped: u64,
}

/// Whether unit `n` comes after `last`, allowing for wrap-around
fn after(n: u8, last: u8) -> bool {
    (1..0x80).contains(&n.wrapping_sub(last))
}

impl Assembler {
    /// The units that can go out now, oldest first
    pub fn push(&mut self, datagram: &[u8]) -> Vec<Unit> {
        if datagram.len() <= HEADER_LEN {
            self.invalid += 1;
            return Vec::new();
        }
        let unit = datagram[16];
        let count = usize::from(datagram[17] & 0x7f);
        let index = usize::from(datagram[18] & 0x1f) * 2 + usize::from(datagram[17] >> 7);
        if index >= count {
            self.invalid += 1;
            return Vec::new();
        }
        if self.last.is_some_and(|last| !after(unit, last)) {
            self.duplicates += 1;
            return Vec::new();
        }

        let partial = self.partial.entry(unit).or_insert_with(|| Partial {
            count,
            fragments: HashMap::new(),
        });
        if partial.count != count {
            self.invalid += 1;
            self.partial.remove(&unit);
            return Vec::new();
        }
        if partial.fragments.contains_key(&index) {
            self.duplicates += 1;
            return Vec::new();
        }
        let seq = u16::from_le_bytes([datagram[4], datagram[5]]);
        partial
            .fragments
            .insert(index, (datagram[HEADER_LEN..].to_vec(), seq));
        if partial.fragments.len() == count {
            let mut partial = self.partial.remove(&unit).expect("just inserted");
            let mut data = Vec::new();
            let mut last_seq = 0;
            for i in 0..count {
                let (bytes, seq) = partial.fragments.remove(&i).expect("all fragments present");
                data.extend_from_slice(&bytes);
                last_seq = seq;
            }
            self.complete.insert(unit, Unit { data, last_seq });
        }

        let mut ready = Vec::new();
        // The first unit seen sets where the order starts
        if self.last.is_none()
            && let Some(first) = self.complete.remove(&unit)
        {
            ready.push(first);
            self.last = Some(unit);
        }
        self.drain(&mut ready);

        if let Some(last) = self.last
            && self.complete.len() >= GIVE_UP_AFTER
            && let Some(next) = self
                .complete
                .keys()
                .copied()
                .filter(|&n| after(n, last))
                .min_by_key(|&n| n.wrapping_sub(last))
        {
            self.skipped += u64::from(next.wrapping_sub(last) - 1);
            ready.extend(self.complete.remove(&next));
            self.last = Some(next);
            self.drain(&mut ready);
        }

        if let Some(last) = self.last {
            self.partial.retain(|&n, _| after(n, last));
        }
        ready
    }

    fn drain(&mut self, ready: &mut Vec<Unit>) {
        while let Some(last) = self.last {
            let next = last.wrapping_add(1);
            let Some(unit) = self.complete.remove(&next) else {
                break;
            };
            ready.push(unit);
            self.last = Some(next);
        }
    }
}

#[cfg(test)]
#[path = "transport.test.rs"]
mod tests;
