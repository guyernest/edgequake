//! Integration test: snapshot-baked read-only guard (REG-03 / Plan 27-02).
//!
//! Verifies that `ServerConfig { read_only: true, .. }` causes `build_router` to
//! layer `readonly_guard` so that:
//!
//! - POST / PUT / PATCH / DELETE → 405 Method Not Allowed (before any handler runs)
//! - GET /health → 200 (reads pass through)
//! - HEAD to a known read route → non-405 (allow-list permits HEAD)
//! - POST with `read_only: false` → NOT 405 (guard is not mounted)
//!
//! The guard is method-based (not route-enumerated), so it covers all write routes
//! including future ones automatically.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use edgequake_api::{AppState, Server, ServerConfig};
use tower::ServiceExt;

/// Build a router with `read_only` set to the given value.
async fn app_with_read_only(read_only: bool) -> axum::Router {
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        enable_cors: false,
        enable_compression: false,
        enable_swagger: false,
        read_only,
    };
    Server::new(config, AppState::new_memory(None::<String>).await).build_router()
}

// ---------------------------------------------------------------------------
// Tests: read_only = true
// ---------------------------------------------------------------------------

/// POST to a write route returns 405 on a baked server.
#[tokio::test]
async fn test_post_returns_405_when_read_only() {
    let app = app_with_read_only(true).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/documents")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "POST must be blocked with 405 on a snapshot-baked server"
    );
}

/// PUT to a write route returns 405 on a baked server.
#[tokio::test]
async fn test_put_returns_405_when_read_only() {
    let app = app_with_read_only(true).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/v1/documents/00000000-0000-0000-0000-000000000001")
                .header("content-type", "application/json")
                .body(Body::from(r#"{}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "PUT must be blocked with 405 on a snapshot-baked server"
    );
}

/// PATCH to a write route returns 405 on a baked server.
#[tokio::test]
async fn test_patch_returns_405_when_read_only() {
    let app = app_with_read_only(true).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri("/api/v1/costs/budget")
                .header("content-type", "application/json")
                .body(Body::from(r#"{}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "PATCH must be blocked with 405 on a snapshot-baked server"
    );
}

/// DELETE to a write route returns 405 on a baked server.
#[tokio::test]
async fn test_delete_returns_405_when_read_only() {
    let app = app_with_read_only(true).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/v1/documents/00000000-0000-0000-0000-000000000001")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "DELETE must be blocked with 405 on a snapshot-baked server"
    );
}

/// GET /health returns 200 on a baked server (read passes through).
#[tokio::test]
async fn test_get_health_passes_when_read_only() {
    let app = app_with_read_only(true).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "GET must NOT be rejected by the read-only guard"
    );
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "GET /health must return 200 on a baked server"
    );
}

/// HEAD to a known read-route returns a non-405 status on a baked server.
///
/// The allow-list permits HEAD (axum serves it via the GET handler).
/// This pairs with the Task 1 audit verdict that GET handlers have no
/// durable write side-effects — so HEAD-via-GET is safe to allow.
#[tokio::test]
async fn test_head_passes_when_read_only() {
    let app = app_with_read_only(true).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::HEAD)
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "HEAD must NOT be rejected by the read-only guard"
    );
}

// ---------------------------------------------------------------------------
// Tests: read_only = false (guard NOT mounted)
// ---------------------------------------------------------------------------

/// POST is NOT rejected with 405 when the guard is not mounted.
///
/// The handler may return a different error (400, 422, etc.) because the
/// request body is minimal — what matters is it is NOT the guard's 405.
#[tokio::test]
async fn test_post_not_blocked_when_read_only_false() {
    let app = app_with_read_only(false).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/documents")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // The guard is absent — the request reaches the handler which returns
    // 200/400/422 depending on validation, but NOT 405 from the guard.
    // (Axum would return 405 for a truly unregistered method, but the handler
    //  is registered for POST — so any 405 here would come from the guard.)
    assert_ne!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "POST must NOT be blocked by the guard when read_only=false"
    );
}
