//! Namespace management API handlers.
//!
//! Provides REST endpoints for namespace CRUD operations and pipeline
//! configuration management. Namespaces are the top-level isolation unit
//! in EdgeQuake -- each namespace gets its own Neptune label prefix,
//! S3 Vectors index, and DynamoDB key-value namespace.
//!
//! # Endpoints
//!
//! | Method | Path | Handler | Description |
//! |--------|------|---------|-------------|
//! | POST | `/api/v1/namespaces` | [`create_namespace`] | Create new namespace |
//! | GET | `/api/v1/namespaces` | [`list_namespaces`] | List all namespaces |
//! | GET | `/api/v1/namespaces/{namespace}` | [`describe_namespace`] | Get namespace details |
//! | GET | `/api/v1/namespaces/{namespace}/config` | [`get_namespace_config`] | Get pipeline config |
//! | PUT | `/api/v1/namespaces/{namespace}/config` | [`update_namespace_config`] | Update pipeline config |

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use tracing::debug;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use edgequake_core::{NamespaceRegistryError, NamespaceSlug};

pub use crate::handlers::namespace_types::*;

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
fn get_registry(
    state: &AppState,
) -> Result<&dyn edgequake_core::NamespaceRegistry, ApiError> {
    state
        .namespace_registry
        .as_ref()
        .map(|r| r.as_ref())
        .ok_or_else(|| ApiError::NotImplemented {
            feature: "Namespace registry not configured (requires AWS DynamoDB)".to_string(),
        })
}

/// Create a new namespace.
///
/// Validates the slug format and creates the namespace in the registry
/// with a default pipeline configuration.
///
/// # Errors
///
/// - 400: Invalid slug format
/// - 409: Namespace with this slug already exists
/// - 501: Namespace registry not configured
pub async fn create_namespace(
    State(state): State<AppState>,
    Json(body): Json<CreateNamespaceRequest>,
) -> ApiResult<(StatusCode, Json<CreateNamespaceResponse>)> {
    let registry = get_registry(&state)?;

    // Validate slug format
    let slug = NamespaceSlug::parse(&body.slug)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Creating namespace");

    let record = registry
        .create_namespace(&slug, body.description)
        .await
        .map_err(map_registry_error)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateNamespaceResponse {
            slug: record.slug.as_str().to_string(),
            description: record.description,
            created_at: record.created_at,
        }),
    ))
}

/// List all namespaces.
///
/// Returns all registered namespaces. Stats fields (entity_count, document_count)
/// are currently returned as-is from the registry (None). A future enhancement
/// will enrich them via namespace-scoped graph stats.
///
/// # Errors
///
/// - 501: Namespace registry not configured
pub async fn list_namespaces(
    State(state): State<AppState>,
) -> ApiResult<Json<NamespaceListResponse>> {
    let registry = get_registry(&state)?;

    debug!("Listing namespaces");

    let namespaces = registry
        .list_namespaces()
        .await
        .map_err(map_registry_error)?;

    // Note: Stats enrichment (entity_count, document_count) via get_graph_stats
    // is deferred. For the expected low namespace count (<20), per-namespace
    // Neptune round-trips are acceptable but require namespace-scoped storage
    // resolution which is wired in Task 2.

    Ok(Json(NamespaceListResponse { namespaces }))
}

/// Describe a single namespace.
///
/// # Errors
///
/// - 400: Invalid slug format
/// - 404: Namespace not found
/// - 501: Namespace registry not configured
pub async fn describe_namespace(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> ApiResult<Json<NamespaceDetailResponse>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Describing namespace");

    let record = registry
        .describe_namespace(&slug)
        .await
        .map_err(map_registry_error)?
        .ok_or_else(|| ApiError::NotFound(format!("Namespace not found: {}", slug)))?;

    Ok(Json(NamespaceDetailResponse {
        slug: record.slug.as_str().to_string(),
        description: record.description,
        created_at: record.created_at,
        created_by: record.created_by,
    }))
}

/// Get pipeline configuration for a namespace.
///
/// # Errors
///
/// - 400: Invalid slug format
/// - 404: Namespace or config not found
/// - 501: Namespace registry not configured
pub async fn get_namespace_config(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> ApiResult<Json<PipelineConfigResponse>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Getting namespace config");

    let config = registry
        .get_config(&slug)
        .await
        .map_err(map_registry_error)?
        .ok_or_else(|| {
            ApiError::NotFound(format!("Pipeline config not found for namespace: {}", slug))
        })?;

    Ok(Json(PipelineConfigResponse {
        namespace: slug.as_str().to_string(),
        config,
    }))
}

/// Update pipeline configuration for a namespace.
///
/// Performs a partial update -- only fields present in the request body are
/// modified. The `updated_at` timestamp is always set to the current time.
///
/// # Errors
///
/// - 400: Invalid slug format
/// - 404: Namespace not found
/// - 501: Namespace registry not configured
pub async fn update_namespace_config(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Json(body): Json<UpdatePipelineConfigRequest>,
) -> ApiResult<Json<PipelineConfigResponse>> {
    let registry = get_registry(&state)?;

    let slug = NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace slug: {}", e)))?;

    debug!(namespace = slug.as_str(), "Updating namespace config");

    // Get existing config to merge with updates
    let mut config = registry
        .get_config(&slug)
        .await
        .map_err(map_registry_error)?
        .ok_or_else(|| {
            ApiError::NotFound(format!("Pipeline config not found for namespace: {}", slug))
        })?;

    // Apply partial updates
    if let Some(v) = body.llm_provider {
        config.llm_provider = v;
    }
    if let Some(v) = body.llm_model {
        config.llm_model = v;
    }
    if let Some(v) = body.embedding_provider {
        config.embedding_provider = v;
    }
    if let Some(v) = body.embedding_model {
        config.embedding_model = v;
    }
    if let Some(v) = body.embedding_dimension {
        config.embedding_dimension = v;
    }
    if let Some(v) = body.chunking_strategy {
        config.chunking_strategy = v;
    }
    if let Some(v) = body.chunk_size {
        config.chunk_size = v;
    }
    if let Some(v) = body.chunk_overlap {
        config.chunk_overlap = v;
    }
    if body.extraction_prompt.is_some() {
        config.extraction_prompt = body.extraction_prompt;
    }
    if let Some(v) = body.entity_types {
        config.entity_types = v;
    }
    if let Some(v) = body.relation_types {
        config.relation_types = v;
    }

    // Always update the timestamp
    config.updated_at = chrono::Utc::now().timestamp_millis();

    registry
        .update_config(&slug, &config)
        .await
        .map_err(map_registry_error)?;

    Ok(Json(PipelineConfigResponse {
        namespace: slug.as_str().to_string(),
        config,
    }))
}
