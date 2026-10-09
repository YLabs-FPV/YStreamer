use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use super::{ApiError, Services, bad, failed};

pub(super) async fn network_status(State(sv): State<Services>) -> impl IntoResponse {
    Json(sv.network.status().await)
}

pub(super) async fn network_countries() -> impl IntoResponse {
    Json(crate::net::network::countries().await)
}

pub(super) async fn wireguard_status() -> impl IntoResponse {
    Json(crate::net::wireguard::status().await)
}

pub(super) async fn wireguard_import(
    config: axum::body::Bytes,
) -> Result<impl IntoResponse, ApiError> {
    crate::net::wireguard::import(&config)
        .await
        .map(Json)
        .map_err(bad)
}

pub(super) async fn wireguard_action(
    Path(action): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    use crate::net::wireguard;
    let result = match action.as_str() {
        "connect" => wireguard::connect().await,
        "disconnect" => wireguard::disconnect().await,
        _ => {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                format!("Unknown action '{action}'"),
            ));
        }
    };
    result.map(Json).map_err(failed)
}

pub(super) async fn wireguard_remove() -> Result<impl IntoResponse, ApiError> {
    crate::net::wireguard::remove()
        .await
        .map(Json)
        .map_err(failed)
}

pub(super) async fn tailscale_status() -> impl IntoResponse {
    Json(crate::net::tailscale::status().await)
}

pub(super) async fn tailscale_action(
    State(sv): State<Services>,
    Path(action): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    use crate::net::tailscale;
    let result = match action.as_str() {
        "connect" => tailscale::connect(&sv.store.get().device.hostname).await,
        "disconnect" => tailscale::disconnect().await,
        "logout" => tailscale::logout().await,
        _ => {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                format!("Unknown action '{action}'"),
            ));
        }
    };
    result.map(Json).map_err(failed)
}

pub(super) async fn network_scan() -> Result<impl IntoResponse, ApiError> {
    crate::net::network::scan().await.map(Json).map_err(failed)
}
