use serde::Serialize;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    H264,
    H265,
}

/// Slice headers are weaker evidence than a parameter set, so it takes a
/// run of them
const SLICES_NEEDED: u32 = 8;

/// Works out from the bytes which codec a DJI link carries. The goggles send
/// H.264; an RC-N remote sends H.265, and sends its parameter sets only once,
/// right when the cable goes in, so waiting for one isn't enough: slice
/// headers count too. The two codecs' header bytes don't overlap, read as
/// the other codec they'd be types no encoder emits
#[derive(Default)]
pub struct Sniffer {
    h264_slices: u32,
    h265_slices: u32,
}

impl Sniffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The codec, once a chunk settles it
    pub fn feed(&mut self, chunk: &[u8]) -> Option<Codec> {
        let mut i = 0;
        while i + 4 < chunk.len() {
            let start = chunk[i] == 0
                && chunk[i + 1] == 0
                && (chunk[i + 2] == 1 || (chunk[i + 2] == 0 && chunk[i + 3] == 1));
            if !start {
                i += 1;
                continue;
            }
            let header = if chunk[i + 2] == 1 { i + 3 } else { i + 4 };
            let rest = &chunk[header + 1..];
            match chunk[header] {
                0x67 if plausible_h264_sps(rest) => return Some(Codec::H264),
                0x68 if !rest.is_empty() && !plausible_h265_header(rest) => {
                    return Some(Codec::H264);
                }
                // VPS, SPS, PPS
                0x40 | 0x42 | 0x44 if plausible_h265_header(rest) => return Some(Codec::H265),
                // IDR_N_LP, CRA, TRAIL_R, AUD, suffix SEI
                0x28 | 0x26 | 0x02 | 0x46 | 0x50 if plausible_h265_header(rest) => {
                    self.h265_slices += 1;
                    if self.h265_slices >= SLICES_NEEDED {
                        return Some(Codec::H265);
                    }
                }
                0x65 | 0x41 | 0x61 if plausible_h264_slice(chunk[header], rest) => {
                    self.h264_slices += 1;
                    if self.h264_slices >= SLICES_NEEDED {
                        return Some(Codec::H264);
                    }
                }
                _ => {}
            }
            i = header + 1;
        }
        None
    }
}

/// `0x67` is only an SPS when a known profile, clear reserved bits and a
/// real level follow. Without this, slice bytes that happen to read
/// `00 00 01 67` mid-stream pass for one
fn plausible_h264_sps(rest: &[u8]) -> bool {
    const PROFILES: [u8; 16] = [
        66, 77, 88, 100, 110, 122, 244, 44, 83, 86, 118, 128, 134, 135, 138, 139,
    ];
    const LEVELS: [u8; 20] = [
        9, 10, 11, 12, 13, 20, 21, 22, 30, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62,
    ];
    rest.len() >= 3
        && PROFILES.contains(&rest[0])
        && rest[1] & 0x03 == 0
        && LEVELS.contains(&rest[2])
}

/// The second byte of an H.265 NAL header: layer 0 and a temporal id of 1-7
fn plausible_h265_header(rest: &[u8]) -> bool {
    matches!(rest.first(), Some(b) if b & 0xf8 == 0 && b & 0x07 != 0)
}

fn plausible_h264_slice(header: u8, rest: &[u8]) -> bool {
    let Some(&b) = rest.first() else {
        return false;
    };
    if plausible_h265_header(rest) {
        return false;
    }
    // An IDR slice starts at the first macroblock
    if header == 0x65 {
        b & 0x80 != 0
    } else {
        b & 0x80 != 0 || (rest.len() >= 4 && b != 0)
    }
}

#[cfg(test)]
#[path = "codec.test.rs"]
mod tests;
