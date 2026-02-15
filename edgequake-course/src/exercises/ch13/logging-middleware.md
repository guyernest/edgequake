# Exercise: Logging Middleware

::: exercise
id: ch13-logging-middleware
difficulty: intermediate
time: 25 minutes
:::

In chapter 13 you learned that EdgeQuake uses structured logging with the
`tracing` ecosystem for production observability. Every API request is logged
with timing, status codes, and a unique request ID that enables end-to-end
tracing across services.

In this exercise you will implement structured logging middleware for Axum
that creates a tracing span for each request, records timing information,
and logs the response with appropriate severity levels.

::: objectives
thinking:
  - Understand the difference between tracing spans (duration) and events (points in time)
  - Know why structured logging is preferred over println-style logging in production
  - Recognize how request IDs enable cross-service correlation
doing:
  - Create a tracing span with request metadata for each incoming request
  - Measure request duration using std::time::Instant
  - Log responses with status code, duration, and request ID
  - Use appropriate log levels based on HTTP status codes
:::

::: discussion
- What is the difference between `tracing::info!` and `println!`? Why does it matter in production?
- How does a request ID help when debugging an issue that spans multiple services?
- Why should 4xx responses be logged at warn level instead of error level?
:::

::: starter file="src/main.rs"
```rust
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::time::Instant;
use tracing::{info, info_span, warn, Instrument};
use uuid::Uuid;

/// Structured logging middleware for Axum.
///
/// For each request, this middleware:
/// 1. Generates a unique request ID
/// 2. Creates a tracing span with request metadata
/// 3. Records the start time
/// 4. Calls the next handler
/// 5. Logs the response with status, duration, and request ID
pub async fn logging_middleware(
    request: Request,
    next: Next,
) -> Response {
    // TODO Step 1: Generate a unique request ID.
    // Use uuid::Uuid::new_v4() and take the first 8 characters for brevity.
    let request_id = todo!("Generate request ID");

    // TODO Step 2: Extract request metadata before consuming the request.
    // Capture the HTTP method and path (as owned Strings) because the
    // request will be moved into next.run().
    let method = todo!("Extract HTTP method as string");
    let path = todo!("Extract URI path as string");

    // TODO Step 3: Create a tracing span with the request metadata.
    // Use info_span! with fields: request_id, method, path.
    // Example: info_span!("http_request", %request_id, %method, %path)
    let span = todo!("Create tracing span");

    // TODO Step 4: Record the start time.
    let start = todo!("Capture start instant");

    // TODO Step 5: Call the next handler within the span.
    // Use .instrument(span) to attach the span to the future.
    let response = todo!("Call next.run(request) within the span");

    // TODO Step 6: Calculate the elapsed duration in milliseconds.
    let duration_ms = todo!("Calculate elapsed time");

    // TODO Step 7: Extract the response status code.
    let status = todo!("Get response status code");

    // TODO Step 8: Log the response with appropriate severity.
    // - 5xx responses: tracing::error!
    // - 4xx responses: tracing::warn!
    // - Everything else: tracing::info!
    //
    // Include: status (as u16), duration_ms, request_id
    todo!("Log based on status code severity");

    response
}

// --- Example handlers ---

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "healthy"}))
}

async fn get_document() -> impl IntoResponse {
    // Simulate some work
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    Json(serde_json::json!({
        "id": "doc-123",
        "title": "Sample Document"
    }))
}

async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"})))
}

async fn internal_error() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "Something went wrong"})),
    )
}

#[tokio::main]
async fn main() {
    // Initialize the tracing subscriber with JSON output.
    tracing_subscriber::fmt()
        .json()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
        .init();

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/documents/:id", get(get_document))
        .route("/not-found", get(not_found))
        .route("/error", get(internal_error))
        .layer(middleware::from_fn(logging_middleware));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("Server starting on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}
```
:::

::: hint level=1 title="Request ID and metadata extraction"
```rust
let request_id = Uuid::new_v4().to_string()[..8].to_string();
let method = request.method().to_string();
let path = request.uri().path().to_string();
```
:::

::: hint level=2 title="Creating and using the span"
```rust
let span = info_span!(
    "http_request",
    %request_id,
    %method,
    %path,
);

let start = Instant::now();
let response = next.run(request).instrument(span).await;
```
:::

::: hint level=3 title="Status-based logging"
```rust
let duration_ms = start.elapsed().as_millis();
let status = response.status().as_u16();

if status >= 500 {
    tracing::error!(status, duration_ms, %request_id, "Server error");
} else if status >= 400 {
    warn!(status, duration_ms, %request_id, "Client error");
} else {
    info!(status, duration_ms, %request_id, "Request completed");
}
```
:::

::: solution
```rust
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::time::Instant;
use tracing::{info, info_span, warn, Instrument};
use uuid::Uuid;

pub async fn logging_middleware(
    request: Request,
    next: Next,
) -> Response {
    // Step 1: Generate a unique request ID
    let request_id = Uuid::new_v4().to_string()[..8].to_string();

    // Step 2: Extract request metadata before moving the request
    let method = request.method().to_string();
    let path = request.uri().path().to_string();

    // Step 3: Create a tracing span with request metadata
    let span = info_span!(
        "http_request",
        %request_id,
        %method,
        %path,
    );

    // Step 4: Record the start time
    let start = Instant::now();

    // Step 5: Execute the next handler within the span
    let response = next.run(request).instrument(span).await;

    // Step 6: Calculate elapsed duration
    let duration_ms = start.elapsed().as_millis();

    // Step 7: Extract response status
    let status = response.status().as_u16();

    // Step 8: Log with appropriate severity
    if status >= 500 {
        tracing::error!(
            status,
            duration_ms,
            %request_id,
            "Server error"
        );
    } else if status >= 400 {
        warn!(
            status,
            duration_ms,
            %request_id,
            "Client error"
        );
    } else {
        info!(
            status,
            duration_ms,
            %request_id,
            "Request completed"
        );
    }

    response
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "healthy"}))
}

async fn get_document() -> impl IntoResponse {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    Json(serde_json::json!({
        "id": "doc-123",
        "title": "Sample Document"
    }))
}

async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"})))
}

async fn internal_error() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "Something went wrong"})),
    )
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
        .init();

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/documents/:id", get(get_document))
        .route("/not-found", get(not_found))
        .route("/error", get(internal_error))
        .layer(middleware::from_fn(logging_middleware));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("Server starting on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}
```

### Explanation

The logging middleware wraps every request with structured observability:

- **Request ID**: A truncated UUID provides a unique correlation key. In production,
  this propagates through headers (`X-Request-Id`) to downstream services.

- **Tracing span**: `info_span!("http_request", ...)` creates a named scope. All log
  events emitted by downstream handlers appear nested within this span in the JSON
  output, providing automatic context.

- **`.instrument(span)`**: Attaches the span to the `next.run(request)` future. This
  ensures the span is active for the entire request lifecycle, including all async
  operations within handlers.

- **Duration**: `Instant::now()` before the handler and `.elapsed()` after gives
  accurate wall-clock timing without being affected by system clock adjustments.

- **Severity levels**: 5xx errors use `error!` (alerts operations teams), 4xx use
  `warn!` (expected client mistakes), and 2xx/3xx use `info!` (normal operations).
  This enables filtering in log aggregation systems like CloudWatch or Datadog.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest};
    use tower::ServiceExt;

    fn build_app() -> Router {
        Router::new()
            .route("/health", get(health))
            .route("/not-found", get(not_found))
            .route("/error", get(internal_error))
            .layer(middleware::from_fn(logging_middleware))
    }

    #[tokio::test]
    async fn test_successful_request_returns_200() {
        let app = build_app();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "healthy");
    }

    #[tokio::test]
    async fn test_not_found_returns_404() {
        let app = build_app();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/not-found")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_error_returns_500() {
        let app = build_app();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/error")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_middleware_does_not_alter_response_body() {
        let app = build_app();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Middleware should pass through the handler's response unchanged
        assert_eq!(json["status"], "healthy");
    }

    #[test]
    fn test_request_id_is_8_characters() {
        let id = Uuid::new_v4().to_string();
        let short_id = &id[..8];
        assert_eq!(short_id.len(), 8);
        // Should be valid hex characters
        assert!(short_id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn test_status_classification() {
        // Verify the status code classification logic
        let classify = |status: u16| -> &str {
            if status >= 500 {
                "error"
            } else if status >= 400 {
                "warn"
            } else {
                "info"
            }
        };

        assert_eq!(classify(200), "info");
        assert_eq!(classify(201), "info");
        assert_eq!(classify(301), "info");
        assert_eq!(classify(400), "warn");
        assert_eq!(classify(401), "warn");
        assert_eq!(classify(404), "warn");
        assert_eq!(classify(500), "error");
        assert_eq!(classify(503), "error");
    }
}
```
:::

::: reflection
- The request ID is generated inside the middleware. How would you support
  propagating an existing `X-Request-Id` header from an upstream load balancer
  while falling back to generating one if the header is absent?
- How would you add response body size logging without buffering the entire
  response in memory?
- In a microservices architecture, how would you propagate the tracing span
  context (trace ID, span ID) to downstream HTTP calls for distributed tracing?
:::
