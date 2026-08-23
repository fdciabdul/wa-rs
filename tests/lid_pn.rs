mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

/// `GET /sessions/{id}/contacts/{jid}/lid` drives `Client::get_lid_pn_entry`,
/// so it needs an installed client instance — a session that exists only as
/// a DB row gets 503, not 404.
#[tokio::test]
async fn resolve_lid_requires_an_installed_client() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-lid"}),
        ),
    )
    .await;

    let (status, _) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/s-lid/contacts/100000012345678@lid/lid",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// A totally unknown session returns 503 too (`get_client` checks the live
/// registry, not the sessions table) — same class of response as any other
/// endpoint gated on `handlers::messages::get_client`.
#[tokio::test]
async fn resolve_lid_unknown_session_returns_503() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/does-not-exist/contacts/100000012345678@lid/lid",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
