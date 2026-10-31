//! The H.264 hardware this board has, from the GStreamer registry: the Pi 4's
//! V4L2 codecs, or the Rockchip MPP ones on a ROCK 3A

use std::sync::OnceLock;

use gstreamer as gst;

use crate::system::board::Board;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoder {
    /// The Pi 4's
    V4l2,
    /// Rockchip MPP, from gstreamer1.0-rockchip1
    Mpp,
    /// libav, which keeps up with 1080p on a quiet core but not more
    Software,
}

impl Decoder {
    pub fn element(self) -> &'static str {
        match self {
            Self::V4l2 => "v4l2h264dec",
            Self::Mpp => "mppvideodec",
            Self::Software => "avdec_h264",
        }
    }

    /// The Pi 4's decoder refuses a stream whose SPS claims a level above
    /// 4.2, which the goggles' 4K modes do
    pub fn needs_level_patch(self) -> bool {
        self == Self::V4l2
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwEncoder {
    /// The Pi 4's
    V4l2,
    /// Rockchip MPP
    Mpp,
}

impl HwEncoder {
    pub fn element(self) -> &'static str {
        match self {
            Self::V4l2 => "v4l2h264enc",
            Self::Mpp => "mpph264enc",
        }
    }

    /// Raw video in, H.264 out, ending in ` ! `. SPS and PPS before every
    /// keyframe are the parser's job downstream, so a stream or recording
    /// can start at any of them
    pub fn pipeline(self, bitrate_kbps: u32, keyframe_period: u32) -> String {
        match self {
            Self::V4l2 => format!(
                "videoconvert ! video/x-raw,format=I420 ! \
                 v4l2h264enc extra-controls=\"controls,video_bitrate={},h264_i_frame_period={keyframe_period},repeat_sequence_header=1\" ! \
                 video/x-h264,level=(string)4,profile=(string)high ! ",
                bitrate_kbps * 1000,
            ),
            // NV12 is what MPP takes natively; anything else gets converted
            // on the way in
            Self::Mpp => format!(
                "videoconvert ! video/x-raw,format=NV12 ! \
                 mpph264enc bps={} gop={keyframe_period} ! ",
                bitrate_kbps * 1000,
            ),
        }
    }
}

static DECODER: OnceLock<Decoder> = OnceLock::new();
static ENCODER: OnceLock<Option<HwEncoder>> = OnceLock::new();

/// The decoder HDMI uses, decided once
pub fn decoder() -> Decoder {
    *DECODER.get_or_init(|| {
        let d = choose_decoder(Board::detect(), available);
        println!("[codec] decoding with {}", d.element());
        d
    })
}

/// The encoder for USB cameras, if the board has one
pub fn hw_encoder() -> Option<HwEncoder> {
    *ENCODER.get_or_init(|| {
        let e = choose_encoder(Board::detect(), available);
        match e {
            Some(e) => println!("[codec] encoding with {}", e.element()),
            None => println!("[codec] no hardware encoder, USB cameras encode in software"),
        }
        e
    })
}

fn available(element: &str) -> bool {
    // Idempotent; the HDMI output normally gets there first
    let _ = gst::init();
    gst::ElementFactory::find(element).is_some()
}

/// The board's own first. Radxa's images carry libv4l-rkmpp, a V4L2 face on
/// MPP, so the Pi's element names can turn up there too; the MPP elements
/// are the ones the bring-up ran
fn choose_decoder(board: Board, has: impl Fn(&str) -> bool) -> Decoder {
    let order = match board {
        Board::Rockchip => [Decoder::Mpp, Decoder::V4l2],
        _ => [Decoder::V4l2, Decoder::Mpp],
    };
    order
        .into_iter()
        .find(|d| has(d.element()))
        .unwrap_or(Decoder::Software)
}

fn choose_encoder(board: Board, has: impl Fn(&str) -> bool) -> Option<HwEncoder> {
    let order = match board {
        Board::Rockchip => [HwEncoder::Mpp, HwEncoder::V4l2],
        _ => [HwEncoder::V4l2, HwEncoder::Mpp],
    };
    order.into_iter().find(|e| has(e.element()))
}

#[cfg(test)]
#[path = "codec.test.rs"]
mod tests;
