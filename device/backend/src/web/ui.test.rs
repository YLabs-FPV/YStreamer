use super::*;
use axum::http::HeaderValue;

async fn get(path: &str, if_none_match: Option<&str>) -> Response {
    let mut headers = HeaderMap::new();
    if let Some(tag) = if_none_match {
        headers.insert(IF_NONE_MATCH, HeaderValue::from_str(tag).unwrap());
    }
    serve(path.parse().unwrap(), headers).await
}

fn header(res: &Response, name: axum::http::HeaderName) -> &str {
    res.headers().get(name).unwrap().to_str().unwrap()
}

// These need the frontend to have been built, as a release build does
fn built() -> bool {
    Files::get("index.html").is_some()
}

#[tokio::test]
async fn serves_the_page_at_the_root_and_never_lets_it_go_stale() {
    if !built() {
        return;
    }
    let res = get("/", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header(&res, CONTENT_TYPE), "text/html");
    assert_eq!(header(&res, CACHE_CONTROL), ASK_AGAIN);

    // unchanged since the browser last asked
    let tag = header(&res, ETAG).to_string();
    let again = get("/", Some(&tag)).await;
    assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(get("/", Some("\"other\"")).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn lets_content_named_files_be_kept() {
    if !built() {
        return;
    }
    let script = Files::iter()
        .find(|f| f.starts_with("assets/") && f.ends_with(".js"))
        .unwrap();
    let res = get(&format!("/{script}"), None).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header(&res, CACHE_CONTROL), KEEP);
    assert!(header(&res, CONTENT_TYPE).contains("javascript"));

    assert_eq!(
        header(&get("/logo.svg", None).await, CACHE_CONTROL),
        ASK_AGAIN
    );
}

#[tokio::test]
async fn has_nothing_outside_the_build() {
    assert_eq!(get("/nope.js", None).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        get("/../Cargo.toml", None).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("/assets/../../Cargo.toml", None).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn serves_the_page_at_its_own_addresses() {
    if !built() {
        return;
    }
    for path in ["/system", "/system/updates", "/login"] {
        let res = get(path, None).await;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert_eq!(header(&res, CONTENT_TYPE), "text/html");
        assert_eq!(header(&res, CACHE_CONTROL), ASK_AGAIN);
    }
    assert_eq!(get("/api/nope", None).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        get("/system/nope.js", None).await.status(),
        StatusCode::NOT_FOUND
    );
}
