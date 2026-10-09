use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../frontend/dist"]
struct Files;

const NAMED_BY_CONTENT: &str = "assets/";
const KEEP: &str = "public, max-age=31536000, immutable";
const ASK_AGAIN: &str = "no-cache";

pub async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let path = match uri.path().trim_start_matches('/') {
        "" => "index.html",
        path => path,
    };
    // The page does its own routing, so its addresses all load the page;
    // a missing file or API call is still missing
    let page = || {
        let file = path.rsplit('/').next().unwrap_or(path);
        if path.starts_with("api/") || file.contains('.') {
            return None;
        }
        Files::get("index.html").map(|file| ("index.html", file))
    };
    let Some((path, file)) = Files::get(path).map(|file| (path, file)).or_else(page) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let tag = format!("\"{}\"", hex::encode(&file.metadata.sha256_hash()[..8]));
    let cache = if path.starts_with(NAMED_BY_CONTENT) {
        KEEP
    } else {
        ASK_AGAIN
    };
    let common = [(CACHE_CONTROL, cache.to_string()), (ETAG, tag.clone())];
    if headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(tag.as_str()) {
        return (StatusCode::NOT_MODIFIED, common).into_response();
    }

    let body = match file.data {
        std::borrow::Cow::Borrowed(bytes) => Body::from(bytes),
        std::borrow::Cow::Owned(bytes) => Body::from(bytes),
    };
    (
        common,
        [(CONTENT_TYPE, file.metadata.mimetype().to_string())],
        body,
    )
        .into_response()
}

#[cfg(test)]
#[path = "ui.test.rs"]
mod tests;
