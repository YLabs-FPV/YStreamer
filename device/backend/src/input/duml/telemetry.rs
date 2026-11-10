use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::Frame;

const CMD_SET_FC: u8 = 0x03;
const CMD_FC_OSD: u8 = 0x43;
const CMD_FC_HOME: u8 = 0x44;
const CMD_SET_BATTERY: u8 = 0x0D;
const CMD_BATTERY_PUSH: u8 = 0x02;

/// The flight controller pushes its OSD frame about ten times a second and
/// the home point about twice, so a few seconds of silence means it's gone
const STALE: Duration = Duration::from_secs(3);
/// The battery frame comes about once a second
const BATTERY_STALE: Duration = Duration::from_secs(5);

/// Closer than this to home, GPS noise would spin the bearing around
const MIN_BEARING_DISTANCE_M: f64 = 1.0;

/// What the aircraft reports about its flight. Every field is null when the
/// aircraft hasn't sent it lately; null never means zero
#[derive(serde::Serialize, Clone, Default, Debug, PartialEq)]
pub struct Telemetry {
    pub battery_percent: Option<u8>,
    /// Whether the latest position from the aircraft was a real fix
    pub gps_fix: Option<bool>,
    /// The O4 units' satellite count
    pub satellites: Option<u8>,
    /// What some units send instead of a count: DJI Fly's 0 to 5 GPS level
    pub gps_level: Option<u8>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Height above the takeoff point
    pub altitude_m: Option<f32>,
    /// Positive when climbing
    pub vertical_speed_mps: Option<f32>,
    pub horizontal_speed_mps: Option<f32>,
    pub pitch_deg: Option<f32>,
    pub roll_deg: Option<f32>,
    pub yaw_deg: Option<f32>,
    pub home_set: Option<bool>,
    pub home_lat: Option<f64>,
    pub home_lon: Option<f64>,
    /// The home point's own altitude from GPS, not a height above it
    pub home_altitude_m: Option<f32>,
    pub home_distance_m: Option<f32>,
    /// From the aircraft towards home, clockwise from north
    pub home_bearing_deg: Option<f32>,
    /// Signal bars, 0 to 5. Which of the two is the remote's link and which
    /// the video's hasn't been confirmed
    pub rc_signal: Option<u8>,
    pub vtx_signal: Option<u8>,
}

impl Telemetry {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// For browsers that may be viewers rather than the pilot: where the
    /// aircraft and its home are is left out
    pub fn without_position(mut self) -> Self {
        self.lat = None;
        self.lon = None;
        self.home_lat = None;
        self.home_lon = None;
        self.home_altitude_m = None;
        self
    }
}

#[derive(Clone, Copy, Default)]
struct Osd {
    position: Option<(f64, f64)>,
    satellites: Option<u8>,
    gps_level: Option<u8>,
    altitude_m: f32,
    vertical_speed_mps: f32,
    horizontal_speed_mps: f32,
    pitch_deg: f32,
    roll_deg: f32,
    yaw_deg: f32,
}

#[derive(Clone, Copy)]
struct Home {
    set: bool,
    position: Option<(f64, f64)>,
    altitude_m: Option<f32>,
}

#[derive(Clone, Copy)]
struct Battery {
    percent: u8,
    rc_signal: Option<u8>,
    vtx_signal: Option<u8>,
}

#[derive(Default)]
struct State {
    osd: Option<(Osd, Instant)>,
    home: Option<(Home, Instant)>,
    battery: Option<(Battery, Instant)>,
}

/// Keeps the latest of each telemetry frame, and when it came
pub struct Tracker {
    state: Mutex<State>,
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }

    pub fn on_frame(&self, f: &Frame) {
        self.state.lock().unwrap().apply(f, Instant::now());
    }

    pub fn snapshot(&self) -> Telemetry {
        self.state.lock().unwrap().snapshot(Instant::now())
    }
}

impl State {
    fn apply(&mut self, f: &Frame, now: Instant) {
        let p = f.payload;
        match (f.cmd_set, f.cmd_id) {
            (CMD_SET_FC, CMD_FC_OSD) => {
                if let Some(osd) = parse_osd(p) {
                    self.osd = Some((osd, now));
                }
            }
            (CMD_SET_FC, CMD_FC_HOME) => {
                if let Some(home) = parse_home(p) {
                    self.home = Some((home, now));
                }
            }
            (CMD_SET_BATTERY, CMD_BATTERY_PUSH) => {
                if let Some(battery) = parse_battery(p) {
                    self.battery = Some((battery, now));
                }
            }
            _ => {}
        }
    }

    fn snapshot(&self, now: Instant) -> Telemetry {
        let fresh = |at: Instant, limit: Duration| now.saturating_duration_since(at) < limit;
        let osd = self.osd.filter(|(_, at)| fresh(*at, STALE)).map(|(o, _)| o);
        let home = self
            .home
            .filter(|(_, at)| fresh(*at, STALE))
            .map(|(h, _)| h);
        let battery = self
            .battery
            .filter(|(_, at)| fresh(*at, BATTERY_STALE))
            .map(|(b, _)| b);

        let mut t = Telemetry::default();
        if let Some(o) = osd {
            t.gps_fix = Some(o.position.is_some());
            t.lat = o.position.map(|(lat, _)| lat);
            t.lon = o.position.map(|(_, lon)| lon);
            t.satellites = o.satellites;
            t.gps_level = o.gps_level;
            t.altitude_m = Some(o.altitude_m);
            t.vertical_speed_mps = Some(o.vertical_speed_mps);
            t.horizontal_speed_mps = Some(o.horizontal_speed_mps);
            t.pitch_deg = Some(o.pitch_deg);
            t.roll_deg = Some(o.roll_deg);
            t.yaw_deg = Some(o.yaw_deg);
        }
        if let Some(h) = home {
            t.home_set = Some(h.set);
            t.home_lat = h.position.map(|(lat, _)| lat);
            t.home_lon = h.position.map(|(_, lon)| lon);
            t.home_altitude_m = h.altitude_m;
        }
        let here = osd.and_then(|o| o.position);
        let home_at = home.filter(|h| h.set).and_then(|h| h.position);
        if let (Some(from), Some(to)) = (here, home_at) {
            let (distance, bearing) = distance_and_bearing(from, to);
            t.home_distance_m = Some(tenths(distance));
            t.home_bearing_deg =
                (distance >= MIN_BEARING_DISTANCE_M).then(|| tenths(bearing) % 360.0);
        }
        if let Some(b) = battery {
            t.battery_percent = Some(b.percent);
            t.rc_signal = b.rc_signal;
            t.vtx_signal = b.vtx_signal;
        }
        t
    }
}

/// The flight controller's OSD frame. Longitude and latitude are f64
/// radians at 0 and 8, then i16 tenths: height above takeoff at 16, north,
/// east and down speed at 18, 20 and 22, pitch, roll and yaw at 24, 26, 28.
/// Matches the public DJI dissector and was checked against the goggles'
/// own readout on an Avata 2 with Goggles 3
fn parse_osd(p: &[u8]) -> Option<Osd> {
    if p.len() < 30 {
        return None;
    }
    let tenth = |at| i16_at(p, at) as f32 / 10.0;
    let (north, east) = (tenth(18), tenth(20));
    Some(Osd {
        position: position_at(p, 0, 8),
        // The 73-byte frame (Avata 2, O4) has the count at 69. The 85-byte
        // one (Neo 2) has a constant there, and DJI Fly's GPS level at 36,
        // which can read 6 on a scale that stops at 5
        satellites: (p.len() == 73).then(|| p[69]),
        gps_level: (p.len() == 85).then(|| p[36].min(5)),
        altitude_m: tenth(16),
        // Down is positive on the wire; negate the integer so still air
        // reads 0, not -0
        vertical_speed_mps: -(i16_at(p, 22) as i32) as f32 / 10.0,
        horizontal_speed_mps: tenths(f64::from(north.hypot(east))),
        pitch_deg: tenth(24),
        roll_deg: tenth(26),
        yaw_deg: tenth(28),
    })
}

/// The home point frame: the same f64 radians at 0 and 8, an f32 GPS
/// altitude in tenths of a metre at 16, and a state word at 20 whose bit 0
/// says a home point is set. The other state bits stay unnamed: they didn't
/// change during a return-to-home flight
fn parse_home(p: &[u8]) -> Option<Home> {
    if p.len() < 22 {
        return None;
    }
    let set = u16::from_le_bytes([p[20], p[21]]) & 1 != 0;
    if !set {
        return Some(Home {
            set,
            position: None,
            altitude_m: None,
        });
    }
    let altitude = f32::from_le_bytes([p[16], p[17], p[18], p[19]]);
    Some(Home {
        set,
        position: position_at(p, 0, 8),
        altitude_m: altitude.is_finite().then(|| altitude / 10.0),
    })
}

/// The battery push: the aircraft's battery at 20. The bars at 19 and 47 are
/// from one unit only, and the frame is too short for 47 on others
fn parse_battery(p: &[u8]) -> Option<Battery> {
    let percent = *p.get(20)?;
    if percent > 100 {
        return None;
    }
    Some(Battery {
        percent,
        rc_signal: p.get(19).copied().filter(|&v| v <= 5),
        vtx_signal: p.get(47).copied().filter(|&v| v <= 5),
    })
}

fn i16_at(p: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([p[at], p[at + 1]])
}

fn f64_at(p: &[u8], at: usize) -> f64 {
    f64::from_le_bytes(p[at..at + 8].try_into().unwrap())
}

/// Latitude and longitude in degrees, or None for no fix: the aircraft sends
/// zeros until it has one
fn position_at(p: &[u8], lon_at: usize, lat_at: usize) -> Option<(f64, f64)> {
    let lon = f64_at(p, lon_at).to_degrees();
    let lat = f64_at(p, lat_at).to_degrees();
    let valid = lat.is_finite()
        && lon.is_finite()
        && lat.abs() <= 90.0
        && lon.abs() <= 180.0
        && (lat.abs() > 1e-4 || lon.abs() > 1e-4);
    valid.then_some((lat, lon))
}

/// Great-circle distance in metres and the initial bearing in degrees
fn distance_and_bearing(from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let (lat1, lat2) = (from.0.to_radians(), to.0.to_radians());
    let dlat = lat2 - lat1;
    let dlon = (to.1 - from.1).to_radians();
    let a = ((dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2))
        .clamp(0.0, 1.0);
    let distance = 2.0 * EARTH_RADIUS_M * a.sqrt().atan2((1.0 - a).sqrt());
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    let bearing = y.atan2(x).to_degrees().rem_euclid(360.0);
    (distance, bearing)
}

/// Rounded to a tenth, so GPS noise below that doesn't count as a change
fn tenths(v: f64) -> f32 {
    ((v * 10.0).round() / 10.0) as f32
}

#[cfg(test)]
#[path = "telemetry.test.rs"]
mod tests;
