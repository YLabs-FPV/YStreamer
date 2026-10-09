use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{ApiError, Services, bad, failed};

#[derive(Serialize)]
pub(super) struct DisplayStatus {
    modes: Vec<crate::output::display::ModeInfo>,
    current: Option<crate::output::display::ModeInfo>,
    /// Why modes couldn't be listed, e.g. nothing plugged in
    error: Option<String>,
}

pub(super) async fn display_status(State(sv): State<Services>) -> Json<DisplayStatus> {
    let player = Arc::clone(&sv.player);
    let result = tokio::task::spawn_blocking(move || player.modes())
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    Json(match result {
        Ok((modes, current)) => DisplayStatus {
            modes,
            current,
            error: None,
        },
        Err(e) => DisplayStatus {
            modes: Vec::new(),
            current: None,
            error: Some(e),
        },
    })
}

#[derive(Serialize)]
pub(super) struct SplashStatus {
    custom_image: bool,
}

#[derive(Serialize)]
pub(super) struct OverlayStatus {
    /// Pixel size of the uploaded logo, None without one
    image: Option<(u32, u32)>,
    version: u64,
}

pub(super) async fn overlay_status(State(sv): State<Services>) -> Json<OverlayStatus> {
    Json(OverlayStatus {
        image: sv.overlay.logo().map(|l| (l.width, l.height)),
        version: sv.overlay.version(),
    })
}

/// Redraw wherever the logo shows. A USB camera has it encoded in, so its
/// pipeline restarts
pub(super) fn overlay_changed(sv: &Services) {
    if sv.input.mode() == crate::settings::InputMode::Uvc {
        sv.uvc.apply(&sv.store.get().input.uvc);
    }
    sv.player.refresh_logo();
}

pub(super) async fn overlay_upload(
    State(sv): State<Services>,
    image: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    let overlay = Arc::clone(&sv.overlay);
    tokio::task::spawn_blocking(move || overlay.set_image(&image))
        .await
        .map_err(|e| failed(e.to_string()))?
        .map_err(|e| bad(format!("Couldn't use that image: {e}")))?;
    overlay_changed(&sv);
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn overlay_remove(State(sv): State<Services>) -> Result<StatusCode, ApiError> {
    sv.overlay.remove().map_err(failed)?;
    overlay_changed(&sv);
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn splash_status(State(sv): State<Services>) -> Json<SplashStatus> {
    Json(SplashStatus {
        custom_image: sv.splash.has_custom_image(),
    })
}

#[derive(Deserialize)]
pub(super) struct PreviewQuery {
    status_over_image: Option<bool>,
    image_scaling: Option<crate::settings::Scaling>,
}

pub(super) async fn splash_preview(
    State(sv): State<Services>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, ApiError> {
    use axum::http::header;
    let (image, mime) = tokio::task::spawn_blocking(move || {
        sv.splash
            .preview(query.status_over_image, query.image_scaling)
    })
    .await
    .map_err(|e| failed(e.to_string()))?
    .map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-store"),
        ],
        image,
    )
        .into_response())
}

pub(super) async fn splash_upload(
    State(sv): State<Services>,
    image: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    tokio::task::spawn_blocking(move || sv.splash.set_custom_image(&image))
        .await
        .map_err(|e| failed(e.to_string()))?
        .map_err(|e| bad(format!("Couldn't use that image: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn splash_remove(State(sv): State<Services>) -> Result<StatusCode, ApiError> {
    sv.splash
        .remove_custom_image()
        .map_err(|e| failed(format!("Couldn't remove the image: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}
