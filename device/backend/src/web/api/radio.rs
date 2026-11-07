use axum::{Json, extract::State, http::StatusCode};

use crate::input::duml::region::RegionStatus;

use super::{ApiError, Services, bad};

pub(super) async fn region_status(State(sv): State<Services>) -> Json<RegionStatus> {
    Json(sv.region.status())
}

/// Send the whole region set again, for when the aircraft was swapped
/// without the goggles noticing
pub(super) async fn region_apply(State(sv): State<Services>) -> Result<StatusCode, ApiError> {
    if !sv.store.get().radio_region.enabled {
        return Err(bad("Turn the radio region on first"));
    }
    sv.region.reapply();
    Ok(StatusCode::NO_CONTENT)
}
