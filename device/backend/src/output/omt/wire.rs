//! OMT's framing, as its reference library (libomtnet) writes it. Little
//! endian. Every frame starts with a 16-byte header; video adds 32 bytes
//! describing the picture, then the VMX1 data. Receivers send metadata
//! frames, XML text, to say what they want

pub const HEADER_LEN: usize = 16;
const VIDEO_HEADER_LEN: usize = 32;
const VERSION: u8 = 1;
pub const METADATA: u8 = 1;
pub const VIDEO: u8 = 2;
const VMX1: u32 = u32::from_le_bytes(*b"VMX1");
const BT709: i32 = 709;
/// What receivers send to subscribe, compared whole as libomtnet does.
/// One connection takes video and metadata, a second audio
const SUBSCRIBE_VIDEO: &str = r#"<OMTSubscribe Video="true" />"#;
const SUBSCRIBE_AUDIO: &str = r#"<OMTSubscribe Audio="true" />"#;
const SUBSCRIBE_METADATA: &str = r#"<OMTSubscribe Metadata="true" />"#;
/// Receivers only send short XML; anything larger means the connection is
/// out of step, or isn't a receiver
pub const MAX_INCOMING: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: u8,
    pub timestamp: i64,
    pub data_len: usize,
}

pub fn parse_header(b: &[u8; HEADER_LEN]) -> Option<Header> {
    if b[0] != VERSION {
        return None;
    }
    let data_len = i32::from_le_bytes([b[12], b[13], b[14], b[15]]);
    Some(Header {
        kind: b[1],
        timestamp: i64::from_le_bytes(b[2..10].try_into().unwrap()),
        data_len: usize::try_from(data_len).ok()?,
    })
}

fn header(out: &mut Vec<u8>, kind: u8, timestamp: i64, data_len: usize) {
    out.push(VERSION);
    out.push(kind);
    out.extend_from_slice(&timestamp.to_le_bytes());
    // Per-frame metadata, which we don't attach
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(data_len as i32).to_le_bytes());
}

/// `timestamp` is in 100 ns units
pub fn video(
    vmx: &[u8],
    width: u32,
    height: u32,
    frame_rate: (i32, i32),
    timestamp: i64,
) -> Vec<u8> {
    let data_len = VIDEO_HEADER_LEN + vmx.len();
    let mut out = Vec::with_capacity(HEADER_LEN + data_len);
    header(&mut out, VIDEO, timestamp, data_len);
    out.extend_from_slice(&VMX1.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&frame_rate.0.to_le_bytes());
    out.extend_from_slice(&frame_rate.1.to_le_bytes());
    out.extend_from_slice(&(width as f32 / height.max(1) as f32).to_le_bytes());
    // Progressive, no alpha, full frame rather than a preview
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&BT709.to_le_bytes());
    out.extend_from_slice(vmx);
    out
}

pub fn metadata(xml: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + xml.len());
    header(&mut out, METADATA, 0, xml.len());
    out.extend_from_slice(xml.as_bytes());
    out
}

/// Who's sending, shown by receivers that display it
pub fn sender_info(version: &str) -> String {
    format!(r#"<OMTInfo ProductName="YStreamer" Manufacturer="YStreamer" Version="{version}" />"#)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subscription {
    Video,
    Audio,
    Metadata,
}

/// What a receiver's metadata subscribes to, if it's a subscription
pub fn subscription(xml: &[u8]) -> Option<Subscription> {
    let text = String::from_utf8_lossy(xml);
    match text.trim_matches(|c: char| c == '\0' || c.is_whitespace()) {
        SUBSCRIBE_VIDEO => Some(Subscription::Video),
        SUBSCRIBE_AUDIO => Some(Subscription::Audio),
        SUBSCRIBE_METADATA => Some(Subscription::Metadata),
        _ => None,
    }
}

#[cfg(test)]
#[path = "wire.test.rs"]
mod tests;
