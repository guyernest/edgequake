## 13.1 Structured Logging

Traditional logging produces freeform text lines that are easy for humans to
read but difficult for machines to parse. When you have a single server and
ten requests per second, `println!` debugging works. When you have a
distributed system processing thousands of requests across multiple
services, you need structured logging -- log entries with typed, queryable
fields that can be aggregated, filtered, and correlated.

The `tracing` crate is the Rust ecosystem's standard for structured
diagnostics. Unlike `log`, which produces flat text lines, `tracing` provides
**spans** (regions of execution with duration) and **events** (point-in-time
occurrences) with arbitrary key-value fields.

### Setting Up tracing

Add the required dependencies to your `Cargo.toml`:

```toml
[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
tracing-appender = "0.2"
uuid = { version = "1", features = ["v4"] }
```

Initialize the subscriber at application startup. The subscriber determines
how span and event data is collected and formatted:

```rust
use tracing_subscriber::{
    fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

/// Initialize the tracing subscriber for structured logging.
///
/// Configuration is driven by the `RUST_LOG` environment variable:
///   - RUST_LOG=info           -- info level for all crates
///   - RUST_LOG=edgequake=debug -- debug level for EdgeQuake crates only
///   - RUST_LOG=warn,edgequake=trace -- trace for EdgeQuake, warn for everything else
pub fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            // Default: info for EdgeQuake crates, warn for dependencies.
            EnvFilter::new("warn,edgequake=info,edgequake_api=info,edgequake_batch=info")
        });

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .json()                    // JSON output for machine parsing
                .with_target(true)         // Include module path
                .with_thread_ids(true)     // Include thread ID
                .with_span_list(true)      // Include active span hierarchy
                .with_timer(fmt::time::UtcTime::rfc_3339())
        )
        .init();

    tracing::info!("tracing initialized");
}
```

### Structured Log Output

With JSON formatting enabled, a log entry looks like this:

```json
{
  "timestamp": "2025-03-15T14:32:01.456Z",
  "level": "INFO",
  "target": "edgequake_api::handlers::query",
  "message": "query completed",
  "span": {
    "name": "handle_query",
    "request_id": "req-abc-123",
    "workspace_id": "acme"
  },
  "fields": {
    "entities_found": 12,
    "relationships_found": 34,
    "duration_ms": 247
  },
  "threadId": 7
}
```

Every field is queryable. In CloudWatch Logs Insights, you can write:

```sql
fields @timestamp, fields.duration_ms, span.workspace_id
| filter span.workspace_id = "acme"
| filter fields.duration_ms > 1000
| sort @timestamp desc
| limit 50
```

### Span Hierarchies

Spans create a tree structure that mirrors the call hierarchy. When a query
handler calls the graph storage, which calls the database, the spans nest
naturally:

```rust
use tracing::{info, info_span, instrument, Instrument};

/// Handle a query request.
///
/// The #[instrument] macro automatically creates a span with
/// the function name and any parameters marked for inclusion.
#[instrument(
    name = "handle_query",
    skip(factory, query),
    fields(
        request_id = %request_id,
        workspace_id = %claims.workspace_id,
        query_mode = %query.mode,
    )
)]
async fn handle_query(
    Extension(claims): Extension<Claims>,
    Extension(factory): Extension<Arc<StorageFactory>>,
    Json(query): Json<QueryRequest>,
    request_id: String,
) -> Result<Json<QueryResponse>, AppError> {
    // This span is active for the entire handler execution.
    // All tracing calls within this function are children of this span.

    info!("starting query processing");

    let storage = factory.for_workspace(&claims.workspace_id);

    // Extract keywords -- creates a child span.
    let keywords = extract_keywords(&query.text)
        .instrument(info_span!("extract_keywords"))
        .await?;

    info!(keyword_count = keywords.len(), "keywords extracted");

    // Query the graph -- creates another child span.
    let graph_results = query_graph(&storage, &keywords)
        .instrument(info_span!("query_graph", keyword_count = keywords.len()))
        .await?;

    // Query vectors -- creates another child span.
    let vector_results = query_vectors(&storage, &query.text)
        .instrument(info_span!("query_vectors"))
        .await?;

    info!(
        graph_entities = graph_results.entities.len(),
        vector_chunks = vector_results.chunks.len(),
        "retrieval complete"
    );

    // Synthesize the answer.
    let response = synthesize(&graph_results, &vector_results, &query)
        .instrument(info_span!("synthesize"))
        .await?;

    info!(
        response_tokens = response.token_count,
        "query completed"
    );

    Ok(Json(response))
}
```

The resulting span hierarchy:

```text
handle_query [request_id=req-abc-123, workspace_id=acme, query_mode=hybrid]
  |-- extract_keywords                    (12ms)
  |-- query_graph [keyword_count=5]       (89ms)
  |-- query_vectors                       (134ms)
  |-- synthesize                          (1,240ms)
```

This hierarchy immediately tells you that synthesis (the LLM call) dominates
latency, not the retrieval steps.

### Request Correlation IDs

Every request should carry a unique correlation ID that ties together all log
entries for that request. This is essential for debugging in multi-tenant
systems where logs from different tenants are interleaved:

```rust
use axum::{extract::Request, middleware::Next, response::Response};
use uuid::Uuid;

/// Middleware that generates a request correlation ID.
///
/// The ID is added to the request extensions and included in
/// the response headers for client-side correlation.
pub async fn correlation_id_middleware(
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = request
        .headers()
        .get("X-Request-ID")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Store in request extensions for handlers to access.
    request.extensions_mut().insert(RequestId(request_id.clone()));

    // Create a tracing span for the entire request.
    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %request.method(),
        path = %request.uri().path(),
    );

    // Execute the rest of the middleware stack within this span.
    let response = {
        let _guard = span.enter();
        next.run(request).await
    };

    // Include the request ID in the response headers.
    let mut response = response;
    response.headers_mut().insert(
        "X-Request-ID",
        request_id.parse().unwrap(),
    );

    response
}

/// Newtype wrapper for the request ID, used as an Axum extension.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);
```

### Log Levels by Component

Different components warrant different log levels in production:

```rust
// In your RUST_LOG configuration:
// RUST_LOG=warn,edgequake_api=info,edgequake_storage=info,edgequake_batch=info,edgequake_llm=debug

// API handlers: info level -- log request start/end, not internals.
#[instrument(skip_all, fields(workspace_id = %claims.workspace_id))]
async fn handle_ingest(claims: Claims, payload: IngestRequest) -> Result<()> {
    tracing::info!(documents = payload.documents.len(), "ingestion started");
    // ... processing ...
    tracing::info!(entities = stats.entities, "ingestion complete");
    Ok(())
}

// Storage operations: info for lifecycle, debug for details.
async fn upsert_entities(entities: &[Entity]) -> Result<()> {
    tracing::info!(count = entities.len(), "upserting entities");
    for entity in entities {
        tracing::debug!(name = %entity.name, entity_type = %entity.entity_type, "upserting entity");
    }
    Ok(())
}

// LLM calls: debug level with token counts for cost tracking.
async fn call_llm(prompt: &str) -> Result<LlmResponse> {
    tracing::debug!(prompt_tokens = prompt.len() / 4, "calling LLM"); // rough estimate
    let response = client.chat(prompt).await?;
    tracing::debug!(
        completion_tokens = response.usage.completion_tokens,
        total_tokens = response.usage.total_tokens,
        model = %response.model,
        "LLM response received"
    );
    Ok(response)
}

// Errors: always warn or error level, with full context.
async fn query_graph(storage: &dyn GraphStorage, keywords: &[String]) -> Result<GraphResults> {
    match storage.get_entities(&query).await {
        Ok(entities) => {
            tracing::info!(count = entities.len(), "graph query succeeded");
            Ok(GraphResults { entities })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                keywords = ?keywords,
                "graph query failed"
            );
            Err(e.into())
        }
    }
}
```

### Recommended Log Level Guidelines

| Level | Use For | Example |
|-------|---------|---------|
| `error` | Failures that require attention | Database connection lost, LLM API 500 |
| `warn` | Unexpected but recoverable situations | Token approaching limit, slow query |
| `info` | Business-significant events | Request start/end, ingestion complete |
| `debug` | Diagnostic detail for troubleshooting | Individual entity upserts, LLM token counts |
| `trace` | Fine-grained execution flow | SQL queries, HTTP headers, serialization steps |

### Conditional Compilation for Performance

In hot paths, even constructing the log message has a cost. The `tracing`
crate's macros are zero-cost when the level is disabled, but you should still
be mindful:

```rust
// Good: the tracing macro checks the level before evaluating arguments.
tracing::trace!(embedding_dim = embedding.len(), "processing embedding");

// Potentially expensive: building a debug representation on every call.
// Only do this at trace level inside hot loops.
tracing::trace!(embedding = ?embedding, "full embedding vector");

// For very hot paths, use the enabled! macro for conditional logic:
if tracing::enabled!(tracing::Level::TRACE) {
    let similarity = compute_cosine_similarity(a, b);
    tracing::trace!(similarity = %similarity, "vector similarity computed");
}
```

### Key Takeaways

- Structured logging with `tracing` produces machine-parseable JSON output
  with typed fields, enabling powerful log queries.
- Span hierarchies mirror the call tree, making it easy to trace execution
  flow and identify latency bottlenecks.
- Every request should carry a correlation ID that appears in all log entries,
  enabling end-to-end request tracing.
- Use appropriate log levels: `info` for business events, `debug` for
  diagnostics, `error` for failures requiring attention.
- The `#[instrument]` macro is the simplest way to create spans with
  function-level granularity.

---

*Next: [13.2 Middleware Stack](ch13-02-middleware-stack.md)*
