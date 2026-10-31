use super::*;

// Built from the documented layouts, at a made-up place
const LAT: f64 = 45.0;
const LON: f64 = 7.0;

fn frame(cmd_set: u8, cmd_id: u8, payload: &[u8]) -> Frame<'_> {
    Frame {
        src: 14,
        dst: 2,
        seq: 1,
        flags: 0,
        cmd_set,
        cmd_id,
        payload,
    }
}

struct Osd {
    lat: f64,
    lon: f64,
    height: i16,
    north: i16,
    east: i16,
    down: i16,
    pitch: i16,
    roll: i16,
    yaw: i16,
}

impl Default for Osd {
    fn default() -> Self {
        Self {
            lat: LAT,
            lon: LON,
            height: 0,
            north: 0,
            east: 0,
            down: 0,
            pitch: 0,
            roll: 0,
            yaw: 0,
        }
    }
}

fn osd(len: usize, o: Osd) -> Vec<u8> {
    let mut p = vec![0u8; len];
    p[0..8].copy_from_slice(&o.lon.to_radians().to_le_bytes());
    p[8..16].copy_from_slice(&o.lat.to_radians().to_le_bytes());
    for (at, v) in [
        (16, o.height),
        (18, o.north),
        (20, o.east),
        (22, o.down),
        (24, o.pitch),
        (26, o.roll),
        (28, o.yaw),
    ] {
        p[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    p
}

fn home(lat: f64, lon: f64, altitude_tenths: f32, state: u16) -> Vec<u8> {
    let mut p = vec![0u8; 102];
    p[0..8].copy_from_slice(&lon.to_radians().to_le_bytes());
    p[8..16].copy_from_slice(&lat.to_radians().to_le_bytes());
    p[16..20].copy_from_slice(&altitude_tenths.to_le_bytes());
    p[20..22].copy_from_slice(&state.to_le_bytes());
    p
}

fn battery(len: usize, percent: u8, rc: u8) -> Vec<u8> {
    let mut p = vec![0u8; len];
    p[19] = rc;
    p[20] = percent;
    p
}

#[test]
fn reads_the_flight_controller_osd() {
    let mut p = osd(
        73,
        Osd {
            height: 16,
            north: 3,
            east: 4,
            down: 5,
            pitch: -13,
            roll: 12,
            yaw: 381,
            ..Default::default()
        },
    );
    p[69] = 17;
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x43, &p), now);
    let t = s.snapshot(now);

    assert_eq!(t.gps_fix, Some(true));
    assert!((t.lat.unwrap() - LAT).abs() < 1e-9);
    assert!((t.lon.unwrap() - LON).abs() < 1e-9);
    assert_eq!(t.satellites, Some(17));
    assert_eq!(t.gps_level, None);
    assert_eq!(t.altitude_m, Some(1.6));
    // Descending at 0.5 m/s
    assert_eq!(t.vertical_speed_mps, Some(-0.5));
    assert_eq!(t.horizontal_speed_mps, Some(0.5));
    assert_eq!(t.pitch_deg, Some(-1.3));
    assert_eq!(t.roll_deg, Some(1.2));
    assert_eq!(t.yaw_deg, Some(38.1));
}

#[test]
fn the_longer_osd_has_a_level_not_a_count() {
    let mut p = osd(85, Osd::default());
    p[69] = 7;
    p[36] = 6;
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x43, &p), now);
    let t = s.snapshot(now);

    assert_eq!(t.satellites, None);
    assert_eq!(t.gps_level, Some(5));
}

#[test]
fn zeros_mean_no_fix() {
    let p = osd(
        73,
        Osd {
            lat: 0.0,
            lon: 0.0,
            height: 12,
            ..Default::default()
        },
    );
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x43, &p), now);
    let t = s.snapshot(now);

    assert_eq!(t.gps_fix, Some(false));
    assert_eq!(t.lat, None);
    assert_eq!(t.lon, None);
    // The rest of the frame still counts
    assert_eq!(t.altitude_m, Some(1.2));
}

#[test]
fn works_out_distance_and_bearing_to_home() {
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x44, &home(LAT, LON, 2480.4, 0x47)), now);

    let t = s.snapshot(now);
    assert_eq!(t.home_set, Some(true));
    assert!((t.home_lat.unwrap() - LAT).abs() < 1e-9);
    assert!((t.home_altitude_m.unwrap() - 248.04).abs() < 0.001);
    assert_eq!(t.home_distance_m, None, "no aircraft position yet");

    // About 11.1 m north of home, so home is due south
    let p = osd(
        73,
        Osd {
            lat: LAT + 0.0001,
            ..Default::default()
        },
    );
    s.apply(&frame(0x03, 0x43, &p), now);
    let t = s.snapshot(now);
    assert_eq!(t.home_distance_m, Some(11.1));
    assert_eq!(t.home_bearing_deg, Some(180.0));
}

#[test]
fn no_bearing_right_at_home() {
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x44, &home(LAT, LON, 2400.0, 1)), now);
    s.apply(&frame(0x03, 0x43, &osd(73, Osd::default())), now);
    let t = s.snapshot(now);

    assert_eq!(t.home_distance_m, Some(0.0));
    assert_eq!(t.home_bearing_deg, None);
}

#[test]
fn an_unset_home_point_drops_the_old_one() {
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x44, &home(LAT, LON, 2400.0, 1)), now);
    s.apply(&frame(0x03, 0x43, &osd(73, Osd::default())), now);
    s.apply(&frame(0x03, 0x44, &[0u8; 22]), now);
    let t = s.snapshot(now);

    assert_eq!(t.home_set, Some(false));
    assert_eq!(t.home_lat, None);
    assert_eq!(t.home_lon, None);
    assert_eq!(t.home_altitude_m, None);
    assert_eq!(t.home_distance_m, None);
    assert_eq!(t.home_bearing_deg, None);
}

#[test]
fn reads_the_battery_push() {
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x0D, 0x02, &battery(44, 81, 2)), now);
    let t = s.snapshot(now);

    assert_eq!(t.battery_percent, Some(81));
    assert_eq!(t.rc_signal, Some(2));
    // Too short for the other bars
    assert_eq!(t.vtx_signal, None);

    let mut p = battery(60, 44, 4);
    p[47] = 5;
    s.apply(&frame(0x0D, 0x02, &p), now);
    let t = s.snapshot(now);
    assert_eq!(t.battery_percent, Some(44));
    assert_eq!(t.vtx_signal, Some(5));
}

#[test]
fn goes_blank_when_the_frames_stop() {
    let mut s = State::default();
    let start = Instant::now();
    s.apply(&frame(0x03, 0x43, &osd(73, Osd::default())), start);
    s.apply(&frame(0x03, 0x44, &home(LAT, LON, 2400.0, 1)), start);
    s.apply(&frame(0x0D, 0x02, &battery(44, 50, 3)), start);
    assert!(!s.snapshot(start).is_empty());

    let t = s.snapshot(start + STALE);
    assert_eq!(t.gps_fix, None);
    assert_eq!(t.altitude_m, None);
    assert_eq!(t.home_set, None);
    assert_eq!(t.home_distance_m, None);
    // The battery is pushed less often and is given longer
    assert_eq!(t.battery_percent, Some(50));

    assert!(s.snapshot(start + BATTERY_STALE).is_empty());
}

#[test]
fn ignores_short_and_other_frames() {
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x43, &[0u8; 29]), now);
    s.apply(&frame(0x03, 0x44, &[1u8; 21]), now);
    s.apply(&frame(0x0D, 0x02, &[0u8; 20]), now);
    s.apply(&frame(0x0D, 0x02, &battery(44, 0xFF, 0)), now);
    s.apply(&frame(0x02, 0x43, &osd(73, Osd::default())), now);
    assert!(s.snapshot(now).is_empty());
}

#[test]
fn browsers_get_no_coordinates() {
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x44, &home(LAT, LON, 2400.0, 1)), now);
    s.apply(
        &frame(
            0x03,
            0x43,
            &osd(
                73,
                Osd {
                    lat: LAT + 0.001,
                    ..Default::default()
                },
            ),
        ),
        now,
    );
    let t = s.snapshot(now).without_position();

    assert_eq!((t.lat, t.lon), (None, None));
    assert_eq!(
        (t.home_lat, t.home_lon, t.home_altitude_m),
        (None, None, None)
    );
    assert_eq!(t.gps_fix, Some(true));
    assert_eq!(t.home_set, Some(true));
    assert_eq!(t.home_distance_m, Some(111.2));
}

#[test]
fn every_field_is_in_the_json() {
    let mut s = State::default();
    let now = Instant::now();
    s.apply(&frame(0x03, 0x43, &osd(73, Osd::default())), now);
    let json = serde_json::to_value(s.snapshot(now)).unwrap();

    // Null rather than missing, so pollers see a fixed set of keys
    assert!(json["battery_percent"].is_null());
    assert!(json["home_distance_m"].is_null());
    // Still air is 0, not -0
    assert_eq!(json["vertical_speed_mps"].to_string(), "0.0");
}
