use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::output::ndi::{NdiStatus, install};
use crate::output::omt::OmtStatus;
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

pub(super) async fn ndi_status(State(sv): State<Services>) -> Json<NdiStatus> {
    Json(
        tokio::task::spawn_blocking(move || {
            let mut status = sv.ndi.status();
            status.installed = status.installed || sv.ndi.installed();
            status
        })
        .await
        .unwrap_or_default(),
    )
}

pub(super) async fn omt_status(State(sv): State<Services>) -> Json<OmtStatus> {
    Json(sv.omt.status())
}

pub(super) async fn rtmp_status(State(sv): State<Services>) -> Json<RtmpStatus> {
    Json(sv.rtmp.status())
}

pub(super) async fn srt_status(State(sv): State<Services>) -> Json<SrtStatus> {
    Json(sv.srt.status())
}

pub(super) async fn ndi_runtime() -> Json<install::Status> {
    Json(install::status())
}

pub(super) async fn ndi_runtime_download() -> Result<Json<install::Status>, ApiError> {
    install::download()
        .await
        .map(Json)
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))
}

/// Streamed to disk: the SDK is over 60 MB
pub(super) async fn ndi_runtime_upload(
    body: axum::body::Body,
) -> Result<Json<install::Status>, ApiError> {
    install::upload(body.into_data_stream())
        .await
        .map(Json)
        .map_err(bad)
}

/// The owner declined NDI's licence
pub(super) async fn ndi_runtime_discard() -> Result<Json<install::Status>, ApiError> {
    install::discard()
        .await
        .map(Json)
        .map_err(|e| ApiError(StatusCode::CONFLICT, e))
}

/// The owner accepted NDI's licence
pub(super) async fn ndi_runtime_install() -> Result<StatusCode, ApiError> {
    install::install().await.map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}
