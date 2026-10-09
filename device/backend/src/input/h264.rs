pub const IDR: u8 = 5;
pub const SPS: u8 = 7;
pub const AUD: u8 = 9;

pub struct Nal {
    /// Offset of the start code prefix
    pub start: usize,
    /// Offset of the header byte, just past the start code
    pub header: usize,
    /// nal_unit_type, the low 5 bits of the header byte
    pub kind: u8,
}

impl Nal {
    /// A coded slice rather than metadata
    pub fn is_vcl(&self) -> bool {
        (1..=5).contains(&self.kind)
    }

    pub fn is_sync(&self) -> bool {
        self.kind == IDR || self.kind == SPS
    }
}

/// Both the three and four-byte forms appear in the wild, sometimes in one
/// stream
pub fn find_start_code(buf: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 {
            if buf[i + 2] == 1 {
                return Some((i, 3));
            }
            if i + 4 <= buf.len() && buf[i + 2] == 0 && buf[i + 3] == 1 {
                return Some((i, 4));
            }
        }
        i += 1;
    }
    None
}

/// Stops at a start code whose header byte hasn't arrived yet, leaving a NAL
/// split across chunk boundaries for a later call
pub fn nals(buf: &[u8]) -> impl Iterator<Item = Nal> + '_ {
    let mut from = 0;
    std::iter::from_fn(move || {
        let (start, len) = find_start_code(buf, from)?;
        let header = start + len;
        let &byte = buf.get(header)?;
        from = header;
        Some(Nal {
            start,
            header,
            kind: byte & 0x1F,
        })
    })
}

pub fn has_keyframe(buf: &[u8]) -> bool {
    nals(buf).any(|nal| nal.is_sync())
}

/// Cuts the goggles' byte stream into access units. DJI puts the delimiter
/// at the end of each picture, so a picture is complete the moment it arrives
pub struct AccessUnitAssembler {
    buf: Vec<u8>,
}

impl AccessUnitAssembler {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(256 * 1024),
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// An AU ends where the next one starts: at a delimiter, or at the first
    /// slice after we have already seen one
    pub fn next_au(&mut self) -> Option<Vec<u8>> {
        loop {
            let (first, _) = find_start_code(&self.buf, 0)?;
            let mut have_vcl = false;
            let mut only_delimiters = true;
            let mut next = None;

            for nal in nals(&self.buf) {
                if (nal.kind == AUD || (nal.is_vcl() && have_vcl)) && nal.start > first {
                    next = Some(nal.start);
                    break;
                }
                have_vcl |= nal.is_vcl();
                only_delimiters &= nal.kind == AUD;
            }

            let next = next?;
            let au = self.buf[first..next].to_vec();
            self.buf.drain(0..next);
            // Two delimiters in a row, e.g. one closing a picture and one
            // opening the next. Passed on as a picture it would count as a
            // frame and throw off every consumer's timing
            if only_delimiters {
                continue;
            }
            return Some(au);
        }
    }
}

/// `au` without a delimiter at its start
pub fn strip_leading_aud(au: &[u8]) -> &[u8] {
    let mut nals = nals(au);
    match (nals.next(), nals.next()) {
        (Some(first), Some(second)) if first.kind == AUD => &au[second.start..],
        _ => au,
    }
}

#[cfg(test)]
#[path = "h264.test.rs"]
mod tests;
