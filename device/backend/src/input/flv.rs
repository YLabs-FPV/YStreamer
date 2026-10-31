//! RTMP carries video as FLV video tags: H.264 in length-prefixed NAL units,
//! with the SPS and PPS sent once up front. Everything after the input wants
//! the goggles' form instead: start codes, parameter sets before every
//! keyframe and a delimiter closing each picture.

use crate::input::h264;

const START_CODE: [u8; 4] = [0, 0, 0, 1];
/// Appended to every picture, as the goggles do, so the HDMI and browser
/// paths know a picture is complete without waiting for the next one
const AUD: [u8; 6] = [0, 0, 0, 1, 0x09, 0xf0];

const CODEC_AVC: u8 = 7;
/// Not in the FLV spec, but what Chinese encoders and some servers use
const CODEC_HEVC: u8 = 12;
/// "Video info / command frame": no picture in it
const FRAME_COMMAND: u8 = 5;

/// Enhanced RTMP packet types
const EX_SEQUENCE_START: u8 = 0;
const EX_CODED_FRAMES: u8 = 1;
const EX_CODED_FRAMES_X: u8 = 3;

#[derive(Debug, PartialEq)]
pub enum TagError {
    /// Video this device can't pass on, named for the user
    Unsupported(&'static str),
    Malformed(&'static str),
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagError::Unsupported(codec) => {
                write!(f, "The stream is {codec}; set the app or encoder to H.264")
            }
            TagError::Malformed(what) => write!(f, "Unreadable video from the app: {what}"),
        }
    }
}

enum Packet<'a> {
    Config(&'a [u8]),
    /// NAL units, and whether pictures are shown in another order than sent
    Frames(&'a [u8], bool),
    Nothing,
}

/// Turns one publisher's video tags into H.264 access units
#[derive(Default)]
pub struct AvcTags {
    /// Bytes in each NAL unit's length prefix, known from the config record
    nal_length: usize,
    /// SPS and PPS with start codes, for the front of each keyframe
    params: Vec<u8>,
    reorders: bool,
}

impl AvcTags {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stream has B-frames. Each picture still goes out as it arrives,
    /// but decoders hold some back, which adds delay
    pub fn reorders(&self) -> bool {
        self.reorders
    }

    /// One tag's body in, the picture it carries out
    pub fn push(&mut self, tag: &[u8]) -> Result<Option<Vec<u8>>, TagError> {
        match split(tag)? {
            Packet::Config(record) => {
                self.config(record)?;
                Ok(None)
            }
            Packet::Frames(nals, reorders) => {
                self.reorders |= reorders;
                self.picture(nals)
            }
            Packet::Nothing => Ok(None),
        }
    }

    /// An AVCDecoderConfigurationRecord
    fn config(&mut self, record: &[u8]) -> Result<(), TagError> {
        const BAD: TagError = TagError::Malformed("short H.264 configuration");
        if record.len() < 6 {
            return Err(BAD);
        }
        let nal_length = (record[4] & 3) as usize + 1;
        let mut params = Vec::new();
        let mut at = 5;
        // SPS count in the low five bits, then the PPS count in a whole byte
        for mask in [0x1f, 0xff] {
            let count = *record.get(at).ok_or(BAD)? & mask;
            at += 1;
            for _ in 0..count {
                let len = u16::from_be_bytes([
                    *record.get(at).ok_or(BAD)?,
                    *record.get(at + 1).ok_or(BAD)?,
                ]) as usize;
                at += 2;
                let nal = record.get(at..at + len).ok_or(BAD)?;
                at += len;
                params.extend_from_slice(&START_CODE);
                params.extend_from_slice(nal);
            }
        }
        self.nal_length = nal_length;
        self.params = params;
        Ok(())
    }

    fn picture(&self, mut data: &[u8]) -> Result<Option<Vec<u8>>, TagError> {
        // Pictures before the configuration can't be cut up
        if self.nal_length == 0 {
            return Ok(None);
        }
        let mut au = Vec::with_capacity(data.len() + self.params.len() + 16);
        let (mut keyframe, mut has_sps) = (false, false);
        while !data.is_empty() {
            let Some(prefix) = data.get(..self.nal_length) else {
                return Err(TagError::Malformed("cut-off NAL unit"));
            };
            let len = prefix.iter().fold(0usize, |n, &b| (n << 8) | b as usize);
            let Some(nal) = data.get(self.nal_length..self.nal_length + len) else {
                return Err(TagError::Malformed("cut-off NAL unit"));
            };
            data = &data[self.nal_length + len..];
            let Some(&header) = nal.first() else {
                continue;
            };
            match header & 0x1f {
                // Ours goes at the end
                h264::AUD => continue,
                h264::SPS => has_sps = true,
                h264::IDR => keyframe = true,
                _ => {}
            }
            au.extend_from_slice(&START_CODE);
            au.extend_from_slice(nal);
        }
        if au.is_empty() {
            return Ok(None);
        }
        // Outputs and players that join later start at an SPS
        if keyframe && !has_sps {
            au.splice(0..0, self.params.iter().copied());
        }
        au.extend_from_slice(&AUD);
        Ok(Some(au))
    }
}

/// What a tag holds, from its header
fn split(tag: &[u8]) -> Result<Packet<'_>, TagError> {
    let &first = tag.first().ok_or(TagError::Malformed("empty video tag"))?;
    // Enhanced RTMP sets the top bit and names the codec by FourCC
    if first & 0x80 != 0 {
        let frame_type = (first >> 4) & 0x07;
        let packet_type = first & 0x0f;
        let fourcc = tag
            .get(1..5)
            .ok_or(TagError::Malformed("short video tag"))?;
        match fourcc {
            b"avc1" => {}
            b"hvc1" => return Err(TagError::Unsupported("H.265")),
            b"av01" => return Err(TagError::Unsupported("AV1")),
            b"vp09" => return Err(TagError::Unsupported("VP9")),
            _ => {
                return Err(TagError::Unsupported(
                    "in a format this device doesn't know",
                ));
            }
        }
        if frame_type == FRAME_COMMAND {
            return Ok(Packet::Nothing);
        }
        return match packet_type {
            EX_SEQUENCE_START => Ok(Packet::Config(&tag[5..])),
            EX_CODED_FRAMES => {
                let cts = tag
                    .get(5..8)
                    .ok_or(TagError::Malformed("short video tag"))?;
                Ok(Packet::Frames(&tag[8..], cts != [0, 0, 0]))
            }
            EX_CODED_FRAMES_X => Ok(Packet::Frames(&tag[5..], false)),
            // End of sequence, metadata, several tracks: nothing to show
            _ => Ok(Packet::Nothing),
        };
    }

    match first & 0x0f {
        CODEC_AVC => {}
        CODEC_HEVC => return Err(TagError::Unsupported("H.265")),
        2 => return Err(TagError::Unsupported("Sorenson H.263")),
        4 | 5 => return Err(TagError::Unsupported("VP6")),
        _ => {
            return Err(TagError::Unsupported(
                "in a format this device doesn't know",
            ));
        }
    }
    if first >> 4 == FRAME_COMMAND {
        return Ok(Packet::Nothing);
    }
    let header = tag
        .get(1..5)
        .ok_or(TagError::Malformed("short video tag"))?;
    match header[0] {
        0 => Ok(Packet::Config(&tag[5..])),
        1 => Ok(Packet::Frames(&tag[5..], header[1..] != [0, 0, 0])),
        // End of sequence
        _ => Ok(Packet::Nothing),
    }
}

#[cfg(test)]
#[path = "flv.test.rs"]
mod tests;
