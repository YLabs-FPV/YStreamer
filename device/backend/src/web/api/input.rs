use axum::{Json, extract::State};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::settings::InputMode;
use crate::system::button::{ButtonStatus, InputRequest};

use super::settings::{SAVING, commit};
use super::{ApiError, Services};

#[derive(Serialize)]
pub(super) struct ModeReply {
    mode: InputMode,
}

/// `POST /api/input/toggle`: swap between the input button's two inputs,
/// whether or not a button is wired
pub(super) async fn toggle(State(sv): State<Services>) -> Result<Json<ModeReply>, ApiError> {
    switch(&sv, |s| s.input.button.toggled(s.input.mode))
        .await
        .map(|mode| Json(ModeReply { mode }))
}

pub(super) async fn button_status(State(sv): State<Services>) -> Json<ButtonStatus> {
    Json(sv.input_button.status())
}

/// Save the input `pick` chooses from the current settings, the same way
/// the Input page does. Returns the input now in use
async fn switch(
    sv: &Services,
    pick: impl FnOnce(&crate::settings::Settings) -> InputMode,
) -> Result<InputMode, ApiError> {
    let _saving = SAVING.lock().await;
    let mut next = sv.store.get();
    let mode = pick(&next);
    if mode == next.input.mode {
        return Ok(mode);
    }
    next.input.mode = mode;
    let saved = commit(sv, next).await?.0.settings.input.mode;
    println!("[input] switched to {saved:?}");
    Ok(saved)
}

/// Act on the input button, one press at a time
pub async fn input_button_requests(
    sv: Services,
    mut requests: mpsc::UnboundedReceiver<InputRequest>,
) {
    while let Some(request) = requests.recv().await {
        let result = match request {
            InputRequest::Toggle => switch(&sv, |s| s.input.button.toggled(s.input.mode)).await,
            InputRequest::Switch { closed } => {
                switch(&sv, |s| s.input.button.for_switch(closed)).await
            }
        };
        if let Err(ApiError(_, e)) = result {
            eprintln!("[button] couldn't switch the input: {e}");
        }
    }
}
