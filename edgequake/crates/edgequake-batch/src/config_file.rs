//! TOML pipeline config file for `--config-file` flag.
//!
//! Provides an alternative to DynamoDB namespace config for CLI-only pipelines.
//! When `--config-file` is provided, DynamoDB config lookup is skipped entirely.
//!
//! ## Example
//!
//! ```toml
//! [pipeline]
//! llm_provider = "openai"
//! llm_model = "gpt-4o-mini"
//! embedding_provider = "openai"
//! embedding_model = "text-embedding-3-small"
//! embedding_dimension = 1536
//!
//! [chunking]
//! strategy = "token"
//! chunk_size = 1200
//! chunk_overlap = 100
//!
//! [schema]
//! entity_types = [
//!     { name = "PERSON", description = "Named individuals" },
//!     { name = "ORGANIZATION", description = "Companies and agencies" },
//! ]
//! relation_types = [
//!     { name = "employs", description = "Employment relationship" },
//! ]
//! ```

use serde::Deserialize;
use std::path::Path;

/// Top-level pipeline config file structure.
///
/// Mirrors the fields from `PipelineConfig` in edgequake-core, allowing
/// CLI users to specify pipeline configuration without DynamoDB.
#[derive(Debug, Deserialize)]
pub struct PipelineConfigFile {
    /// Pipeline settings (models, providers).
    pub pipeline: PipelineSection,

    /// Chunking settings (optional -- defaults used if absent).
    pub chunking: Option<ChunkingSection>,

    /// Schema definition (optional -- equivalent to an approved schema in DynamoDB).
    pub schema: Option<SchemaSection>,
}

/// Pipeline model and provider settings.
#[derive(Debug, Deserialize)]
pub struct PipelineSection {
    /// LLM provider name (e.g., "openai").
    pub llm_provider: Option<String>,

    /// LLM model identifier (e.g., "gpt-4o-mini").
    pub llm_model: Option<String>,

    /// Embedding provider name (e.g., "openai").
    pub embedding_provider: Option<String>,

    /// Embedding model identifier (e.g., "text-embedding-3-small").
    pub embedding_model: Option<String>,

    /// Embedding vector dimension.
    pub embedding_dimension: Option<u32>,
}

/// Chunking configuration.
#[derive(Debug, Deserialize)]
pub struct ChunkingSection {
    /// Chunking strategy (e.g., "token", "heading_boundary").
    pub strategy: Option<String>,

    /// Maximum chunk size in tokens.
    pub chunk_size: Option<usize>,

    /// Token overlap between consecutive chunks.
    pub chunk_overlap: Option<usize>,
}

/// Schema definition section -- equivalent to an approved schema in DynamoDB.
///
/// When present in the config file, this replaces the need for an approved
/// schema in DynamoDB. The entity and relation types are used to build
/// extraction prompts.
#[derive(Debug, Deserialize)]
pub struct SchemaSection {
    /// Entity types with names and descriptions.
    pub entity_types: Vec<SchemaEntityType>,

    /// Relation types with names, descriptions, and optional constraints.
    pub relation_types: Vec<SchemaRelationType>,
}

/// An entity type definition in the config file schema.
#[derive(Debug, Deserialize)]
pub struct SchemaEntityType {
    /// Entity type name in UPPER_SNAKE_CASE (e.g., "PERSON").
    pub name: String,

    /// Description of what this entity type represents.
    pub description: String,
}

/// A relation type definition in the config file schema.
#[derive(Debug, Deserialize)]
pub struct SchemaRelationType {
    /// Relation type name in lower_snake_case (e.g., "employs").
    pub name: String,

    /// Description of what this relation represents.
    pub description: String,

    /// Source entity type constraint (optional).
    pub source_type: Option<String>,

    /// Target entity type constraint (optional).
    pub target_type: Option<String>,
}

impl PipelineConfigFile {
    /// Load and parse a pipeline config TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!(
                "Failed to read pipeline config file '{}': {}",
                path.display(),
                e
            )
        })?;
        let config: Self = toml::from_str(&content).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse pipeline config file '{}': {}",
                path.display(),
                e
            )
        })?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[pipeline]
llm_provider = "openai"
llm_model = "gpt-4o-mini"
embedding_provider = "openai"
embedding_model = "text-embedding-3-small"
embedding_dimension = 1536

[chunking]
strategy = "token"
chunk_size = 1200
chunk_overlap = 100

[schema]
entity_types = [
    { name = "PERSON", description = "Named individuals" },
    { name = "ORGANIZATION", description = "Companies and agencies" },
]
relation_types = [
    { name = "employs", description = "Employment relationship", source_type = "ORGANIZATION", target_type = "PERSON" },
]
"#;
        let config: PipelineConfigFile = toml::from_str(toml_str).unwrap();
        assert_eq!(config.pipeline.llm_model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(config.pipeline.embedding_dimension, Some(1536));

        let schema = config.schema.unwrap();
        assert_eq!(schema.entity_types.len(), 2);
        assert_eq!(schema.relation_types.len(), 1);
        assert_eq!(schema.entity_types[0].name, "PERSON");
        assert_eq!(schema.relation_types[0].source_type.as_deref(), Some("ORGANIZATION"));
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[pipeline]
llm_model = "gpt-4o"
"#;
        let config: PipelineConfigFile = toml::from_str(toml_str).unwrap();
        assert_eq!(config.pipeline.llm_model.as_deref(), Some("gpt-4o"));
        assert!(config.pipeline.embedding_model.is_none());
        assert!(config.chunking.is_none());
        assert!(config.schema.is_none());
    }
}
