use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::input::duml::flight::{Action, Outcome};

use super::{ApiError, Services, failed};

/// Why nobody may send aircraft commands right now, if so
fn refusal(sv: &Services) -> Option<&'static str> {
    if !sv.store.get().input.rc_actions {
        Some("Aircraft actions are switched off")
    } else if !sv.auth.enabled() {
        // Without a password everyone who can open the page is an admin
        Some("Set a password under System → General → Access before using aircraft actions")
    } else {
        None
    }
}

pub(super) async fn aircraft_action(
    State(sv): State<Services>,
    Path(name): Path<String>,
) -> Result<Json<Outcome>, ApiError> {
    let action = Action::parse(&name)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("Unknown action '{name}'")))?;
    let label = action.label();
    if let Some(reason) = refusal(&sv) {
        eprintln!("[aircraft] {} {label} refused: {reason}", now());
        return Err(ApiError(StatusCode::CONFLICT, reason.into()));
    }

    eprintln!("[aircraft] {} {label} pressed", now());
    let flight = std::sync::Arc::clone(&sv.flight);
    let result = tokio::task::spawn_blocking(move || flight.send(action))
        .await
        .map_err(|e| failed(e.to_string()))?;
    match result {
        Ok(outcome) => {
            if outcome.acknowledged {
                eprintln!(
                    "[aircraft] {} {label} acknowledged after {} send(s), reply {}",
                    now(),
                    outcome.sends,
                    outcome.reply.as_deref().unwrap_or("")
                );
            } else {
                eprintln!(
                    "[aircraft] {} {label} not acknowledged after {} send(s)",
                    now(),
                    outcome.sends
                );
            }
            Ok(Json(outcome))
        }
        Err(refused) => {
            eprintln!("[aircraft] {} {label} refused: {refused}", now());
            Err(ApiError(StatusCode::CONFLICT, refused.to_string()))
        }
    }
}

/// Local time to the second, so the line says when even outside the journal
fn now() -> String {
    // localtime_r only writes into the tm it is given
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}
