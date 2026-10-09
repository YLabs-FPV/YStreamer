use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::output::rtmp::RtmpStatus;
use crate::output::srt::SrtStatus;
use crate::output::udp::UdpStatus;

use super::{ApiError, Services, bad, failed};

#[derive(Serialize)]
pub(super) struct InputStatus {
    mode: crate::settings::InputMode,
    uvc: crate::input::uvc::UvcStatus,
    legacy: crate::input::legacy::LegacyStatus,
}

pub(super) async fn input_status(State(sv): State<Services>) -> Json<InputStatus> {
    Json(InputStatus {
        mode: sv.input.mode(),
        uvc: sv.uvc.status(),
        legacy: sv.legacy.status(),
    })
}

/// A developer tool for the experimental FPV Goggles V1/V2 input
pub(super) async fn legacy_capture_start(
    State(sv): State<Services>,
) -> Result<StatusCode, ApiError> {
    sv.legacy.start_capture().map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn legacy_capture_download() -> Result<Response, ApiError> {
    let data = tokio::fs::read(crate::input::legacy::CAPTURE_PATH)
        .await
        .map_err(|_| bad("Nothing has been captured yet"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"fpv-goggles-capture.h264\"",
            ),
        ],
        data,
    )
        .into_response())
}

/// USB video devices plugged in, with what they can do. Probes each one,
/// so it's not for polling
pub(super) async fn input_devices() -> Json<Vec<crate::input::uvc::UvcDevice>> {
    Json(
        tokio::task::spawn_blocking(crate::input::uvc::list_devices)
            .await
            .unwrap_or_default(),
    )
}

pub(super) async fn udp_status(State(sv): State<Services>) -> Json<UdpStatus> {
    Json(sv.udp.status())
}

pub(super) async fn rtmp_status(State(sv): State<Services>) -> Json<RtmpStatus> {
    Json(sv.rtmp.status())
}

pub(super) async fn srt_status(State(sv): State<Services>) -> Json<SrtStatus> {
    Json(sv.srt.status())
}
