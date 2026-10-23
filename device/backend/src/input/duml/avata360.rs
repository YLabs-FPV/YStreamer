//! The Avata 360's lens field of view and Vision Assist, as DJI Fly sends
//! them to our own aircraft. The goggles don't report this aircraft by a code
//! YStreamer knows, so these are offered when the user says it's an Avata 360

use super::{FLAGS_REQUEST, encode};

const CAMERA: u8 = 0x01;
/// Type 0x12 at index 4
const VISION: u8 = 0x92;
const VIDEO_ENCODER: u8 = 0x08;

const CMD_SET_CAMERA: u8 = 0x02;
const CMD_LENS_FOV: u8 = 0xFF;
const CMD_SET_FC: u8 = 0x03;
/// Writes a parameter addressed by the hash of its name
const CMD_WRITE_PARAM: u8 = 0xF9;
const CMD_SET_VIDEO_ENCODER: u8 = 0x08;
const CMD_STREAM: u8 = 0x69;

const PARAM_VISION_ASSIST: u32 = 0x6CD3_361E;

/// 104° is the flattest view, 272° the whole sphere
pub const FOV_MIN: f32 = 104.0;
pub const FOV_MAX: f32 = 272.0;

/// The field of view in hundredths of a degree, at offset 16 of an
/// otherwise fixed payload. DJI Fly sends one per step of its zoom control
pub fn lens_fov(deg: f32, seq: u16) -> Vec<u8> {
    let centi = (deg.clamp(FOV_MIN, FOV_MAX) * 100.0).round() as u16;
    let mut payload = [0u8; 19];
    payload[..5].copy_from_slice(&[0x00, 0x39, 0x00, 0x01, 0x0D]);
    payload[16..18].copy_from_slice(&centi.to_le_bytes());
    encode(
        CAMERA,
        seq,
        FLAGS_REQUEST,
        CMD_SET_CAMERA,
        CMD_LENS_FOV,
        &payload,
    )
}

/// The first of the two frames DJI Fly sends: the setting itself. On its own
/// the aircraft takes it but doesn't start the second camera
pub fn vision_assist_param(on: bool, seq: u16) -> Vec<u8> {
    let mut payload = [0u8; 5];
    payload[..4].copy_from_slice(&PARAM_VISION_ASSIST.to_le_bytes());
    payload[4] = on as u8;
    encode(
        VISION,
        seq,
        FLAGS_REQUEST,
        CMD_SET_FC,
        CMD_WRITE_PARAM,
        &payload,
    )
}

/// The second: the video encoder starts the second camera's stream. Turning
/// it off clears the same byte, which hasn't been recorded from DJI Fly yet
pub fn vision_assist_stream(on: bool, seq: u16) -> Vec<u8> {
    let payload = [0x00, 0x00, 0x02, 0x00, 0x5A, 0x00, on as u8, 0x0A, 0x00];
    encode(
        VIDEO_ENCODER,
        seq,
        FLAGS_REQUEST,
        CMD_SET_VIDEO_ENCODER,
        CMD_STREAM,
        &payload,
    )
}

#[cfg(test)]
#[path = "avata360.test.rs"]
mod tests;
