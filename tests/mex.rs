//! Contract tests for the mex handlers (`src/handlers/mex.rs`), the
//! passthrough to WhatsApp's internal GraphQL ("mex") endpoint.
//!
//! # These routes are not reachable
//!
//! `mex_query` and `mex_mutate` are written, compiled, and registered with
//! utoipa in `src/main.rs`, so `/api/v1/sessions/{session_id}/mex/query` and
//! `.../mex/mutate` both appear in the published OpenAPI document. Neither is
//! wired into `create_router` in `src/routes/mod.rs`. Every request to them
//! therefore falls through to the router's 404, and the handler bodies are
//! dead code at runtime.
//!
//! That makes the honest contract for this module "the documented path does
//! not exist", and that is what the first two tests assert. They are written
//! as characterization tests: they pin the current, wrong behaviour so that
//! it is visible in CI instead of only in the OpenAPI diff, and they fail the
//! moment someone registers the routes. The change that registers them is
//! expected to delete those two tests and un-`ignore` the four below it,
//! which spell out the contract the handlers already implement.
//!
//! Per the workstream rule, the fix is not in this PR.
//!
//! # The intended contract
//!
//! Both handlers resolve their client through the same local `get_client` as
//! every other session-scoped module, so once routed they follow the pattern
//! documented in `tests/presence.rs` exactly: 401 without a token, 503 for an
//! unknown session, 503 for a session that exists in the database but has
//! never connected, and a body rejection at extraction time for a body that
//! does not deserialize.
//!
//! Nothing past the client gate is assertable — `mex()` is a thin passthrough
//! whose only logic is `build_mex_doc`'s fallback name and the flattening of
//! upstream GraphQL errors, both of which need a live client to reach.

mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_json, Harness, TEST_TOKEN};
use serde_json::json;

const SESSION: &str = "s-mex";

const QUERY_PATH: &str = "/api/v1/sessions/s-mex/mex/query";
const MUTATE_PATH: &str = "/api/v1/sessions/s-mex/mex/mutate";

/// Creates a session row (no live client is ever attached in tests).
async fn seed_session(h: &Harness, id: &str) {
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({ "id": id }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "seeding session {id} should succeed: {body:?}"
    );
}

fn query_body() -> serde_json::Value {
    json!({ "doc_id": "1234567890", "variables": {} })
}

/// Characterization test. Both mex paths are absent from the router, so an
/// authenticated request with a well-formed body gets 404 rather than the 503
/// that `get_client` would produce. The presence route on the same session is
/// the control: it is registered, takes the same shape, and returns 503, so a
/// 404 here is a routing gap and not something about the session or the body.
#[tokio::test]
async fn mex_routes_are_documented_but_not_registered() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    for path in [QUERY_PATH, MUTATE_PATH] {
        let (status, _) = call(
            &h.app,
            req_json(Method::POST, path, Some(TEST_TOKEN), query_body()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} is in the OpenAPI document but not in create_router"
        );
    }

    let (control_status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/presence/set"),
            Some(TEST_TOKEN),
            json!({ "status": "available" }),
        ),
    )
    .await;
    assert_eq!(
        control_status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a registered session-scoped route reaches the client gate"
    );
}

/// The JWT middleware is layered around the whole router, so it runs before
/// routing and an unauthenticated request to a path that does not exist is
/// still 401. Worth pinning precisely because it is misleading: a 401 here
/// says nothing about whether the route is registered, which is why the test
/// above supplies a token.
#[tokio::test]
async fn mex_routes_return_401_before_routing_is_consulted() {
    let h = Harness::new().await;

    for path in [QUERY_PATH, MUTATE_PATH] {
        let (status, _) = call(&h.app, req_json(Method::POST, path, None, query_body())).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{path}: auth is checked ahead of routing"
        );
    }
}

/// Un-ignore together with the three below once the routes are registered.
#[tokio::test]
#[ignore = "mex routes are not registered in create_router; see the module doc"]
async fn mex_routes_require_a_token() {
    let h = Harness::new().await;

    for path in [QUERY_PATH, MUTATE_PATH] {
        let (status, _) = call(&h.app, req_json(Method::POST, path, None, query_body())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "unauthenticated {path}");
    }
}

#[tokio::test]
#[ignore = "mex routes are not registered in create_router; see the module doc"]
async fn mex_routes_reject_an_unknown_session() {
    let h = Harness::new().await;

    for path in [
        "/api/v1/sessions/does-not-exist/mex/query",
        "/api/v1/sessions/does-not-exist/mex/mutate",
    ] {
        let (status, _) = call(
            &h.app,
            req_json(Method::POST, path, Some(TEST_TOKEN), query_body()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} should fail at the client gate"
        );
    }
}

/// `get_client` reads the live registry, not the `sessions` table, so a
/// session row on its own does not change the outcome — 503, never 404.
#[tokio::test]
#[ignore = "mex routes are not registered in create_router; see the module doc"]
async fn mex_routes_reject_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    for path in [QUERY_PATH, MUTATE_PATH] {
        let (status, _) = call(
            &h.app,
            req_json(Method::POST, path, Some(TEST_TOKEN), query_body()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} has a DB row but no live client"
        );
    }
}

/// `doc_id` and `variables` are both required — only `doc_name` has a serde
/// default. `Json<T>` is the last extractor, so a body missing either one is
/// rejected before the handler body and never reaches the client gate.
#[tokio::test]
#[ignore = "mex routes are not registered in create_router; see the module doc"]
async fn mex_routes_reject_a_body_missing_required_fields() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let bad_bodies = [
        json!({}),
        json!({ "variables": {} }),
        json!({ "doc_id": "1234567890" }),
        json!({ "doc_id": 1234567890, "variables": {} }),
    ];

    for path in [QUERY_PATH, MUTATE_PATH] {
        for body in &bad_bodies {
            let (status, _) = call(
                &h.app,
                req_json(Method::POST, path, Some(TEST_TOKEN), body.clone()),
            )
            .await;
            assert_ne!(
                status,
                StatusCode::SERVICE_UNAVAILABLE,
                "{path} with {body}: extraction must fail before the client gate"
            );
            assert!(
                status.is_client_error(),
                "{path} with {body}: expected a client error, got {status}"
            );
        }
    }
}
