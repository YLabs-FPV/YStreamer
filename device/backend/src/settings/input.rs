use std::net::IpAddr;

use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    // G2, Integra, G3, N3: DUML
    DjiFpv,
    // USB cameras, analog receivers
    Uvc,
    /// DJI Goggles V1 and V2: video only
    DjiFpvLegacy,
    /// Pushed to us by OBS, a phone app or DJI Fly's live streaming
    Rtmp,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct InputSettings {
    pub mode: InputMode,
    pub uvc: UvcSettings,
    pub rtmp: RtmpInputSettings,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct UvcSettings {
    /// /dev/v4l/by-id path, which survives reboots unlike /dev/videoN.
    /// Empty: the first USB video device found
    pub device: String,
    /// `WxH@fps`, empty for the best the device offers up to 1080p30
    pub mode: String,
    /// The device's video gets encoded, unlike the goggles' H.264
    pub bitrate_kbps: u32,
    pub keyframe_secs: u32,
}

impl Default for UvcSettings {
    fn default() -> Self {
        Self {
            device: String::new(),
            mode: String::new(),
            bitrate_kbps: 6000,
            keyframe_secs: 1,
        }
    }
}

impl UvcSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(500..=20_000).contains(&self.bitrate_kbps) {
            return Err("USB camera bitrate must be 500-20000 kbit/s".into());
        }
        if !(1..=10).contains(&self.keyframe_secs) {
            return Err("USB camera keyframe interval must be 1-10 seconds".into());
        }
        if !self.mode.is_empty() && parse_uvc_mode(&self.mode).is_none() {
            return Err(format!(
                "USB camera mode \"{}\" isn't like 1280x720@30",
                self.mode
            ));
        }
        Ok(())
    }
}

/// The usual RTMP port, what apps assume when the URL has none
pub const RTMP_INPUT_PORT: u16 = 1935;
/// As long as the made-up keys, so a key can't be guessed in a few tries
pub const RTMP_MIN_KEY_LEN: usize = 10;
const MAX_ALLOWED_PUBLISHERS: usize = 8;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RtmpInputSettings {
    pub port: u16,
    /// What the app must stream to. Empty until RTMP is first picked,
    /// when a random one is made
    pub key: String,
    /// Addresses allowed to stream, comma or space separated. Empty:
    /// anyone on the network who has the key
    pub allowed_from: String,
}

impl Default for RtmpInputSettings {
    fn default() -> Self {
        Self {
            port: RTMP_INPUT_PORT,
            key: String::new(),
            allowed_from: String::new(),
        }
    }
}

impl RtmpInputSettings {
    /// Ten letters and digits, leaving out the ones that look alike, so it
    /// can be typed into a phone
    pub fn new_key() -> String {
        const CHARS: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
        let mut rng = rand::rng();
        (0..RTMP_MIN_KEY_LEN)
            .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
            .collect()
    }

    pub fn allowed_list(&self) -> Result<Vec<IpAddr>, String> {
        self.allowed_from
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<IpAddr>()
                    .map_err(|_| format!("\"{s}\" isn't an IP address, e.g. 192.168.1.20"))
            })
            .collect()
    }

    /// Whether an app at `ip` may stream
    pub fn allows(&self, ip: IpAddr) -> bool {
        let list = self.allowed_list().unwrap_or_default();
        list.is_empty() || list.iter().any(|a| a.to_canonical() == ip.to_canonical())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.port < 1024 && self.port != RTMP_INPUT_PORT {
            return Err(format!(
                "RTMP input port must be {RTMP_INPUT_PORT} or 1024-65535"
            ));
        }
        if self.port == 8080 {
            return Err("Port 8080 is used by the web interface".into());
        }
        if !self.key.is_empty() && !(RTMP_MIN_KEY_LEN..=64).contains(&self.key.len()) {
            return Err(format!(
                "RTMP stream key must be {RTMP_MIN_KEY_LEN}-64 characters; New makes one"
            ));
        }
        if !self
            .key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        {
            return Err("RTMP stream key can only use letters, digits, '-', '_' and '.'".into());
        }
        let allowed = self.allowed_list()?;
        if allowed.len() > MAX_ALLOWED_PUBLISHERS {
            return Err(format!(
                "RTMP input can allow at most {MAX_ALLOWED_PUBLISHERS} addresses"
            ));
        }
        Ok(())
    }
}

/// `1280x720@30` -> (1280, 720, 30)
pub fn parse_uvc_mode(mode: &str) -> Option<(u32, u32, u32)> {
    let (size, fps) = mode.split_once('@')?;
    let (w, h) = size.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, fps.parse().ok()?))
}

#[cfg(test)]
#[path = "input.test.rs"]
mod tests;
