mod device;
mod display;
mod ethernet;
mod input;
mod recording;
mod streaming;
mod wifi;

pub use device::*;
pub use display::*;
pub use ethernet::*;
pub use input::*;
pub use recording::*;
pub use streaming::*;
pub use wifi::*;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

pub static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// The settings file's format, stored in it as `version`. Raise it together
/// with a step in `migrate` whenever a release can't read the old meaning
/// of a field through serde's defaults alone
const VERSION: u64 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub input: InputSettings,
    pub device: DeviceSettings,
    pub wifi: WifiSettings,
    pub ethernet: EthernetSettings,
    pub rtsp: RtspSettings,
    pub srt: SrtSettings,
    pub rtmp: RtmpSettings,
    pub udp: UdpSettings,
    pub recording: RecordingSettings,
    #[serde(alias = "webrtc")]
    pub viewer: ViewerSettings,
    pub display: DisplaySettings,
    pub splash: SplashSettings,
    pub overlay: OverlaySettings,
    pub advanced: AdvancedSettings,
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

/// The ready-made SD image marks itself with a file next to the settings
pub fn is_image() -> bool {
    static IMAGE: OnceLock<bool> = OnceLock::new();
    *IMAGE.get_or_init(|| config_path().with_file_name("image").exists())
}

impl Settings {
    /// What these settings mean on this install: on the image nothing else
    /// sets up Ethernet, so "System" becomes DHCP. WiFi's "no network
    /// joined" is a real choice everywhere, so it stays
    pub fn for_install(mut self) -> Self {
        if is_image() && self.ethernet.mode == EthernetMode::System {
            self.ethernet.mode = EthernetMode::Auto;
        }
        self
    }
}

impl SettingsStore {
    pub fn load() -> Self {
        let path = config_path();
        crate::system::reset::at_startup(&path);
        let mut fresh = false;
        let mut current = match fs::read_to_string(&path) {
            Ok(text) => match parse(&text) {
                Ok((s, version)) => {
                    eprintln!("[settings] loaded {}", path.display());
                    if version != VERSION {
                        keep_copy(&path, version);
                    }
                    s
                }
                Err(e) => {
                    eprintln!(
                        "[settings] {} is invalid ({e}); using defaults",
                        path.display()
                    );
                    Settings::default()
                }
            },
            Err(_) => {
                eprintln!(
                    "[settings] no settings at {}; using defaults",
                    path.display()
                );
                fresh = true;
                Settings::default()
            }
        }
        .for_install();
        if is_image() {
            eprintln!("[settings] running from the YStreamer image");
            // A fresh image has no other way in until someone sets one up
            if fresh {
                current.wifi.mode = WifiMode::Ap;
            }
        }
        VERBOSE.store(current.advanced.verbose_logs, Ordering::Relaxed);
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().unwrap().clone()
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Mutate a copy, persist it, and only then make it current - a failed
    /// write leaves the in-memory settings untouched
    pub fn update(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        let mut guard = self.current.lock().unwrap();
        let mut next = guard.clone();
        f(&mut next);
        self.write(&next)?;
        VERBOSE.store(next.advanced.verbose_logs, Ordering::Relaxed);
        *guard = next.clone();
        Ok(next)
    }

    fn write(&self, s: &Settings) -> Result<(), String> {
        let err =
            |e: std::io::Error| format!("Couldn't save settings to {}: {e}", self.path.display());
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(err)?;
        }
        let json = serialize(s);
        // Write-then-rename so a power cut mid-save can't leave a torn file
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(err)?;
        fs::rename(&tmp, &self.path).map_err(err)?;
        Ok(())
    }
}

fn serialize(s: &Settings) -> String {
    let mut json = serde_json::to_value(s).expect("settings serialize");
    json["version"] = VERSION.into();
    serde_json::to_string_pretty(&json).expect("settings serialize")
}

/// The settings and the version they were written in
fn parse(text: &str) -> Result<(Settings, u64), serde_json::Error> {
    let mut json: serde_json::Value = serde_json::from_str(text)?;
    // Files from before the field existed are in the first format
    let version = json["version"].as_u64().unwrap_or(1);
    migrate(&mut json, version);
    Ok((serde_json::from_value(json)?, version))
}

/// Bring a file written by an older release up to `VERSION`, one step per
/// version so any old file walks through all of them. A file from a newer
/// release is read as it is: what this release doesn't know is ignored
fn migrate(_json: &mut serde_json::Value, from: u64) {
    if from > VERSION {
        eprintln!("[settings] written by a newer release (format {from}, this is {VERSION})");
    }
}

/// The next save rewrites the file in this release's format, so what was
/// there stays beside it for going back to the release that wrote it
fn keep_copy(path: &std::path::Path, version: u64) {
    let copy = path.with_extension(format!("v{version}.json"));
    match fs::copy(path, &copy) {
        Ok(_) => eprintln!("[settings] format {version} kept as {}", copy.display()),
        Err(e) => eprintln!("[settings] couldn't keep a copy of format {version}: {e}"),
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("YSTREAMER_CONFIG") {
        return PathBuf::from(p);
    }
    if unsafe { libc::geteuid() } == 0 {
        return PathBuf::from("/etc/ystreamer/settings.json");
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("ystreamer/settings.json")
}

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
