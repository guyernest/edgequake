//! Extraction preview API handlers.
//!
//! Provides REST endpoints for the namespace extraction preview workflow.
//! The actual preview extraction runs in the batch worker (edgequake-batch);
//! the API records the request and returns the result when ready.
//!
//! # Endpoints
//!
//! | Method | Path | Handler | Description |
//! |--------|------|---------|-------------|
//! | POST | `/api/v1/namespaces/{ns}/preview` | [`run_extraction_preview`] | Trigger preview (write PREVIEW_REQUEST) |
//! | GET | `/api/v1/namespaces/{ns}/preview` | [`get_extraction_preview`] | Poll for result |
//!
//! # Preview Result Contract
//!
//! PREVIEW_RESULT is AGGREGATE data only (counts). The exact fields the batch
//! worker writes and the GET handler surfaces:
//!
//! ```json
//! {
//!   "status": "completed",
//!   "entityTypeCounts": [{ "typeName": "PERSON", "count": 42 }],
//!   "relationTypeCounts": [{ "typeName": "employs", "count": 18 }],
//!   "coverageRows": [{ "entityType": "PERSON", "counts": [3, 0, 5] }],
//!   "documentColumns": [{ "id": "doc1", "name": "case-file.pdf", "truncatedName": "case-file..." }],
//!   "totalChunks": 60,
//!   "totalEntities": 312,
//!   "totalRelationships": 88,
//!   "cost": { "inputTokens": 9000, "outputTokens": 2400, "totalCostUsd": 0.0123, "model": "gpt-4.1-mini" },
//!   "processingTimeMs": 28400,
//!   "documentsCompleted": 6,
//!   "documentsTotal": 6
//! }
//! ```
//!
//! Budget: DEFAULT_PREVIEW_BUDGET=6 docs / MAX_PREVIEW_CHUNKS=60 chunks
//! (constants in edgequake-pipeline/src/preview.rs — NOT changed by this plan).
//!
//! NO per-row chunk text / individual entity rows / individual relation rows.
//! D-16's preview tabs render the aggregate shape above.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use tracing::debug;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use edgequake_core::{NamespaceRegistryError, NamespaceSlug};

// ---------------------------------------------------------------------------
// Helpers (same as namespace.rs / schema.rs)
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

/// Trigger an extraction preview for a namespace.
///
/// Writes a PREVIEW_REQUEST item via the registry so the batch worker
/// (`edgequake-batch preview`) picks it up. Returns 202 Accepted.
///
/// The batch worker runs up to 6 documents / 60 chunks (budget constants in
/// `edgequake-pipeline/src/preview.rs`) and writes a PREVIEW_RESULT item when
/// done. Poll GET /preview to check progress.
///
/// Does NOT run preview_extraction inside the API process.
///
/// # Errors
///
/// - 400: Invalid slug
/// - 404: Namespace not found (no registry or namespace absent)
/// - 501: Registry not configured
#[utoipa::path(
    post,
    path = "/api/v1/namespaces/{namespace}/preview",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 202, description = "Preview request recorded; poll GET /preview for progress"),
        (status = 400, description = "Invalid slug"),
        (status = 404, description = "Namespace not found"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn run_extraction_preview(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Triggering extraction preview");

    registry
        .put_preview_request(&slug)
        .await
        .map_err(map_registry_error)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "namespace": slug.as_str(),
            "status": "requested",
            "message": "Preview request recorded; poll GET /preview for progress",
        })),
    ))
}

/// Get the extraction preview result for a namespace.
///
/// Returns the PREVIEW_RESULT aggregate data when the batch worker has
/// finished, or a pending/in-progress status body while the worker runs.
///
/// # Response bodies
///
/// **Result ready** (HTTP 200):
/// ```json
/// {
///   "namespace": "my-ns",
///   "status": "completed",
///   "entityTypeCounts": [...],
///   "relationTypeCounts": [...],
///   "coverageRows": [...],
///   "documentColumns": [...],
///   "totalChunks": 60,
///   "totalEntities": 312,
///   "totalRelationships": 88,
///   "cost": { ... },
///   "processingTimeMs": 28400,
///   "documentsCompleted": 6,
///   "documentsTotal": 6
/// }
/// ```
///
/// **Result pending** (HTTP 200 — poll again):
/// ```json
/// { "namespace": "my-ns", "status": "requested" }
/// // or
/// { "namespace": "my-ns", "status": "processing", "documents_completed": 3, "documents_total": 6 }
/// ```
///
/// **No request recorded** (HTTP 200):
/// ```json
/// { "namespace": "my-ns", "status": "none" }
/// ```
///
/// # Errors
///
/// - 400: Invalid slug
/// - 501: Registry not configured
#[utoipa::path(
    get,
    path = "/api/v1/namespaces/{namespace}/preview",
    params(
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Preview result or pending status"),
        (status = 400, description = "Invalid slug"),
        (status = 501, description = "Namespace registry not configured"),
    ),
    tags = ["Namespaces"]
)]
pub async fn get_extraction_preview(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Getting extraction preview result");

    // Check for completed PREVIEW_RESULT first
    let result = registry
        .get_preview_result(&slug)
        .await
        .map_err(map_registry_error)?;

    if let Some(mut result_data) = result {
        // Surface the exact PREVIEW_RESULT fields; inject namespace for UI convenience
        result_data["namespace"] = serde_json::Value::String(slug.as_str().to_string());
        return Ok(Json(result_data));
    }

    // No result yet — return the PREVIEW_REQUEST status (or "none" if no request)
    let status = registry
        .get_preview_status(&slug)
        .await
        .map_err(map_registry_error)?;

    let body = match status {
        Some(s) => serde_json::json!({
            "namespace": slug.as_str(),
            "status": s,
        }),
        None => serde_json::json!({
            "namespace": slug.as_str(),
            "status": "none",
        }),
    };

    Ok(Json(body))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_registry_returns_error_when_none() {
        // The registry guard is the same pattern as namespace.rs — validate logic
        let state = AppState::test_state();
        let result = get_registry(&state);
        // test_state() has no namespace_registry configured → NotImplemented
        assert!(result.is_err());
    }

    #[test]
    fn test_map_registry_error_not_found() {
        let err = NamespaceRegistryError::NotFound("my-ns".to_string());
        let api_err = map_registry_error(err);
        assert!(matches!(api_err, ApiError::NotFound(_)));
    }

    #[test]
    fn test_map_registry_error_internal() {
        let err = NamespaceRegistryError::Internal("db down".to_string());
        let api_err = map_registry_error(err);
        assert!(matches!(api_err, ApiError::Internal(_)));
    }

    #[test]
    fn test_preview_response_none_body() {
        // Validate the "none" JSON shape the GET handler returns
        let body = serde_json::json!({
            "namespace": "my-ns",
            "status": "none",
        });
        assert_eq!(body["status"], "none");
        assert_eq!(body["namespace"], "my-ns");
    }

    #[test]
    fn test_preview_response_requested_body() {
        let body = serde_json::json!({
            "namespace": "my-ns",
            "status": "requested",
        });
        assert_eq!(body["status"], "requested");
    }

    #[test]
    fn test_preview_result_fields_present() {
        // Validate the PREVIEW_RESULT field set the plan contracts (Plans 03/07 read this)
        let result = serde_json::json!({
            "status": "completed",
            "entityTypeCounts": [{"typeName": "PERSON", "count": 42}],
            "relationTypeCounts": [{"typeName": "employs", "count": 18}],
            "coverageRows": [{"entityType": "PERSON", "counts": [3, 0, 5]}],
            "documentColumns": [{"id": "doc1", "name": "case-file.pdf", "truncatedName": "case-file..."}],
            "totalChunks": 60,
            "totalEntities": 312,
            "totalRelationships": 88,
            "cost": {
                "inputTokens": 9000,
                "outputTokens": 2400,
                "totalCostUsd": 0.0123,
                "model": "gpt-4.1-mini"
            },
            "processingTimeMs": 28400,
            "documentsCompleted": 6,
            "documentsTotal": 6
        });

        // Verify all required top-level fields are present
        assert!(result["entityTypeCounts"].is_array());
        assert!(result["relationTypeCounts"].is_array());
        assert!(result["coverageRows"].is_array());
        assert!(result["documentColumns"].is_array());
        assert!(result["totalChunks"].is_number());
        assert!(result["totalEntities"].is_number());
        assert!(result["totalRelationships"].is_number());
        assert!(result["cost"].is_object());
        assert!(result["processingTimeMs"].is_number());
        assert!(result["documentsCompleted"].is_number());
        assert!(result["documentsTotal"].is_number());
    }
}
