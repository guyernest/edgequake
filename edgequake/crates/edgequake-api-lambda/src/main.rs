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

    // ------------------------------------------------------------------------------------
    // Admin-storage selection rule (33-09)
    // ------------------------------------------------------------------------------------
    // The live admin Lambda MUST persist tenants/workspaces/documents across cold starts —
    // the in-memory stores reset on every cold start (the 501/data-loss bug this wave kills).
    // So the live (non-baked) path REFUSES to start with in-memory stores unless an explicit
    // local-dev opt-in is set. The branches are mutually exclusive:
    //
    //   (1) SNAPSHOT_BAKED == "true"  -> baked read-only path: new_memory (unchanged). Baked
    //       servers are snapshot-backed and read-only; they never write admin state.
    //   (2) NOT baked + ALL THREE table env vars present -> live persistent path: build the
    //       AWS client ONCE here and call new_dynamodb(client, ...).
    //   (3) NOT baked + PARTIAL config (some-but-not-all of the three) -> FAIL FAST naming the
    //       missing var(s). Never silently degrade to in-memory.
    //   (4) NOT baked + ALL THREE absent -> FAIL FAST too, UNLESS EDGEQUAKE_ADMIN_STORAGE=memory
    //       (explicit dev opt-in), in which case new_memory is allowed with a loud warn.
    //
    // NAMESPACE_TABLE / WORKSPACE_TABLE / KV_TABLE must match the CDK env vars set in 33-10.
    let namespace_table = std::env::var("NAMESPACE_TABLE").ok();
    let workspace_table = std::env::var("WORKSPACE_TABLE").ok();
    let kv_table = std::env::var("KV_TABLE").ok();

    let state = if read_only {
        // (1) Baked read-only path — unchanged. new_memory is async (Phase 30 fix completion):
        // it awaits RawDocsStorage::new during AppState init. The earlier synchronous form used
        // Handle::current().block_on(), which panicked ("Cannot start a runtime from within a
        // runtime") inside this tokio runtime on every cold start.
        tracing::info!("admin storage: SNAPSHOT_BAKED read-only path (in-memory snapshot)");
        AppState::new_memory(None::<String>).await
    } else {
        match (namespace_table, workspace_table, kv_table) {
            // (2) Live persistent path — all three tables present. Build the AWS client ONCE
            // (mirrors edgequake-batch/src/main.rs) and pass it into new_dynamodb, which clones
            // it into all three DynamoDB stores. NEVER block_on (cold-start panic, Pitfall 3).
            (Some(ns), Some(ws), Some(kv)) => {
                tracing::info!(
                    namespace_table = %ns,
                    workspace_table = %ws,
                    kv_table = %kv,
                    "admin storage: live DynamoDB path (new_dynamodb)"
                );
                let cfg = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
                let client = aws_sdk_dynamodb::Client::new(&cfg);
                AppState::new_dynamodb(client, ns, ws, kv, None::<String>).await
            }
            // (4) All three absent.
            (None, None, None) => {
                if std::env::var("EDGEQUAKE_ADMIN_STORAGE").as_deref() == Ok("memory") {
                    tracing::warn!(
                        "EDGEQUAKE_ADMIN_STORAGE=memory: using in-memory admin stores \
                         (dev only; state will NOT survive a cold start)"
                    );
                    AppState::new_memory(None::<String>).await
                } else {
                    anyhow::bail!(
                        "live admin path: no admin storage configured (NAMESPACE_TABLE/\
                         WORKSPACE_TABLE/KV_TABLE all unset); refusing to start with in-memory \
                         stores. Set all three table env vars for the live path, or set \
                         EDGEQUAKE_ADMIN_STORAGE=memory for local dev."
                    );
                }
            }
            // (3) Partial config — some but not all three present. Fail fast naming the missing var(s).
            (ns, ws, kv) => {
                let mut missing = Vec::new();
                if ns.is_none() {
                    missing.push("NAMESPACE_TABLE");
                }
                if ws.is_none() {
                    missing.push("WORKSPACE_TABLE");
                }
                if kv.is_none() {
                    missing.push("KV_TABLE");
                }
                anyhow::bail!(
                    "live admin path: missing required env var(s) {}; refusing to start with \
                     in-memory stores (partial admin-storage config)",
                    missing.join(", ")
                );
            }
        }
    };

    let server = Server::new(config, state);
    server.run().await.map_err(Into::into)
}
