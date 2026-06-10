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

/// Convert a stored SchemaProposal into a SchemaResponseBody.
fn schema_proposal_to_body(proposal: SchemaProposal) -> SchemaResponseBody {
    match proposal.status {
        SchemaStatus::None => {
            // Stored None carries either the pending sentinel (proposing),
            // a failure marker (failed), or nothing (no schema).
            match proposal.domain_hint.as_deref() {
                Some(PENDING_SENTINEL) => SchemaResponseBody::Proposing,
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
                SchemaResponseBody::Proposing
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
        Some(p) => schema_proposal_to_body(p),
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
pub async fn suggest_namespace_schema(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Json(body): Json<SuggestSchemaRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Triggering schema suggestion");

    // --- Deduplication: if already proposing, don't spawn a second task ---
    if let Ok(Some(existing)) = registry.get_schema(&slug).await {
        if existing.domain_hint.as_deref() == Some(PENDING_SENTINEL) {
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
    }

    // --- Resolve data location ---
    // The namespace's pipeline config contains snapshot_uri. If absent, we
    // cannot determine a data location and must return an error (per plan:
    // "do NOT silently no-op").
    let config = registry
        .get_config(&slug)
        .await
        .map_err(map_registry_error)?
        .ok_or_else(|| {
            ApiError::NotFound(format!("Pipeline config not found for namespace: {}", slug))
        })?;

    let location = config
        .snapshot_uri
        .clone()
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Cannot suggest schema for namespace '{}': snapshot_uri is not configured. \
                 Set snapshot_uri in the pipeline config first.",
                slug
            ))
        })?;

    // Validate location has no path traversal
    if location.contains("..") {
        return Err(ApiError::BadRequest(
            "snapshot_uri must not contain '..' path-traversal sequences".to_string(),
        ));
    }

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
    tokio::spawn(async move {
        info!(
            namespace = slug_clone.as_str(),
            location = %location_clone,
            "Schema suggestion task started"
        );

        match edgequake_schema::suggest_schema(&location_clone, &suggest_input, &openai_config)
            .await
        {
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
                let _ = registry_arc.store_schema(&slug_clone, &failed_marker).await;
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

    // Apply only the fields present in the PATCH body
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

    // Store merged proposal
    registry
        .store_schema(&slug, &proposal)
        .await
        .map_err(map_registry_error)?;

    let body = schema_proposal_to_body(proposal);
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

    #[test]
    fn test_none_proposal_returns_none_body() {
        // Behavior: GET with no proposal returns "none" status variant
        let body = schema_proposal_to_body(SchemaProposal {
            status: SchemaStatus::None,
            domain_hint: None,
            ..make_proposal(SchemaStatus::None)
        });
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
        let body = schema_proposal_to_body(proposal);
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
        let body = schema_proposal_to_body(proposal);
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
        let body = schema_proposal_to_body(proposal);
        match body {
            SchemaResponseBody::Failed { error } => {
                assert_eq!(error, "LLM call timed out");
            }
            _ => panic!("Expected Failed, got {:?}", body),
        }
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
}
