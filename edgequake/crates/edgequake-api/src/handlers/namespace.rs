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

/// Validate a pipeline config update request against the current config.
///
/// Called before applying any updates. Returns `ApiError::BadRequest` on
/// the first failed constraint; returns `Ok(())` if all checks pass.
///
/// Implements T-23-02 and T-23-11 server-side validation from the threat model.
fn validate_pipeline_config_update(
    body: &UpdatePipelineConfigRequest,
    config: &edgequake_core::PipelineConfig,
) -> Result<(), ApiError> {
    // -- snapshot_mode: must be "write-and-store" or "snapshot-only" (T-23-11)
    if let Some(ref mode) = body.snapshot_mode {
        if mode != "write-and-store" && mode != "snapshot-only" {
            return Err(ApiError::BadRequest(format!(
                "Invalid snapshot_mode '{}': must be 'write-and-store' or 'snapshot-only'",
                mode
            )));
        }
    }

    // -- chunking_strategy: must be "token" or "heading_boundary" (T-23-11 / D-07)
    if let Some(ref strategy) = body.chunking_strategy {
        if strategy != "token" && strategy != "heading_boundary" {
            return Err(ApiError::BadRequest(format!(
                "Invalid chunking_strategy '{}': must be 'token' or 'heading_boundary'",
                strategy
            )));
        }
    }

    // -- snapshot_uri format guard (T-23-02): reject path-traversal and bad schemes
    if let Some(ref uri) = body.snapshot_uri {
        if !uri.is_empty() {
            if uri.contains("..") {
                return Err(ApiError::BadRequest(
                    "snapshot_uri must not contain '..' path-traversal sequences".to_string(),
                ));
            }
            let has_s3_prefix = uri.starts_with("s3://");
            let has_abs_path = uri.starts_with('/');
            if !has_s3_prefix && !has_abs_path {
                return Err(ApiError::BadRequest(
                    "snapshot_uri must start with 's3://' or be an absolute path starting with '/'".to_string(),
                ));
            }
        }
    }

    // -- target_tokens range: 64..=2048 (T-23-11)
    // Compute effective target for the overlap check below.
    let effective_target = body.target_tokens.unwrap_or(config.target_tokens);
    if let Some(t) = body.target_tokens {
        if !(64..=2048).contains(&t) {
            return Err(ApiError::BadRequest(format!(
                "target_tokens {} is out of range: must be between 64 and 2048 inclusive",
                t
            )));
        }
    }

    // -- overlap_tokens vs target_tokens (T-23-11 / D-08):
    // Validate the EFFECTIVE merged pair whenever EITHER field changes, so
    // lowering target_tokens alone cannot persist a stored overlap that
    // violates the overlap <= target/2 invariant (WR-01).
    if body.target_tokens.is_some() || body.overlap_tokens.is_some() {
        let effective_overlap = body.overlap_tokens.unwrap_or(config.overlap_tokens);
        if effective_overlap >= effective_target {
            return Err(ApiError::BadRequest(format!(
                "effective overlap_tokens {} must be less than effective target_tokens {} \
                 (after merging this update with the stored config)",
                effective_overlap, effective_target
            )));
        }
        if effective_overlap * 2 > effective_target {
            return Err(ApiError::BadRequest(format!(
                "effective overlap_tokens {} exceeds 50% of effective target_tokens {} \
                 (overlap must be <= target/2 after merging this update with the stored config)",
                effective_overlap, effective_target
            )));
        }
    }

    Ok(())
}

/// Update pipeline configuration for a namespace.
///
/// Performs a partial update -- only fields present in the request body are
/// modified. The `updated_at` timestamp is always set to the current time.
///
/// # Errors
///
/// - 400: Invalid slug format or validation failure
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

    // Validate incoming body before applying any changes (T-23-02, T-23-11)
    validate_pipeline_config_update(&body, &config)?;

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
    // Phase 23: snapshot + chunking merge arms (snapshot_uri before snapshot_uri_clear so clear wins)
    if let Some(v) = body.snapshot_uri {
        config.snapshot_uri = Some(v);
    }
    if body.snapshot_uri_clear == Some(true) {
        config.snapshot_uri = None;
    }
    if let Some(v) = body.snapshot_mode {
        config.snapshot_mode = v;
    }
    // Note: chunking_strategy is merged above with the existing fields (line ~322)
    if let Some(v) = body.chunking_enabled {
        config.chunking_enabled = v;
    }
    if let Some(v) = body.target_tokens {
        config.target_tokens = v;
    }
    if let Some(v) = body.overlap_tokens {
        config.overlap_tokens = v;
    }
    if let Some(v) = body.prepend_header_path {
        config.prepend_header_path = v;
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

#[cfg(test)]
mod tests {
    use super::*;
    use edgequake_core::PipelineConfig;

    fn make_config() -> PipelineConfig {
        PipelineConfig::default()
    }

    // RED: these tests call validate_pipeline_config_update which doesn't exist yet
    #[test]
    fn test_valid_update_body_passes_validation() {
        let body = UpdatePipelineConfigRequest {
            llm_provider: None,
            llm_model: None,
            embedding_provider: None,
            embedding_model: None,
            embedding_dimension: None,
            chunking_strategy: Some("heading_boundary".to_string()),
            chunk_size: None,
            chunk_overlap: None,
            extraction_prompt: None,
            entity_types: None,
            relation_types: None,
            snapshot_uri: Some("s3://mybucket/prefix".to_string()),
            snapshot_uri_clear: None,
            snapshot_mode: Some("snapshot-only".to_string()),
            chunking_enabled: Some(true),
            target_tokens: Some(512),
            overlap_tokens: Some(64),
            prepend_header_path: Some(false),
        };
        let config = make_config();
        assert!(validate_pipeline_config_update(&body, &config).is_ok());
    }

    #[test]
    fn test_invalid_snapshot_mode_rejected() {
        let body = UpdatePipelineConfigRequest {
            snapshot_mode: Some("garbage".to_string()),
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_invalid_chunking_strategy_rejected() {
        let body = UpdatePipelineConfigRequest {
            chunking_strategy: Some("garbage".to_string()),
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_target_tokens_too_small_rejected() {
        let body = UpdatePipelineConfigRequest {
            target_tokens: Some(32), // < 64
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_target_tokens_too_large_rejected() {
        let body = UpdatePipelineConfigRequest {
            target_tokens: Some(4096), // > 2048
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_overlap_tokens_gte_target_rejected() {
        let body = UpdatePipelineConfigRequest {
            target_tokens: Some(256),
            overlap_tokens: Some(256), // overlap >= target
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_overlap_tokens_exceeds_50_percent_rejected() {
        let body = UpdatePipelineConfigRequest {
            target_tokens: Some(256),
            overlap_tokens: Some(129), // 129 * 2 = 258 > 256 → > 50%
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_target_only_update_validates_effective_overlap() {
        // WR-01: lowering target_tokens alone must be checked against the
        // STORED overlap. Default config: target=256, overlap=38.
        // target=64 with stored overlap 38 → 38*2=76 > 64 → reject.
        let body = UpdatePipelineConfigRequest {
            target_tokens: Some(64),
            overlap_tokens: None,
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_target_only_update_passes_when_effective_pair_valid() {
        // WR-01: target=128 with stored overlap 38 → 38*2=76 <= 128 → OK.
        let body = UpdatePipelineConfigRequest {
            target_tokens: Some(128),
            overlap_tokens: None,
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        assert!(validate_pipeline_config_update(&body, &config).is_ok());
    }

    #[test]
    fn test_snapshot_uri_path_traversal_rejected() {
        let body = UpdatePipelineConfigRequest {
            snapshot_uri: Some("s3://bucket/../hack".to_string()),
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_snapshot_uri_invalid_scheme_rejected() {
        let body = UpdatePipelineConfigRequest {
            snapshot_uri: Some("http://evil.com/bucket".to_string()),
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        let err = validate_pipeline_config_update(&body, &config).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn test_snapshot_uri_absolute_path_accepted() {
        let body = UpdatePipelineConfigRequest {
            snapshot_uri: Some("/var/data/snapshots/my-ns".to_string()),
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        assert!(validate_pipeline_config_update(&body, &config).is_ok());
    }

    #[test]
    fn test_snapshot_uri_clear_sets_none() {
        let body = UpdatePipelineConfigRequest {
            snapshot_uri_clear: Some(true),
            ..UpdatePipelineConfigRequest::default()
        };
        let config = make_config();
        assert!(validate_pipeline_config_update(&body, &config).is_ok());
    }
}
