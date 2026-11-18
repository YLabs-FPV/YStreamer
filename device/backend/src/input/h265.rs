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

/// Where the first picture a decoder can start from begins, at its start
/// code, along with any parameter sets right in front of it
pub fn irap_start(data: &[u8]) -> Option<usize> {
    let mut from = 0;
    // The start of a run of parameter sets that may lead up to the picture
    let mut params_from = None;
    while let Some((start, len)) = find_start_code(data, from) {
        let header = start + len;
        let k = kind(*data.get(header)?);
        if is_irap(k) {
            return Some(params_from.unwrap_or(start));
        }
        params_from = match k {
            VPS | SPS | PPS => params_from.or(Some(start)),
            _ => None,
        };
        from = header;
    }
    None
}

/// The stream's latest VPS, SPS and PPS. The drone behind an RC-N remote
/// sends them only with a keyframe: when it links to the remote, or when
/// asked. So they're kept for every decoder started later
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

#[cfg(test)]
#[path = "h265.test.rs"]
mod tests;
