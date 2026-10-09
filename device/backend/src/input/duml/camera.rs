use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::devices::Kind;
use super::session::Session;
use super::{FLAGS_REQUEST, encode, next_seq};
use crate::input::goggles::{Goggles, LinkState};

const DST: u8 = 1;
pub const CMD_SET: u8 = 2;

const CMD_VIDEO_FORMAT_SET: u8 = 24;
const CMD_EXPOSURE_MODE_SET: u8 = 30;
const CMD_SHUTTER_SET: u8 = 40;
const CMD_ISO_SET: u8 = 42;
const CMD_WB_SET: u8 = 44;
const CMD_EV_SET: u8 = 46;
const CMD_SHARPNESS_SET: u8 = 56;
const CMD_COLOR_PROFILE_SET: u8 = 66;
const CMD_NR_SET: u8 = 68;
const CMD_ANTI_FLICKER_SET: u8 = 70;

/// What `Poller` asks for; the get command is always the set command plus one
const POLLED: [u8; 10] = [
    CMD_VIDEO_FORMAT_SET,
    CMD_EXPOSURE_MODE_SET,
    CMD_SHUTTER_SET,
    CMD_ISO_SET,
    CMD_WB_SET,
    CMD_EV_SET,
    CMD_SHARPNESS_SET,
    CMD_COLOR_PROFILE_SET,
    CMD_NR_SET,
    CMD_ANTI_FLICKER_SET,
];

// D-Cinelike is the O3's flat profile, D-Log the O4 Pro's
const COLOR_PROFILES: [(&str, u8); 3] = [("normal", 0x00), ("dcinelike", 0x06), ("dlog", 0x3D)];
const ANTI_FLICKER: [(&str, u8); 4] = [("auto", 0), ("60hz", 1), ("50hz", 2), ("off", 3)];

fn to_wire(table: &[(&str, u8)], name: &str) -> Option<u8> {
    table.iter().find(|(n, _)| *n == name).map(|(_, w)| *w)
}

fn from_wire(table: &[(&'static str, u8)], wire: u8) -> Option<&'static str> {
    table.iter().find(|(_, w)| *w == wire).map(|(n, _)| *n)
}

pub fn iso_to_wire(iso: u32) -> Option<u8> {
    Some(match iso {
        25 => 1,
        50 => 2,
        100 => 3,
        200 => 4,
        400 => 5,
        800 => 6,
        1600 => 7,
        3200 => 8,
        6400 => 9,
        12800 => 10,
        25600 => 11,
        _ => return None,
    })
}

/// Shutter speed (denominator, e.g. 1000 for 1/1000) -> DJI table code
/// (Only used to validate; the on-wire value uses the 32768+speed math below)
fn shutter_is_valid(speed: u32) -> bool {
    matches!(
        speed,
        8000 | 6400
            | 5000
            | 4000
            | 3200
            | 2500
            | 2000
            | 1600
            | 1250
            | 1000
            | 800
            | 640
            | 500
            | 400
            | 320
            | 250
            | 240
            | 200
            | 160
            | 120
            | 100
            | 80
            | 60
            | 50
            | 40
            | 30
    )
}

/// EV steps, index 0..=18 maps to wire value (index + 7)
const EV_VALUES: [f32; 19] = [
    -3.0, -2.7, -2.3, -2.0, -1.7, -1.3, -1.0, -0.7, -0.3, 0.0, 0.3, 0.7, 1.0, 1.3, 1.7, 2.0, 2.3,
    2.7, 3.0,
];

pub fn res_ar_to_wire(res: &str, ar: &str) -> Option<u8> {
    Some(match (res, ar) {
        ("1080p", "16:9") => 10,
        ("1080p", "4:3") => 98,
        ("2.7K", "16:9") => 45,
        ("2.7K", "4:3") => 95,
        ("4K", "16:9") => 16,
        ("4K", "4:3") => 103,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExposureMode {
    Auto,
    Manual,
}

/// Builds DUML camera-control packets
pub struct CameraCommands;

impl CameraCommands {
    pub fn new() -> Self {
        Self
    }

    pub fn build_iso(&self, iso: u32) -> Option<Vec<u8>> {
        let wire = if iso == 0 { 0x00 } else { iso_to_wire(iso)? };
        Some(self.build_duml(CMD_ISO_SET, &[wire]))
    }

    pub fn build_shutter(&self, speed: u32) -> Option<Vec<u8>> {
        // Auto shutter: 4-byte payload with zeroed speed
        if speed == 0 {
            let payload = [0x01, 0x00, 0x00, 0x00];
            return Some(self.build_duml(CMD_SHUTTER_SET, &payload));
        }
        if !shutter_is_valid(speed) {
            return None;
        }
        let dji_value = 32768 + speed;
        let payload = [
            0x01,
            (dji_value & 0xFF) as u8,
            ((dji_value >> 8) & 0xFF) as u8,
            0x00,
        ];
        Some(self.build_duml(CMD_SHUTTER_SET, &payload))
    }

    pub fn build_ev(&self, ev: f32) -> Option<Vec<u8>> {
        let index = EV_VALUES.iter().position(|&v| (v - ev).abs() < 0.05)?;
        let wire = (index + 7) as u8;
        Some(self.build_duml(CMD_EV_SET, &[wire]))
    }

    pub fn build_exposure_mode(&self, mode: ExposureMode) -> Vec<u8> {
        let payload: [u8; 2] = match mode {
            ExposureMode::Auto => [0x01, 0x00],
            ExposureMode::Manual => [0x04, 0x00],
        };
        self.build_duml(CMD_EXPOSURE_MODE_SET, &payload)
    }

    pub fn build_wb(&self, is_auto: bool, color_temp: u32) -> Vec<u8> {
        if is_auto {
            self.build_duml(CMD_WB_SET, &[0x00, 0x00])
        } else {
            let temp_val = color_temp / 100;
            let payload = [
                0x06,
                (temp_val & 0xFF) as u8,
                ((temp_val >> 8) & 0xFF) as u8,
                0xFF,
                0xFF,
            ];
            self.build_duml(CMD_WB_SET, &payload)
        }
    }

    pub fn build_video_format(&self, res_wire: u8, fps_wire: u8) -> Vec<u8> {
        let payload = [res_wire, fps_wire, 0x00];
        self.build_duml(CMD_VIDEO_FORMAT_SET, &payload)
    }

    pub fn build_sharpness(&self, value: i8) -> Vec<u8> {
        self.build_duml(CMD_SHARPNESS_SET, &[value as u8])
    }

    pub fn build_noise_reduction(&self, value: i8) -> Option<Vec<u8>> {
        (-2..=1)
            .contains(&value)
            .then(|| self.build_duml(CMD_NR_SET, &[value as u8]))
    }

    pub fn build_color_profile(&self, profile: &str) -> Option<Vec<u8>> {
        let wire = to_wire(&COLOR_PROFILES, profile)?;
        Some(self.build_duml(CMD_COLOR_PROFILE_SET, &[wire]))
    }

    pub fn build_anti_flicker(&self, mode: &str) -> Option<Vec<u8>> {
        let wire = to_wire(&ANTI_FLICKER, mode)?;
        Some(self.build_duml(CMD_ANTI_FLICKER_SET, &[wire]))
    }

    fn build_duml(&self, cmd_id: u8, payload: &[u8]) -> Vec<u8> {
        encode(DST, next_seq(), FLAGS_REQUEST, CMD_SET, cmd_id, payload)
    }
}

pub fn iso_from_wire(wire: u8) -> Option<u32> {
    Some(match wire {
        1 => 25,
        2 => 50,
        3 => 100,
        4 => 200,
        5 => 400,
        6 => 800,
        7 => 1600,
        8 => 3200,
        9 => 6400,
        10 => 12800,
        11 => 25600,
        _ => return None,
    })
}

pub fn ev_from_wire(wire: u8) -> Option<f32> {
    let idx = wire as i32 - 7;
    if idx < 0 {
        return None;
    }
    EV_VALUES.get(idx as usize).copied()
}

/// DJI shutter math: values >= 32768 encode the denominator (1/x); 0 = Auto
pub fn shutter_from_dji_math(dji: u32) -> u32 {
    if dji == 0 {
        return 0;
    }
    if dji >= 32768 {
        return dji - 32768;
    }
    0
}

pub fn wire_to_res_ar(wire: u8) -> Option<(&'static str, &'static str)> {
    Some(match wire {
        10 => ("1080p", "16:9"),
        98 => ("1080p", "4:3"),
        45 => ("2.7K", "16:9"),
        95 => ("2.7K", "4:3"),
        16 => ("4K", "16:9"),
        103 => ("4K", "4:3"),
        _ => return None,
    })
}

/// A partial snapshot of goggle/camera state. Only the fields present in a
/// given DUML response are Some; the frontend merges partial updates. Skipped
/// (None) fields are omitted from the JSON
#[derive(serde::Serialize, Default, Clone, Debug)]
pub struct CameraState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iso: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shutter: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ev: Option<f32>,
    /// How far off the picture is by the camera's own meter; on manual
    /// exposure it stands in for the compensation, which then does nothing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ev_metered: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wb_auto: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wb_temp: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub res: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharpness: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise_reduction: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_profile: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anti_flicker: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goggles_battery: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_quality: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aircraft_recorder: Option<super::aircraft_recorder::AircraftRecorder>,
}

/// None for frames we don't care about
pub fn parse_duml_response(cmd_set: u8, cmd_id: u8, payload: &[u8]) -> Option<CameraState> {
    let mut s = CameraState::default();

    if cmd_set == CMD_SET {
        // Full camera-state snapshot
        if cmd_id == 129 && payload.len() >= 25 {
            let shutter_dji = (payload[2] as u32) | ((payload[3] as u32) << 8);
            s.shutter = Some(shutter_from_dji_math(shutter_dji));

            let iso_wire = payload[5];
            if iso_wire == 0 || iso_wire == 0xFF {
                s.iso = Some(0);
            } else if let Some(iso) = iso_from_wire(iso_wire) {
                s.iso = Some(iso);
            }

            if let Some(ev) = ev_from_wire(payload[6]) {
                s.ev = Some(ev);
            }

            match payload[20] {
                0x01 => s.exposure_mode = Some("auto".to_string()),
                0x04 => s.exposure_mode = Some("manual".to_string()),
                _ => {}
            }

            // On auto the temperature is the one the camera has settled on
            let wb_mode = payload[23];
            if wb_mode == 0x00 || wb_mode == 0x06 {
                s.wb_auto = Some(wb_mode == 0x00);
                if payload[24] != 0 {
                    s.wb_temp = Some((payload[24] as u32) * 100);
                }
            }

            if let Some((r, a)) = wire_to_res_ar(payload[13]) {
                s.res = Some(r.to_string());
                s.ar = Some(a.to_string());
            }
            s.fps = Some(payload[14]);

            if payload.len() > 27 {
                s.sharpness = Some(payload[27] as i8);
            }
            if payload.len() > 32 {
                s.color_profile = from_wire(&COLOR_PROFILES, payload[31]);
                s.anti_flicker = from_wire(&ANTI_FLICKER, payload[32]);
            }
            if payload.len() > 47 {
                s.ev_metered = ev_from_wire(payload[47]);
            }
            if payload.len() > 103 {
                s.noise_reduction = Some(payload[103] as i8);
            }

            return Some(s);
        }
    } else if cmd_set == 6 {
        // Goggles / RC telemetry: battery
        if cmd_id == 30 && payload.len() > 4 {
            s.goggles_battery = Some(payload[4]);
            return Some(s);
        }
    }

    None
}

/// The answer to one of `Poller`'s questions: a status byte, then the same
/// bytes the matching set command takes
pub fn parse_get_reply(cmd_id: u8, payload: &[u8]) -> Option<CameraState> {
    let (&status, v) = payload.split_first()?;
    if status != 0 || v.is_empty() {
        return None;
    }
    let mut s = CameraState::default();
    match cmd_id.checked_sub(1)? {
        CMD_VIDEO_FORMAT_SET if v.len() >= 2 => {
            if let Some((r, a)) = wire_to_res_ar(v[0]) {
                s.res = Some(r.to_string());
                s.ar = Some(a.to_string());
            }
            s.fps = Some(v[1]);
        }
        CMD_EXPOSURE_MODE_SET => match v[0] {
            0x01 => s.exposure_mode = Some("auto".to_string()),
            0x04 => s.exposure_mode = Some("manual".to_string()),
            _ => return None,
        },
        // The O3 refuses this one while exposure is on auto
        CMD_SHUTTER_SET if v.len() >= 3 => {
            let dji = u16::from_le_bytes([v[1], v[2]]) as u32;
            s.shutter = Some(shutter_from_dji_math(dji));
        }
        CMD_ISO_SET => {
            s.iso = match v[0] {
                0x00 | 0xFF => Some(0),
                wire => Some(iso_from_wire(wire)?),
            }
        }
        CMD_WB_SET => match v[0] {
            0x00 => s.wb_auto = Some(true),
            0x06 if v.len() >= 3 => {
                s.wb_auto = Some(false);
                s.wb_temp = Some(u16::from_le_bytes([v[1], v[2]]) as u32 * 100);
            }
            _ => return None,
        },
        CMD_EV_SET => s.ev = Some(ev_from_wire(v[0])?),
        CMD_SHARPNESS_SET => s.sharpness = Some(v[0] as i8),
        CMD_NR_SET => s.noise_reduction = Some(v[0] as i8),
        CMD_COLOR_PROFILE_SET => s.color_profile = Some(from_wire(&COLOR_PROFILES, v[0])?),
        CMD_ANTI_FLICKER_SET => s.anti_flicker = Some(from_wire(&ANTI_FLICKER, v[0])?),
        _ => return None,
    }
    Some(s)
}

/// The O3 and older air units never push the state frame, but answer when
/// asked for one setting at a time
pub struct Poller {
    last_push: Mutex<Option<Instant>>,
}

impl Poller {
    const STEP: Duration = Duration::from_millis(200);
    const PUSH_TIMEOUT: Duration = Duration::from_secs(3);

    pub fn start(goggles: Goggles, session: Arc<Session>) -> Arc<Self> {
        let poller = Arc::new(Self {
            last_push: Mutex::new(None),
        });
        let this = Arc::clone(&poller);
        thread::spawn(move || {
            let cam = CameraCommands::new();
            let mut next = POLLED.iter().cycle();
            loop {
                thread::sleep(Self::STEP);
                if !this.needed(&goggles, &session) {
                    continue;
                }
                if let Some(set_cmd) = next.next() {
                    let _ = goggles.send(&cam.build_duml(set_cmd + 1, &[]));
                }
            }
        });
        poller
    }

    pub fn saw_push(&self) {
        *self.last_push.lock().unwrap() = Some(Instant::now());
    }

    fn needed(&self, goggles: &Goggles, session: &Session) -> bool {
        let pushed = self
            .last_push
            .lock()
            .unwrap()
            .is_some_and(|at| at.elapsed() < Self::PUSH_TIMEOUT);
        !pushed
            && goggles.state() == LinkState::Live
            && session.devices().iter().any(|d| d.kind == Kind::Aircraft)
    }
}

#[cfg(test)]
#[path = "camera.test.rs"]
mod tests;
