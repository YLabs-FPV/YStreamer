use super::*;

fn mode(w: u32, h: u32, fps: u32, formats: &[Format]) -> UvcMode {
    UvcMode {
        id: format!("{w}x{h}@{fps}"),
        width: w,
        height: h,
        fps,
        format: formats[0],
        formats: formats
            .iter()
            .map(|f| (*f, (*f == Format::Raw).then(|| "YUY2".to_string())))
            .collect(),
    }
}

fn encoders(attempts: &[Attempt]) -> Vec<(String, Format, Encoder)> {
    attempts
        .iter()
        .map(|a| (a.mode.id.clone(), a.format, a.encoder))
        .collect()
}

/// A capture stick with its own H.264 and MJPEG, on the Pi
#[test]
fn with_a_hardware_encoder_the_rounds_are_hardware_then_software() {
    let modes = [mode(1920, 1080, 30, &[Format::H264, Format::Mjpeg])];
    let got = encoders(&attempts(&modes, "", true));
    assert_eq!(
        got,
        vec![
            ("1920x1080@30".into(), Format::H264, Encoder::Passthrough),
            ("1920x1080@30".into(), Format::Mjpeg, Encoder::Hardware),
        ]
    );
    // 1080p is past what software encoding keeps up with: no second round
}

/// The same stick on a board without a hardware encoder: its H.264 passes
/// through at full size; MJPEG only where software encoding copes
#[test]
fn without_a_hardware_encoder_passthrough_still_comes_first() {
    let modes = [
        mode(1920, 1080, 30, &[Format::H264, Format::Mjpeg]),
        mode(1280, 720, 30, &[Format::H264, Format::Mjpeg, Format::Raw]),
    ];
    let got = encoders(&attempts(&modes, "", false));
    assert_eq!(
        got,
        vec![
            ("1920x1080@30".into(), Format::H264, Encoder::Passthrough),
            ("1280x720@30".into(), Format::Mjpeg, Encoder::Software),
            ("1280x720@30".into(), Format::Raw, Encoder::Software),
        ]
    );
}

/// A webcam without H.264 never gets a hardware attempt it can't have
#[test]
fn a_raw_camera_without_hardware_is_software_only() {
    let modes = [mode(1280, 720, 30, &[Format::Mjpeg, Format::Raw])];
    let got = encoders(&attempts(&modes, "", false));
    assert_eq!(
        got,
        vec![
            ("1280x720@30".into(), Format::Mjpeg, Encoder::Software),
            ("1280x720@30".into(), Format::Raw, Encoder::Software),
        ]
    );
    let got = encoders(&attempts(&modes, "", true));
    assert_eq!(got[0].2, Encoder::Hardware);
    assert_eq!(got.len(), 4);
}
