use super::h264::find_start_code;

pub const VPS: u8 = 32;
pub const SPS: u8 = 33;
pub const PPS: u8 = 34;

/// Longest stretch kept while waiting for the end of a NAL. Parameter sets
/// are tens of bytes; anything longer is a slice nobody here needs whole
const MAX_TAIL: usize = 64 * 1024;

/// nal_unit_type, from the first header byte
pub fn kind(header: u8) -> u8 {
    (header >> 1) & 0x3F
}

/// A picture a decoder can start from
pub fn is_irap(kind: u8) -> bool {
    (16..=23).contains(&kind)
}

/// Whether `data` holds the start of a picture a decoder can start from
pub fn has_irap(data: &[u8]) -> bool {
    let mut from = 0;
    while let Some((start, len)) = find_start_code(data, from) {
        let header = start + len;
        match data.get(header) {
            Some(&b) if is_irap(kind(b)) => return true,
            Some(_) => from = header,
            None => return false,
        }
    }
    false
}

/// The stream's latest VPS, SPS and PPS. An RC-N remote sends them once,
/// when the cable goes in, so they're kept for every decoder started later
#[derive(Default)]
pub struct ParamSets {
    vps: Option<Vec<u8>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    /// From the last start code on: a NAL whose end hasn't arrived
    tail: Vec<u8>,
    /// How much of `tail` has been searched for the next start code
    searched: usize,
}

impl ParamSets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, data: &[u8]) {
        self.tail.extend_from_slice(data);
        let mut nal_start = match find_start_code(&self.tail, 0) {
            Some((start, _)) => start,
            None => {
                self.searched = 0;
                self.trim();
                return;
            }
        };
        loop {
            let (_, len) = find_start_code(&self.tail, nal_start).expect("start code at nal_start");
            // A start code can straddle the end of what was searched last time
            let from = (nal_start + len).max(self.searched.saturating_sub(3));
            let Some((next, _)) = find_start_code(&self.tail, from) else {
                self.searched = self.tail.len();
                break;
            };
            let nal = &self.tail[nal_start..next];
            if let Some(&header) = nal.get(len) {
                let slot = match kind(header) {
                    VPS => Some(&mut self.vps),
                    SPS => Some(&mut self.sps),
                    PPS => Some(&mut self.pps),
                    _ => None,
                };
                if let Some(slot) = slot {
                    *slot = Some(nal.to_vec());
                }
            }
            nal_start = next;
        }
        self.tail.drain(..nal_start);
        self.searched = self.searched.saturating_sub(nal_start);
        self.trim();
    }

    /// The latest SPS, start code included
    pub fn sps(&self) -> Option<&[u8]> {
        self.sps.as_deref()
    }

    /// All three, ready to put in front of a stream, once all have been seen
    pub fn annex_b(&self) -> Option<Vec<u8>> {
        let (vps, sps, pps) = (self.vps.as_ref()?, self.sps.as_ref()?, self.pps.as_ref()?);
        Some([vps.as_slice(), sps, pps].concat())
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Keeps a few bytes, enough for a start code split across chunks
    fn trim(&mut self) {
        if self.tail.len() > MAX_TAIL {
            let drop = self.tail.len() - 3;
            self.tail.drain(..drop);
            self.searched = 0;
        }
    }
}

/// The picture size an SPS describes, after cropping. Read up to the
/// conformance window and no further, as nothing else here needs the rest
pub fn picture_size(sps: &[u8]) -> Option<(u32, u32)> {
    let nal = match find_start_code(sps, 0) {
        Some((0, len)) => &sps[len..],
        _ => sps,
    };
    let mut r = Bits::new(nal.get(2..)?);
    r.skip(4); // sps_video_parameter_set_id
    let sub_layers = r.read(3)? as usize;
    r.skip(1);
    // profile_tier_level: the general part, then what each sub-layer adds
    r.skip(96);
    let mut present = [(false, false); 8];
    for p in present.iter_mut().take(sub_layers) {
        *p = (r.read(1)? == 1, r.read(1)? == 1);
    }
    if sub_layers > 0 {
        r.skip(2 * (8 - sub_layers));
    }
    for &(profile, level) in &present[..sub_layers] {
        r.skip(if profile { 88 } else { 0 } + if level { 8 } else { 0 });
    }
    r.ue()?; // sps_seq_parameter_set_id
    let chroma = r.ue()?;
    if chroma == 3 {
        r.skip(1);
    }
    let (mut w, mut h) = (r.ue()?, r.ue()?);
    if r.read(1)? == 1 {
        // In chroma samples: two luma samples across for 4:2:0 and 4:2:2,
        // two down for 4:2:0
        let sub_w = if chroma == 1 || chroma == 2 { 2 } else { 1 };
        let sub_h = if chroma == 1 { 2 } else { 1 };
        let (left, right, top, bottom) = (r.ue()?, r.ue()?, r.ue()?, r.ue()?);
        w = w.checked_sub(sub_w * (left + right))?;
        h = h.checked_sub(sub_h * (top + bottom))?;
    }
    (w > 0 && h > 0).then_some((w, h))
}

/// Reads an RBSP, dropping the emulation prevention bytes on the way
struct Bits<'a> {
    data: &'a [u8],
    byte: usize,
    bit: u8,
    zeros: u8,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte: 0,
            bit: 0,
            zeros: 0,
        }
    }

    fn bit(&mut self) -> Option<u32> {
        if self.bit == 0 {
            // 00 00 03: the 03 is only there to keep start codes out
            if self.zeros >= 2 && self.data.get(self.byte) == Some(&3) {
                self.byte += 1;
                self.zeros = 0;
            }
            let &b = self.data.get(self.byte)?;
            self.zeros = if b == 0 { self.zeros + 1 } else { 0 };
        }
        let b = (self.data[self.byte] >> (7 - self.bit)) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.byte += 1;
        }
        Some(b as u32)
    }

    fn read(&mut self, n: u32) -> Option<u32> {
        (0..n).try_fold(0, |v, _| Some((v << 1) | self.bit()?))
    }

    fn skip(&mut self, n: usize) {
        for _ in 0..n {
            self.bit();
        }
    }

    /// Exp-Golomb, as sizes and ids are coded
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1 << zeros) - 1 + self.read(zeros)?)
    }
}

#[cfg(test)]
#[path = "h265.test.rs"]
mod tests;
