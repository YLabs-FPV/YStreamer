use axum::{Json, extract::State};

use crate::input::duml::telemetry::Telemetry;

use super::Services;

/// The aircraft's latest telemetry, for ground stations to poll. Unlike the
/// Watch page's copy, it includes where the aircraft and its home are
pub(super) async fn telemetry(State(sv): State<Services>) -> Json<Telemetry> {
    Json(sv.telemetry.snapshot())
}
