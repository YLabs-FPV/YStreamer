use super::*;

/// How often the simulated link hands over a chunk of each stream
const STEP: Duration = Duration::from_millis(50);
const BIG: u32 = 17_000;
const SMALL: u32 = 1_500;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A chunk's worth of a stream at `kbps`
fn bytes(kbps: u32) -> usize {
    (kbps as u64 * 1000 / 8 * STEP.as_millis() as u64 / 1000) as usize
}

/// Runs both streams at the given rates, 0 for silent, until the selector
/// wants a switch. Returns the camera and how long it took
fn run(
    s: &mut Selector,
    t: &mut Instant,
    secs: u64,
    (main, second): (u32, u32),
    choice: CameraChoice,
) -> Option<(Lane, Duration)> {
    let start = *t;
    while *t < start + Duration::from_secs(secs) {
        *t += STEP;
        for (lane, kbps) in [(Lane::Main, main), (Lane::Second, second)] {
            if kbps > 0 {
                s.on_data(lane, bytes(kbps), true, *t);
            }
        }
        if let Some(lane) = s.tick(*t, choice) {
            return Some((lane, *t - start));
        }
    }
    None
}

#[test]
fn measures_the_rate() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    run(&mut s, &mut t, 3, (BIG, SMALL), CameraChoice::Auto);
    let main = s.status(Lane::Main, t);
    assert!(
        (BIG * 7 / 10..=BIG * 11 / 10).contains(&main.kbps),
        "{main:?}"
    );
    assert!(main.live);
    // A stream that stops drops to nothing
    t += Duration::from_secs(2);
    s.tick(t, CameraChoice::Auto);
    assert_eq!(s.status(Lane::Main, t).kbps, 0);
    assert!(!s.status(Lane::Main, t).live);
}

#[test]
fn stays_on_the_big_stream() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    assert_eq!(
        run(&mut s, &mut t, 60, (BIG, SMALL), CameraChoice::Auto),
        None
    );
    assert_eq!(s.program(), Lane::Main);
}

#[test]
fn follows_the_big_stream_to_the_other_port() {
    // DJI Fly's picture-in-picture moved the live view onto the second port
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    let (lane, after) = run(&mut s, &mut t, 10, (SMALL, BIG), CameraChoice::Auto).unwrap();
    assert_eq!(lane, Lane::Second);
    // Not before the start settled and the lead held
    assert!(after >= WARMUP, "{after:?}");
    assert!(after < Duration::from_secs(5), "{after:?}");
}

#[test]
fn a_short_burst_takes_nothing() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    assert_eq!(
        run(&mut s, &mut t, 5, (BIG, SMALL), CameraChoice::Auto),
        None
    );
    // Keyframes, say: far ahead, but for less than the confirm window
    let burst = t + ms(500);
    while t < burst {
        t += STEP;
        s.on_data(Lane::Main, bytes(BIG), true, t);
        s.on_data(Lane::Second, bytes(60_000), true, t);
        assert_eq!(s.tick(t, CameraChoice::Auto), None);
    }
    assert_eq!(
        run(&mut s, &mut t, 10, (BIG, SMALL), CameraChoice::Auto),
        None
    );
}

#[test]
fn never_switches_to_a_smaller_picture_by_itself() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    s.set_size(Lane::Main, Some((1920, 1080)));
    s.set_size(Lane::Second, Some((640, 360)));
    assert_eq!(
        run(&mut s, &mut t, 20, (3_000, BIG), CameraChoice::Auto),
        None
    );
    // Rate decides once the sizes agree
    s.set_size(Lane::Second, Some((1920, 1080)));
    let (lane, _) = run(&mut s, &mut t, 5, (3_000, BIG), CameraChoice::Auto).unwrap();
    assert_eq!(lane, Lane::Second);
}

#[test]
fn a_camera_that_stops_is_left_at_once() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    assert_eq!(
        run(&mut s, &mut t, 10, (BIG, SMALL), CameraChoice::Auto),
        None
    );
    let (lane, after) = run(&mut s, &mut t, 5, (0, SMALL), CameraChoice::Auto).unwrap();
    assert_eq!(lane, Lane::Second);
    assert!(after < SILENT + TICK * 2, "{after:?}");
}

#[test]
fn takes_the_second_stream_when_it_is_the_only_one() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    let (lane, after) = run(&mut s, &mut t, 5, (0, BIG), CameraChoice::Auto).unwrap();
    assert_eq!(lane, Lane::Second);
    assert!(after <= TICK, "{after:?}");
}

#[test]
fn a_chosen_camera_wins_outright() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    let (lane, after) = run(&mut s, &mut t, 5, (BIG, SMALL), CameraChoice::Second).unwrap();
    assert_eq!(lane, Lane::Second);
    assert!(after <= TICK, "{after:?}");
    s.switched(Lane::Second, t);
    assert_eq!(
        run(&mut s, &mut t, 10, (BIG, SMALL), CameraChoice::Second),
        None
    );
    // And back, without waiting out the time between automatic switches
    let (lane, after) = run(&mut s, &mut t, 5, (BIG, SMALL), CameraChoice::Main).unwrap();
    assert_eq!(lane, Lane::Main);
    assert!(after <= TICK, "{after:?}");
}

#[test]
fn the_main_camera_chosen_stays_whatever_the_rates() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    assert_eq!(
        run(&mut s, &mut t, 20, (SMALL, BIG), CameraChoice::Main),
        None
    );
}

#[test]
fn a_second_camera_chosen_but_missing_changes_nothing() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    assert_eq!(
        run(&mut s, &mut t, 10, (BIG, 0), CameraChoice::Second),
        None
    );
    assert_eq!(s.program(), Lane::Main);
}

#[test]
fn waits_between_switches() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    let (lane, _) = run(&mut s, &mut t, 10, (SMALL, BIG), CameraChoice::Auto).unwrap();
    s.switched(lane, t);
    // DJI Fly moves the live view straight back
    let (lane, after) = run(&mut s, &mut t, 10, (BIG, SMALL), CameraChoice::Auto).unwrap();
    assert_eq!(lane, Lane::Main);
    assert!(after >= DWELL - TICK, "{after:?}");
}

#[test]
fn a_failed_switch_is_not_tried_again_straight_away() {
    let mut t = Instant::now();
    let mut s = Selector::new(t);
    run(&mut s, &mut t, 1, (BIG, SMALL), CameraChoice::Second).unwrap();
    s.failed(t);
    let (lane, after) = run(&mut s, &mut t, 10, (BIG, SMALL), CameraChoice::Second).unwrap();
    assert_eq!(lane, Lane::Second);
    assert!(after >= DWELL - TICK, "{after:?}");
}

fn chunk(kbps: u32, params: Option<usize>, keyframe: Option<usize>) -> Marks {
    Marks {
        bytes: bytes(kbps),
        picture: true,
        params,
        keyframe,
    }
}

/// Both cameras' chunks for one step; what each was routed to
fn step(p: &mut Program, t: &mut Instant, second: &Marks, choice: CameraChoice) -> (Route, Route) {
    *t += STEP;
    let main = p.route(
        Lane::Main,
        Codec::H265,
        &chunk(BIG, None, None),
        true,
        choice,
        *t,
    );
    let second = p.route(Lane::Second, Codec::H265, second, true, choice, *t);
    (main, second)
}

/// Both cameras running for a while, the main one on the outputs
fn running(t: &mut Instant) -> Program {
    let mut p = Program::new(*t);
    for _ in 0..60 {
        let routes = step(&mut p, t, &chunk(SMALL, None, None), CameraChoice::Auto);
        assert_eq!(routes, (Route::Send, Route::Drop));
    }
    p
}

#[test]
fn a_switch_starts_at_the_new_cameras_parameter_sets() {
    let mut t = Instant::now();
    let mut p = running(&mut t);
    // Asked for the second camera: the outputs keep the main one until the
    // second one can start, never a mix of both
    for _ in 0..6 {
        let routes = step(
            &mut p,
            &mut t,
            &chunk(SMALL, None, None),
            CameraChoice::Second,
        );
        assert_eq!(routes, (Route::Send, Route::Drop));
    }
    let routes = step(
        &mut p,
        &mut t,
        &chunk(SMALL, Some(40), None),
        CameraChoice::Second,
    );
    assert_eq!(routes, (Route::Send, Route::Switch(40)));
    assert_eq!(p.lane(), Lane::Second);
    let routes = step(
        &mut p,
        &mut t,
        &chunk(SMALL, None, None),
        CameraChoice::Second,
    );
    assert_eq!(routes, (Route::Drop, Route::Send));
}

#[test]
fn without_fresh_parameter_sets_a_switch_starts_at_a_keyframe() {
    let mut t = Instant::now();
    let mut p = running(&mut t);
    let asked = t;
    let switched = loop {
        let routes = step(
            &mut p,
            &mut t,
            &chunk(SMALL, None, Some(8)),
            CameraChoice::Second,
        );
        if routes.1 == Route::Switch(8) {
            break t;
        }
        assert_eq!(routes, (Route::Send, Route::Drop));
        assert!(t - asked < Duration::from_secs(1), "never switched");
    };
    // It waited for parameter sets first
    assert!(switched - asked >= SWITCH_SETTLE, "{:?}", switched - asked);
    assert_eq!(p.lane(), Lane::Second);
}

#[test]
fn a_camera_with_nothing_to_start_from_is_given_up() {
    let mut t = Instant::now();
    let mut p = running(&mut t);
    let asked = t;
    // Keyframes, but none of its parameter sets ever came
    while t - asked < SWITCH_DEADLINE + DWELL {
        t += STEP;
        let main = p.route(
            Lane::Main,
            Codec::H265,
            &chunk(BIG, None, None),
            true,
            CameraChoice::Second,
            t,
        );
        assert_eq!(main, Route::Send);
        let second = p.route(
            Lane::Second,
            Codec::H265,
            &chunk(SMALL, None, Some(8)),
            false,
            CameraChoice::Second,
            t,
        );
        assert_eq!(second, Route::Drop);
    }
    assert_eq!(p.lane(), Lane::Main);
    // Once they come, it's tried again
    let tried = (0..40).any(|_| {
        let routes = step(
            &mut p,
            &mut t,
            &chunk(SMALL, Some(0), None),
            CameraChoice::Second,
        );
        routes.1 == Route::Switch(0)
    });
    assert!(tried);
}

#[test]
fn choosing_the_other_camera_again_calls_off_a_switch() {
    let mut t = Instant::now();
    let mut p = running(&mut t);
    for _ in 0..6 {
        step(
            &mut p,
            &mut t,
            &chunk(SMALL, None, None),
            CameraChoice::Second,
        );
    }
    let routes = step(
        &mut p,
        &mut t,
        &chunk(SMALL, Some(0), None),
        CameraChoice::Main,
    );
    assert_eq!(routes, (Route::Send, Route::Drop));
    assert_eq!(p.lane(), Lane::Main);
}

#[test]
fn the_only_camera_sending_goes_out_from_its_first_chunk() {
    let t = Instant::now();
    let mut p = Program::new(t);
    let first = p.route(
        Lane::Second,
        Codec::H265,
        &chunk(BIG, Some(0), None),
        false,
        CameraChoice::Auto,
        t + STEP,
    );
    assert_eq!(first, Route::Switch(0));
    assert_eq!(p.lane(), Lane::Second);
}

#[test]
fn h264_starts_only_at_an_sps() {
    let keyframe = chunk(SMALL, None, Some(8));
    assert_eq!(join_at(Codec::H264, &keyframe, true, true), None);
    assert_eq!(join_at(Codec::H265, &keyframe, true, true), Some(8));
    assert_eq!(join_at(Codec::H265, &keyframe, false, true), None);
    assert_eq!(join_at(Codec::H265, &keyframe, true, false), None);
    let sps = chunk(SMALL, Some(3), Some(8));
    assert_eq!(join_at(Codec::H264, &sps, false, false), Some(3));
    assert_eq!(join_at(Codec::H265, &sps, false, false), Some(3));
}

const START: [u8; 4] = [0, 0, 0, 1];

#[test]
fn marks_where_an_h265_stream_can_be_joined() {
    let trail = [&START[..], &[0x02, 0x01, 0xD0, 0x33]].concat();
    // An IDR's second slice, then the start of a picture after parameter sets
    let idr_rest = [&START[..], &[0x26, 0x01, 0x2F, 0x11]].concat();
    let vps = [&START[..], &[0x40, 0x01, 0x0C, 0x01]].concat();
    let sps = [&START[..], &[0x42, 0x01, 0x01, 0x01]].concat();
    let idr = [&START[..], &[0x26, 0x01, 0xAF, 0x11]].concat();

    let data = [trail.as_slice(), &idr_rest, &vps, &sps, &idr].concat();
    let m = marks(Codec::H265, &data);
    assert!(m.picture);
    assert_eq!(m.bytes, data.len());
    assert_eq!(m.params, Some(trail.len() + idr_rest.len()));
    assert_eq!(
        m.keyframe,
        Some(trail.len() + idr_rest.len() + vps.len() + sps.len())
    );

    let m = marks(Codec::H265, &[vps.as_slice(), &sps].concat());
    assert!(!m.picture);
    assert_eq!((m.params, m.keyframe), (Some(0), None));
}

#[test]
fn marks_where_an_h264_stream_can_be_joined() {
    let p = [&START[..], &[0x41, 0x9A, 0x22]].concat();
    let sps = [&START[..], &[0x67, 0x64, 0x00, 0x28]].concat();
    let pps = [&START[..], &[0x68, 0xEE, 0x3C]].concat();
    let idr = [&START[..], &[0x65, 0x88, 0x84]].concat();

    let data = [p.as_slice(), &sps, &pps, &idr].concat();
    let m = marks(Codec::H264, &data);
    assert!(m.picture);
    assert_eq!(m.params, Some(p.len()));
    assert_eq!(m.keyframe, Some(p.len() + sps.len() + pps.len()));
}
