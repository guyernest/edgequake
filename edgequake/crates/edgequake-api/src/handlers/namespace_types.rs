//! Request/response DTOs for namespace management API.
//!
//! These types define the HTTP interface for namespace CRUD operations
//! and pipeline configuration management.

use edgequake_core::{NamespaceListItem, PipelineConfig};
use serde::{Deserialize, Serialize};

/// Request body for creating a new namespace.
#[derive(Debug, Deserialize)]
pub struct CreateNamespaceRequest {
    /// The namespace slug (DNS-safe identifier).
    pub slug: String,
    /// Optional human-readable description.
    pub description: Option<String>,
}

/// Response for a successfully created namespace.
#[derive(Debug, Serialize)]
pub struct CreateNamespaceResponse {
    /// The validated namespace slug.
    pub slug: String,
    /// Optional description.
    pub description: Option<String>,
    /// Creation timestamp (epoch milliseconds).
    pub created_at: i64,
}

/// Response containing a list of namespaces.
#[derive(Debug, Serialize)]
pub struct NamespaceListResponse {
    /// The list of namespaces.
    pub namespaces: Vec<NamespaceListItem>,
}

/// Detailed response for a single namespace.
#[derive(Debug, Serialize)]
pub struct NamespaceDetailResponse {
    /// The namespace slug.
    pub slug: String,
    /// Optional description.
    pub description: Option<String>,
    /// Creation timestamp (epoch milliseconds).
    pub created_at: i64,
    /// Optional creator identifier.
    pub created_by: Option<String>,
}

/// Response wrapping pipeline configuration.
#[derive(Debug, Serialize)]
pub struct PipelineConfigResponse {
    /// The namespace this config belongs to.
    pub namespace: String,
    /// The pipeline configuration.
    #[serde(flatten)]
    pub config: PipelineConfig,
}

/// Request for updating pipeline configuration (all fields optional for partial update).
#[derive(Debug, Deserialize)]
pub struct UpdatePipelineConfigRequest {
    /// LLM provider name.
    pub llm_provider: Option<String>,
    /// LLM model identifier.
    pub llm_model: Option<String>,
    /// Embedding provider name.
    pub embedding_provider: Option<String>,
    /// Embedding model identifier.
    pub embedding_model: Option<String>,
    /// Embedding vector dimension.
    pub embedding_dimension: Option<usize>,
    /// Chunking strategy.
    pub chunking_strategy: Option<String>,
    /// Maximum chunk size in tokens.
    pub chunk_size: Option<usize>,
    /// Token overlap between consecutive chunks.
    pub chunk_overlap: Option<usize>,
    /// Custom extraction prompt.
    pub extraction_prompt: Option<String>,
    /// Allowed entity types for extraction.
    pub entity_types: Option<Vec<String>>,
    /// Allowed relation types for extraction.
    pub relation_types: Option<Vec<String>>,
}
