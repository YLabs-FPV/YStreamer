use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Send `method /slow` to a server whose handler takes a moment, hang up
/// before it's done, and say whether the handler finished anyway
async fn finished_after_hanging_up(method: &str) -> bool {
    let done = Arc::new(AtomicBool::new(false));
    let slow = {
        let done = Arc::clone(&done);
        move || async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            done.store(true, Ordering::SeqCst);
        }
    };
    let app = Router::new()
        .route("/slow", get(slow.clone()).put(slow))
        .route_layer(axum::middleware::from_fn(run_to_completion));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });

    let mut browser = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!("{method} /slow HTTP/1.1\r\nHost: device\r\nContent-Length: 0\r\n\r\n");
    browser.write_all(request.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(browser);

    tokio::time::sleep(Duration::from_millis(500)).await;
    done.load(Ordering::SeqCst)
}

#[tokio::test]
async fn a_change_finishes_after_the_browser_has_left() {
    assert!(finished_after_hanging_up("PUT").await);
}

#[tokio::test]
async fn a_read_is_dropped_with_the_browser() {
    assert!(!finished_after_hanging_up("GET").await);
}
