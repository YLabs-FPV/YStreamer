use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{sink::SinkExt, stream::StreamExt};
use tokio::sync::broadcast;

use crate::input::duml::camera::ExposureMode;
use crate::web::auth::Access;

use super::AppState;

/// Commands from the browser. Tagged by "cmd"; snake_case values
// The Set prefix is the wire format the browser sends, not just a naming habit
#[allow(clippy::enum_variant_names)]
#[derive(serde::Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum ControlMsg {
    SetIso {
        value: u32,
    },
    SetShutter {
        value: u32,
    },
    SetEv {
        value: f32,
    },
    SetExposureMode {
        value: String,
    },
    SetWb {
        auto: bool,
        #[serde(default)]
        temp: u32,
    },
    SetVideoFormat {
        res: String,
        ar: String,
        fps: u8,
    },
    SetSharpness {
        value: i8,
    },
    SetNr {
        value: i8,
    },
    SetColorProfile {
        value: String,
    },
    SetAntiFlicker {
        value: String,
    },
}

pub(super) async fn control_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Decided once: the browser reconnects the socket after logging in
    let admin = state.services.auth.access(&headers) == Access::Admin;
    ws.on_upgrade(move |socket| control_socket(socket, state, admin))
}

async fn control_socket(socket: WebSocket, state: AppState, admin: bool) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.control_tx.subscribe();

    // Broadcasts only carry changes, so a new browser needs the current state
    let current = [
        super::link_msg(state.services.goggles.state()),
        super::devices_msg(&state.services.session.devices()),
        super::input_msg(&state.services),
        super::overlay_msg(&state.services),
    ];
    for msg in current {
        if sink.send(Message::Text(msg.into())).await.is_err() {
            return;
        }
    }

    // Forward broadcast camera-state JSON to this browser
    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(text) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Read commands from this browser
    while let Some(Ok(msg)) = stream.next().await {
        if let Message::Text(t) = msg {
            match serde_json::from_str::<ControlMsg>(t.as_str()) {
                Ok(cmd) if admin => apply_command(&state, cmd),
                Ok(_) => {}
                Err(e) => eprintln!("[web] bad control message: {} ({})", t.as_str(), e),
            }
        }
    }

    send_task.abort();
}

/// Map a parsed command to DUML bytes and send them to the goggles
fn apply_command(state: &AppState, cmd: ControlMsg) {
    let cam = &state.camera;
    // Each arm builds one or more packets; send() failures (link momentarily
    // down) are logged and ignored
    let send = |bytes: Vec<u8>| {
        if let Err(e) = state.services.goggles.send(&bytes) {
            eprintln!("[web] command send failed: {}", e);
        }
    };

    match cmd {
        ControlMsg::SetIso { value } => {
            if let Some(p) = cam.build_iso(value) {
                send(p);
            }
        }
        ControlMsg::SetShutter { value } => {
            if let Some(p) = cam.build_shutter(value) {
                send(p);
            }
        }
        ControlMsg::SetEv { value } => {
            if let Some(p) = cam.build_ev(value) {
                send(p);
            }
        }
        ControlMsg::SetExposureMode { value } => {
            let mode = match value.as_str() {
                "manual" => ExposureMode::Manual,
                _ => ExposureMode::Auto,
            };
            // Release ISO+shutter to 0 first
            if mode == ExposureMode::Auto {
                if let Some(p) = cam.build_iso(0) {
                    send(p);
                }
                if let Some(p) = cam.build_shutter(0) {
                    send(p);
                }
            }
            send(cam.build_exposure_mode(mode));
        }
        ControlMsg::SetWb { auto, temp } => {
            let t = if auto { 5000 } else { temp.max(2000) };
            send(cam.build_wb(auto, t));
        }
        ControlMsg::SetVideoFormat { res, ar, fps } => {
            let res_wire = crate::input::duml::camera::res_ar_to_wire(&res, &ar).unwrap_or(10);
            send(cam.build_video_format(res_wire, fps));
        }
        ControlMsg::SetSharpness { value } => send(cam.build_sharpness(value)),
        ControlMsg::SetNr { value } => {
            if let Some(p) = cam.build_noise_reduction(value) {
                send(p);
            }
        }
        ControlMsg::SetColorProfile { value } => {
            if let Some(p) = cam.build_color_profile(&value) {
                send(p);
            }
        }
        ControlMsg::SetAntiFlicker { value } => {
            if let Some(p) = cam.build_anti_flicker(&value) {
                send(p);
            }
        }
    }
}
