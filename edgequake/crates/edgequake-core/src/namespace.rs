//! Namespace core types for multi-tenant isolation.
//!
//! This module defines the foundational types used throughout EdgeQuake for
//! namespace-based data isolation:
//!
//! - [`NamespaceSlug`] — Validated, DNS-safe identifier for a namespace
//! - [`NamespaceRecord`] — Immutable namespace metadata (created once)
//! - [`PipelineConfig`] — Mutable per-namespace pipeline configuration

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

use crate::mcp_descriptor::McpDescriptor;
use crate::schema::SchemaProposal;

/// A validated namespace slug (DNS-safe identifier).
///
/// Format: lowercase alphanumeric characters and hyphens only, 1-63 characters.
/// Cannot start or end with a hyphen. This format is compatible with DNS labels,
/// S3 index names, and Neptune label prefixes.
///
/// # Examples
///
/// ```
/// use edgequake_core::NamespaceSlug;
///
/// let slug = NamespaceSlug::parse("epstein-files").unwrap();
/// assert_eq!(slug.as_str(), "epstein-files");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct NamespaceSlug(String);

impl NamespaceSlug {
    /// Parse and validate a namespace slug.
    ///
    /// # Validation rules
    ///
    /// - Must be 1-63 characters long
    /// - Only lowercase alphanumeric characters (`a-z`, `0-9`) and hyphens (`-`)
    /// - Cannot start or end with a hyphen
    ///
    /// # Errors
    ///
    /// Returns an error if the input fails any validation rule.
    pub fn parse(s: &str) -> Result<Self, crate::Error> {
        if s.is_empty() {
            return Err(crate::Error::Validation(
                "Namespace slug cannot be empty".into(),
            ));
        }
        if s.len() > 63 {
            return Err(crate::Error::Validation(format!(
                "Namespace slug too long ({} chars, max 63): '{}'",
                s.len(),
                &s[..32]
            )));
        }
        if s.starts_with('-') {
            return Err(crate::Error::Validation(format!(
                "Namespace slug cannot start with a hyphen: '{}'",
                s
            )));
        }
        if s.ends_with('-') {
            return Err(crate::Error::Validation(format!(
                "Namespace slug cannot end with a hyphen: '{}'",
                s
            )));
        }
        if !s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(crate::Error::Validation(format!(
                "Namespace slug contains invalid characters (only lowercase alphanumeric and hyphens allowed): '{}'",
                s
            )));
        }
        Ok(Self(s.to_string()))
    }

    /// Return the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NamespaceSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for NamespaceSlug {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NamespaceSlug {
    type Error = crate::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl<'de> Deserialize<'de> for NamespaceSlug {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        NamespaceSlug::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Namespace metadata record (immutable after creation).
///
/// Stored in DynamoDB as the authority for namespace existence.
/// The slug is the primary key and cannot be changed after creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceRecord {
    /// The unique namespace identifier.
    pub slug: NamespaceSlug,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Creation timestamp (epoch milliseconds).
    pub created_at: i64,
    /// Optional creator identifier.
    pub created_by: Option<String>,
}

/// Per-namespace pipeline configuration (mutable after creation).
///
/// Controls how documents are processed within a namespace: which LLM and
/// embedding models to use, chunking parameters, and extraction settings.
/// Partners can customize these per dataset or domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// LLM provider name (e.g., "openai", "anthropic").
    pub llm_provider: String,
    /// LLM model identifier (e.g., "gpt-4o-mini").
    pub llm_model: String,
    /// Embedding provider name (e.g., "openai").
    pub embedding_provider: String,
    /// Embedding model identifier (e.g., "text-embedding-3-small").
    pub embedding_model: String,
    /// Embedding vector dimension.
    pub embedding_dimension: usize,
    /// Chunking strategy (e.g., "token", "sentence").
    pub chunking_strategy: String,
    /// Maximum chunk size in tokens.
    pub chunk_size: usize,
    /// Token overlap between consecutive chunks.
    pub chunk_overlap: usize,
    /// Optional custom extraction prompt (overrides default).
    pub extraction_prompt: Option<String>,
    /// Allowed entity types for extraction (empty = all types).
    pub entity_types: Vec<String>,
    /// Allowed relation types for extraction (empty = all types).
    pub relation_types: Vec<String>,

    // D-03/D-04: Snapshot config (Phase 23 — backward-compat with #[serde(default)])
    /// Destination URI for snapshot export (s3://bucket/prefix or local path).
    /// None = no snapshot export configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_uri: Option<String>,

    /// Snapshot output mode: "write-and-store" or "snapshot-only".
    /// Default: "write-and-store" (matches SnapshotMode::WriteAndStore::as_str()).
    #[serde(default = "default_snapshot_mode")]
    pub snapshot_mode: String,

    // D-05/D-06: Hierarchical chunking config (Phase 23 — additive, OFF by default)
    /// Whether hierarchical header-aware chunking is enabled.
    /// Default: false (byte-identical legacy behavior when OFF).
    #[serde(default)]
    pub chunking_enabled: bool,

    /// Target chunk size in tokens (default 256, Phase 22 D-07 locked value).
    /// Distinct from chunk_size (legacy token-window field) to avoid conflicts.
    #[serde(default = "default_target_tokens")]
    pub target_tokens: usize,

    /// Overlap tokens (default 38, ~15% of 256, Phase 22 D-07 locked value).
    #[serde(default = "default_overlap_tokens")]
    pub overlap_tokens: usize,

    /// Prepend header breadcrumb to embedded text (default true when enabled).
    #[serde(default = "default_true")]
    pub prepend_header_path: bool,

    /// Last update timestamp (epoch milliseconds).
    pub updated_at: i64,
}

fn default_snapshot_mode() -> String {
    "write-and-store".to_string()
}
fn default_target_tokens() -> usize {
    256
}
fn default_overlap_tokens() -> usize {
    38
}
fn default_true() -> bool {
    true
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            llm_provider: "openai".to_string(),
            llm_model: "gpt-4o-mini".to_string(),
            embedding_provider: "openai".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimension: 1536,
            chunking_strategy: "token".to_string(),
            chunk_size: 1200,
            chunk_overlap: 100,
            extraction_prompt: None,
            entity_types: vec![],
            relation_types: vec![],
            snapshot_uri: None,
            snapshot_mode: default_snapshot_mode(),
            chunking_enabled: false,
            target_tokens: default_target_tokens(),
            overlap_tokens: default_overlap_tokens(),
            prepend_header_path: default_true(),
            updated_at: 0,
        }
    }
}

/// An item in the namespace listing.
///
/// Contains the slug and creation timestamp plus optional stats fields.
/// The `entity_count` and `document_count` are always `None` from the
/// registry layer -- they are enriched by the API handler using
/// namespace-scoped `get_graph_stats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceListItem {
    /// The namespace slug.
    pub slug: String,
    /// Creation timestamp (epoch milliseconds).
    pub created_at: i64,
    /// Number of entities in this namespace (populated by API layer, not registry).
    pub entity_count: Option<u64>,
    /// Number of documents in this namespace (populated by API layer, not registry).
    pub document_count: Option<u64>,
}

/// Namespace registry errors.
#[derive(Debug, thiserror::Error)]
pub enum NamespaceRegistryError {
    /// Namespace already exists.
    #[error("Namespace already exists: {0}")]
    AlreadyExists(String),
    /// Namespace not found.
    #[error("Namespace not found: {0}")]
    NotFound(String),
    /// Schema not found for namespace.
    #[error("Schema not found for namespace: {0}")]
    SchemaNotFound(String),
    /// Schema is in an invalid state for the requested operation.
    #[error("Invalid schema state: {0}")]
    InvalidSchemaState(String),
    /// Internal error (storage backend failure).
    #[error("Registry error: {0}")]
    Internal(String),
}

/// Trait for namespace registry operations.
///
/// Abstracts the DynamoDB-backed namespace registry so the API crate does
/// not depend on `edgequake-storage-aws`. Concrete implementation lives in
/// `edgequake_storage_aws::DynamoNamespaceRegistry`.
#[async_trait]
pub trait NamespaceRegistry: Send + Sync {
    /// Create a new namespace.
    ///
    /// Returns `AlreadyExists` if the slug is taken.
    async fn create_namespace(
        &self,
        slug: &NamespaceSlug,
        description: Option<String>,
    ) -> Result<NamespaceRecord, NamespaceRegistryError>;

    /// List all namespaces.
    async fn list_namespaces(&self) -> Result<Vec<NamespaceListItem>, NamespaceRegistryError>;

    /// Describe a single namespace by slug.
    async fn describe_namespace(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<Option<NamespaceRecord>, NamespaceRegistryError>;

    /// Get the pipeline configuration for a namespace.
    async fn get_config(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<Option<PipelineConfig>, NamespaceRegistryError>;

    /// Update the pipeline configuration for an existing namespace.
    async fn update_config(
        &self,
        slug: &NamespaceSlug,
        config: &PipelineConfig,
    ) -> Result<(), NamespaceRegistryError>;

    /// Get the MCP descriptor for a namespace.
    ///
    /// Returns the self-contained [`McpDescriptor`] if the namespace has one,
    /// or `None` if the descriptor has not been generated yet.
    async fn get_descriptor(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<Option<McpDescriptor>, NamespaceRegistryError>;

    /// Get the schema proposal for a namespace.
    ///
    /// Returns the [`SchemaProposal`] if one exists, or `None` if no schema
    /// has been proposed yet.
    async fn get_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<Option<SchemaProposal>, NamespaceRegistryError>;

    /// Store a schema proposal for a namespace.
    ///
    /// Replaces any existing proposal. Also clears `entity_types` and
    /// `relation_types` from the `PipelineConfig` to force re-approval.
    async fn store_schema(
        &self,
        slug: &NamespaceSlug,
        proposal: &SchemaProposal,
    ) -> Result<(), NamespaceRegistryError>;

    /// Approve the current schema proposal.
    ///
    /// Sets status to `Approved`, copies entity/relation type names into
    /// `PipelineConfig`, and returns the updated proposal.
    ///
    /// # Errors
    ///
    /// Returns `SchemaNotFound` if no proposal exists, or `InvalidSchemaState`
    /// if the proposal is not in `Proposed` status.
    async fn approve_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<SchemaProposal, NamespaceRegistryError>;

    /// Reject the current schema proposal.
    ///
    /// Sets status to `Rejected` and returns the updated proposal.
    ///
    /// # Errors
    ///
    /// Returns `SchemaNotFound` if no proposal exists, or `InvalidSchemaState`
    /// if the proposal is not in `Proposed` status.
    async fn reject_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<SchemaProposal, NamespaceRegistryError>;
}

/// Type alias for a shared namespace registry.
pub type SharedNamespaceRegistry = Arc<dyn NamespaceRegistry>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_slugs() {
        let valid = vec!["epstein-files", "a", "my-namespace-123", "test", "a1b2c3"];
        for s in valid {
            assert!(
                NamespaceSlug::parse(s).is_ok(),
                "Expected '{}' to be valid",
                s
            );
        }
    }

    #[test]
    fn test_invalid_empty() {
        assert!(NamespaceSlug::parse("").is_err());
    }

    #[test]
    fn test_invalid_uppercase() {
        assert!(NamespaceSlug::parse("UPPERCASE").is_err());
    }

    #[test]
    fn test_invalid_spaces() {
        assert!(NamespaceSlug::parse("has spaces").is_err());
    }

    #[test]
    fn test_invalid_leading_hyphen() {
        assert!(NamespaceSlug::parse("-leading").is_err());
    }

    #[test]
    fn test_invalid_trailing_hyphen() {
        assert!(NamespaceSlug::parse("trailing-").is_err());
    }

    #[test]
    fn test_invalid_too_long() {
        let long = "a".repeat(64);
        assert!(NamespaceSlug::parse(&long).is_err());
    }

    #[test]
    fn test_invalid_special_chars() {
        assert!(NamespaceSlug::parse("special!chars").is_err());
    }

    #[test]
    fn test_max_length_valid() {
        let max = "a".repeat(63);
        assert!(NamespaceSlug::parse(&max).is_ok());
    }

    #[test]
    fn test_display() {
        let slug = NamespaceSlug::parse("epstein-files").unwrap();
        assert_eq!(format!("{}", slug), "epstein-files");
    }

    #[test]
    fn test_as_ref() {
        let slug = NamespaceSlug::parse("test-ns").unwrap();
        let s: &str = slug.as_ref();
        assert_eq!(s, "test-ns");
    }

    #[test]
    fn test_try_from_string() {
        let result = NamespaceSlug::try_from("valid-slug".to_string());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_str(), "valid-slug");
    }

    #[test]
    fn test_try_from_string_invalid() {
        let result = NamespaceSlug::try_from("INVALID".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_serde_roundtrip() {
        let slug = NamespaceSlug::parse("test-ns").unwrap();
        let json = serde_json::to_string(&slug).unwrap();
        assert_eq!(json, "\"test-ns\"");
        let deserialized: NamespaceSlug = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, slug);
    }

    #[test]
    fn test_serde_deserialize_invalid() {
        let result: Result<NamespaceSlug, _> = serde_json::from_str("\"INVALID\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_config_default() {
        let config = PipelineConfig::default();
        assert_eq!(config.llm_provider, "openai");
        assert_eq!(config.llm_model, "gpt-4o-mini");
        assert_eq!(config.embedding_provider, "openai");
        assert_eq!(config.embedding_model, "text-embedding-3-small");
        assert_eq!(config.embedding_dimension, 1536);
        assert_eq!(config.chunking_strategy, "token");
        assert_eq!(config.chunk_size, 1200);
        assert_eq!(config.chunk_overlap, 100);
        assert!(config.extraction_prompt.is_none());
        assert!(config.entity_types.is_empty());
        assert!(config.relation_types.is_empty());
        assert_eq!(config.updated_at, 0);
        // Phase 23: new snapshot + chunking fields with defaults
        assert!(config.snapshot_uri.is_none());
        assert_eq!(config.snapshot_mode, "write-and-store");
        assert!(!config.chunking_enabled);
        assert_eq!(config.target_tokens, 256);
        assert_eq!(config.overlap_tokens, 38);
        assert!(config.prepend_header_path);
    }

    #[test]
    fn test_pipeline_config_new_fields_serde_default() {
        // Old record without new fields should deserialize cleanly with Phase 22 defaults
        let json = r#"{
            "llm_provider": "openai",
            "llm_model": "gpt-4o-mini",
            "embedding_provider": "openai",
            "embedding_model": "text-embedding-3-small",
            "embedding_dimension": 1536,
            "chunking_strategy": "token",
            "chunk_size": 1200,
            "chunk_overlap": 100,
            "entity_types": [],
            "relation_types": [],
            "updated_at": 0
        }"#;
        let config: PipelineConfig = serde_json::from_str(json).expect("deserialize old record");
        assert!(config.snapshot_uri.is_none());
        assert_eq!(config.snapshot_mode, "write-and-store");
        assert!(!config.chunking_enabled);
        assert_eq!(config.target_tokens, 256);
        assert_eq!(config.overlap_tokens, 38);
        assert!(config.prepend_header_path);
        // Existing strategy field unchanged
        assert_eq!(config.chunking_strategy, "token");
    }

    #[test]
    fn test_namespace_record_serde() {
        let record = NamespaceRecord {
            slug: NamespaceSlug::parse("epstein-files").unwrap(),
            description: Some("Epstein investigation files".into()),
            created_at: 1700000000000,
            created_by: Some("admin".into()),
        };
        let json = serde_json::to_string(&record).unwrap();
        let deserialized: NamespaceRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.slug, record.slug);
        assert_eq!(deserialized.description, record.description);
    }
}
