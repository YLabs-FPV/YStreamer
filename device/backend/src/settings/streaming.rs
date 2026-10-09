use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RtspSettings {
    pub enabled: bool,
    pub port: u16,
    pub path: String,
    pub tcp_only: bool,
    pub auth: bool,
    pub username: String,
    pub password: String,
}

impl Default for RtspSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 554,
            path: "/stream".into(),
            tcp_only: false,
            auth: false,
            username: "admin".into(),
            password: String::new(),
        }
    }
}

impl RtspSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.port < 1024 && self.port != 554 {
            return Err("Port must be 554 or 1024-65535".into());
        }
        if [8080].contains(&self.port) {
            return Err("Port 8080 is used by the web interface".into());
        }
        let p = &self.path;
        if !p.starts_with('/')
            || p.len() < 2
            || !p[1..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
        {
            return Err(
                "Path must start with '/' and use letters, digits, '-', '_', '.' or '/'".into(),
            );
        }
        if self.auth && (self.username.is_empty() || self.password.is_empty()) {
            return Err("Username and password are required when authentication is on".into());
        }
        if self.username.contains(':') {
            return Err("Username can't contain ':'".into());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SrtMode {
    #[default]
    Listener,
    Caller,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SrtSettings {
    pub enabled: bool,
    pub mode: SrtMode,
    /// Caller only
    pub host: String,
    pub port: u16,
    pub latency_ms: u32,
    /// Empty = unencrypted
    pub passphrase: String,
    /// Caller only; some servers route or authenticate by it
    pub stream_id: String,
}

impl Default for SrtSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: SrtMode::Listener,
            host: String::new(),
            port: 9000,
            latency_ms: 200,
            passphrase: String::new(),
            stream_id: String::new(),
        }
    }
}

impl SrtSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.port == 0 || (self.mode == SrtMode::Listener && self.port < 1024) {
            return Err("SRT port must be 1024-65535".into());
        }
        if self.mode == SrtMode::Caller {
            let h = self.host.trim();
            if h.is_empty() {
                return Err("SRT needs the receiver's address".into());
            }
            if h.contains(|c: char| c.is_whitespace() || "/?#@".contains(c)) {
                return Err(
                    "SRT address should be a hostname or IP, without srt:// or a port".into(),
                );
            }
        }
        if !(20..=8000).contains(&self.latency_ms) {
            return Err("SRT latency must be 20-8000 ms".into());
        }
        let n = self.passphrase.chars().count();
        if n != 0 && !(10..=79).contains(&n) {
            return Err("SRT passphrase must be 10-79 characters".into());
        }
        if self.stream_id.len() > 512 {
            return Err("SRT stream ID is too long".into());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RtmpSettings {
    pub enabled: bool,
    /// Server URL without the key, e.g. rtmp://a.rtmp.youtube.com/live2
    pub url: String,
    pub key: String,
    /// Some platforms refuse or flag streams without audio
    pub silent_audio: bool,
}

impl Default for RtmpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            key: String::new(),
            silent_audio: true,
        }
    }
}

impl RtmpSettings {
    /// What rtmp2sink connects to: the URL with the key as the last segment
    pub fn location(&self) -> String {
        let url = self.url.trim().trim_end_matches('/');
        match self.key.trim() {
            "" => url.to_string(),
            key => format!("{url}/{key}"),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        let url = self.url.trim();
        let rest = url
            .strip_prefix("rtmp://")
            .or_else(|| url.strip_prefix("rtmps://"))
            .ok_or("RTMP server URL must start with rtmp:// or rtmps://")?;
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        if host.is_empty() {
            return Err("RTMP server URL is missing the server name".into());
        }
        if !rest.contains('/') {
            return Err("RTMP server URL is missing the application, e.g. /live2".into());
        }
        if url.contains(char::is_whitespace) || self.key.trim().contains(char::is_whitespace) {
            return Err("RTMP URL and stream key can't contain spaces".into());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UdpFormat {
    /// RTP H.264, what QGroundControl and Mission Planner expect on 5600
    #[default]
    Rtp,
    /// MPEG-TS, 7 packets per datagram: VLC, ffmpeg, OBS, multicast
    Mpegts,
}

const MAX_UDP_DESTINATIONS: usize = 8;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct UdpSettings {
    pub enabled: bool,
    pub format: UdpFormat,
    /// `host:port`, comma or space separated; multicast addresses work too
    pub destinations: String,
}

impl Default for UdpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            format: UdpFormat::Rtp,
            destinations: String::new(),
        }
    }
}

impl UdpSettings {
    pub fn destination_list(&self) -> Vec<String> {
        self.destinations
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        let list = self.destination_list();
        if list.is_empty() {
            return Err("UDP needs at least one destination, e.g. 192.168.1.20:5600".into());
        }
        if list.len() > MAX_UDP_DESTINATIONS {
            return Err(format!(
                "UDP can send to at most {MAX_UDP_DESTINATIONS} destinations"
            ));
        }
        for d in &list {
            let (host, port) = d
                .rsplit_once(':')
                .ok_or_else(|| format!("UDP destination \"{d}\" needs a port, e.g. {d}:5600"))?;
            if host.is_empty() || host.contains(['/', '@']) {
                return Err(format!("UDP destination \"{d}\" needs a hostname or IP"));
            }
            if !port.parse::<u16>().is_ok_and(|p| p > 0) {
                return Err(format!("UDP destination \"{d}\" has an invalid port"));
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ViewerTransport {
    #[default]
    Websocket,
    Webrtc,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct ViewerSettings {
    pub transport: ViewerTransport,
    pub max_viewers: u8,
}

#[cfg(test)]
#[path = "streaming.test.rs"]
mod tests;
