use super::*;

fn snapshot() -> Vec<u8> {
    vec![0u8; 28]
}

#[test]
fn reads_the_goggles_battery() {
    let s = parse_duml_response(6, 30, &[0, 0, 0, 0, 87]).unwrap();
    assert_eq!(s.goggles_battery, Some(87));
    assert_eq!(s.iso, None);
}

#[test]
fn ignores_frames_it_does_not_know() {
    assert!(parse_duml_response(6, 31, &[0; 8]).is_none());
    assert!(parse_duml_response(3, 0x43, &[0; 64]).is_none());
    assert!(parse_duml_response(CMD_SET, 128, &[0; 64]).is_none());
}

#[test]
fn ignores_frames_too_short_to_hold_the_value() {
    assert!(parse_duml_response(6, 30, &[0, 0, 0, 0]).is_none());
    assert!(parse_duml_response(CMD_SET, 129, &[0; 24]).is_none());
}

#[test]
fn reads_manual_exposure_and_white_balance() {
    let mut p = snapshot();
    p[14] = 60;
    p[20] = 0x04;
    p[23] = 0x06;
    p[24] = 56;
    p[27] = 0xFF;
    let s = parse_duml_response(CMD_SET, 129, &p).unwrap();
    assert_eq!(s.fps, Some(60));
    assert_eq!(s.exposure_mode.as_deref(), Some("manual"));
    assert_eq!(s.wb_auto, Some(false));
    assert_eq!(s.wb_temp, Some(5600));
    assert_eq!(s.sharpness, Some(-1));
    assert_eq!(s.goggles_battery, None);
}

#[test]
fn reads_auto_exposure_and_white_balance() {
    let mut p = snapshot();
    p[20] = 0x01;
    p[23] = 0x00;
    p[24] = 56;
    let s = parse_duml_response(CMD_SET, 129, &p).unwrap();
    assert_eq!(s.exposure_mode.as_deref(), Some("auto"));
    assert_eq!(s.wb_auto, Some(true));
    // what the camera settled on
    assert_eq!(s.wb_temp, Some(5600));
    // auto ISO comes as 0
    assert_eq!(s.iso, Some(0));
}

#[test]
fn leaves_out_fields_past_the_end_of_a_short_snapshot() {
    let s = parse_duml_response(CMD_SET, 129, &[0; 25]).unwrap();
    assert_eq!(s.sharpness, None);
    assert_eq!(s.noise_reduction, None);

    let mut long = vec![0u8; 104];
    long[103] = 0xFE;
    let s = parse_duml_response(CMD_SET, 129, &long).unwrap();
    assert_eq!(s.noise_reduction, Some(-2));
}

#[test]
fn reads_color_profile_and_anti_flicker() {
    let mut p = vec![0u8; 33];
    let s = parse_duml_response(CMD_SET, 129, &p).unwrap();
    assert_eq!(s.color_profile, Some("normal"));
    assert_eq!(s.anti_flicker, Some("auto"));

    p[31] = 0x3D;
    p[32] = 2;
    let s = parse_duml_response(CMD_SET, 129, &p).unwrap();
    assert_eq!(s.color_profile, Some("dlog"));
    assert_eq!(s.anti_flicker, Some("50hz"));

    p[31] = 0x07;
    p[32] = 9;
    let s = parse_duml_response(CMD_SET, 129, &p).unwrap();
    assert_eq!(s.color_profile, None);
    assert_eq!(s.anti_flicker, None);

    let s = parse_duml_response(CMD_SET, 129, &[0; 32]).unwrap();
    assert_eq!(s.color_profile, None);
}

#[test]
fn builds_color_profile_and_anti_flicker_commands() {
    let cam = CameraCommands::new();
    // 8 bytes of link header, then 11 of DUML header before the payload
    let body = |f: Vec<u8>| (f[8 + 9], f[8 + 10], f[8 + 11..f.len() - 2].to_vec());

    assert_eq!(
        body(cam.build_color_profile("dlog").unwrap()),
        (CMD_SET, 66, vec![0x3D])
    );
    assert_eq!(
        body(cam.build_color_profile("normal").unwrap()),
        (CMD_SET, 66, vec![0x00])
    );
    assert_eq!(
        body(cam.build_anti_flicker("60hz").unwrap()),
        (CMD_SET, 70, vec![1])
    );
    assert_eq!(
        body(cam.build_anti_flicker("off").unwrap()),
        (CMD_SET, 70, vec![3])
    );
    assert!(cam.build_color_profile("hlg").is_none());
    assert!(cam.build_anti_flicker("55hz").is_none());
}

// Replies captured from an O3 Air Unit, exposure and white balance on auto
#[test]
fn reads_replies_from_an_o3() {
    let s = parse_get_reply(0x1F, &[0x00, 0x01, 0x00]).unwrap();
    assert_eq!(s.exposure_mode.as_deref(), Some("auto"));

    let s = parse_get_reply(0x19, &[0x00, 0x62, 0x06, 0x00, 0x34, 0x12]).unwrap();
    assert_eq!(s.res.as_deref(), Some("1080p"));
    assert_eq!(s.ar.as_deref(), Some("4:3"));
    assert_eq!(s.fps, Some(6));

    assert_eq!(parse_get_reply(0x2B, &[0x00, 0x00]).unwrap().iso, Some(0));

    let s = parse_get_reply(0x2D, &[0x00, 0x00, 0x32, 0x00, 0xFF, 0xFF]).unwrap();
    assert_eq!(s.wb_auto, Some(true));
    assert_eq!(s.wb_temp, None);

    assert_eq!(parse_get_reply(0x2F, &[0x00, 0x10]).unwrap().ev, Some(0.0));
    assert_eq!(
        parse_get_reply(0x39, &[0x00, 0x00]).unwrap().sharpness,
        Some(0)
    );
    assert_eq!(
        parse_get_reply(0x45, &[0x00, 0x00])
            .unwrap()
            .noise_reduction,
        Some(0)
    );
    assert_eq!(
        parse_get_reply(0x43, &[0x00, 0x00]).unwrap().color_profile,
        Some("normal")
    );
    assert_eq!(
        parse_get_reply(0x47, &[0x00, 0x00]).unwrap().anti_flicker,
        Some("auto")
    );
}

#[test]
fn ignores_refused_and_unknown_replies() {
    // The O3's answer to a shutter request while on auto exposure
    assert!(parse_get_reply(0x29, &[0xDF]).is_none());
    assert!(parse_get_reply(0x2B, &[0x00]).is_none());
    assert!(parse_get_reply(0x2B, &[]).is_none());
    // Acknowledgements of our own set commands
    assert!(parse_get_reply(0x2A, &[0x00, 0x04]).is_none());
    assert!(parse_get_reply(0x00, &[0x00, 0x00]).is_none());
}

#[test]
fn reads_and_sets_the_o3_flat_profile() {
    let s = parse_get_reply(0x43, &[0x00, 0x06]).unwrap();
    assert_eq!(s.color_profile, Some("dcinelike"));
    // which the O3 pairs with less noise reduction
    assert_eq!(
        parse_get_reply(0x45, &[0x00, 0xFF])
            .unwrap()
            .noise_reduction,
        Some(-1)
    );

    let f = CameraCommands::new()
        .build_color_profile("dcinelike")
        .unwrap();
    assert_eq!((f[8 + 10], f[8 + 11]), (66, 0x06));
}

#[test]
fn reads_manual_values_from_replies() {
    assert_eq!(parse_get_reply(0x2B, &[0x00, 0x05]).unwrap().iso, Some(400));
    let s = parse_get_reply(0x1F, &[0x00, 0x04, 0x00]).unwrap();
    assert_eq!(s.exposure_mode.as_deref(), Some("manual"));
    let s = parse_get_reply(0x2D, &[0x00, 0x06, 0x38, 0x00, 0xFF, 0xFF]).unwrap();
    assert_eq!(s.wb_auto, Some(false));
    assert_eq!(s.wb_temp, Some(5600));
}

#[test]
fn refuses_noise_reduction_the_camera_would_wrap() {
    let cam = CameraCommands::new();
    let f = cam.build_noise_reduction(-2).unwrap();
    assert_eq!((f[8 + 10], f[8 + 11]), (68, 0xFE));
    assert!(cam.build_noise_reduction(1).is_some());
    assert!(cam.build_noise_reduction(2).is_none());
    assert!(cam.build_noise_reduction(-3).is_none());
}

#[test]
fn builds_the_slow_shutter_speeds() {
    let cam = CameraCommands::new();
    for speed in [30, 40, 50, 60, 80, 100, 250, 8000] {
        let f = cam.build_shutter(speed).unwrap();
        let wire = u16::from_le_bytes([f[8 + 12], f[8 + 13]]);
        assert_eq!(wire as u32, 32768 + speed, "1/{speed}");
    }
    assert!(cam.build_shutter(25).is_none());
    assert!(cam.build_shutter(90).is_none());
}

// Values captured from an O4 Air Unit Pro on manual exposure and auto
// white balance, in a dark and then a brighter scene
#[test]
fn reads_the_metered_exposure_and_the_white_balance_in_use() {
    let mut p = vec![0u8; 48];
    p[6] = 0x10;
    p[20] = 0x04;
    p[23] = 0x00;
    p[24] = 0x27;
    p[47] = 0x07;
    let s = parse_duml_response(CMD_SET, 129, &p).unwrap();
    assert_eq!(s.exposure_mode.as_deref(), Some("manual"));
    assert_eq!(s.wb_auto, Some(true));
    assert_eq!(s.wb_temp, Some(3900));
    assert_eq!(s.ev, Some(0.0));
    assert_eq!(s.ev_metered, Some(-3.0));

    p[24] = 0x2E;
    p[47] = 0x0F;
    let s = parse_duml_response(CMD_SET, 129, &p).unwrap();
    assert_eq!(s.wb_temp, Some(4600));
    assert_eq!(s.ev_metered, Some(-0.3));

    assert_eq!(
        parse_duml_response(CMD_SET, 129, &p[..47])
            .unwrap()
            .ev_metered,
        None
    );
}
