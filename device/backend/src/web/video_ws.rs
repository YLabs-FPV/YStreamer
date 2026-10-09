use std::sync::atomic::Ordering;
use std::time::Instant;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::response::IntoResponse;
use futures_util::{sink::SinkExt, stream::StreamExt};
use tokio::sync::broadcast;

use super::AppState;

const MAX_QUEUED: usize = 30;

pub(super) async fn video_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| video_socket(socket, state))
}

struct ViewerSlot(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Drop for ViewerSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Each binary message is one access unit: an 8-byte big-endian timestamp in
/// microseconds, then the Annex B bytes
async fn video_socket(mut socket: WebSocket, state: AppState) {
    // A browser can't read the HTTP status of a refused upgrade, but it can
    // read a close code
    if state.viewer_limit_reached().await {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: close_code::AGAIN,
                reason: "viewer limit reached".into(),
            })))
            .await;
        return;
    }
    state.socket_viewers.fetch_add(1, Ordering::Relaxed);
    let _slot = ViewerSlot(state.socket_viewers.clone());

    eprintln!("[web] video socket viewer connected");
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.au_tx.subscribe();
    let mut synced = false;
    let mut base: Option<Instant> = None;

    loop {
        let au = tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => continue,
            },
            au = rx.recv() => match au {
                Ok(au) => au,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    synced = false;
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        };

        if synced && rx.len() > MAX_QUEUED {
            if crate::settings::verbose() {
                println!(
                    "[web] video socket: {} AUs queued, skipping to next keyframe",
                    rx.len()
                );
            }
            synced = false;
        }
        if !synced {
            if !au.keyframe {
                continue;
            }
            synced = true;
        }

        let base = *base.get_or_insert(au.assembled);
        let micros = au.assembled.saturating_duration_since(base).as_micros() as u64;
        let mut msg = Vec::with_capacity(8 + au.data.len());
        msg.extend_from_slice(&micros.to_be_bytes());
        msg.extend_from_slice(&au.data);
        if sink.send(Message::Binary(msg.into())).await.is_err() {
            break;
        }
    }

    eprintln!("[web] video socket viewer disconnected");
}
