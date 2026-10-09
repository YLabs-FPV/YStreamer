use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use std::sync::Arc;

use crate::output::recording;

use super::{ApiError, Services, bad, failed};

pub(super) async fn recording_status(State(sv): State<Services>) -> impl IntoResponse {
    Json(sv.recorder.status())
}

pub(super) async fn recording_button(State(sv): State<Services>) -> impl IntoResponse {
    Json(sv.button.status())
}

pub(super) async fn recording_start(
    State(sv): State<Services>,
) -> Result<impl IntoResponse, ApiError> {
    sv.recorder
        .start()
        .map(|_| Json(sv.recorder.status()))
        .map_err(|e| ApiError(StatusCode::CONFLICT, e))
}

pub(super) async fn recording_stop(
    State(sv): State<Services>,
) -> Result<impl IntoResponse, ApiError> {
    sv.recorder
        .stop()
        .map(|_| Json(sv.recorder.status()))
        .map_err(|e| ApiError(StatusCode::CONFLICT, e))
}

/// Which of the places recordings can be to look in; the one being recorded
/// to when it isn't said
#[derive(serde::Deserialize)]
pub(super) struct Place {
    location: Option<String>,
}

/// Only the folders offered as storage: this runs as root, and a browser
/// naming any path it likes would reach every file on the device
fn place(sv: &Services, asked: Option<&str>) -> Result<std::path::PathBuf, ApiError> {
    let current = sv.recorder.dir();
    let Some(asked) = asked else {
        return Ok(current);
    };
    recording::storage_options(&current)
        .into_iter()
        .find(|o| o.path == asked && !o.missing)
        .map(|o| std::path::PathBuf::from(o.path))
        .ok_or_else(|| bad("That isn't a place recordings are kept"))
}

pub(super) async fn recording_files(
    State(sv): State<Services>,
    Query(q): Query<Place>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(recording::list(&place(&sv, q.location.as_deref())?)))
}

pub(super) async fn recording_storage_options(State(sv): State<Services>) -> impl IntoResponse {
    // So a drive plugged in a moment ago is in the list
    crate::system::drives::mount_plugged_in().await;
    Json(recording::storage_options(&sv.recorder.dir()))
}

#[derive(serde::Deserialize)]
pub(super) struct Eject {
    path: String,
}

pub(super) async fn recording_eject(
    State(sv): State<Services>,
    Json(req): Json<Eject>,
) -> Result<StatusCode, ApiError> {
    let folder = std::path::Path::new(&req.path);
    let drive = recording::drive_of(folder).ok_or_else(|| bad("That isn't a plugged-in drive"))?;
    if sv.recorder.is_active() && sv.recorder.dir().starts_with(&drive.point) {
        return Err(bad("Stop recording before ejecting the drive"));
    }
    crate::system::drives::eject(&drive.device)
        .await
        .map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn recording_download(
    State(sv): State<Services>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<Place>,
) -> Result<Response, ApiError> {
    use axum::http::header;
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let path = recording::resolve(&place(&sv, q.location.as_deref())?, &name).map_err(bad)?;
    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| failed(format!("Couldn't open {name}: {e}")))?;
    let total = file
        .metadata()
        .await
        .map(|m| m.len())
        .map_err(|e| failed(format!("Couldn't read {name}: {e}")))?;
    let mime = if name.ends_with(".mp4") {
        "video/mp4"
    } else {
        "video/mp2t"
    };

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_range(v, total));

    let mut response_headers = axum::http::HeaderMap::new();
    let mut set = |key: axum::http::HeaderName, value: String| {
        if let Ok(v) = value.parse() {
            response_headers.insert(key, v);
        }
    };
    set(header::CONTENT_TYPE, mime.to_string());
    set(header::ACCEPT_RANGES, "bytes".to_string());
    set(
        header::CONTENT_DISPOSITION,
        format!("inline; filename=\"{name}\""),
    );

    let (status, start, len) = match range {
        Some((start, end)) => {
            set(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{total}"),
            );
            (StatusCode::PARTIAL_CONTENT, start, end - start + 1)
        }
        None => (StatusCode::OK, 0, total),
    };
    set(header::CONTENT_LENGTH, len.to_string());

    if start > 0 {
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|e| failed(format!("Couldn't seek {name}: {e}")))?;
    }
    let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file.take(len)));
    Ok((status, response_headers, body).into_response())
}

pub(super) fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?.trim();
    if spec.contains(',') || total == 0 {
        return None;
    }
    let (from, to) = spec.split_once('-')?;
    let (start, end) = if from.is_empty() {
        // "-N": the last N bytes
        let n: u64 = to.parse().ok()?;
        (total.saturating_sub(n.min(total)), total - 1)
    } else {
        let start: u64 = from.parse().ok()?;
        let end = if to.is_empty() {
            total - 1
        } else {
            to.parse::<u64>().ok()?.min(total - 1)
        };
        (start, end)
    };
    (start <= end && start < total).then_some((start, end))
}

pub(super) async fn recording_delete(
    State(sv): State<Services>,
    Path(name): Path<String>,
    Query(q): Query<Place>,
) -> Result<StatusCode, ApiError> {
    let dir = place(&sv, q.location.as_deref())?;
    let path = recording::resolve(&dir, &name).map_err(bad)?;
    // Deleting a file splitmuxsink still has open would lose the recording
    if sv.recorder.is_writing(&dir, &name) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "That file is being recorded right now".into(),
        ));
    }
    if sv.transfers.running() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Wait for the transfer to finish before deleting".into(),
        ));
    }
    std::fs::remove_file(path).map_err(|e| failed(format!("Couldn't delete {name}: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn transfer_status(State(sv): State<Services>) -> impl IntoResponse {
    Json(sv.transfers.progress())
}

#[derive(serde::Deserialize)]
pub(super) struct Transfer {
    from: String,
    to: String,
    /// Left out, it's every recording the destination doesn't have yet
    files: Option<Vec<String>>,
    #[serde(default, rename = "move")]
    moving: bool,
}

pub(super) async fn transfer_start(
    State(sv): State<Services>,
    Json(req): Json<Transfer>,
) -> Result<impl IntoResponse, ApiError> {
    use crate::output::transfer;

    // So a drive plugged in a moment ago counts
    crate::system::drives::mount_plugged_in().await;
    let from = place(&sv, Some(&req.from))?;
    let to = place(&sv, Some(&req.to))?;
    let conflict = |e: String| ApiError(StatusCode::CONFLICT, e);

    let started = {
        let sv = sv.clone();
        tokio::task::spawn_blocking(move || {
            if !recording::storage(&to).writable {
                return Err("Can't write to the destination".to_string());
            }
            let mounts = recording::mounts();
            let destination = transfer::Destination {
                dir: &to,
                fstype: recording::mount_of(&to, &mounts).map_or("", |m| m.fstype.as_str()),
                free_bytes: recording::disk_usage(&to).map_or(0, |(_, free)| free),
            };
            let recorder = Arc::clone(&sv.recorder);
            let source = from.clone();
            let busy = move |name: &str| recorder.is_writing(&source, name);
            let plan =
                transfer::plan(&from, &destination, req.files.as_deref(), req.moving, &busy)?;
            sv.transfers.start(plan)
        })
        .await
        .map_err(|e| failed(e.to_string()))?
    };
    started.map_err(conflict)?;
    Ok((StatusCode::ACCEPTED, Json(sv.transfers.progress())))
}

/// Stops the one that's running, or clears the one that has ended
pub(super) async fn transfer_cancel(State(sv): State<Services>) -> impl IntoResponse {
    sv.transfers.cancel();
    Json(sv.transfers.progress())
}

#[cfg(test)]
#[path = "recording.test.rs"]
mod tests;
