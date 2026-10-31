use super::*;

fn only(names: &'static [&'static str]) -> impl Fn(&str) -> bool {
    move |n| names.contains(&n)
}

#[test]
fn the_pi_keeps_its_v4l2_codecs() {
    let pi = only(&["v4l2h264dec", "v4l2h264enc", "avdec_h264", "x264enc"]);
    assert_eq!(choose_decoder(Board::RaspberryPi, &pi), Decoder::V4l2);
    assert_eq!(
        choose_encoder(Board::RaspberryPi, &pi),
        Some(HwEncoder::V4l2)
    );
    assert!(Decoder::V4l2.needs_level_patch());
}

/// Radxa's bookworm image: gstreamer1.0-rockchip1, no V4L2 codecs
#[test]
fn a_rockchip_board_uses_mpp() {
    let rock = only(&["mppvideodec", "mpph264enc", "avdec_h264", "x264enc"]);
    assert_eq!(choose_decoder(Board::Rockchip, &rock), Decoder::Mpp);
    assert_eq!(choose_encoder(Board::Rockchip, &rock), Some(HwEncoder::Mpp));
    assert!(!Decoder::Mpp.needs_level_patch());
}

/// The same image with libv4l-rkmpp showing up as V4L2 codecs: still MPP,
/// which is what was run on the board
#[test]
fn a_rockchip_board_prefers_mpp_over_a_v4l2_face_on_it() {
    let both = only(&["v4l2h264dec", "v4l2h264enc", "mppvideodec", "mpph264enc"]);
    assert_eq!(choose_decoder(Board::Rockchip, &both), Decoder::Mpp);
    assert_eq!(choose_encoder(Board::Rockchip, &both), Some(HwEncoder::Mpp));
    // And the Pi stays the Pi
    assert_eq!(choose_decoder(Board::RaspberryPi, &both), Decoder::V4l2);
    assert_eq!(
        choose_encoder(Board::RaspberryPi, &both),
        Some(HwEncoder::V4l2)
    );
}

#[test]
fn without_hardware_it_falls_back_to_software() {
    let plain = only(&["avdec_h264", "x264enc"]);
    for board in [Board::RaspberryPi, Board::Rockchip, Board::Other] {
        assert_eq!(choose_decoder(board, &plain), Decoder::Software);
        assert_eq!(choose_encoder(board, &plain), None);
    }
    assert_eq!(Decoder::Software.element(), "avdec_h264");
}

#[test]
fn encoder_pipelines_carry_the_bitrate_and_keyframe_period() {
    let pi = HwEncoder::V4l2.pipeline(6000, 30);
    assert!(pi.contains("v4l2h264enc"));
    assert!(pi.contains("video_bitrate=6000000"));
    assert!(pi.contains("h264_i_frame_period=30"));
    assert!(pi.ends_with(" ! "));

    let rock = HwEncoder::Mpp.pipeline(6000, 30);
    assert!(rock.contains("mpph264enc bps=6000000 gop=30"));
    assert!(rock.contains("format=NV12"));
    assert!(rock.ends_with(" ! "));
}
