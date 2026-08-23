mod common;

use axum::body::to_bytes;
use axum::http::StatusCode;
use common::{req_get, Harness, TEST_TOKEN};
use tower::ServiceExt;

/// `GET /sessions/{id}/connect/wait` creates the session on first hit (no
/// prior `POST /sessions` needed), streams SSE, and gives up cleanly with a
/// `timeout` event once `timeout_seconds` elapses without a real WhatsApp
/// connection to pair against -- exactly what happens in this test harness,
/// which never touches the real network.
#[tokio::test]
async fn connect_wait_creates_session_and_times_out() {
    let h = Harness::new().await;

    let req = req_get(
        "/api/v1/sessions/s-wait/connect/wait?timeout_seconds=5",
        Some(TEST_TOKEN),
    );
    let res = h.app.clone().oneshot(req).await.expect("router response");
    let status = res.status();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = to_bytes(res.into_body(), 1024 * 1024)
        .await
        .expect("body bytes");
    let text = String::from_utf8_lossy(&body);

    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/event-stream"));
    assert!(text.contains("event: timeout"), "body was: {text}");

    // The session row now exists even though no client ever connected.
    let (status, body) = common::call(
        &h.app,
        req_get("/api/v1/sessions/s-wait/status", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.pointer("/status").and_then(|v| v.as_str()).is_some());
}
