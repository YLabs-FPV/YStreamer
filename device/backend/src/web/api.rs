mod access;
mod display;
mod network;
mod recording;
mod settings;
mod streaming;
mod system;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Serialize;

use crate::input::duml::monitor::Monitor;
use crate::input::duml::session::Session;
use crate::input::goggles::Goggles;
use crate::net::network::Network;
use crate::output::hdmi::GstPlayer;
use crate::output::recording::Recorder;
use crate::output::rtmp::RtmpOutput;
use crate::output::rtsp::RtspServer;
use crate::output::splash::Splash;
use crate::output::srt::SrtOutput;
use crate::output::udp::UdpOutput;
use crate::settings::SettingsStore;
use crate::web::auth::{self, Auth};

#[derive(Clone)]
pub struct Services {
    pub store: Arc<SettingsStore>,
    pub auth: Arc<Auth>,
    pub network: Arc<Network>,
    pub rtsp: Arc<RtspServer>,
    pub srt: Arc<SrtOutput>,
    pub rtmp: Arc<RtmpOutput>,
    pub udp: Arc<UdpOutput>,
    pub player: Arc<GstPlayer>,
    pub recorder: Arc<Recorder>,
    pub transfers: Arc<crate::output::transfer::Transfers>,
    pub button: Arc<crate::system::button::RecordButton>,
    pub goggles: Goggles,
    pub session: Arc<Session>,
    pub monitor: Arc<Monitor>,
    pub splash: Arc<Splash>,
    pub overlay: Arc<crate::output::overlay::Overlay>,
    pub input: Arc<crate::input::InputState>,
    pub metrics: Arc<crate::system::metrics::Metrics>,
    pub uvc: Arc<crate::input::uvc::UvcSource>,
    pub legacy: Arc<crate::input::legacy::LegacySource>,
}

pub fn router<S>(services: Services) -> Router<S> {
    let public = Router::new()
        .route("/auth", get(access::auth_status))
        .route("/auth/login", post(access::login))
        .route("/auth/logout", post(access::logout));

    Router::new()
        .route("/auth/password", put(access::set_password))
        .route("/auth/viewing", put(access::set_viewing))
        .route(
            "/settings",
            get(settings::get_settings).put(settings::put_all),
        )
        .route("/settings/reset", post(settings::reset))
        .route("/settings/{section}", put(settings::put_section))
        .route("/network", get(network::network_status))
        .route("/network/scan", get(network::network_scan))
        .route("/network/countries", get(network::network_countries))
        .route(
            "/wireguard",
            get(network::wireguard_status).delete(network::wireguard_remove),
        )
        .route(
            "/wireguard/config",
            put(network::wireguard_import).layer(DefaultBodyLimit::max(64 << 10)),
        )
        .route("/wireguard/{action}", post(network::wireguard_action))
        .route("/tailscale", get(network::tailscale_status))
        .route("/tailscale/{action}", post(network::tailscale_action))
        .route("/system", get(system::system_info))
        .route("/system/logs", get(system::system_logs))
        .route("/system/ssh", get(system::ssh_status).put(system::ssh_set))
        .route("/system/ssh/password", put(system::ssh_password))
        .route("/system/update", get(system::update_status))
        .route(
            "/system/update/package",
            put(system::update_upload)
                .delete(system::update_discard)
                .layer(DefaultBodyLimit::max(64 << 20)),
        )
        .route("/system/update/check", post(system::update_check))
        .route("/system/update/download", post(system::update_download))
        .route("/system/update/install", post(system::update_install))
        .route("/system/update/rollback", post(system::update_rollback))
        .route("/metrics", get(system::metrics))
        .route("/terminal", get(system::terminal_info))
        .route("/terminal/shell", get(system::terminal_shell))
        .route("/system/{action}", post(system::system_action))
        .route("/recording", get(recording::recording_status))
        .route("/recording/start", post(recording::recording_start))
        .route("/recording/stop", post(recording::recording_stop))
        .route("/recording/button", get(recording::recording_button))
        .route("/recording/files", get(recording::recording_files))
        .route(
            "/recording/files/{name}",
            get(recording::recording_download).delete(recording::recording_delete),
        )
        .route(
            "/recording/storage-options",
            get(recording::recording_storage_options),
        )
        .route("/recording/eject", post(recording::recording_eject))
        .route(
            "/recording/transfer",
            get(recording::transfer_status)
                .post(recording::transfer_start)
                .delete(recording::transfer_cancel),
        )
        .route("/duml", get(system::duml_status).delete(system::duml_clear))
        .route("/duml/send", post(system::duml_send))
        .route("/srt", get(streaming::srt_status))
        .route("/rtmp", get(streaming::rtmp_status))
        .route("/udp", get(streaming::udp_status))
        .route("/display", get(display::display_status))
        .route("/input", get(streaming::input_status))
        .route("/input/devices", get(streaming::input_devices))
        .route(
            "/input/legacy/capture",
            get(streaming::legacy_capture_download).post(streaming::legacy_capture_start),
        )
        .route("/overlay", get(display::overlay_status))
        .route(
            "/overlay/image",
            put(display::overlay_upload)
                .delete(display::overlay_remove)
                .layer(DefaultBodyLimit::max(16 << 20)),
        )
        .route("/splash", get(display::splash_status))
        .route("/splash/preview", get(display::splash_preview))
        .route(
            "/splash/image",
            put(display::splash_upload)
                .delete(display::splash_remove)
                .layer(DefaultBodyLimit::max(16 << 20)),
        )
        .route_layer(axum::middleware::from_fn(run_to_completion))
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&services.auth),
            auth::admin_only,
        ))
        .merge(public)
        .with_state(services)
}

/// A browser that reloads or leaves mid-request makes the server drop the
/// handler, and whatever command it was running gets killed halfway.
/// Anything that changes the device is finished regardless; reads can go
async fn run_to_completion(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if req.method() == axum::http::Method::GET {
        return next.run(req).await;
    }
    tokio::spawn(next.run(req))
        .await
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body {
            error: String,
        }
        (self.0, Json(Body { error: self.1 })).into_response()
    }
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn failed(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg.into())
}

#[cfg(test)]
#[path = "api.test.rs"]
mod tests;
