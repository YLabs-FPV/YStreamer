use super::*;

use std::mem::{offset_of, size_of};

#[test]
fn frame_layout_matches_the_sdk() {
    // NDIlib_video_frame_v2_t on a 64-bit build
    assert_eq!(size_of::<runtime::VideoFrame>(), 72);
    assert_eq!(offset_of!(runtime::VideoFrame, four_cc), 8);
    assert_eq!(offset_of!(runtime::VideoFrame, frame_format_type), 24);
    assert_eq!(offset_of!(runtime::VideoFrame, timecode), 32);
    assert_eq!(offset_of!(runtime::VideoFrame, p_data), 40);
    assert_eq!(offset_of!(runtime::VideoFrame, line_stride_in_bytes), 48);
    assert_eq!(offset_of!(runtime::VideoFrame, p_metadata), 56);
    assert_eq!(offset_of!(runtime::VideoFrame, timestamp), 64);
}

#[test]
fn nv12_four_cc_reads_as_its_name() {
    assert_eq!(runtime::FOURCC_NV12.to_le_bytes(), *b"NV12");
}

#[test]
fn off_by_default() {
    let shared = SharedDecoder::new();
    let ndi = NdiOutput::new(&NetworkSourceSettings::default(), Arc::clone(&shared));
    let status = ndi.status();
    assert!(!status.active);
    assert_eq!(status.error, None);
    // Nothing to decode for
    assert!(shared.pipeline().is_none());
}

/// Runs against an NDI runtime when one is installed, e.g. on a Pi set up
/// for it; skips everywhere else
#[test]
fn sends_through_an_installed_runtime() {
    let Ok(rt) = runtime::load() else {
        eprintln!("skipping: no NDI runtime");
        return;
    };
    let sender = runtime::Sender::new(rt, "YStreamer test").unwrap();
    let r = Running {
        sender: Arc::new(sender),
        limit: Limit {
            max_height: 720,
            max_fps: 30,
        },
        size: Arc::new(AtomicU64::new(0)),
        idle_since: None,
    };
    let (y, uv) = (vec![16u8; 64 * 32], vec![128u8; 64 * 16]);
    let mut send = sending(&r);
    for _ in 0..3 {
        send(&Picture {
            width: 64,
            height: 32,
            y: &y,
            y_stride: 64,
            uv: &uv,
            uv_stride: 64,
            frame_rate: None,
        });
    }
    assert_eq!(r.size.load(Ordering::Relaxed), 64 << 32 | 32);
    assert_eq!(r.sender.connections(), 0);
}
