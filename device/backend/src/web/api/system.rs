use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::system;

use super::{ApiError, Services, bad, failed};

pub(super) async fn system_info(State(sv): State<Services>) -> impl IntoResponse {
    Json(system::info(sv.store.path()).await)
}

pub(super) fn terminal_refusal(sv: &Services) -> Option<&'static str> {
    if !sv.store.get().advanced.terminal {
        Some("The terminal is switched off")
    } else if !sv.auth.enabled() {
        // It gives root on this device: not to everyone on the network. The
        // login cookie also keeps other websites out: browsers let any page
        // open a socket here, but don't send it our cookie
        Some("Set a password under System → General → Access before using the terminal")
    } else {
        None
    }
}

#[derive(Serialize)]
pub(super) struct TerminalInfo {
    /// Set when the terminal can't be used, saying why
    refused: Option<&'static str>,
    users: Vec<String>,
}

pub(super) async fn terminal_info(State(sv): State<Services>) -> impl IntoResponse {
    Json(TerminalInfo {
        refused: terminal_refusal(&sv),
        users: crate::web::terminal::info().users,
    })
}

#[derive(Deserialize)]
pub(super) struct ShellQuery {
    user: String,
    cols: u16,
    rows: u16,
}

pub(super) async fn terminal_shell(
    State(sv): State<Services>,
    Query(q): Query<ShellQuery>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(reason) = terminal_refusal(&sv) {
        return Err(ApiError(StatusCode::FORBIDDEN, reason.into()));
    }
    if !crate::web::terminal::info().users.contains(&q.user) {
        return Err(bad("No such account to open a shell as"));
    }
    Ok(ws.on_upgrade(move |socket| crate::web::terminal::serve(socket, q.user, q.cols, q.rows)))
}

#[derive(Deserialize)]
pub(super) struct MetricsQuery {
    #[serde(default)]
    after: u64,
}

pub(super) async fn metrics(
    State(sv): State<Services>,
    Query(q): Query<MetricsQuery>,
) -> impl IntoResponse {
    Json(sv.metrics.snapshot(q.after))
}

#[derive(Deserialize)]
pub(super) struct LogsQuery {
    #[serde(default = "default_lines")]
    lines: u32,
}

pub(super) fn default_lines() -> u32 {
    300
}

pub(super) async fn system_logs(Query(q): Query<LogsQuery>) -> Result<String, ApiError> {
    system::logs(q.lines)
        .await
        .map_err(|e| ApiError(StatusCode::CONFLICT, e))
}

pub(super) async fn duml_status(State(sv): State<Services>) -> impl IntoResponse {
    #[derive(Serialize)]
    struct Body {
        session: crate::input::duml::session::SessionStatus,
        frames: Vec<crate::input::duml::monitor::FrameStats>,
    }
    Json(Body {
        session: sv.session.status(),
        frames: sv.monitor.snapshot(),
    })
}

pub(super) async fn duml_clear(State(sv): State<Services>) -> StatusCode {
    sv.monitor.clear();
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub(super) struct DumlRequest {
    dst: u8,
    cmd_set: u8,
    cmd_id: u8,
    /// Hex, spaces allowed
    #[serde(default)]
    payload: String,
}

/// A developer tool: any reply shows up in the monitor as a response row
pub(super) async fn duml_send(
    State(sv): State<Services>,
    Json(req): Json<DumlRequest>,
) -> Result<StatusCode, ApiError> {
    use crate::input::duml::{FLAGS_REQUEST, encode, next_seq};

    let hex: String = req.payload.split_whitespace().collect();
    let payload = hex::decode(hex).map_err(|_| bad("Payload must be hex bytes, e.g. 01 0a ff"))?;
    // The frame length field is 10 bits and covers 13 bytes of framing
    if payload.len() > 1010 {
        return Err(bad("Payload is too long for one frame"));
    }
    let frame = encode(
        req.dst,
        next_seq(),
        FLAGS_REQUEST,
        req.cmd_set,
        req.cmd_id,
        &payload,
    );
    sv.goggles.send(&frame).map_err(|e| failed(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn ssh_status(State(sv): State<Services>) -> impl IntoResponse {
    Json(system::ssh::status(sv.store.path()).await)
}

#[derive(Deserialize)]
pub(super) struct SshSwitch {
    enabled: bool,
}

pub(super) async fn ssh_set(
    State(sv): State<Services>,
    Json(req): Json<SshSwitch>,
) -> Result<impl IntoResponse, ApiError> {
    if !system::ssh::status(sv.store.path()).await.available {
        return Err(bad("SSH isn't managed from here on this system"));
    }
    system::ssh::set_enabled(req.enabled)
        .await
        .map_err(failed)?;
    Ok(Json(system::ssh::status(sv.store.path()).await))
}

#[derive(Deserialize)]
pub(super) struct SshPassword {
    password: String,
}

pub(super) async fn ssh_password(
    State(sv): State<Services>,
    Json(req): Json<SshPassword>,
) -> Result<impl IntoResponse, ApiError> {
    if !system::ssh::status(sv.store.path()).await.available {
        return Err(bad("SSH isn't managed from here on this system"));
    }
    system::ssh::set_password(sv.store.path(), &req.password)
        .await
        .map_err(bad)?;
    Ok(Json(system::ssh::status(sv.store.path()).await))
}

pub(super) async fn system_action(
    State(sv): State<Services>,
    Path(action): Path<String>,
) -> Result<StatusCode, ApiError> {
    // Runs in-process, so unlike the others it doesn't need systemd
    if action == "restart-usb" {
        sv.goggles.reset();
        return Ok(StatusCode::ACCEPTED);
    }
    let action = system::Action::parse(&action)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("Unknown action '{action}'")))?;
    system::schedule(action).map_err(|e| ApiError(StatusCode::CONFLICT, e))?;
    Ok(StatusCode::ACCEPTED)
}

pub(super) async fn update_status() -> impl IntoResponse {
    Json(system::update::status().await)
}

#[derive(Deserialize)]
pub(super) struct PackageQuery {
    signature: String,
}

pub(super) async fn update_upload(
    Query(q): Query<PackageQuery>,
    deb: axum::body::Bytes,
) -> Result<impl IntoResponse, ApiError> {
    system::update::stage(&deb, &q.signature)
        .await
        .map_err(bad)?;
    Ok(Json(system::update::status().await))
}

pub(super) async fn update_discard() -> impl IntoResponse {
    system::update::discard();
    Json(system::update::status().await)
}

pub(super) async fn update_install() -> Result<StatusCode, ApiError> {
    system::update::install()
        .await
        .map_err(|e| ApiError(StatusCode::CONFLICT, e))?;
    Ok(StatusCode::ACCEPTED)
}

pub(super) async fn update_rollback() -> Result<StatusCode, ApiError> {
    system::update::rollback()
        .await
        .map_err(|e| ApiError(StatusCode::CONFLICT, e))?;
    Ok(StatusCode::ACCEPTED)
}

pub(super) async fn update_check() -> Result<impl IntoResponse, ApiError> {
    let release = system::update::check()
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(release))
}

pub(super) async fn update_download() -> Result<impl IntoResponse, ApiError> {
    system::update::download()
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(system::update::status().await))
}
