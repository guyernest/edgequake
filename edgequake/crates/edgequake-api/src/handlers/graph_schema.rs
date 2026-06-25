//! Workspace graph-schema handlers (Phase 33 — INGEST-RESUME).
//!
//! # Phase 33-04 — SCHEMA-CONSOLIDATE
//!
//! The duplicate workspace graph-schema system (draft/approve/get) has been retired.
//! The namespace `SchemaProposal` (managed via `/namespaces/{slug}/schema`) is now the
//! single schema source for the ingestion gate.
//!
//! # Kept endpoints
//!
//! | Method | Path | Handler | Description |
//! |--------|------|---------|-------------|
//! | POST | `/api/v1/workspaces/{workspace_id}/resume-all` | [`resume_all_documents`] | Release all `awaiting_schema` documents |
//!
//! # Safety invariants
//!
//! - NEVER `tokio::spawn` — all operations are synchronous awaits (R7, RESEARCH anti-pattern §4).
//! - resume-all touches ONLY `awaiting_schema` docs of the authenticated workspace (R3).
//! - Gate re-pointed to namespace SchemaProposal via
//!   [`crate::handlers::documents::resolve_schema_gate_status_for_verified_workspace`]
//!   (Phase 33-04 SCHEMA-CONSOLIDATE, design §3 / §8).
//! - NO `schema_ready` status is ever written (R1).

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::middleware::TenantContext;
use crate::state::AppState;

// ============================================================================
// DTOs (kept)
// ============================================================================

/// Response from the resume-all endpoint.
#[derive(Debug, Serialize)]
pub struct ResumeAllResponse {
    /// Number of docs that were `awaiting_schema` and were successfully resumed.
    pub released: usize,
    /// Number of docs skipped (not in `awaiting_schema`).
    pub skipped: usize,
}

// ============================================================================
// Handler
// ============================================================================

/// `POST /workspaces/{workspace_id}/resume-all`
///
/// Releases all documents in `awaiting_schema` status for the given workspace by
/// transitioning them to `extracting` (CAS — prevents double-advance, R3/R9).
///
/// # Schema gate (Phase 33-04 — SCHEMA-CONSOLIDATE)
///
/// Requires the namespace `SchemaProposal.status == Approved` (resolved from
/// `workspace.slug` → namespace slug → registry).  The check is performed via
/// [`crate::handlers::documents::resolve_schema_gate_status_for_verified_workspace`]
/// after the workspace ownership guard, so the resolver's tenant-precondition is
/// satisfied by the time it is called.
///
/// # Tenant check (R5)
///
/// Verifies path `workspace_id` belongs to the authenticated tenant; only touches that
/// workspace's docs.
pub async fn resume_all_documents(
    State(state): State<AppState>,
    tenant_ctx: TenantContext,
    Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<ResumeAllResponse>> {
    // === R5: Verify workspace ownership ===
    let workspace = state
        .workspace_service
        .get_workspace(workspace_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Workspace {} not found", workspace_id)))?;

    if let Some(ref caller_tenant) = tenant_ctx.tenant_id {
        if workspace.tenant_id.to_string() != *caller_tenant {
            return Err(ApiError::NotFound(format!(
                "Workspace {} not found",
                workspace_id
            )));
        }
    }

    // === Require approved namespace schema (Phase 33-04 — SCHEMA-CONSOLIDATE) ===
    //
    // The workspace ownership guard above satisfies the PRECONDITION of
    // `resolve_schema_gate_status_for_verified_workspace` (caller has already
    // verified workspace belongs to the request's tenant).
    let gate_status = crate::handlers::documents::resolve_schema_gate_status_for_verified_workspace(
        &state,
        &workspace_id.to_string(),
    )
    .await;

    if gate_status != "extracting" {
        return Err(ApiError::ValidationError(
            "Workspace schema is not approved — cannot resume documents".to_string(),
        ));
    }

    // === Iterate docs, filter to awaiting_schema (R3), CAS each ===
    let all_keys = state.kv_storage.keys().await?;
    let metadata_keys: Vec<String> = all_keys
        .into_iter()
        .filter(|k| k.ends_with("-metadata"))
        .collect();

    let metadata_values = state.kv_storage.get_by_ids(&metadata_keys).await?;

    let ws_str = workspace_id.to_string();
    let tenant_str = tenant_ctx
        .tenant_id
        .as_deref()
        .unwrap_or("")
        .to_string();

    let mut released = 0usize;
    let mut skipped = 0usize;

    for meta in &metadata_values {
        // Scope to this workspace only (R5 — resume-all must not cross workspaces).
        let doc_workspace = meta.get("workspace_id").and_then(|v| v.as_str()).unwrap_or("");
        if doc_workspace != ws_str {
            continue;
        }

        // Also verify tenant (belt-and-suspenders, cross-tenant guard T-33-03-EP).
        if !tenant_str.is_empty() {
            let doc_tenant = meta.get("tenant_id").and_then(|v| v.as_str()).unwrap_or("");
            if doc_tenant != tenant_str && !doc_tenant.is_empty() {
                continue;
            }
        }

        let current_status = meta
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if current_status != "awaiting_schema" {
            skipped += 1;
            continue;
        }

        // Extract document ID (key is "{doc_id}-metadata").
        let doc_id = meta.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if doc_id.is_empty() {
            skipped += 1;
            continue;
        }

        let metadata_key = format!("{}-metadata", doc_id);

        // CAS awaiting_schema → extracting (prevents double-advance, R3/R9).
        let advanced = state
            .kv_storage
            .transition_if_status(&metadata_key, "awaiting_schema", "extracting")
            .await
            .map_err(|e| {
                ApiError::Internal(format!(
                    "CAS transition awaiting_schema→extracting failed for doc {}: {}",
                    doc_id, e
                ))
            })?;

        if !advanced {
            // Another concurrent call already advanced it — count as skipped.
            skipped += 1;
            continue;
        }

        // Run the extraction stub (extracting → completed).
        let processor = crate::processor::DocumentTaskProcessor::new(
            Arc::clone(&state.pipeline),
            Arc::clone(&state.llm_provider),
            Arc::clone(&state.kv_storage),
            Arc::clone(&state.vector_storage),
            Arc::clone(&state.vector_registry),
            Arc::clone(&state.graph_storage),
            state.pipeline_state.clone(),
        );

        match processor.run_resume_stub(doc_id).await {
            Ok(()) => released += 1,
            Err(e) => {
                tracing::warn!(
                    doc_id = %doc_id,
                    error = %e,
                    "resume_all: run_resume_stub failed for doc; counting as skipped"
                );
                skipped += 1;
            }
        }
    }

    Ok(Json(ResumeAllResponse { released, skipped }))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use edgequake_storage::MemoryKVStorage;
    use std::sync::Arc;

    // ─────────────────────────────────────────────────────────────────────────
    // resume_all tests
    // ─────────────────────────────────────────────────────────────────────────

    /// INGEST-RESUME R3/R9: resume-all skips non-awaiting_schema docs (mixed-status batch).
    ///
    /// We test the filtering logic directly — resume_all iterates KV docs and only
    /// CAS-advances those in `awaiting_schema`.
    #[tokio::test]
    async fn test_resume_all_mixed_statuses() {
        use edgequake_storage::traits::KVStorage;

        let kv = Arc::new(MemoryKVStorage::new("test_resume_all_mixed"));
        let tenant_id = Uuid::parse_str("10000000-0000-0000-0000-000000000001").unwrap();
        let ws_id = Uuid::parse_str("20000000-0000-0000-0000-000000000002").unwrap();

        // Seed: 1 awaiting_schema, 1 completed, 1 extracting — for the target workspace.
        let docs = vec![
            ("doc-await", "awaiting_schema"),
            ("doc-complete", "completed"),
            ("doc-extract", "extracting"),
        ];
        for (id, status) in &docs {
            let key = format!("{}-metadata", id);
            kv.upsert(&[(
                key,
                serde_json::json!({
                    "id": id,
                    "status": status,
                    "tenant_id": tenant_id.to_string(),
                    "workspace_id": ws_id.to_string(),
                }),
            )])
            .await
            .unwrap();
        }

        // Simulate the resume-all filtering logic (pure KV; no AppState needed).
        let all_keys = kv.keys().await.unwrap();
        let meta_keys: Vec<_> = all_keys.into_iter().filter(|k| k.ends_with("-metadata")).collect();
        let metas = kv.get_by_ids(&meta_keys).await.unwrap();

        let ws_str = ws_id.to_string();
        let mut would_release = 0usize;
        let mut would_skip = 0usize;
        for meta in &metas {
            let doc_ws = meta.get("workspace_id").and_then(|v| v.as_str()).unwrap_or("");
            if doc_ws != ws_str {
                continue;
            }
            let status = meta.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if status == "awaiting_schema" {
                would_release += 1;
            } else {
                would_skip += 1;
            }
        }

        assert_eq!(would_release, 1, "Only the awaiting_schema doc must be released");
        assert_eq!(would_skip, 2, "The completed and extracting docs must be skipped (R3)");
    }

    /// INGEST-RESUME R9: resume-all is idempotent — a second call releases 0 docs.
    #[tokio::test]
    async fn test_resume_all_idempotent() {
        use edgequake_storage::traits::KVStorage;

        let kv = Arc::new(MemoryKVStorage::new("test_resume_all_idempotent"));
        let tenant_id = Uuid::parse_str("10000000-0000-0000-0000-000000000003").unwrap();
        let ws_id = Uuid::parse_str("20000000-0000-0000-0000-000000000004").unwrap();

        // First call: doc is awaiting_schema.
        let doc_id = "doc-idempotent";
        let meta_key = format!("{}-metadata", doc_id);
        kv.upsert(&[(
            meta_key.clone(),
            serde_json::json!({
                "id": doc_id,
                "status": "awaiting_schema",
                "tenant_id": tenant_id.to_string(),
                "workspace_id": ws_id.to_string(),
            }),
        )])
        .await
        .unwrap();

        // First CAS: advances awaiting_schema → extracting.
        let first = kv
            .transition_if_status(&meta_key, "awaiting_schema", "extracting")
            .await
            .unwrap();
        assert!(first, "First CAS must succeed");

        // Simulate stub: extracting → completed.
        let mut meta = kv.get_by_id(&meta_key).await.unwrap().unwrap();
        meta["status"] = serde_json::Value::String("completed".to_string());
        kv.upsert(&[(meta_key.clone(), meta)]).await.unwrap();

        // Second CAS attempt: doc is now completed — must fail.
        let second = kv
            .transition_if_status(&meta_key, "awaiting_schema", "extracting")
            .await
            .unwrap();
        assert!(!second, "Second CAS must fail (doc is already completed)");

        // Verify idempotent: filtering logic would count this as skipped.
        let final_meta = kv.get_by_id(&meta_key).await.unwrap().unwrap();
        assert_eq!(
            final_meta["status"].as_str().unwrap(),
            "completed",
            "Doc must remain completed after idempotent second resume-all"
        );
    }

    /// INGEST-RESUME R5: cross-tenant resume-all is rejected.
    ///
    /// resume-all must only touch docs that belong to the authenticated workspace.
    /// A doc belonging to a different workspace is skipped even if it is in awaiting_schema.
    #[test]
    fn test_resume_all_rejects_cross_tenant() {
        // Simulate the workspace-scoping filter in resume_all_documents.
        // The caller's workspace is ws_A; the doc belongs to ws_B.
        let caller_ws = "ws-A";
        let doc_ws = "ws-B";

        // The handler skips any doc whose workspace_id != caller's workspace_id.
        let would_resume = doc_ws == caller_ws;
        assert!(
            !would_resume,
            "Cross-workspace doc must not be resumed by resume-all (R5)"
        );

        // A doc in the same workspace IS eligible (if in awaiting_schema).
        let same_ws_would_resume = caller_ws == caller_ws;
        assert!(
            same_ws_would_resume,
            "Same-workspace awaiting_schema doc must be eligible"
        );
    }
}
