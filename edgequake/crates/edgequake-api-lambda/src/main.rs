//! Lambda Web Adapter entry point for the edgequake-api axum server.
//!
//! Mounts the existing in-memory edgequake-api Server router on 0.0.0.0:8080.
//! The Lambda Web Adapter (LWA) layer forwards Lambda invocations to this port
//! and polls GET /health as the readiness probe before accepting traffic.
//!
//! Phase 25 D-06 scope fence: this file adds NO new routes, NO Cognito proxy,
//! and NO CDK route mounting. GET /health is the only surface verified this phase.
//! All gateway wiring is deferred to Phase 26.

use edgequake_api::{AppState, Server, ServerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().json().init();

    // REG-03: Mount the read-only guard when SNAPSHOT_BAKED=true.
    // This env var is injected at deploy time by Plan 01 Task 2
    // (upload-deployment's injectSnapshotBakedEnvironment). This code only reads it.
    let read_only = std::env::var("SNAPSHOT_BAKED").as_deref() == Ok("true");

    if read_only {
        tracing::info!("SNAPSHOT_BAKED=true: mounting read-only guard — write methods will return 405");
    }

    let config = ServerConfig {
        host: "0.0.0.0".to_string(),
        port: 8080, // must match AWS_LWA_PORT env var (Phase 26 CDK wiring)
        enable_cors: true,
        enable_compression: false, // LWA handles compression
        enable_swagger: false,     // no Swagger UI in Lambda
        read_only,
    };

    // new_memory is async (Phase 30 fix completion): it awaits RawDocsStorage::new during
    // AppState init. Falls back to Mock LLM provider when no API key is set (correct for this
    // skeleton, T-25-READY). The earlier synchronous form used Handle::current().block_on(),
    // which panicked ("Cannot start a runtime from within a runtime") inside this tokio
    // runtime on every cold start.
    let state = AppState::new_memory(None::<String>).await;

    let server = Server::new(config, state);
    server.run().await.map_err(Into::into)
}
