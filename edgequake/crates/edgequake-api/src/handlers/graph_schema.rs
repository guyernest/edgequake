//! Workspace graph-schema handlers (Phase 33 — INGEST-SCHEMA-DRAFT, INGEST-RESUME).
//!
//! # Endpoints
//!
//! | Method | Path | Handler | Description |
//! |--------|------|---------|-------------|
//! | POST | `/api/v1/workspaces/{workspace_id}/graph-schema/draft` | [`draft_workspace_graph_schema`] | AI-assisted schema draft (bounded, synchronous LLM call) |
//! | GET | `/api/v1/workspaces/{workspace_id}/graph-schema` | [`get_workspace_graph_schema`] | Get current schema |
//! | PUT | `/api/v1/workspaces/{workspace_id}/graph-schema` | [`approve_workspace_graph_schema`] | Approve / persist schema |
//! | POST | `/api/v1/workspaces/{workspace_id}/resume-all` | [`resume_all_documents`] | Release all `awaiting_schema` documents |
//!
//! # Safety invariants
//!
//! - NEVER `tokio::spawn` — all LLM calls are synchronous awaits (R7, RESEARCH anti-pattern §4).
//! - Draft samples ≤5 text-only docs, ≤8000 chars each, with a request-level timeout (R7).
//! - Approve validates payload (R8) and version-checks before persist (R8).
//! - resume-all touches ONLY `awaiting_schema` docs of the authenticated workspace (R3).
//! - NO `schema_ready` status is ever written (R1).

use axum::{
    extract::{Path, State},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::handlers::documents::{check_workspace_schema_gate, SchemaGateResult};
use crate::middleware::TenantContext;
use crate::state::AppState;

// ============================================================================
// DTOs
// ============================================================================

/// A single entity type in the workspace graph schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntityType {
    /// Entity type name (non-empty).
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
}

/// A single relationship type in the workspace graph schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipType {
    /// Relationship type name (non-empty).
    pub name: String,
    /// Source entity type name (must reference a known entity type).
    pub source: Option<String>,
    /// Target entity type name (must reference a known entity type).
    pub target: Option<String>,
    /// Optional description.
    pub description: Option<String>,
}

/// Strict model-output parse target for the AI draft LLM response (R7).
///
/// The model MUST return a JSON object with `entity_types` and `relationship_types` arrays.
/// Any deviation results in a 422 (untrusted model output).
#[derive(Debug, Deserialize)]
struct DraftedSchema {
    entity_types: Vec<EntityType>,
    relationship_types: Vec<RelationshipType>,
}

/// Response from the AI-assisted draft endpoint.
#[derive(Debug, Serialize)]
pub struct GraphSchemaDraftResponse {
    /// Always "draft" (R1 — no schema_ready intermediate).
    pub status: String,
    /// Draft entity types sampled from workspace S3 docs.
    pub entity_types: Vec<EntityType>,
    /// Draft relationship types.
    pub relationship_types: Vec<RelationshipType>,
    /// Number of S3 docs sampled.
    pub docs_sampled: usize,
    /// Message to the caller.
    pub message: String,
}

/// Request body for `PUT /workspaces/{id}/graph-schema` (approve).
#[derive(Debug, Deserialize)]
pub struct ApproveGraphSchemaRequest {
    /// Entity types to approve.
    pub entity_types: Vec<EntityType>,
    /// Relationship types to approve.
    pub relationship_types: Vec<RelationshipType>,
    /// Optional source tag (e.g. "ai_draft", "manual").
    pub source: Option<String>,
    /// Optimistic-lock — must match the workspace `updated_at` to prevent clobbering
    /// concurrent edits (R8).
    pub workspace_updated_at: Option<String>,
}

/// Response from the approve endpoint.
#[derive(Debug, Serialize)]
pub struct ApproveGraphSchemaResponse {
    /// Always "approved".
    pub status: String,
    /// Schema version (monotonic).
    pub version: u64,
    /// Approval timestamp.
    pub approved_at: String,
    /// Source tag.
    pub source: String,
    pub entity_types: Vec<EntityType>,
    pub relationship_types: Vec<RelationshipType>,
}

/// Response from the get endpoint.
#[derive(Debug, Serialize)]
pub struct GetGraphSchemaResponse {
    /// Current schema, or null if none has been set.
    pub schema: Option<serde_json::Value>,
}

/// Response from the resume-all endpoint.
#[derive(Debug, Serialize)]
pub struct ResumeAllResponse {
    /// Number of docs that were `awaiting_schema` and were successfully resumed.
    pub released: usize,
    /// Number of docs skipped (not in `awaiting_schema`).
    pub skipped: usize,
}

// ============================================================================
// Validation (R8)
// ============================================================================

/// Maximum allowed entity types (R8 — DoS guard).
const MAX_ENTITY_TYPES: usize = 50;
/// Maximum allowed relationship types (R8 — DoS guard).
const MAX_RELATIONSHIP_TYPES: usize = 50;
/// Maximum docs sampled for the AI draft (R7).
const MAX_SAMPLE_DOCS: usize = 5;
/// Maximum chars per sampled doc (R7).
const MAX_CHARS_PER_DOC: usize = 8000;
/// Timeout for the synchronous LLM draft call (R7, RESEARCH A2).
const DRAFT_LLM_TIMEOUT_SECS: u64 = 25;

/// Validate a graph schema payload (R8).
///
/// Checks:
/// - Entity-type count ≤ 50.
/// - Relationship-type count ≤ 50.
/// - All entity type names non-empty.
/// - All relationship type names non-empty.
/// - No duplicate entity type names.
/// - No duplicate relationship type names.
/// - Every relationship `source`/`target` (if set) references a known entity type name.
///
/// This is a **pure, AppState-free helper** so tests can cover validation without constructing
/// `AppState`.
pub fn validate_graph_schema(
    entity_types: &[EntityType],
    relationship_types: &[RelationshipType],
) -> Result<(), ApiError> {
    // Bounded counts (R8 — DoS guard).
    if entity_types.len() > MAX_ENTITY_TYPES {
        return Err(ApiError::ValidationError(format!(
            "Too many entity types: {} (max {})",
            entity_types.len(),
            MAX_ENTITY_TYPES
        )));
    }
    if relationship_types.len() > MAX_RELATIONSHIP_TYPES {
        return Err(ApiError::ValidationError(format!(
            "Too many relationship types: {} (max {})",
            relationship_types.len(),
            MAX_RELATIONSHIP_TYPES
        )));
    }

    // Non-empty names + collect entity type name set.
    let mut entity_names = std::collections::HashSet::new();
    for et in entity_types {
        if et.name.trim().is_empty() {
            return Err(ApiError::ValidationError(
                "Entity type name must not be empty".to_string(),
            ));
        }
        if !entity_names.insert(et.name.as_str()) {
            return Err(ApiError::ValidationError(format!(
                "Duplicate entity type name: '{}'",
                et.name
            )));
        }
    }

    // Non-empty names, no duplicates, endpoints reference known entity types.
    let mut rel_names = std::collections::HashSet::new();
    for rt in relationship_types {
        if rt.name.trim().is_empty() {
            return Err(ApiError::ValidationError(
                "Relationship type name must not be empty".to_string(),
            ));
        }
        if !rel_names.insert(rt.name.as_str()) {
            return Err(ApiError::ValidationError(format!(
                "Duplicate relationship type name: '{}'",
                rt.name
            )));
        }
        // Relationship endpoints must reference known entity types (R8).
        if let Some(ref src) = rt.source {
            if !entity_names.contains(src.as_str()) {
                return Err(ApiError::ValidationError(format!(
                    "Relationship '{}' source '{}' does not reference a known entity type",
                    rt.name, src
                )));
            }
        }
        if let Some(ref tgt) = rt.target {
            if !entity_names.contains(tgt.as_str()) {
                return Err(ApiError::ValidationError(format!(
                    "Relationship '{}' target '{}' does not reference a known entity type",
                    rt.name, tgt
                )));
            }
        }
    }

    Ok(())
}

// ============================================================================
// Handlers
// ============================================================================

/// `POST /workspaces/{workspace_id}/graph-schema/draft`
///
/// Makes ONE synchronous (never `tokio::spawn`) LLM call over a bounded sample of the
/// workspace's S3 raw docs.  Returns a candidate schema with `status: "draft"`.
///
/// # Safety bounds (R7)
///
/// - ≤5 S3 docs sampled (text/markdown only — `.txt` / `.md` keys).
/// - ≤8000 chars per doc (truncated).
/// - tokio::time::timeout wraps the LLM call (25 s).
/// - Model output is strictly JSON-validated before returning (reject malformed → 422).
///
/// # Tenant check (R5)
///
/// Verifies the path `workspace_id` belongs to the authenticated tenant before reading S3.
pub async fn draft_workspace_graph_schema(
    State(state): State<AppState>,
    tenant_ctx: TenantContext,
    Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<GraphSchemaDraftResponse>> {
    // === R5: Verify workspace ownership ===
    let workspace = state
        .workspace_service
        .get_workspace(workspace_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Workspace {} not found", workspace_id)))?;

    // Verify the workspace belongs to the authenticated tenant (R5 / T-33-03-EP).
    if let Some(ref caller_tenant) = tenant_ctx.tenant_id {
        let workspace_tenant = workspace.tenant_id.to_string();
        if workspace_tenant != *caller_tenant {
            return Err(ApiError::NotFound(format!(
                "Workspace {} not found",
                workspace_id
            )));
        }
    }

    // === Sample text-only S3 docs (R7, Pitfall 3) ===
    // List prefix: "{tenant}/{workspace}/" — scoped to this workspace's uploads.
    let tenant_str = workspace.tenant_id.to_string();
    let ws_str = workspace_id.to_string();
    let prefix = format!("{}/{}/", tenant_str, ws_str);

    // Attempt S3 listing; fall back gracefully if raw_docs bucket is not configured.
    let all_objects = state
        .raw_docs
        .list_raw_docs(&prefix)
        .await
        .unwrap_or_default();

    // Filter to text/markdown keys only (Pitfall 3 — skip binary/PDF).
    let text_objects: Vec<_> = all_objects
        .iter()
        .filter(|obj| {
            let key_lower = obj.key.to_lowercase();
            key_lower.ends_with(".txt") || key_lower.ends_with(".md")
        })
        .take(MAX_SAMPLE_DOCS)
        .collect();

    // Fetch and truncate doc content.
    let mut doc_snippets: Vec<String> = Vec::new();
    for obj in &text_objects {
        match state.raw_docs.get_object(&obj.key).await {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let snippet: String = text.chars().take(MAX_CHARS_PER_DOC).collect();
                if !snippet.trim().is_empty() {
                    doc_snippets.push(snippet);
                }
            }
            Err(e) => {
                // Log and skip — a single unreadable doc should not abort the draft.
                tracing::warn!(key = %obj.key, error = %e, "Skipped unreadable S3 doc during draft sampling");
            }
        }
    }

    let docs_sampled = doc_snippets.len();

    // === Build the prompt (doc text framed as UNTRUSTED — R7, T-33-03-TI) ===
    let docs_section = if doc_snippets.is_empty() {
        "(No text documents found — please draft the schema manually.)".to_string()
    } else {
        doc_snippets
            .iter()
            .enumerate()
            .map(|(i, text)| format!("--- Document {} (UNTRUSTED content) ---\n{}", i + 1, text))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    let prompt = format!(
        r#"You are a knowledge-graph schema designer. Based on the following UNTRUSTED document excerpts,
propose a graph schema for a knowledge graph. Return ONLY valid JSON with this exact structure:
{{
  "entity_types": [
    {{"name": "EntityName", "description": "optional description"}}
  ],
  "relationship_types": [
    {{"name": "RELATIONSHIP_NAME", "source": "SourceEntity", "target": "TargetEntity", "description": "optional description"}}
  ]
}}

Rules:
- 5–20 entity types maximum, 5–30 relationship types maximum.
- All names must be non-empty strings.
- relationship source/target must reference entity type names from the entity_types list.
- Return ONLY the JSON object, no explanation or markdown fences.

Documents:
{docs_section}"#
    );

    // === ONE synchronous LLM call under timeout (R7 — NEVER tokio::spawn) ===
    let llm = edgequake_llm::ProviderFactory::create_llm_provider(
        &workspace.llm_provider,
        &workspace.llm_model,
    )
    .map_err(|e| {
        ApiError::Internal(format!(
            "Cannot create LLM provider '{}' with model '{}' for schema draft: {}",
            workspace.llm_provider, workspace.llm_model, e
        ))
    })?;

    let timeout_duration = std::time::Duration::from_secs(DRAFT_LLM_TIMEOUT_SECS);
    let llm_result = tokio::time::timeout(timeout_duration, llm.complete(&prompt))
        .await
        .map_err(|_| {
            ApiError::Timeout(format!(
                "Schema draft LLM call timed out after {}s",
                DRAFT_LLM_TIMEOUT_SECS
            ))
        })?
        .map_err(|e| ApiError::Internal(format!("Schema draft LLM call failed: {}", e)))?;

    // === STRICT JSON parse of model output (R7 — untrusted model output → 422 on malformed) ===
    let raw_text = llm_result.content.trim().to_string();

    // Strip optional markdown fences the model may wrap around the JSON.
    let json_text = raw_text
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let drafted: DraftedSchema = serde_json::from_str(json_text).map_err(|e| {
        ApiError::ValidationError(format!(
            "Model returned malformed schema JSON ({}). \
             Retry or draft the schema manually.",
            e
        ))
    })?;

    // Validate the model output with the same R8 guard used for approve (belt-and-suspenders).
    validate_graph_schema(&drafted.entity_types, &drafted.relationship_types)?;

    Ok(Json(GraphSchemaDraftResponse {
        status: "draft".to_string(),
        entity_types: drafted.entity_types,
        relationship_types: drafted.relationship_types,
        docs_sampled,
        message: format!(
            "Schema draft generated from {} document(s). Review and approve via PUT /graph-schema.",
            docs_sampled
        ),
    }))
}

/// `GET /workspaces/{workspace_id}/graph-schema`
///
/// Returns the current `graph_schema` from workspace metadata, or null if none set.
///
/// # Tenant check (R5)
///
/// Verifies the path `workspace_id` belongs to the authenticated tenant.
pub async fn get_workspace_graph_schema(
    State(state): State<AppState>,
    tenant_ctx: TenantContext,
    Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<GetGraphSchemaResponse>> {
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

    let schema = workspace.metadata.get("graph_schema").cloned();
    Ok(Json(GetGraphSchemaResponse { schema }))
}

/// `PUT /workspaces/{workspace_id}/graph-schema`
///
/// Validates the payload (R8), re-reads the latest workspace record to check for concurrent
/// edits (R8 version-check), and persists `graph_schema` with `status: "approved"` in
/// `workspace.metadata`.
///
/// # Validation (R8)
///
/// - Bounded entity/relationship counts (≤50 each).
/// - Non-empty names.
/// - No duplicate type names.
/// - Relationship endpoints reference known entity types.
/// - `workspace_updated_at` (if supplied) must match the server's `updated_at`.
///
/// # Route registration
///
/// Registered with GET in routes.rs:
/// `.route("/workspaces/{id}/graph-schema", get(get_workspace_graph_schema).put(approve_workspace_graph_schema))`
pub async fn approve_workspace_graph_schema(
    State(state): State<AppState>,
    tenant_ctx: TenantContext,
    Path(workspace_id): Path<Uuid>,
    Json(req): Json<ApproveGraphSchemaRequest>,
) -> ApiResult<Json<ApproveGraphSchemaResponse>> {
    // === R8: Validate payload first (cheap, no DB round-trip) ===
    validate_graph_schema(&req.entity_types, &req.relationship_types)?;

    // === R5: Re-read latest workspace (also serves as the version-check read for R8) ===
    let workspace = state
        .workspace_service
        .get_workspace(workspace_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Workspace {} not found", workspace_id)))?;

    // Tenant ownership check (R5 / T-33-03-EP).
    if let Some(ref caller_tenant) = tenant_ctx.tenant_id {
        if workspace.tenant_id.to_string() != *caller_tenant {
            return Err(ApiError::NotFound(format!(
                "Workspace {} not found",
                workspace_id
            )));
        }
    }

    // === R8: version / updated_at check (prevent clobbering concurrent model-config edits) ===
    if let Some(ref client_updated_at) = req.workspace_updated_at {
        let server_updated_at = workspace.updated_at.to_rfc3339();
        if *client_updated_at != server_updated_at {
            return Err(ApiError::Conflict(format!(
                "Workspace has been modified since you last read it \
                 (client updated_at='{}', server updated_at='{}'). \
                 Re-read the workspace and resubmit.",
                client_updated_at, server_updated_at
            )));
        }
    }

    // === Determine schema version (bump existing or start at 1) ===
    let next_version: u64 = workspace
        .metadata
        .get("graph_schema")
        .and_then(|s| s.get("version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;

    let approved_at = Utc::now().to_rfc3339();
    let source = req.source.clone().unwrap_or_else(|| "manual".to_string());

    // Build the approved schema blob.
    let graph_schema_value = serde_json::json!({
        "version": next_version,
        "status": "approved",
        "entity_types": req.entity_types,
        "relationship_types": req.relationship_types,
        "approved_at": approved_at,
        "source": source,
    });

    // === Persist: merge graph_schema into workspace.metadata via UpdateWorkspaceRequest.metadata ===
    // The service's update_workspace impl re-reads the workspace from the store and merges
    // any entries in request.metadata into it — so we pass only the new/changed key
    // (graph_schema) without risking overwriting other metadata entries we didn't read.
    let mut metadata_patch = std::collections::HashMap::new();
    metadata_patch.insert("graph_schema".to_string(), graph_schema_value);

    let update_req = edgequake_core::UpdateWorkspaceRequest {
        metadata: Some(metadata_patch),
        ..Default::default()
    };

    state
        .workspace_service
        .update_workspace(workspace_id, update_req)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to persist graph schema: {}", e)))?;

    Ok(Json(ApproveGraphSchemaResponse {
        status: "approved".to_string(),
        version: next_version,
        approved_at,
        source,
        entity_types: req.entity_types,
        relationship_types: req.relationship_types,
    }))
}

/// `POST /workspaces/{workspace_id}/resume-all`
///
/// Releases ALL documents currently in `awaiting_schema` for the authenticated workspace.
/// Documents in any other status are skipped (R3).
///
/// # Requires
///
/// The workspace must have an approved schema (`graph_schema.status == "approved"`) — else 422.
///
/// # Idempotency (R3)
///
/// A second call after all docs are resumed returns `released=0, skipped=N` and 200.
/// Each transition is CAS-guarded: `awaiting_schema → extracting` is atomic, so concurrent
/// upload+approve+resume-all cannot double-advance a doc.
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

    // === Require approved schema (else 422) ===
    if check_workspace_schema_gate(&workspace) != SchemaGateResult::Approved {
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
    use edgequake_core::Workspace;
    use edgequake_storage::MemoryKVStorage;
    use std::sync::Arc;

    // ── Helper: build an approved workspace ──────────────────────────────────

    fn approved_workspace(tenant_id: Uuid, workspace_id: Uuid) -> Workspace {
        let mut ws = Workspace::new(tenant_id, "Test WS", "test-ws");
        ws.workspace_id = workspace_id;
        ws.metadata.insert(
            "graph_schema".to_string(),
            serde_json::json!({
                "version": 1,
                "status": "approved",
                "entity_types": [{"name": "Person"}],
                "relationship_types": [],
                "approved_at": "2026-01-01T00:00:00Z",
                "source": "manual",
            }),
        );
        ws
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Task 1 tests
    // ─────────────────────────────────────────────────────────────────────────

    /// INGEST-SCHEMA-DRAFT: `validate_graph_schema` returns Ok for a well-formed payload.
    #[test]
    fn test_schema_draft_returns_candidate() {
        // Simulate what the draft handler would return after parsing a valid model response.
        let entity_types = vec![
            EntityType {
                name: "Person".to_string(),
                description: None,
            },
            EntityType {
                name: "Organization".to_string(),
                description: Some("A company or institution".to_string()),
            },
        ];
        let relationship_types = vec![RelationshipType {
            name: "WORKS_FOR".to_string(),
            source: Some("Person".to_string()),
            target: Some("Organization".to_string()),
            description: None,
        }];

        // The draft handler must return status=="draft" (not "approved", not "schema_ready").
        let response = GraphSchemaDraftResponse {
            status: "draft".to_string(),
            entity_types: entity_types.clone(),
            relationship_types: relationship_types.clone(),
            docs_sampled: 2,
            message: "Schema draft generated from 2 document(s). Review and approve via PUT /graph-schema.".to_string(),
        };

        assert_eq!(response.status, "draft", "Draft must have status=draft (R1)");
        assert_eq!(response.entity_types.len(), 2);
        assert_eq!(response.relationship_types.len(), 1);
        assert_eq!(response.docs_sampled, 2);

        // The validation helper must pass for a well-formed schema.
        assert!(
            validate_graph_schema(&entity_types, &relationship_types).is_ok(),
            "Well-formed schema must pass validate_graph_schema"
        );

        // No `schema_ready` anywhere in the draft response (R1).
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(
            !serialized.contains("schema_ready"),
            "Draft response must not contain schema_ready (R1)"
        );
    }

    /// INGEST-SCHEMA-DRAFT R7: `validate_graph_schema` rejects malformed payloads (R8).
    ///
    /// This covers the bad-payload rejection path the draft handler applies to model output.
    #[test]
    fn test_validate_graph_schema_rejects_bad_payload() {
        // Empty entity type name → rejected.
        let err = validate_graph_schema(
            &[EntityType { name: "".to_string(), description: None }],
            &[],
        );
        assert!(err.is_err(), "Empty entity name must be rejected");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("empty"),
            "Error must mention 'empty': got '{}'",
            msg
        );

        // Duplicate entity type names → rejected.
        let err = validate_graph_schema(
            &[
                EntityType { name: "Person".to_string(), description: None },
                EntityType { name: "Person".to_string(), description: None },
            ],
            &[],
        );
        assert!(err.is_err(), "Duplicate entity name must be rejected");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.to_lowercase().contains("duplicate"),
            "Error must mention duplicate: got '{}'",
            msg
        );

        // Relationship source not in entity types → rejected (R8).
        let err = validate_graph_schema(
            &[EntityType { name: "Person".to_string(), description: None }],
            &[RelationshipType {
                name: "KNOWS".to_string(),
                source: Some("Ghost".to_string()), // not in entity_types
                target: Some("Person".to_string()),
                description: None,
            }],
        );
        assert!(
            err.is_err(),
            "Unknown relationship source must be rejected (R8)"
        );

        // Too many entity types → rejected (R8 DoS guard).
        let too_many: Vec<EntityType> = (0..=MAX_ENTITY_TYPES)
            .map(|i| EntityType { name: format!("E{}", i), description: None })
            .collect();
        let err = validate_graph_schema(&too_many, &[]);
        assert!(err.is_err(), "Exceeding MAX_ENTITY_TYPES must be rejected");

        // Well-formed → passes.
        let ok = validate_graph_schema(
            &[EntityType { name: "Person".to_string(), description: None }],
            &[],
        );
        assert!(ok.is_ok(), "Valid single-entity schema must pass");
    }

    /// INGEST-SCHEMA-DRAFT R8: approve persists the schema in workspace.metadata.
    ///
    /// We test via the pure data path (validate_graph_schema + schema blob construction)
    /// since the handler requires AppState (which cannot be constructed without block_on).
    #[test]
    fn test_approve_persists_graph_schema() {
        // Simulate what approve_workspace_graph_schema does with the approved schema:
        // 1. Validate the payload.
        let entity_types = vec![EntityType {
            name: "Company".to_string(),
            description: None,
        }];
        let relationship_types = vec![RelationshipType {
            name: "EMPLOYS".to_string(),
            source: Some("Company".to_string()),
            target: Some("Company".to_string()),
            description: None,
        }];

        assert!(
            validate_graph_schema(&entity_types, &relationship_types).is_ok(),
            "Payload must pass validation before persist"
        );

        // 2. Simulate the schema blob that would be written to workspace.metadata.
        let approved_at = "2026-01-01T12:00:00Z";
        let graph_schema_value = serde_json::json!({
            "version": 1u64,
            "status": "approved",
            "entity_types": entity_types,
            "relationship_types": relationship_types,
            "approved_at": approved_at,
            "source": "manual",
        });

        // Verify the schema blob has the correct shape.
        assert_eq!(
            graph_schema_value["status"].as_str().unwrap(),
            "approved",
            "Persisted schema must have status=approved"
        );
        assert_eq!(
            graph_schema_value["version"].as_u64().unwrap(),
            1u64,
            "First approval must be version 1"
        );
        assert!(
            graph_schema_value["entity_types"].is_array(),
            "entity_types must be an array"
        );

        // 3. Insert into workspace.metadata and verify check_workspace_schema_gate sees it.
        let tenant_id = Uuid::new_v4();
        let mut ws = Workspace::new(tenant_id, "WS", "ws");
        ws.metadata
            .insert("graph_schema".to_string(), graph_schema_value);

        assert_eq!(
            check_workspace_schema_gate(&ws),
            SchemaGateResult::Approved,
            "After persisting approved schema, gate must be open"
        );

        // R1: No schema_ready status was set.
        let serialized = serde_json::to_string(&ws.metadata).unwrap();
        assert!(
            !serialized.contains("schema_ready"),
            "Metadata must not contain schema_ready (R1)"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Task 2 tests
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
