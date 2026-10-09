use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::web::auth::{Access, Auth};

use super::{ApiError, Services, bad};

#[derive(Serialize)]
pub(super) struct AuthStatus {
    enabled: bool,
    protect_viewing: bool,
    access: Access,
    /// How to fetch the video: viewers can't read the settings for it
    transport: crate::settings::ViewerTransport,
}

pub(super) async fn auth_status(
    State(sv): State<Services>,
    headers: HeaderMap,
) -> Json<AuthStatus> {
    Json(AuthStatus {
        enabled: sv.auth.enabled(),
        protect_viewing: sv.auth.protect_viewing(),
        access: sv.auth.access(&headers),
        transport: sv.store.get().viewer.transport,
    })
}

#[derive(Deserialize)]
pub(super) struct Login {
    password: String,
}

pub(super) async fn login(
    State(sv): State<Services>,
    Json(body): Json<Login>,
) -> Result<Response, ApiError> {
    if !sv.auth.enabled() {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if !sv.auth.check_password(body.password).await {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Wrong password".into()));
    }
    Ok(with_cookie(sv.auth.session_cookie()))
}

pub(super) async fn logout() -> Response {
    with_cookie(Auth::clear_cookie())
}

#[derive(Deserialize)]
pub(super) struct PasswordChange {
    #[serde(default)]
    current: String,
    /// None removes the password
    new: Option<String>,
}

pub(super) async fn set_password(
    State(sv): State<Services>,
    Json(body): Json<PasswordChange>,
) -> Result<Response, ApiError> {
    // A session alone isn't enough, so a browser left logged in can't lock
    // the owner out
    if sv.auth.enabled() && !sv.auth.check_password(body.current).await {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Current password is wrong".into(),
        ));
    }
    let removing = body.new.is_none();
    sv.auth.set_password(body.new).await.map_err(bad)?;
    Ok(with_cookie(if removing {
        Auth::clear_cookie()
    } else {
        sv.auth.session_cookie()
    }))
}

#[derive(Deserialize)]
pub(super) struct Viewing {
    protect: bool,
}

pub(super) async fn set_viewing(
    State(sv): State<Services>,
    Json(body): Json<Viewing>,
) -> Result<StatusCode, ApiError> {
    sv.auth.set_protect_viewing(body.protect).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn with_cookie(cookie: String) -> Response {
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}
