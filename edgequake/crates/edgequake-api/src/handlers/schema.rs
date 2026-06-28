//! Schema lifecycle API handlers.
//!
//! Provides REST endpoints for the namespace schema proposal lifecycle:
//! get, suggest (in-process via tokio::spawn), merge-patch, approve, reject.
//!
//! # Endpoints
//!
//! | Method | Path | Handler | Description |
//! |--------|------|---------|-------------|
//! | GET | `/api/v1/namespaces/{ns}/schema` | [`get_namespace_schema`] | Get current proposal |
//! | POST | `/api/v1/namespaces/{ns}/schema` | [`suggest_namespace_schema`] | Trigger in-process LLM suggest |
//! | PATCH | `/api/v1/namespaces/{ns}/schema` | [`update_namespace_schema`] | Merge-patch proposal |
//! | POST | `/api/v1/namespaces/{ns}/schema/approve` | [`approve_namespace_schema`] | Approve proposal |
//! | POST | `/api/v1/namespaces/{ns}/schema/reject` | [`reject_namespace_schema`] | Reject proposal |

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use edgequake_core::{NamespaceRegistryError, NamespaceSlug};
use edgequake_core::schema::{SchemaProposal, SchemaStatus, EntityTypeProposal, RelationTypeProposal};
use edgequake_storage_aws::{RawDocsReader, RawDocsStorageReader};
#[cfg(test)]
use edgequake_storage_aws::InMemoryRawDocsReader;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Response wrapper for schema endpoints.
#[derive(Debug, Serialize, Deserialize)]
pub struct SchemaResponse {
    /// Namespace slug.
    pub namespace: String,
    /// The schema proposal, or a "none" sentinel when no proposal exists.
    pub schema: SchemaResponseBody,
}

/// The inner schema body — either a full proposal or a none/pending sentinel.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SchemaResponseBody {
    /// No schema has been proposed for this namespace.
    None,
    /// Schema suggestion is running in-process; result will arrive soon.
    Proposing,
    /// A full schema proposal with all fields.
    Proposed {
        entity_types: Vec<EntityTypeProposal>,
        relation_types: Vec<RelationTypeProposal>,
        sample_size: usize,
        total_documents: usize,
        domain_hint: Option<String>,
        proposed_at: i64,
        reviewed_at: Option<i64>,
    },
    /// The proposal has been approved.
    Approved {
        entity_types: Vec<EntityTypeProposal>,
        relation_types: Vec<RelationTypeProposal>,
        sample_size: usize,
        total_documents: usize,
        domain_hint: Option<String>,
        proposed_at: i64,
        reviewed_at: Option<i64>,
    },
    /// The proposal was rejected.
    Rejected {
        entity_types: Vec<EntityTypeProposal>,
        relation_types: Vec<RelationTypeProposal>,
        sample_size: usize,
        total_documents: usize,
        domain_hint: Option<String>,
        proposed_at: i64,
        reviewed_at: Option<i64>,
    },
    /// Schema suggestion failed; a new POST /schema will retry.
    Failed {
        error: String,
    },
}

/// Sentinel value stored in the registry during in-process suggestion.
/// We store a SchemaProposal with status=Proposed and a special domain_hint marker
/// so the GET handler can distinguish "pending" from "real Proposed".
pub const PENDING_SENTINEL: &str = "__pending__";

/// Prefix of the failure marker stored when the spawned suggest task fails.
/// Stored as `domain_hint = "__failed__: <message>"` with status=None so the
/// GET handler can surface a `Failed { error }` body (CR-02).
pub const FAILED_SENTINEL_PREFIX: &str = "__failed__:";

/// Staleness cutoff for the pending sentinel (WR-02).
///
/// If the API process dies mid-suggest (or storing the failure marker fails),
/// the `__pending__` marker would otherwise persist forever, deadlocking the
/// schema lifecycle: GET reports "proposing" indefinitely and the dedup guard
/// refuses to spawn a new task. A pending marker older than this window is
/// treated as expired — GET surfaces it as Failed (retryable), and POST
/// /schema spawns a fresh task instead of dedup-blocking.
pub const PENDING_STALENESS_MS: i64 = 15 * 60 * 1000;

/// Error message surfaced when a pending sentinel exceeds the staleness window.
const PENDING_STALE_ERROR: &str =
    "Schema suggestion timed out (the suggest task did not complete). Retry the suggestion.";

/// Returns true when the proposal carries a pending sentinel older than
/// [`PENDING_STALENESS_MS`] (measured against `now_ms`).
fn is_pending_stale(proposal: &SchemaProposal, now_ms: i64) -> bool {
    proposal.domain_hint.as_deref() == Some(PENDING_SENTINEL)
        && now_ms.saturating_sub(proposal.proposed_at) > PENDING_STALENESS_MS
}

/// Convert a stored SchemaProposal into a SchemaResponseBody.
///
/// `now_ms` is the current epoch-millis timestamp, used to expire stale
/// pending sentinels (WR-02).
fn schema_proposal_to_body(proposal: SchemaProposal, now_ms: i64) -> SchemaResponseBody {
    match proposal.status {
        SchemaStatus::None => {
            // Stored None carries either the pending sentinel (proposing),
            // a failure marker (failed), or nothing (no schema).
            match proposal.domain_hint.as_deref() {
                Some(PENDING_SENTINEL) => {
                    if is_pending_stale(&proposal, now_ms) {
                        SchemaResponseBody::Failed {
                            error: PENDING_STALE_ERROR.to_string(),
                        }
                    } else {
                        SchemaResponseBody::Proposing
                    }
                }
                Some(h) if h.starts_with(FAILED_SENTINEL_PREFIX) => {
                    SchemaResponseBody::Failed {
                        error: h
                            .strip_prefix(FAILED_SENTINEL_PREFIX)
                            .unwrap_or(h)
                            .trim()
                            .to_string(),
                    }
                }
                _ => SchemaResponseBody::None,
            }
        }
        SchemaStatus::Proposed => {
            // Check for the pending sentinel
            if proposal.domain_hint.as_deref() == Some(PENDING_SENTINEL) {
                if is_pending_stale(&proposal, now_ms) {
                    SchemaResponseBody::Failed {
                        error: PENDING_STALE_ERROR.to_string(),
                    }
                } else {
                    SchemaResponseBody::Proposing
                }
            } else {
                SchemaResponseBody::Proposed {
                    entity_types: proposal.entity_types,
                    relation_types: proposal.relation_types,
                    sample_size: proposal.sample_size,
                    total_documents: proposal.total_documents,
                    domain_hint: proposal.domain_hint,
                    proposed_at: proposal.proposed_at,
                    reviewed_at: proposal.reviewed_at,
                }
            }
        }
        SchemaStatus::Approved => SchemaResponseBody::Approved {
            entity_types: proposal.entity_types,
            relation_types: proposal.relation_types,
            sample_size: proposal.sample_size,
            total_documents: proposal.total_documents,
            domain_hint: proposal.domain_hint,
            proposed_at: proposal.proposed_at,
            reviewed_at: proposal.reviewed_at,
        },
        SchemaStatus::Rejected => SchemaResponseBody::Rejected {
            entity_types: proposal.entity_types,
            relation_types: proposal.relation_types,
            sample_size: proposal.sample_size,
            total_documents: proposal.total_documents,
            domain_hint: proposal.domain_hint,
            proposed_at: proposal.proposed_at,
            reviewed_at: proposal.reviewed_at,
        },
    }
}

/// Parse an OPTIONAL JSON request body into a `SuggestSchemaRequest`.
///
/// The admin "Propose from documents" / "Update from new documents" actions send
/// `Content-Type: application/json` with an EMPTY body. Neither `Json<T>` (rejects
/// empty with "EOF while parsing a value at line 1 column 0") nor Axum 0.8's
/// `Option<Json<T>>` handles this: `OptionalFromRequest` for `Json` only returns
/// `None` when the JSON content-type is ABSENT — when it's present (as the admin UI
/// always sends) it still runs the parser and propagates the EOF rejection. Reading
/// the raw body as `Bytes` and parsing by hand is the only robust form:
///   - empty body      -> all-default request (every field is `Option`)
///   - non-empty body  -> parse it; malformed JSON -> 400 (restores input validation)
fn parse_optional_suggest_body(body: &Bytes) -> ApiResult<SuggestSchemaRequest> {
    if body.is_empty() {
        Ok(SuggestSchemaRequest::default())
    } else {
        serde_json::from_slice(body)
            .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {}", e)))
    }
}

/// Request body for POST /schema (trigger suggest).
#[derive(Debug, Default, Deserialize)]
pub struct SuggestSchemaRequest {
    /// Domain description (e.g. "legal case files").
    pub domain_description: Option<String>,
    /// Domain hint (backward-compat alias).
    pub domain_hint: Option<String>,
    /// Override the sampling budget (default = edgequake-schema default of 24).
    pub sample_budget: Option<usize>,
}

/// Request body for PATCH /schema (merge-patch).
#[derive(Debug, Default, Deserialize)]
pub struct PatchSchemaRequest {
    /// Replace entity types (omit to preserve existing).
    pub entity_types: Option<Vec<EntityTypeProposal>>,
    /// Replace relation types (omit to preserve existing).
    pub relation_types: Option<Vec<RelationTypeProposal>>,
}

/// Response body for approve/reject endpoints.
#[derive(Debug, Serialize)]
pub struct SchemaActionResponse {
    /// Namespace slug.
    pub namespace: String,
    /// The updated proposal.
    pub schema: SchemaProposal,
}

/// Response body for the POST /schema/resample endpoint.
///
/// Returns a rich diff so the UI can highlight newly-added types without
/// re-fetching the schema and computing the diff client-side.
#[derive(Debug, Serialize)]
pub struct ResampleResponse {
    /// The merged Proposed schema (status = Proposed, admin descriptions preserved).
    pub schema: SchemaResponseBody,
    /// Names of entity types that were added by this resample (new to current).
    pub added: Vec<String>,
    /// Names of entity types that were preserved from the current schema.
    pub preserved: Vec<String>,
    /// Relation-name conflicts where resampled proposed different endpoints
    /// (current endpoints are preserved; these are surfaced for review).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<RelationConflict>,
}

// ---------------------------------------------------------------------------
// Helpers (copied verbatim from namespace.rs)
// ---------------------------------------------------------------------------

/// Map a NamespaceRegistryError to an ApiError.
fn map_registry_error(err: NamespaceRegistryError) -> ApiError {
    match err {
        NamespaceRegistryError::AlreadyExists(slug) => {
            ApiError::Conflict(format!("Namespace already exists: {}", slug))
        }
        NamespaceRegistryError::NotFound(slug) => {
            ApiError::NotFound(format!("Namespace not found: {}", slug))
        }
        NamespaceRegistryError::Internal(msg) => ApiError::Internal(msg),
        NamespaceRegistryError::SchemaNotFound(slug) => {
            ApiError::NotFound(format!("Schema not found for namespace: {}", slug))
        }
        NamespaceRegistryError::InvalidSchemaState(msg) => {
            ApiError::BadRequest(format!("Invalid schema state: {}", msg))
        }
    }
}

/// Get the namespace registry from AppState, returning 501 if not configured.
fn get_registry(state: &AppState) -> Result<&dyn edgequake_core::NamespaceRegistry, ApiError> {
    state
        .namespace_registry
        .as_ref()
        .map(|r| r.as_ref())
        .ok_or_else(|| ApiError::NotImplemented {
            feature: "Namespace registry not configured (requires AWS DynamoDB)".to_string(),
        })
}

/// Find the workspace whose slug equals the namespace slug.
///
/// Phase 23 established workspace.slug == namespace slug (see
/// `workspace_to_response` in workspaces.rs). Workspaces live under tenants,
/// so this scans tenants and probes each by slug.
async fn find_workspace_by_slug(
    state: &AppState,
    slug: &str,
) -> Option<edgequake_core::types::Workspace> {
    let tenants = match state.workspace_service.list_tenants(1000, 0).await {
        Ok(tenants) => tenants,
        Err(e) => {
            warn!(slug = slug, error = %e, "Failed to list tenants for workspace lookup");
            return None;
        }
    };
    for tenant in tenants {
        if let Ok(Some(workspace)) = state
            .workspace_service
            .get_workspace_by_slug(tenant.tenant_id, slug)
            .await
        {
            return Some(workspace);
        }
    }
    None
}

/// Stage the namespace's workspace documents into a fresh tempdir for the
/// schema sampler (D-05, Phase 33-12).
///
/// # Source (Phase 33-12 retarget)
///
/// Reads parked raw uploads from `reader` (backed by `state.raw_docs` S3 in
/// production; an `InMemoryRawDocsReader` in tests — NO live S3, NO `#[ignore]`).
///
/// # Deliberate prefix strategy (Codex HIGH-1 / T-33-12-PX)
///
/// The write side (documents.rs:4310-4331) parks uploads under
/// `{tenant_id_for_storage}/{workspace_id_for_storage}/{namespace}/`, where
/// those identifiers come from the `X-Tenant-ID` / `X-Workspace-ID` request
/// headers (strings, defaulting to `"default"` when absent).
///
/// Because the header-supplied identifiers may NOT be UUIDs, we cannot assume
/// the resolved Workspace UUIDs will match the keys already in the bucket.
/// We therefore try TWO tenant-scoped candidate prefixes (never a cross-tenant
/// wildcard) and use the FIRST one that returns ≥1 object:
///
/// 1. **Primary** — the UUID form: `{tenant_id}/{workspace_id}/{slug}/`
///    (the form produced when the client sends UUID header values, which is the
///    common path for the admin wizard).
/// 2. **Alternate** — `{tenant_id}/default/{slug}/`
///    (the form produced when the write side falls back to "default" workspace,
///    which happens when no X-Workspace-ID header is present; cited from
///    documents.rs:4312-4315 with `"default"` fallback).
///
/// Both prefixes are tenant-scoped (never cross-tenant). The CHOSEN prefix and
/// the staged_count are logged so the 33-15 live proof can assert staged_count >= 1.
///
/// # Filtering
///
/// Only objects with non-zero size are staged. Files are written as
/// `doc-NNNN.md` so the schema sampler can read them as plain text.
///
/// # Return value
///
/// - `Some((tempdir_path, TempDir, staged_count))` when ≥1 doc staged.
/// - `None` when the workspace is unresolved OR no objects exist under any
///   candidate prefix (NOT an `Err` — the caller falls back to snapshot_uri).
///   Warn-logged as "falling back to snapshot_uri".
///
/// The returned `TempDir` guard must be kept alive until sampling finishes.
pub(crate) async fn stage_workspace_sampling_dir(
    state: &AppState,
    slug: &NamespaceSlug,
    reader: &dyn RawDocsReader,
) -> Option<(String, tempfile::TempDir, usize)> {
    let workspace = find_workspace_by_slug(state, slug.as_str()).await?;

    let tenant_id = workspace.tenant_id.to_string();
    let workspace_id = workspace.workspace_id.to_string();
    let namespace = slug.as_str();

    // --- DELIBERATE prefix strategy (Codex HIGH-1 / documents.rs:4310-4331) ---
    //
    // PRIMARY: UUID-form as the admin wizard produces it.
    let primary_prefix = format!("{}/{}/{}/", tenant_id, workspace_id, namespace);
    // ALTERNATE: "default" workspace fallback (documents.rs:4312-4315 default path).
    // Still tenant-scoped; never a cross-tenant wildcard.
    let alternate_prefix = format!("{}/default/{}/", tenant_id, namespace);

    // Try primary first; if empty, try alternate; use the first that yields ≥1 object.
    let (chosen_prefix, raw_objects) = 'find: {
        match reader.list_raw_docs(&primary_prefix).await {
            Ok(objs) if !objs.is_empty() => break 'find (primary_prefix, objs),
            Ok(_) => {
                debug!(
                    namespace,
                    primary = %primary_prefix,
                    "Primary raw-docs prefix empty; trying tenant-scoped alternate"
                );
            }
            Err(e) => {
                warn!(
                    namespace,
                    error = %e,
                    prefix = %primary_prefix,
                    "Failed to list raw docs at primary prefix; trying alternate"
                );
            }
        }
        match reader.list_raw_docs(&alternate_prefix).await {
            Ok(objs) if !objs.is_empty() => break 'find (alternate_prefix, objs),
            Ok(_) => {
                debug!(namespace, alternate = %alternate_prefix, "Alternate raw-docs prefix also empty");
            }
            Err(e) => {
                warn!(
                    namespace,
                    error = %e,
                    prefix = %alternate_prefix,
                    "Failed to list raw docs at alternate prefix"
                );
            }
        }
        // No objects found under any candidate prefix.
        warn!(
            namespace,
            primary = %primary_prefix,
            alternate = %alternate_prefix,
            "No raw docs found under any candidate prefix; falling back to snapshot_uri"
        );
        return None;
    };

    // Create a staging tempdir.
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(e) => {
            warn!(
                namespace,
                error = %e,
                "Failed to create sampling tempdir; falling back to snapshot_uri"
            );
            return None;
        }
    };

    // Stage each non-empty object as doc-NNNN.md.
    // Key layout from documents.rs:4425-4437: {tenant}/{workspace}/{namespace}/{sha256}/{filename}
    // Segment 4 is the filename — but we just copy the full content into the tempdir.
    let mut staged_count: usize = 0;
    for obj in raw_objects.iter().filter(|o| o.size > 0) {
        let bytes = match reader.get_object(&obj.key).await {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    namespace,
                    key = %obj.key,
                    error = %e,
                    "Failed to fetch raw doc for schema sampling; skipping"
                );
                continue;
            }
        };

        let path = dir.path().join(format!("doc-{:04}.md", staged_count));
        if let Err(e) = std::fs::write(&path, &bytes) {
            warn!(
                namespace,
                key = %obj.key,
                error = %e,
                "Failed to write staged doc for schema sampling; falling back to snapshot_uri"
            );
            return None;
        }
        staged_count += 1;
    }

    if staged_count == 0 {
        warn!(
            namespace,
            prefix = %chosen_prefix,
            "Prefix had objects but none could be staged (all zero-byte?); falling back to snapshot_uri"
        );
        return None;
    }

    // Log the evidence needed by the 33-15 live proof (Codex HIGH-1 + MED-6).
    info!(
        namespace,
        chosen_prefix = %chosen_prefix,
        staged_count,
        "Staged raw docs for schema sampling"
    );

    let location = dir.path().display().to_string();
    Some((location, dir, staged_count))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Get the current schema proposal for a namespace.
///
/// Returns the full proposal if one exists, a `Proposing` status while the
/// in-process LLM suggestion runs, or a `None` sentinel if no schema has been
/// suggested yet.
///
/// # Errors
///
/// - 400: Invalid slug format
/// - 404: Namespace not found
/// - 501: Namespace registry not configured
#[utoipa::path(
    get,
    path = "/api/v1/namespaces/{namespace}/schema",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Schema proposal"),
        (status = 400, description = "Invalid slug format"),
        (status = 404, description = "Namespace not found"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn get_namespace_schema(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> ApiResult<Json<SchemaResponse>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Getting namespace schema");

    let proposal = registry
        .get_schema(&slug)
        .await
        .map_err(map_registry_error)?;

    let body = match proposal {
        None => SchemaResponseBody::None,
        Some(p) => schema_proposal_to_body(p, chrono::Utc::now().timestamp_millis()),
    };

    Ok(Json(SchemaResponse {
        namespace: slug.as_str().to_string(),
        schema: body,
    }))
}

/// Trigger in-process schema suggestion for a namespace.
///
/// Immediately stores a `Proposing` pending marker so subsequent GET calls
/// report progress. Then spawns a tokio task that calls
/// `edgequake_schema::suggest_schema` and stores the result.
///
/// Returns 202 Accepted. Deduplication: if a proposing marker is already
/// present, returns 202 without spawning a second task.
///
/// # Errors
///
/// - 400: Invalid slug or unresolvable data location
/// - 404: Namespace not found
/// - 501: Registry not configured
#[utoipa::path(
    post,
    path = "/api/v1/namespaces/{namespace}/schema",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    request_body = inline(serde_json::Value),
    responses(
        (status = 202, description = "Schema suggestion accepted; poll GET /schema for progress"),
        (status = 400, description = "Invalid slug or unresolvable data location"),
        (status = 404, description = "Namespace not found"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn suggest_namespace_schema(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    // Raw bytes, parsed by hand: the admin "Propose from documents" action sends
    // `Content-Type: application/json` with an EMPTY body, which both `Json<T>` and
    // Axum 0.8 `Option<Json<T>>` reject with an EOF error. See
    // `parse_optional_suggest_body`. MUST be the last extractor (consumes the body).
    body: Bytes,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let body = parse_optional_suggest_body(&body)?;
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Triggering schema suggestion");

    // --- Reject reserved sentinel values in user input (WR-02) ---
    // domain_hint / domain_description flow into the stored proposal; a value
    // starting with "__" could forge the pending/failed sentinels and
    // deadlock or spoof the lifecycle.
    for (field, value) in [
        ("domain_description", &body.domain_description),
        ("domain_hint", &body.domain_hint),
    ] {
        if let Some(v) = value {
            if v.starts_with("__") {
                return Err(ApiError::BadRequest(format!(
                    "{} must not start with '__' (reserved for internal sentinels)",
                    field
                )));
            }
        }
    }

    // --- Deduplication + overwrite guard (WR-02 + review item [2]) ---
    //
    // Registry errors are propagated (not swallowed); a pending marker older
    // than PENDING_STALENESS_MS is treated as expired and a fresh task is
    // spawned instead of dedup-blocking forever (WR-02).
    //
    // CONSTRAINT (review item [2]): `suggest_namespace_schema` must NOT wholesale-
    // overwrite an existing non-None proposal (especially an Approved one).
    // - First proposal (no current schema) → store directly (original behaviour).
    // - PENDING_SENTINEL → dedup as before.
    // - Any non-None status (Proposed/Approved/Rejected) → reject the suggest call
    //   and direct the caller to use POST /schema/defaults (for a deliberate reset)
    //   or POST /schema/resample (for an additive non-destructive update).
    //   This closes the destructive overwrite path (T-33-05-DD mitigated).
    let existing_proposal = registry
        .get_schema(&slug)
        .await
        .map_err(map_registry_error)?;
    if let Some(ref existing) = existing_proposal {
        if existing.domain_hint.as_deref() == Some(PENDING_SENTINEL) {
            let now_ms = chrono::Utc::now().timestamp_millis();
            if !is_pending_stale(existing, now_ms) {
                info!(namespace = slug.as_str(), "Schema suggestion already in progress");
                return Ok((
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({
                        "namespace": slug.as_str(),
                        "status": "proposing",
                        "message": "Schema suggestion already in progress"
                    })),
                ));
            }
            warn!(
                namespace = slug.as_str(),
                "Stale pending suggestion marker found (older than {}ms); spawning a fresh task",
                PENDING_STALENESS_MS
            );
            // Fall through: stale sentinel — treat as no existing proposal.
        } else {
            // A real non-None proposal exists (Proposed, Approved, or Rejected).
            // Refuse the destructive suggest path — direct to the safe alternatives.
            match existing.status {
                SchemaStatus::None => {
                    // status=None with no sentinel = effectively no proposal; fall through.
                }
                _ => {
                    // Proposed / Approved / Rejected — refuse overwrite.
                    return Err(ApiError::Conflict(format!(
                        "A {:?} schema proposal already exists for namespace '{}'. \
                         Use POST /namespaces/{}/schema/resample to add new types \
                         non-destructively, or POST /namespaces/{}/schema/defaults \
                         to start over from the 4 baseline types (destructive reset).",
                        existing.status,
                        slug.as_str(),
                        slug.as_str(),
                        slug.as_str()
                    )));
                }
            }
        }
    }

    // --- Resolve data location (D-05 / Phase 33-12 S3 retarget) ---
    // Prefer sampling the workspace's UPLOADED raw documents via the RawDocsReader
    // seam (sources from state.raw_docs S3 in production). Falls back to the legacy
    // snapshot_uri behavior when no workspace / no parked docs are found.
    let raw_docs_reader = RawDocsStorageReader::new(&state.raw_docs);
    let (location, sample_dir) = match stage_workspace_sampling_dir(&state, &slug, &raw_docs_reader).await {
        Some((location, dir, document_count)) => {
            info!(
                namespace = slug.as_str(),
                documents = document_count,
                "Sampling schema suggestion from workspace raw docs (S3)"
            );
            (location, Some(dir))
        }
        None => {
            // Legacy fallback: use snapshot_uri from the pipeline config.
            // No hard get_config error on missing config — return a friendly error
            // only if snapshot_uri is also absent (no data location at all).
            let config = registry
                .get_config(&slug)
                .await
                .map_err(map_registry_error)?;

            let location = config
                .and_then(|c| c.snapshot_uri)
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "Cannot suggest schema for namespace '{}': no parked raw docs found \
                         in S3 and snapshot_uri is not configured. Upload documents first.",
                        slug
                    ))
                })?;

            // Validate location has no path traversal
            if location.contains("..") {
                return Err(ApiError::BadRequest(
                    "snapshot_uri must not contain '..' path-traversal sequences".to_string(),
                ));
            }
            (location, None)
        }
    };

    // --- Write pending marker immediately ---
    let pending = SchemaProposal {
        status: SchemaStatus::Proposed,
        entity_types: vec![],
        relation_types: vec![],
        sample_size: 0,
        total_documents: 0,
        domain_hint: Some(PENDING_SENTINEL.to_string()),
        proposed_at: chrono::Utc::now().timestamp_millis(),
        reviewed_at: None,
        sampling_metadata: None,
    };

    registry
        .store_schema(&slug, &pending)
        .await
        .map_err(map_registry_error)?;

    // --- Build suggest input ---
    let suggest_input = edgequake_schema::SuggestSchemaInput {
        domain_description: body.domain_description,
        domain_hint: body.domain_hint,
        sample_budget: body.sample_budget,
        ..Default::default()
    };

    // --- Get OpenAI key from environment (same source as ProviderFactory::from_env) ---
    let openai_api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    if openai_api_key.is_empty() {
        warn!(
            namespace = slug.as_str(),
            "OPENAI_API_KEY not set; schema suggestion task will fail"
        );
    }
    let openai_config = edgequake_schema::OpenAiConfig::new(&openai_api_key);

    // --- Clone what the spawned task needs ---
    let registry_arc = state
        .namespace_registry
        .clone()
        .expect("registry already checked above");
    let slug_clone = slug.clone();
    let location_clone = location.clone();

    // --- Spawn the task (do NOT await on request path) ---
    // `sample_dir` (the staged workspace-documents tempdir, when used) is
    // moved into the task and dropped after sampling finishes, which removes
    // the directory from disk.
    tokio::spawn(async move {
        info!(
            namespace = slug_clone.as_str(),
            location = %location_clone,
            "Schema suggestion task started"
        );

        let suggest_result =
            edgequake_schema::suggest_schema(&location_clone, &suggest_input, &openai_config)
                .await;

        // Clean up the staged sampling tempdir now that sampling is done.
        drop(sample_dir);

        match suggest_result {
            Ok(proposal) => {
                match registry_arc.store_schema(&slug_clone, &proposal).await {
                    Ok(()) => {
                        info!(
                            namespace = slug_clone.as_str(),
                            entity_types = proposal.entity_types.len(),
                            relation_types = proposal.relation_types.len(),
                            "Schema suggestion complete and stored"
                        );
                    }
                    Err(e) => {
                        error!(
                            namespace = slug_clone.as_str(),
                            error = %e,
                            "Failed to store schema suggestion result"
                        );
                    }
                }
            }
            Err(e) => {
                error!(
                    namespace = slug_clone.as_str(),
                    error = %e,
                    "Schema suggestion task failed"
                );
                // Store a failed marker so the wizard surfaces the error
                // Use domain_hint="__failed__: <message>" to signal failure
                let failed_msg = format!("{} {}", FAILED_SENTINEL_PREFIX, e);
                let failed_marker = SchemaProposal {
                    status: SchemaStatus::None,
                    entity_types: vec![],
                    relation_types: vec![],
                    sample_size: 0,
                    total_documents: 0,
                    domain_hint: Some(failed_msg),
                    proposed_at: chrono::Utc::now().timestamp_millis(),
                    reviewed_at: None,
                    sampling_metadata: None,
                };
                // If storing the failure marker itself fails, the pending
                // sentinel stays behind — log loudly; the staleness window
                // (PENDING_STALENESS_MS) is the recovery path (WR-02).
                if let Err(store_err) =
                    registry_arc.store_schema(&slug_clone, &failed_marker).await
                {
                    error!(
                        namespace = slug_clone.as_str(),
                        error = %store_err,
                        "Failed to store schema failure marker; pending sentinel will expire via staleness window"
                    );
                }
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "namespace": slug.as_str(),
            "status": "proposing",
            "message": "Schema suggestion started; poll GET /schema for progress"
        })),
    ))
}

/// Merge-patch the current schema proposal.
///
/// Loads the existing proposal, applies only the fields present in the
/// request body (entity_types and/or relation_types), strips any relation
/// type named "RELATED_TO", then stores the merged proposal.
///
/// A body that omits entity_types leaves existing entity_types intact.
///
/// # Errors
///
/// - 400: Invalid slug
/// - 404: No schema proposal exists
/// - 501: Registry not configured
#[utoipa::path(
    patch,
    path = "/api/v1/namespaces/{namespace}/schema",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    request_body = inline(serde_json::Value),
    responses(
        (status = 200, description = "Merged schema proposal"),
        (status = 400, description = "Invalid slug"),
        (status = 404, description = "No schema proposal exists"),
        (status = 409, description = "Schema suggestion in progress"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn update_namespace_schema(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Json(body): Json<PatchSchemaRequest>,
) -> ApiResult<Json<SchemaResponse>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Patching namespace schema");

    // Must get first to merge (per REVIEW Gemini #8: store_schema wipes PipelineConfig types)
    let mut proposal = registry
        .get_schema(&slug)
        .await
        .map_err(map_registry_error)?
        .ok_or_else(|| {
            ApiError::NotFound(format!("No schema proposal found for namespace: {}", slug))
        })?;

    // WR-10: never merge into the in-flight pending sentinel — it would race
    // the spawned suggest task (last-writer-wins on SK=SCHEMA) and preserve
    // the sentinel domain_hint, leaving the record permanently "proposing"
    // with user data inside.
    if proposal.domain_hint.as_deref() == Some(PENDING_SENTINEL) {
        return Err(ApiError::Conflict(
            "A schema suggestion is in progress for this namespace; \
             wait for it to complete before editing the schema"
                .to_string(),
        ));
    }

    // Apply only the fields present in the PATCH body
    let types_changed = body.entity_types.is_some() || body.relation_types.is_some();
    if let Some(entity_types) = body.entity_types {
        proposal.entity_types = entity_types;
    }
    if let Some(relation_types) = body.relation_types {
        // Strip RELATED_TO (T-23-04)
        proposal.relation_types = relation_types
            .into_iter()
            .filter(|r| r.name != "RELATED_TO")
            .collect();
    }

    // WR-10: editing an Approved schema invalidates the approval —
    // store_schema clears the live PipelineConfig entity/relation types, so
    // surface that re-approval is required by transitioning the proposal
    // back to Proposed (the response status tells the client explicitly).
    if types_changed && proposal.status == SchemaStatus::Approved {
        proposal.status = SchemaStatus::Proposed;
        proposal.reviewed_at = None;
        info!(
            namespace = slug.as_str(),
            "Approved schema edited via PATCH; status reset to Proposed (re-approval required)"
        );
    }

    // Store merged proposal
    registry
        .store_schema(&slug, &proposal)
        .await
        .map_err(map_registry_error)?;

    let body = schema_proposal_to_body(proposal, chrono::Utc::now().timestamp_millis());
    Ok(Json(SchemaResponse {
        namespace: slug.as_str().to_string(),
        schema: body,
    }))
}

/// Approve the current schema proposal.
///
/// Delegates to `registry.approve_schema`. Returns the updated proposal.
///
/// # Errors
///
/// - 400: Invalid slug or schema not in Proposed state
/// - 404: No schema proposal found
/// - 501: Registry not configured
#[utoipa::path(
    post,
    path = "/api/v1/namespaces/{namespace}/schema/approve",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Schema approved"),
        (status = 400, description = "Invalid slug or schema not in Proposed state"),
        (status = 404, description = "No schema proposal found"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn approve_namespace_schema(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> ApiResult<Json<SchemaActionResponse>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Approving namespace schema");

    let proposal = registry
        .approve_schema(&slug)
        .await
        .map_err(map_registry_error)?;

    Ok(Json(SchemaActionResponse {
        namespace: slug.as_str().to_string(),
        schema: proposal,
    }))
}

/// Reject the current schema proposal.
///
/// Delegates to `registry.reject_schema`. Returns the updated proposal.
///
/// # Errors
///
/// - 400: Invalid slug or schema not in Proposed state
/// - 404: No schema proposal found
/// - 501: Registry not configured
#[utoipa::path(
    post,
    path = "/api/v1/namespaces/{namespace}/schema/reject",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Schema rejected"),
        (status = 400, description = "Invalid slug or schema not in Proposed state"),
        (status = 404, description = "No schema proposal found"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn reject_namespace_schema(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> ApiResult<Json<SchemaActionResponse>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Rejecting namespace schema");

    let proposal = registry
        .reject_schema(&slug)
        .await
        .map_err(map_registry_error)?;

    Ok(Json(SchemaActionResponse {
        namespace: slug.as_str().to_string(),
        schema: proposal,
    }))
}

/// Seed the namespace schema from the 4 BASELINE entity types (no LLM).
///
/// REPLACE semantics (design §4.1): stores a fresh `Proposed` SchemaProposal
/// seeded from the 4 BASELINE types (PERSON, ORGANIZATION, LOCATION, DATE),
/// replacing ANY existing proposal — including an Approved one.
///
/// This is the ONLY sanctioned destructive path on the schema lifecycle. The
/// admin uses this to start from scratch, then edits and approves via the
/// existing PATCH + POST /approve endpoints.
///
/// The caller must exercise explicit intent (POST, not GET) because this action
/// is destructive: an Approved schema is replaced by the fresh Proposed baseline.
///
/// # Errors
///
/// - 400: Invalid slug format
/// - 404: Namespace not found
/// - 501: Namespace registry not configured
#[utoipa::path(
    post,
    path = "/api/v1/namespaces/{namespace}/schema/defaults",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Baseline schema seeded — status=Proposed; admin must edit and approve"),
        (status = 400, description = "Invalid slug format"),
        (status = 404, description = "Namespace not found"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn start_from_defaults(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> ApiResult<Json<SchemaResponse>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    info!(
        namespace = slug.as_str(),
        "start-from-defaults: seeding baseline proposal (REPLACE semantics)"
    );

    let baseline = build_baseline_proposal();

    // REPLACE: store_schema is a wholesale replace — any existing proposal
    // (including Approved) is overwritten with the fresh Proposed baseline.
    // This is the ONLY sanctioned destructive path (design §4.1).
    registry
        .store_schema(&slug, &baseline)
        .await
        .map_err(map_registry_error)?;

    let body = schema_proposal_to_body(baseline, chrono::Utc::now().timestamp_millis());
    Ok(Json(SchemaResponse {
        namespace: slug.as_str().to_string(),
        schema: body,
    }))
}

/// Re-sample the namespace schema via a synchronous LLM call + additive merge.
///
/// PINNED route: `POST /namespaces/{ns}/schema/resample` (never overwrites
/// suggest_namespace_schema's route).
///
/// Unlike the initial suggest endpoint (POST /schema, which uses tokio::spawn),
/// this handler runs ONE bounded synchronous LLM call (`tokio::time::timeout`)
/// and merges the result additively into the current proposal via `merge_schema`.
///
/// **Non-destructive:** An Approved schema is NEVER overwritten wholesale.
/// The merged delta is stored as Proposed so the admin reviews before re-approving.
/// Malformed LLM output → 422 Unprocessable, leaving the approved schema intact (§8).
///
/// Returns `{ schema, added, preserved }` — the rich diff — so the UI can
/// highlight newly-added types without client-side diffing.
///
/// # Errors
///
/// - 400: Invalid slug or no data location resolvable
/// - 404: Namespace not found
/// - 422: LLM output was malformed — approved schema preserved
/// - 501: Registry not configured
#[utoipa::path(
    post,
    path = "/api/v1/namespaces/{namespace}/schema/resample",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    request_body = inline(serde_json::Value),
    responses(
        (status = 200, description = "Re-sampled schema delta — { schema (Proposed), added, preserved }"),
        (status = 400, description = "Invalid slug or no data location resolvable"),
        (status = 404, description = "Namespace not found"),
        (status = 422, description = "Malformed LLM output — approved schema intact"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn resample_namespace_schema(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    // Raw bytes, parsed by hand (same rationale as `suggest_namespace_schema`): the
    // admin "Update from new documents" action sends `Content-Type: application/json`
    // with an empty body, which Axum 0.8 `Option<Json<T>>` still rejects. MUST be the
    // last extractor (consumes the body).
    body: Bytes,
) -> ApiResult<Json<ResampleResponse>> {
    let body = parse_optional_suggest_body(&body)?;
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Re-sampling namespace schema");

    // Read the current proposal (may be None, Proposed, Approved, or Rejected).
    let current_opt = registry
        .get_schema(&slug)
        .await
        .map_err(map_registry_error)?;

    // Guard: do NOT run if a suggest is already in-flight (PENDING_SENTINEL).
    if let Some(ref curr) = current_opt {
        if curr.domain_hint.as_deref() == Some(PENDING_SENTINEL) {
            let now_ms = chrono::Utc::now().timestamp_millis();
            if !is_pending_stale(curr, now_ms) {
                return Err(ApiError::Conflict(
                    "A schema suggestion is already in progress; wait for it to complete before re-sampling".to_string(),
                ));
            }
        }
    }

    // Resolve data location (same logic as suggest_namespace_schema, Phase 33-12 S3 retarget).
    let raw_docs_reader = RawDocsStorageReader::new(&state.raw_docs);
    let (location, _sample_dir) = match stage_workspace_sampling_dir(&state, &slug, &raw_docs_reader).await {
        Some((location, dir, document_count)) => {
            info!(
                namespace = slug.as_str(),
                documents = document_count,
                "Re-sampling schema from workspace raw docs (S3)"
            );
            (location, Some(dir))
        }
        None => {
            // Legacy fallback: use snapshot_uri from the pipeline config.
            let config = registry
                .get_config(&slug)
                .await
                .map_err(map_registry_error)?;

            let location = config
                .and_then(|c| c.snapshot_uri)
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "Cannot re-sample schema for namespace '{}': no parked raw docs found \
                         in S3 and snapshot_uri is not configured.",
                        slug
                    ))
                })?;
            if location.contains("..") {
                return Err(ApiError::BadRequest(
                    "snapshot_uri must not contain '..' path-traversal sequences".to_string(),
                ));
            }
            (location, None)
        }
    };

    // Build suggest input.
    let suggest_input = edgequake_schema::SuggestSchemaInput {
        domain_description: body.domain_description,
        domain_hint: body.domain_hint,
        sample_budget: body.sample_budget,
        ..Default::default()
    };

    let openai_api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    if openai_api_key.is_empty() {
        warn!(namespace = slug.as_str(), "OPENAI_API_KEY not set for re-sample");
    }
    let openai_config = edgequake_schema::OpenAiConfig::new(&openai_api_key);

    // ONE synchronous bounded LLM call (T-33-05-DH: timeout prevents API GW 29s blowout).
    // Do NOT use tokio::spawn — resample is synchronous by design.
    const RESAMPLE_TIMEOUT_SECS: u64 = 25;
    let suggest_result = tokio::time::timeout(
        std::time::Duration::from_secs(RESAMPLE_TIMEOUT_SECS),
        edgequake_schema::suggest_schema(&location, &suggest_input, &openai_config),
    )
    .await
    .map_err(|_| {
        ApiError::ValidationError(
            "Schema re-sample timed out (LLM did not respond in time). \
             The approved schema is intact."
                .to_string(),
        )
    })?
    .map_err(|e| {
        ApiError::ValidationError(format!(
            "Schema re-sample failed (malformed LLM output): {}. \
             The approved schema is intact.",
            e
        ))
    })?;

    let now_ms = chrono::Utc::now().timestamp_millis();

    // Additive merge or first-time store.
    let (merge_added, merge_preserved, merge_conflicts, merged_proposal) =
        if let Some(current) = current_opt {
            let m = merge_schema(&current, &suggest_result);
            let added = m.added.clone();
            let preserved = m.preserved.clone();
            let conflicts = m.conflicts.clone();
            (added, preserved, conflicts, m.proposal)
        } else {
            // No existing proposal: store directly; all types are "added".
            let added: Vec<String> = suggest_result
                .entity_types
                .iter()
                .map(|e| e.name.clone())
                .collect();
            (added, vec![], vec![], suggest_result)
        };

    // Store the merged Proposed delta (never overwrites with Approved state).
    let stored_proposal = SchemaProposal {
        status: SchemaStatus::Proposed,
        reviewed_at: None,
        ..merged_proposal
    };

    registry
        .store_schema(&slug, &stored_proposal)
        .await
        .map_err(map_registry_error)?;

    let schema_body = schema_proposal_to_body(stored_proposal, now_ms);

    Ok(Json(ResampleResponse {
        schema: schema_body,
        added: merge_added,
        preserved: merge_preserved,
        conflicts: merge_conflicts,
    }))
}

// ---------------------------------------------------------------------------
// Baseline seed + merge helpers
// ---------------------------------------------------------------------------

/// The 4 canonical baseline entity types seeded by start-from-defaults.
///
/// These are the ONLY types seeded — NOT the full `default_entity_types()` set.
/// Design §4.1: the admin starts from here, then edits and approves.
const BASELINE_ENTITY_TYPES: &[(&str, &str)] = &[
    ("PERSON", "Named individuals"),
    (
        "ORGANIZATION",
        "Companies, institutions, government agencies",
    ),
    ("LOCATION", "Places, addresses, geographic regions"),
    ("DATE", "Dates, time periods, timestamps"),
];

/// Build a Proposed SchemaProposal seeded from the 4 BASELINE entity types.
///
/// - No LLM call — pure deterministic seed.
/// - Status = Proposed (admin must edit and approve).
/// - Empty relation_types, sample_size=0, total_documents=0.
///
/// This is the building block for the start-from-defaults endpoint (SCHEMA-MANUAL-DEFAULTS).
pub fn build_baseline_proposal() -> SchemaProposal {
    let entity_types = BASELINE_ENTITY_TYPES
        .iter()
        .map(|(name, desc)| EntityTypeProposal {
            name: name.to_string(),
            description: desc.to_string(),
            frequency: 0,
            is_baseline: true,
        })
        .collect();
    SchemaProposal {
        status: SchemaStatus::Proposed,
        entity_types,
        relation_types: vec![],
        sample_size: 0,
        total_documents: 0,
        domain_hint: None,
        proposed_at: chrono::Utc::now().timestamp_millis(),
        reviewed_at: None,
        sampling_metadata: None,
    }
}

/// A conflict recorded when `merge_schema` finds a relation with the same NAME
/// in both `current` and `resampled` but with different source/target endpoints.
///
/// The current relation's endpoints are always preserved verbatim; the resampled
/// endpoints that were rejected are recorded here for surfacing to the caller.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationConflict {
    /// The relation type name that had a conflict.
    pub name: String,
    /// The (source_type, target_type) pair from the current (preserved) relation.
    pub current_endpoints: (String, String),
    /// The (source_type, target_type) pair from the resampled relation that was rejected.
    pub rejected_endpoints: (String, String),
}

/// Output of `merge_schema`: the merged proposal plus partition metadata.
#[derive(Debug)]
pub struct MergeResult {
    /// The merged SchemaProposal (status = Proposed, reviewed_at = None).
    pub proposal: SchemaProposal,
    /// Names of entity types that were added (only in `resampled`, not in `current`).
    pub added: Vec<String>,
    /// Names of entity types that were preserved (existed in `current`).
    pub preserved: Vec<String>,
    /// Relation-name collisions where `resampled` proposed different endpoints.
    ///
    /// The current relation is kept verbatim in `proposal`; the rejected
    /// endpoints are recorded here for the UI/caller to surface.
    pub conflicts: Vec<RelationConflict>,
}

/// Additive merge of a `resampled` SchemaProposal into a `current` one.
///
/// **Union key for entity types:** entity type NAME (case-sensitive UPPER_SNAKE_CASE).
/// **Union key for relation types:** relation type NAME ONLY (NOT name+source_type+target_type).
///
/// # Merge rules
///
/// ## Entity types
/// - A type in `current` is **preserved verbatim**: description (including
///   admin edits) is never overwritten, but frequency is refreshed from the
///   matching `resampled` type when present.
/// - A type only in `resampled` is **added** (recorded in `MergeResult.added`);
///   its frequency comes from `resampled`.
/// - Nothing in `current` is removed.
///
/// ## Relation types
/// - Union by relation NAME only (design "by type name").
/// - If the same name exists in both with **identical** source/target endpoints,
///   the current relation is preserved (no clobbering).
/// - If the same name exists in both but with **different** endpoints, the
///   current relation is kept verbatim and a [`RelationConflict`] is recorded in
///   `MergeResult.conflicts` — endpoints are NEVER silently changed.
/// - Relations only in `resampled` are appended to the merged set.
///
/// ## Result
/// - `status` = Proposed, `reviewed_at` = None.
/// - `proposed_at` = now.
/// - "Unreviewed" is carried by the caller via `MergeResult.added` (no schema field change).
pub fn merge_schema(current: &SchemaProposal, resampled: &SchemaProposal) -> MergeResult {
    let mut merged_entity_types: Vec<EntityTypeProposal> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    let mut preserved: Vec<String> = Vec::new();

    // Build a lookup of resampled entity types by name for O(n) merge.
    let resampled_entity_map: std::collections::HashMap<&str, &EntityTypeProposal> = resampled
        .entity_types
        .iter()
        .map(|e| (e.name.as_str(), e))
        .collect();

    // Pass 1: preserve current types (description verbatim; refresh frequency).
    for current_et in &current.entity_types {
        let mut merged = current_et.clone();
        if let Some(resampled_et) = resampled_entity_map.get(current_et.name.as_str()) {
            // Refresh frequency from resampled; preserve everything else.
            merged.frequency = resampled_et.frequency;
        }
        preserved.push(merged.name.clone());
        merged_entity_types.push(merged);
    }

    // Pass 2: append resampled-only types (additive).
    let current_entity_names: std::collections::HashSet<&str> = current
        .entity_types
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    for resampled_et in &resampled.entity_types {
        if !current_entity_names.contains(resampled_et.name.as_str()) {
            added.push(resampled_et.name.clone());
            merged_entity_types.push(resampled_et.clone());
        }
    }

    // --- Relation type merge (keyed by NAME only) ---
    let mut merged_relation_types: Vec<RelationTypeProposal> = Vec::new();
    let mut conflicts: Vec<RelationConflict> = Vec::new();

    // Build a lookup of resampled relation types by name.
    let resampled_relation_map: std::collections::HashMap<&str, &RelationTypeProposal> = resampled
        .relation_types
        .iter()
        .map(|r| (r.name.as_str(), r))
        .collect();

    // Pass 1: process current relations.
    let mut current_relation_names: std::collections::HashSet<&str> =
        std::collections::HashSet::new();
    for current_rt in &current.relation_types {
        current_relation_names.insert(current_rt.name.as_str());
        if let Some(resampled_rt) = resampled_relation_map.get(current_rt.name.as_str()) {
            // Same name in both — check endpoint compatibility.
            if resampled_rt.source_type != current_rt.source_type
                || resampled_rt.target_type != current_rt.target_type
            {
                // Endpoint conflict: keep current verbatim, record conflict.
                conflicts.push(RelationConflict {
                    name: current_rt.name.clone(),
                    current_endpoints: (
                        current_rt.source_type.clone(),
                        current_rt.target_type.clone(),
                    ),
                    rejected_endpoints: (
                        resampled_rt.source_type.clone(),
                        resampled_rt.target_type.clone(),
                    ),
                });
            }
            // Always keep current verbatim (description + endpoints preserved).
            merged_relation_types.push(current_rt.clone());
        } else {
            // Current-only relation — preserve as-is.
            merged_relation_types.push(current_rt.clone());
        }
    }

    // Pass 2: append resampled-only relations (additive).
    for resampled_rt in &resampled.relation_types {
        if !current_relation_names.contains(resampled_rt.name.as_str()) {
            merged_relation_types.push(resampled_rt.clone());
        }
    }

    let proposal = SchemaProposal {
        status: SchemaStatus::Proposed,
        entity_types: merged_entity_types,
        relation_types: merged_relation_types,
        sample_size: resampled.sample_size,
        total_documents: resampled.total_documents,
        domain_hint: resampled.domain_hint.clone(),
        proposed_at: chrono::Utc::now().timestamp_millis(),
        reviewed_at: None,
        sampling_metadata: resampled.sampling_metadata.clone(),
    };

    MergeResult {
        proposal,
        added,
        preserved,
        conflicts,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use edgequake_core::schema::{EntityTypeProposal, RelationTypeProposal, SchemaProposal, SchemaStatus};

    // Helper to make a minimal proposal
    fn make_proposal(status: SchemaStatus) -> SchemaProposal {
        SchemaProposal {
            status,
            entity_types: vec![],
            relation_types: vec![],
            sample_size: 5,
            total_documents: 100,
            domain_hint: None,
            proposed_at: 1700000000000,
            reviewed_at: None,
            sampling_metadata: None,
        }
    }

    /// Fixed "now" used by tests: 1 second after the proposals' proposed_at.
    const TEST_NOW_MS: i64 = 1700000001000;

    #[test]
    fn test_none_proposal_returns_none_body() {
        // Behavior: GET with no proposal returns "none" status variant
        let body = schema_proposal_to_body(
            SchemaProposal {
                status: SchemaStatus::None,
                domain_hint: None,
                ..make_proposal(SchemaStatus::None)
            },
            TEST_NOW_MS,
        );
        assert!(matches!(body, SchemaResponseBody::None));
    }

    #[test]
    fn test_pending_sentinel_returns_proposing_body() {
        // Behavior: GET while suggest is in-progress returns "proposing" status
        let proposal = SchemaProposal {
            status: SchemaStatus::Proposed,
            domain_hint: Some(PENDING_SENTINEL.to_string()),
            entity_types: vec![],
            relation_types: vec![],
            ..make_proposal(SchemaStatus::Proposed)
        };
        let body = schema_proposal_to_body(proposal, TEST_NOW_MS);
        assert!(matches!(body, SchemaResponseBody::Proposing));
    }

    #[test]
    fn test_stale_pending_sentinel_returns_failed_body() {
        // WR-02: a pending marker older than PENDING_STALENESS_MS is expired —
        // GET surfaces Failed (retryable) instead of "proposing" forever.
        let proposal = SchemaProposal {
            status: SchemaStatus::Proposed,
            domain_hint: Some(PENDING_SENTINEL.to_string()),
            ..make_proposal(SchemaStatus::Proposed)
        };
        let stale_now = proposal.proposed_at + PENDING_STALENESS_MS + 1;
        let body = schema_proposal_to_body(proposal, stale_now);
        assert!(matches!(body, SchemaResponseBody::Failed { .. }));
    }

    #[test]
    fn test_fresh_pending_sentinel_within_window_is_proposing() {
        // WR-02 boundary: exactly at the staleness window is still proposing.
        let proposal = SchemaProposal {
            status: SchemaStatus::Proposed,
            domain_hint: Some(PENDING_SENTINEL.to_string()),
            ..make_proposal(SchemaStatus::Proposed)
        };
        let at_window = proposal.proposed_at + PENDING_STALENESS_MS;
        let body = schema_proposal_to_body(proposal, at_window);
        assert!(matches!(body, SchemaResponseBody::Proposing));
    }

    #[test]
    fn test_real_proposal_returns_proposed_body() {
        // Behavior: completed suggestion returns Proposed with entity/relation types
        let proposal = SchemaProposal {
            status: SchemaStatus::Proposed,
            domain_hint: Some("legal".to_string()),
            entity_types: vec![EntityTypeProposal {
                name: "PERSON".to_string(),
                description: "A named person".to_string(),
                frequency: 10,
                is_baseline: true,
            }],
            relation_types: vec![],
            ..make_proposal(SchemaStatus::Proposed)
        };
        let body = schema_proposal_to_body(proposal, TEST_NOW_MS);
        match body {
            SchemaResponseBody::Proposed { entity_types, domain_hint, .. } => {
                assert_eq!(entity_types.len(), 1);
                assert_eq!(entity_types[0].name, "PERSON");
                assert_eq!(domain_hint, Some("legal".to_string()));
            }
            _ => panic!("Expected Proposed, got {:?}", body),
        }
    }

    #[test]
    fn test_patch_strips_related_to() {
        // Behavior: PATCH strips RELATED_TO from relation_types before store
        let existing_proposal = SchemaProposal {
            status: SchemaStatus::Proposed,
            relation_types: vec![
                RelationTypeProposal {
                    name: "employs".to_string(),
                    description: "employment".to_string(),
                    source_type: "ORG".to_string(),
                    target_type: "PERSON".to_string(),
                    frequency: 5,
                },
                RelationTypeProposal {
                    name: "RELATED_TO".to_string(),
                    description: "generic relation".to_string(),
                    source_type: "ANY".to_string(),
                    target_type: "ANY".to_string(),
                    frequency: 10,
                },
            ],
            ..make_proposal(SchemaStatus::Proposed)
        };

        // Simulate what update_namespace_schema does: filter RELATED_TO
        let filtered: Vec<RelationTypeProposal> = existing_proposal
            .relation_types
            .into_iter()
            .filter(|r| r.name != "RELATED_TO")
            .collect();

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "employs");
    }

    #[test]
    fn test_patch_preserves_entity_types_when_omitted() {
        // Behavior: PATCH with only relation_types preserves existing entity_types
        let mut proposal = SchemaProposal {
            status: SchemaStatus::Proposed,
            entity_types: vec![EntityTypeProposal {
                name: "PERSON".to_string(),
                description: "A person".to_string(),
                frequency: 10,
                is_baseline: true,
            }],
            relation_types: vec![],
            ..make_proposal(SchemaStatus::Proposed)
        };

        // Only relation_types provided in body (entity_types = None)
        let patch_entity_types: Option<Vec<EntityTypeProposal>> = None;
        let patch_relation_types: Option<Vec<RelationTypeProposal>> = Some(vec![
            RelationTypeProposal {
                name: "employs".to_string(),
                description: "employment".to_string(),
                source_type: "ORG".to_string(),
                target_type: "PERSON".to_string(),
                frequency: 5,
            },
        ]);

        if let Some(et) = patch_entity_types {
            proposal.entity_types = et;
        }
        if let Some(rt) = patch_relation_types {
            proposal.relation_types = rt
                .into_iter()
                .filter(|r| r.name != "RELATED_TO")
                .collect();
        }

        // entity_types should be preserved (still has PERSON)
        assert_eq!(proposal.entity_types.len(), 1);
        assert_eq!(proposal.entity_types[0].name, "PERSON");
        // relation_types should be updated
        assert_eq!(proposal.relation_types.len(), 1);
        assert_eq!(proposal.relation_types[0].name, "employs");
    }

    #[test]
    fn test_schema_response_serde_none() {
        // SchemaResponseBody::None serializes correctly
        let response = SchemaResponse {
            namespace: "test-ns".to_string(),
            schema: SchemaResponseBody::None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"none\""));
    }

    #[test]
    fn test_schema_response_serde_proposing() {
        let response = SchemaResponse {
            namespace: "test-ns".to_string(),
            schema: SchemaResponseBody::Proposing,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"proposing\""));
    }

    #[test]
    fn test_pending_sentinel_constant() {
        // The pending sentinel is stable (other plans depend on this string)
        assert_eq!(PENDING_SENTINEL, "__pending__");
    }

    #[test]
    fn test_failed_marker_returns_failed_body() {
        // CR-02: a failed suggest task stores status=None +
        // domain_hint="__failed__: <msg>"; GET must surface Failed { error }.
        let proposal = SchemaProposal {
            status: SchemaStatus::None,
            domain_hint: Some(format!("{} LLM call timed out", FAILED_SENTINEL_PREFIX)),
            ..make_proposal(SchemaStatus::None)
        };
        let body = schema_proposal_to_body(proposal, TEST_NOW_MS);
        match body {
            SchemaResponseBody::Failed { error } => {
                assert_eq!(error, "LLM call timed out");
            }
            _ => panic!("Expected Failed, got {:?}", body),
        }
    }

    // -----------------------------------------------------------------------
    // Phase 33-12 sampling tests: migrated from KV-seeded to RawDocsReader seam
    //
    // These tests now inject an InMemoryRawDocsReader — no real S3, no #[ignore],
    // no block_on panic. The seam (Task 1) is the MANDATORY decoupling that
    // enables CI runs without a live bucket.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_stage_workspace_sampling_prefers_raw_docs() {
        // Phase 33-12 retarget (was: "prefers_workspace_documents" seeded via KV).
        // After retarget: the sampler reads from the RawDocsReader seam.
        // Inject an InMemoryRawDocsReader seeded with a doc under the PRIMARY prefix
        // ({tenant_id}/{workspace_id}/{slug}/); assert the seeded S3 doc is staged.
        use edgequake_core::types::{CreateWorkspaceRequest, Tenant};

        let state = crate::state::AppState::test_state();
        let tenant = state
            .workspace_service
            .create_tenant(Tenant::new("T", "t-slug"))
            .await
            .unwrap();
        let workspace = state
            .workspace_service
            .create_workspace(
                tenant.tenant_id,
                CreateWorkspaceRequest {
                    name: "Sample WS".to_string(),
                    slug: Some("sample-ws".to_string()),
                    description: None,
                    max_documents: None,
                    llm_model: None,
                    llm_provider: None,
                    embedding_model: None,
                    embedding_provider: None,
                    embedding_dimension: None,
                },
            )
            .await
            .unwrap();

        // Build the PRIMARY prefix the sampler will try:
        // {tenant_id}/{workspace_id}/{slug}/  (UUID-form, documents.rs:4312-4330)
        let primary_key = format!(
            "{}/{}/sample-ws/sha-abc/notes.md",
            tenant.tenant_id, workspace.workspace_id
        );

        // Inject the fake reader seeded under the primary prefix.
        let mut reader = InMemoryRawDocsReader::new();
        reader.seed(primary_key.clone(), b"Workspace raw document body");

        let slug = edgequake_core::NamespaceSlug::parse("sample-ws").unwrap();
        let staged = stage_workspace_sampling_dir(&state, &slug, &reader).await;
        let (location, dir, count) = staged.expect("raw docs under primary prefix must be staged");
        assert_eq!(count, 1, "exactly one doc staged");

        // The staged dir contains the document as a .md file the sampler can read.
        let staged_file = std::path::Path::new(&location).join("doc-0000.md");
        let content = std::fs::read_to_string(&staged_file).expect("staged file readable");
        assert_eq!(content, "Workspace raw document body");

        // Dropping the guard cleans up the tempdir.
        let dir_path = dir.path().to_path_buf();
        drop(dir);
        assert!(!dir_path.exists());
    }

    #[tokio::test]
    async fn test_stage_workspace_sampling_picks_alternate_when_primary_empty() {
        // Phase 33-12 new strategy test (Codex HIGH-1):
        // When the PRIMARY prefix ({tenant_id}/{workspace_id}/{slug}/) yields zero objects
        // but the ALTERNATE prefix ({tenant_id}/default/{slug}/) has objects, the sampler
        // must pick the alternate and stage those docs (never silently return None).
        use edgequake_core::types::{CreateWorkspaceRequest, Tenant};

        let state = crate::state::AppState::test_state();
        let tenant = state
            .workspace_service
            .create_tenant(Tenant::new("T", "t-slug"))
            .await
            .unwrap();
        let workspace = state
            .workspace_service
            .create_workspace(
                tenant.tenant_id,
                CreateWorkspaceRequest {
                    name: "Alt WS".to_string(),
                    slug: Some("alt-ws".to_string()),
                    description: None,
                    max_documents: None,
                    llm_model: None,
                    llm_provider: None,
                    embedding_model: None,
                    embedding_provider: None,
                    embedding_dimension: None,
                },
            )
            .await
            .unwrap();

        // Primary prefix (UUID workspace): EMPTY — no objects seeded there.
        // Alternate prefix ("default" workspace): seeded with one doc.
        let alternate_key = format!(
            "{}/default/alt-ws/sha-xyz/upload.md",
            tenant.tenant_id
        );

        // Verify the primary prefix is indeed different from the alternate prefix
        // (so the primary is empty and the test exercises the fallback path).
        let primary_prefix = format!("{}/{}/alt-ws/", tenant.tenant_id, workspace.workspace_id);
        assert!(
            !alternate_key.starts_with(&primary_prefix),
            "alternate key must NOT match primary prefix for this test to be meaningful"
        );

        let mut reader = InMemoryRawDocsReader::new();
        reader.seed(alternate_key, b"Alternate workspace raw document");

        let slug = edgequake_core::NamespaceSlug::parse("alt-ws").unwrap();
        let staged = stage_workspace_sampling_dir(&state, &slug, &reader).await;
        let (_, dir, count) = staged.expect("docs under alternate prefix must be staged");
        assert_eq!(count, 1, "one doc staged from alternate prefix");
        drop(dir);
    }

    #[tokio::test]
    async fn test_stage_workspace_sampling_falls_back_without_documents() {
        // D-05 back-compat (migrated to seam): no workspace matching the slug → None.
        // No documents in the fake reader → None. NOT an error.
        let state = crate::state::AppState::test_state();
        let reader = InMemoryRawDocsReader::new(); // no objects seeded
        let slug = edgequake_core::NamespaceSlug::parse("no-such-ws").unwrap();
        assert!(stage_workspace_sampling_dir(&state, &slug, &reader).await.is_none());
    }

    #[test]
    fn test_failed_marker_serializes_failed_status() {
        // The client TERMINAL_STATUSES list includes "failed" — verify the wire tag.
        let response = SchemaResponse {
            namespace: "test-ns".to_string(),
            schema: SchemaResponseBody::Failed {
                error: "boom".to_string(),
            },
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"failed\""));
        assert!(json.contains("\"error\":\"boom\""));
    }

    // -----------------------------------------------------------------------
    // Task 1 — TDD: build_baseline_proposal + merge_schema pure helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_baseline_proposal_seeds_four_types() {
        // Must seed exactly the 4 baseline entity types; no relations; Proposed.
        let proposal = build_baseline_proposal();
        assert_eq!(proposal.status, SchemaStatus::Proposed);
        assert_eq!(proposal.relation_types.len(), 0, "No relation types in baseline seed");
        assert_eq!(proposal.entity_types.len(), 4, "Baseline must have exactly 4 entity types");

        let names: Vec<&str> = proposal.entity_types.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"PERSON"), "PERSON must be in baseline");
        assert!(names.contains(&"ORGANIZATION"), "ORGANIZATION must be in baseline");
        assert!(names.contains(&"LOCATION"), "LOCATION must be in baseline");
        assert!(names.contains(&"DATE"), "DATE must be in baseline");

        // All must be marked as baseline
        assert!(
            proposal.entity_types.iter().all(|e| e.is_baseline),
            "All baseline types must have is_baseline=true"
        );
        // All have frequency=0 (no documents sampled)
        assert!(
            proposal.entity_types.iter().all(|e| e.frequency == 0),
            "Baseline seed has frequency=0 (no LLM sampling)"
        );
        // sample_size and total_documents are 0 (no LLM call)
        assert_eq!(proposal.sample_size, 0);
        assert_eq!(proposal.total_documents, 0);
    }

    #[test]
    fn test_merge_adds_new_preserves_existing() {
        // Current has admin-edited PERSON. Resampled has PERSON (different desc) + NEW type.
        // Merge must preserve admin description of PERSON, add the new type, remove nothing.
        let current = SchemaProposal {
            status: SchemaStatus::Approved,
            entity_types: vec![EntityTypeProposal {
                name: "PERSON".to_string(),
                description: "edited by admin — authoritative persons".to_string(),
                frequency: 5,
                is_baseline: true,
            }],
            relation_types: vec![],
            sample_size: 10,
            total_documents: 100,
            domain_hint: None,
            proposed_at: 1700000000000,
            reviewed_at: Some(1700000001000),
            sampling_metadata: None,
        };

        let resampled = SchemaProposal {
            status: SchemaStatus::Proposed,
            entity_types: vec![
                EntityTypeProposal {
                    name: "PERSON".to_string(),
                    description: "LLM description of person".to_string(),
                    frequency: 12,
                    is_baseline: true,
                },
                EntityTypeProposal {
                    name: "LEGAL_CASE".to_string(),
                    description: "A legal case file".to_string(),
                    frequency: 8,
                    is_baseline: false,
                },
            ],
            relation_types: vec![],
            sample_size: 20,
            total_documents: 200,
            domain_hint: None,
            proposed_at: 1700001000000,
            reviewed_at: None,
            sampling_metadata: None,
        };

        let result = merge_schema(&current, &resampled);

        // Status must be Proposed (for review)
        assert_eq!(result.proposal.status, SchemaStatus::Proposed);
        // reviewed_at cleared
        assert!(result.proposal.reviewed_at.is_none());

        // PERSON must be preserved with admin description
        let merged_person = result
            .proposal
            .entity_types
            .iter()
            .find(|e| e.name == "PERSON")
            .expect("PERSON must be in merged result");
        assert_eq!(
            merged_person.description,
            "edited by admin — authoritative persons",
            "Admin description must be preserved verbatim"
        );
        // But frequency refreshed from resampled
        assert_eq!(merged_person.frequency, 12, "Frequency refreshed from resampled");

        // LEGAL_CASE must be added
        assert!(
            result.proposal.entity_types.iter().any(|e| e.name == "LEGAL_CASE"),
            "LEGAL_CASE must be added from resampled"
        );

        // added list has LEGAL_CASE, preserved has PERSON
        assert!(result.added.contains(&"LEGAL_CASE".to_string()));
        assert!(result.preserved.contains(&"PERSON".to_string()));

        // Nothing removed: total = 2 (PERSON + LEGAL_CASE)
        assert_eq!(result.proposal.entity_types.len(), 2);

        // No conflicts
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn test_merge_refreshes_frequency() {
        // Verify frequency is refreshed from resampled for existing types.
        let current = SchemaProposal {
            status: SchemaStatus::Proposed,
            entity_types: vec![EntityTypeProposal {
                name: "ORGANIZATION".to_string(),
                description: "admin-edited org description".to_string(),
                frequency: 3,
                is_baseline: true,
            }],
            relation_types: vec![],
            sample_size: 5,
            total_documents: 50,
            domain_hint: None,
            proposed_at: 1700000000000,
            reviewed_at: None,
            sampling_metadata: None,
        };

        let resampled = SchemaProposal {
            status: SchemaStatus::Proposed,
            entity_types: vec![EntityTypeProposal {
                name: "ORGANIZATION".to_string(),
                description: "resampled description".to_string(),
                frequency: 42,
                is_baseline: false,
            }],
            relation_types: vec![],
            sample_size: 20,
            total_documents: 200,
            domain_hint: None,
            proposed_at: 1700001000000,
            reviewed_at: None,
            sampling_metadata: None,
        };

        let result = merge_schema(&current, &resampled);
        let merged_org = result
            .proposal
            .entity_types
            .iter()
            .find(|e| e.name == "ORGANIZATION")
            .expect("ORGANIZATION in merged result");

        // Description preserved (admin-edited)
        assert_eq!(merged_org.description, "admin-edited org description");
        // Frequency refreshed from resampled
        assert_eq!(merged_org.frequency, 42);
        // In preserved list
        assert!(result.preserved.contains(&"ORGANIZATION".to_string()));
        // Not in added list
        assert!(!result.added.contains(&"ORGANIZATION".to_string()));
    }

    #[test]
    fn test_merge_relation_name_conflict_preserves_current_endpoints() {
        // RELATION CONFLICT RULE: same name, different endpoints → keep current verbatim, record conflict.
        let current_rel = RelationTypeProposal {
            name: "employs".to_string(),
            description: "employment relationship".to_string(),
            source_type: "ORGANIZATION".to_string(),
            target_type: "PERSON".to_string(),
            frequency: 10,
        };
        let current = SchemaProposal {
            status: SchemaStatus::Approved,
            entity_types: vec![],
            relation_types: vec![current_rel.clone()],
            sample_size: 10,
            total_documents: 100,
            domain_hint: None,
            proposed_at: 1700000000000,
            reviewed_at: Some(1700000001000),
            sampling_metadata: None,
        };

        // Resampled: same relation name but different (wrong) endpoints
        let resampled = SchemaProposal {
            status: SchemaStatus::Proposed,
            entity_types: vec![],
            relation_types: vec![RelationTypeProposal {
                name: "employs".to_string(),
                description: "employment (resampled, wrong endpoints)".to_string(),
                source_type: "PERSON".to_string(),   // reversed!
                target_type: "ORGANIZATION".to_string(), // reversed!
                frequency: 5,
            }],
            sample_size: 20,
            total_documents: 200,
            domain_hint: None,
            proposed_at: 1700001000000,
            reviewed_at: None,
            sampling_metadata: None,
        };

        let result = merge_schema(&current, &resampled);

        // Only one relation in merged (no duplication)
        assert_eq!(result.proposal.relation_types.len(), 1);

        // The merged relation is the CURRENT one verbatim
        let merged_rel = &result.proposal.relation_types[0];
        assert_eq!(merged_rel.name, "employs");
        assert_eq!(
            merged_rel.source_type, "ORGANIZATION",
            "Current source_type must be preserved"
        );
        assert_eq!(
            merged_rel.target_type, "PERSON",
            "Current target_type must be preserved"
        );
        assert_eq!(
            merged_rel.description, "employment relationship",
            "Current description must be preserved"
        );

        // Conflict recorded
        assert_eq!(result.conflicts.len(), 1);
        let conflict = &result.conflicts[0];
        assert_eq!(conflict.name, "employs");
        assert_eq!(
            conflict.current_endpoints,
            ("ORGANIZATION".to_string(), "PERSON".to_string())
        );
        assert_eq!(
            conflict.rejected_endpoints,
            ("PERSON".to_string(), "ORGANIZATION".to_string())
        );
    }

    // -----------------------------------------------------------------------
    // Task 2 — start-from-defaults + constrained suggest tests
    // (AppState-free in-memory registry mock)
    // -----------------------------------------------------------------------

    /// Minimal in-memory NamespaceRegistry for testing start-from-defaults and
    /// constrained suggest behavior without AppState or DynamoDB.
    ///
    /// Implements only `get_schema` and `store_schema`; all other methods panic.
    #[cfg(test)]
    mod test_registry {
        use async_trait::async_trait;
        use edgequake_core::{
            mcp_descriptor::McpDescriptor,
            namespace::{
                NamespaceListItem, NamespaceRecord, NamespaceRegistry, NamespaceRegistryError,
                NamespaceSlug, PipelineConfig,
            },
            schema::SchemaProposal,
        };
        use std::sync::Mutex;

        /// Simple in-memory registry storing one schema slot.
        pub struct MemoryNamespaceRegistry {
            pub schema: Mutex<Option<SchemaProposal>>,
        }

        impl MemoryNamespaceRegistry {
            pub fn new(initial: Option<SchemaProposal>) -> Self {
                Self {
                    schema: Mutex::new(initial),
                }
            }

            pub fn get_stored(&self) -> Option<SchemaProposal> {
                self.schema.lock().unwrap().clone()
            }
        }

        #[async_trait]
        impl NamespaceRegistry for MemoryNamespaceRegistry {
            async fn create_namespace(
                &self,
                _slug: &NamespaceSlug,
                _description: Option<String>,
            ) -> Result<NamespaceRecord, NamespaceRegistryError> {
                unimplemented!("not needed for schema tests")
            }
            async fn list_namespaces(
                &self,
            ) -> Result<Vec<NamespaceListItem>, NamespaceRegistryError> {
                unimplemented!()
            }
            async fn describe_namespace(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<Option<NamespaceRecord>, NamespaceRegistryError> {
                unimplemented!()
            }
            async fn get_config(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<Option<PipelineConfig>, NamespaceRegistryError> {
                unimplemented!()
            }
            async fn update_config(
                &self,
                _slug: &NamespaceSlug,
                _config: &PipelineConfig,
            ) -> Result<(), NamespaceRegistryError> {
                unimplemented!()
            }
            async fn get_descriptor(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<Option<McpDescriptor>, NamespaceRegistryError> {
                unimplemented!()
            }
            async fn get_schema(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<Option<SchemaProposal>, NamespaceRegistryError> {
                Ok(self.schema.lock().unwrap().clone())
            }
            async fn store_schema(
                &self,
                _slug: &NamespaceSlug,
                proposal: &SchemaProposal,
            ) -> Result<(), NamespaceRegistryError> {
                *self.schema.lock().unwrap() = Some(proposal.clone());
                Ok(())
            }
            async fn approve_schema(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<SchemaProposal, NamespaceRegistryError> {
                unimplemented!()
            }
            async fn reject_schema(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<SchemaProposal, NamespaceRegistryError> {
                unimplemented!()
            }
            async fn put_preview_request(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<(), NamespaceRegistryError> {
                unimplemented!()
            }
            async fn get_preview_status(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<Option<String>, NamespaceRegistryError> {
                unimplemented!()
            }
            async fn get_preview_result(
                &self,
                _slug: &NamespaceSlug,
            ) -> Result<Option<serde_json::Value>, NamespaceRegistryError> {
                unimplemented!()
            }
        }
    }

    #[tokio::test]
    async fn test_start_from_defaults_replaces_existing() {
        // Start-from-defaults must REPLACE any existing proposal, including Approved,
        // with the fresh Proposed 4-baseline-type seed.
        use edgequake_core::NamespaceRegistry;
        use test_registry::MemoryNamespaceRegistry;

        // Set up an existing Approved proposal with extra non-baseline types.
        let existing_approved = SchemaProposal {
            status: SchemaStatus::Approved,
            entity_types: vec![
                EntityTypeProposal {
                    name: "PERSON".to_string(),
                    description: "admin-curated person".to_string(),
                    frequency: 100,
                    is_baseline: true,
                },
                EntityTypeProposal {
                    name: "CUSTOM_DOMAIN_TYPE".to_string(),
                    description: "domain-specific entity".to_string(),
                    frequency: 50,
                    is_baseline: false,
                },
                EntityTypeProposal {
                    name: "ANOTHER_TYPE".to_string(),
                    description: "another type".to_string(),
                    frequency: 25,
                    is_baseline: false,
                },
            ],
            relation_types: vec![RelationTypeProposal {
                name: "employs".to_string(),
                description: "employment".to_string(),
                source_type: "ORGANIZATION".to_string(),
                target_type: "PERSON".to_string(),
                frequency: 30,
            }],
            sample_size: 50,
            total_documents: 500,
            domain_hint: Some("legal".to_string()),
            proposed_at: 1700000000000,
            reviewed_at: Some(1700000001000),
            sampling_metadata: None,
        };

        let registry = MemoryNamespaceRegistry::new(Some(existing_approved));
        let slug = edgequake_core::NamespaceSlug::parse("test-ns").unwrap();

        // Simulate start-from-defaults: store build_baseline_proposal() (REPLACE semantics).
        let baseline = build_baseline_proposal();
        registry.store_schema(&slug, &baseline).await.unwrap();

        // Retrieve what's stored
        let stored = registry.get_schema(&slug).await.unwrap().expect("schema stored");

        // Must be Proposed (not Approved)
        assert_eq!(stored.status, SchemaStatus::Proposed, "Status must be Proposed after defaults reset");

        // Must have exactly the 4 baseline types (prior extra types gone)
        assert_eq!(
            stored.entity_types.len(),
            4,
            "Exactly 4 baseline types; prior extra types replaced"
        );
        let names: Vec<&str> = stored.entity_types.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"PERSON"));
        assert!(names.contains(&"ORGANIZATION"));
        assert!(names.contains(&"LOCATION"));
        assert!(names.contains(&"DATE"));

        // Prior CUSTOM_DOMAIN_TYPE and ANOTHER_TYPE must be gone
        assert!(!names.contains(&"CUSTOM_DOMAIN_TYPE"), "Approved custom type replaced");
        assert!(!names.contains(&"ANOTHER_TYPE"), "Approved extra type replaced");

        // No relation types in baseline seed
        assert_eq!(stored.relation_types.len(), 0, "Baseline seed has no relation types");
        // No domain hint
        assert!(stored.domain_hint.is_none(), "Baseline seed has no domain hint");
    }

    #[tokio::test]
    async fn test_suggest_does_not_overwrite_approved() {
        // Constrained suggest path: when an Approved proposal exists, suggest must NOT
        // wholesale-replace it. It must route through the additive merge (or be rejected).
        // We test the merge path: the Approved proposal's admin types/descriptions survive.
        use edgequake_core::NamespaceRegistry;
        use test_registry::MemoryNamespaceRegistry;

        // Approved proposal with admin-curated PERSON description.
        let approved_proposal = SchemaProposal {
            status: SchemaStatus::Approved,
            entity_types: vec![
                EntityTypeProposal {
                    name: "PERSON".to_string(),
                    description: "admin-curated person".to_string(),
                    frequency: 10,
                    is_baseline: true,
                },
                EntityTypeProposal {
                    name: "ORGANIZATION".to_string(),
                    description: "admin-curated org".to_string(),
                    frequency: 7,
                    is_baseline: true,
                },
            ],
            relation_types: vec![],
            sample_size: 20,
            total_documents: 200,
            domain_hint: None,
            proposed_at: 1700000000000,
            reviewed_at: Some(1700000001000),
            sampling_metadata: None,
        };

        let registry = MemoryNamespaceRegistry::new(Some(approved_proposal.clone()));
        let slug = edgequake_core::NamespaceSlug::parse("test-ns").unwrap();

        // Simulate a constrained suggest: a resampled proposal with PERSON (different desc)
        // + NEW type arrives. The suggest path must merge, not overwrite.
        let resampled = SchemaProposal {
            status: SchemaStatus::Proposed,
            entity_types: vec![
                EntityTypeProposal {
                    name: "PERSON".to_string(),
                    description: "LLM-generated description, should NOT win".to_string(),
                    frequency: 15,
                    is_baseline: true,
                },
                EntityTypeProposal {
                    name: "NEW_TYPE".to_string(),
                    description: "a new type from resampling".to_string(),
                    frequency: 4,
                    is_baseline: false,
                },
            ],
            relation_types: vec![],
            sample_size: 30,
            total_documents: 300,
            domain_hint: None,
            proposed_at: 1700001000000,
            reviewed_at: None,
            sampling_metadata: None,
        };

        // The constrained suggest path uses merge_schema (not wholesale overwrite).
        let current = registry
            .get_schema(&slug)
            .await
            .unwrap()
            .expect("Approved schema exists");
        let merge_result = merge_schema(&current, &resampled);
        // Store the merged proposal (Proposed, for review).
        registry
            .store_schema(&slug, &merge_result.proposal)
            .await
            .unwrap();

        // Verify: the stored proposal is NOT a wholesale replacement of the Approved schema.
        let stored = registry.get_schema(&slug).await.unwrap().expect("stored");

        // Status = Proposed (delta for review — not still Approved)
        assert_eq!(stored.status, SchemaStatus::Proposed, "Merged delta is Proposed");

        // Admin description of PERSON preserved (NOT overwritten by LLM)
        let stored_person = stored
            .entity_types
            .iter()
            .find(|e| e.name == "PERSON")
            .expect("PERSON preserved in merged proposal");
        assert_eq!(
            stored_person.description, "admin-curated person",
            "Admin description must survive the constrained suggest merge"
        );

        // ORGANIZATION also preserved
        let stored_org = stored
            .entity_types
            .iter()
            .find(|e| e.name == "ORGANIZATION")
            .expect("ORGANIZATION preserved");
        assert_eq!(stored_org.description, "admin-curated org");

        // NEW_TYPE was added from resampling
        assert!(
            stored.entity_types.iter().any(|e| e.name == "NEW_TYPE"),
            "NEW_TYPE added from resampled"
        );

        // Total: 3 types (PERSON + ORGANIZATION + NEW_TYPE), nothing removed
        assert_eq!(stored.entity_types.len(), 3, "Nothing removed; new type added");
    }
}
