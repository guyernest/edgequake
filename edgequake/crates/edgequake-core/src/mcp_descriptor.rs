//! MCP namespace descriptor types.
//!
//! The [`McpDescriptor`] is a self-contained JSON document that edgequake
//! generates per namespace. It contains everything an MCP server needs to
//! operate within a single namespace: storage endpoints, authentication
//! config, pipeline settings, and static tool definitions.
//!
//! pmcp.run consumes this descriptor to provision a namespace-scoped MCP
//! server process. The descriptor is stored in DynamoDB alongside the
//! namespace record and auto-generated on namespace creation.

use serde::{Deserialize, Serialize};

/// Top-level MCP namespace descriptor.
///
/// This is the self-contained document exported for pmcp.run to provision
/// an MCP server scoped to a single namespace. It includes all storage
/// endpoints, authentication configuration, pipeline settings, and tool
/// definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpDescriptor {
    /// Schema version for forward compatibility (always "1.0" for now).
    pub schema_version: String,
    /// Namespace identification.
    pub namespace: McpNamespaceInfo,
    /// Storage backend configuration (Neptune, S3 Vectors, DynamoDB).
    pub storage: McpStorageConfig,
    /// Authentication configuration for AWS access.
    pub auth: McpAuthConfig,
    /// Snapshot of the namespace's pipeline configuration.
    pub pipeline_config: McpPipelineConfig,
    /// Static MCP tool definitions (same for every namespace).
    pub tools: Vec<McpToolDefinition>,
    /// Timestamp when this descriptor was generated (epoch milliseconds).
    pub generated_at: i64,
}

/// Namespace identification within the descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpNamespaceInfo {
    /// The namespace slug (DNS-safe identifier).
    pub slug: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Storage backend configuration containing sub-configs for each backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpStorageConfig {
    /// Neptune graph database configuration.
    pub neptune: McpNeptuneConfig,
    /// S3 Vectors configuration for embeddings.
    pub s3_vectors: McpS3VectorsConfig,
    /// DynamoDB key-value storage configuration.
    pub dynamodb: McpDynamoDbConfig,
    /// BM25 keyword search configuration (Athena + Iceberg).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25: Option<McpBm25Config>,
}

/// Neptune graph database configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpNeptuneConfig {
    /// Neptune cluster endpoint (e.g., "host:port").
    pub endpoint: String,
    /// Label prefix for namespace isolation (e.g., "epstein-files").
    pub label_prefix: String,
}

/// S3 Vectors configuration for namespace-scoped embeddings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpS3VectorsConfig {
    /// S3 Vectors bucket name.
    pub bucket_name: String,
    /// Namespace-scoped index name (e.g., "epstein-files-embeddings").
    pub index_name: String,
}

/// DynamoDB key-value storage configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpDynamoDbConfig {
    /// DynamoDB table name.
    pub table_name: String,
    /// Namespace key for data isolation.
    pub namespace_key: String,
}

/// BM25 keyword search configuration using Athena with Iceberg tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpBm25Config {
    /// Athena/Glue database name (e.g., "edgequake_bm25").
    pub database: String,
    /// S3 bucket for Iceberg table data and query results.
    pub s3_bucket: String,
    /// Athena workgroup (must be v3 for Iceberg support).
    pub workgroup: String,
}

/// Authentication configuration for AWS access via IAM role assumption.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpAuthConfig {
    /// IAM role ARN for the MCP server to assume.
    pub role_arn: String,
    /// External ID for cross-account assume-role security.
    pub external_id: String,
    /// AWS region.
    pub region: String,
}

/// Snapshot of the namespace's pipeline configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpPipelineConfig {
    /// Embedding model identifier (e.g., "text-embedding-3-small").
    pub embedding_model: String,
    /// Embedding vector dimension.
    pub embedding_dimension: usize,
    /// LLM model identifier (e.g., "gpt-4o-mini").
    pub llm_model: String,
}

/// Static MCP tool definition.
///
/// These are the same for every namespace — they describe the tools
/// the MCP server exposes to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolDefinition {
    /// Tool name (e.g., "query_knowledge_base").
    pub name: String,
    /// Human-readable tool description.
    pub description: String,
    /// Optional list of modes the tool supports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<Vec<String>>,
}

/// Infrastructure configuration for descriptor generation.
///
/// Captures deployment infrastructure endpoints needed to build an
/// [`McpDescriptor`]. Constructed from environment variables in the
/// binary crate and passed to the namespace registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfrastructureConfig {
    /// Neptune cluster endpoint (e.g., "host:port").
    pub neptune_endpoint: String,
    /// S3 Vectors bucket name.
    pub vector_bucket_name: String,
    /// DynamoDB table name for KV storage.
    pub dynamodb_table_name: String,
    /// AWS account ID.
    pub account_id: String,
    /// AWS region.
    pub region: String,
    /// Deployment environment (e.g., "dev", "staging", "prod").
    pub environment: String,
    /// External ID suffix for cross-account security.
    pub external_id_suffix: String,
    /// Athena BM25 database name (optional — enables BM25 in descriptor).
    pub athena_bm25_database: Option<String>,
    /// S3 bucket for BM25 Iceberg data (required when database is set).
    pub bm25_s3_bucket: Option<String>,
    /// Athena workgroup (required when database is set).
    pub athena_workgroup: Option<String>,
}

/// Returns the static list of MCP tool definitions.
///
/// These tools match the existing MCP server capabilities and are the
/// same for every knowledge base namespace.
pub fn default_mcp_tools() -> Vec<McpToolDefinition> {
    vec![
        McpToolDefinition {
            name: "query_knowledge_base".to_string(),
            description: "Query the knowledge base using natural language".to_string(),
            modes: Some(vec![
                "naive".to_string(),
                "local".to_string(),
                "global".to_string(),
                "hybrid".to_string(),
                "mix".to_string(),
            ]),
        },
        McpToolDefinition {
            name: "search_entities".to_string(),
            description: "Search for entities by name or description".to_string(),
            modes: None,
        },
        McpToolDefinition {
            name: "get_entity_neighborhood".to_string(),
            description: "Get the knowledge graph neighborhood around an entity".to_string(),
            modes: None,
        },
        McpToolDefinition {
            name: "get_document".to_string(),
            description: "Retrieve a source document by ID".to_string(),
            modes: None,
        },
        McpToolDefinition {
            name: "get_graph_stats".to_string(),
            description: "Get statistics about the knowledge graph".to_string(),
            modes: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_descriptor() -> McpDescriptor {
        McpDescriptor {
            schema_version: "1.0".to_string(),
            namespace: McpNamespaceInfo {
                slug: "epstein-files".to_string(),
                description: Some("Epstein investigation files".to_string()),
            },
            storage: McpStorageConfig {
                neptune: McpNeptuneConfig {
                    endpoint: "neptune-cluster.us-east-1.neptune.amazonaws.com:8182".to_string(),
                    label_prefix: "epstein-files".to_string(),
                },
                s3_vectors: McpS3VectorsConfig {
                    bucket_name: "edgequake-vectors".to_string(),
                    index_name: "epstein-files-embeddings".to_string(),
                },
                dynamodb: McpDynamoDbConfig {
                    table_name: "edgequake-kv".to_string(),
                    namespace_key: "epstein-files".to_string(),
                },
                bm25: None,
            },
            auth: McpAuthConfig {
                role_arn: "arn:aws:iam::123456789012:role/edgequake-mcp-epstein-files-dev"
                    .to_string(),
                external_id: "eq-epstein-files-abc123".to_string(),
                region: "us-east-1".to_string(),
            },
            pipeline_config: McpPipelineConfig {
                embedding_model: "text-embedding-3-small".to_string(),
                embedding_dimension: 1536,
                llm_model: "gpt-4o-mini".to_string(),
            },
            tools: default_mcp_tools(),
            generated_at: 1700000000000,
        }
    }

    #[test]
    fn test_descriptor_serde_roundtrip() {
        let descriptor = sample_descriptor();
        let json = serde_json::to_string_pretty(&descriptor).unwrap();
        let deserialized: McpDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, descriptor);
    }

    #[test]
    fn test_default_mcp_tools_count() {
        assert_eq!(default_mcp_tools().len(), 5);
    }

    #[test]
    fn test_descriptor_schema_version() {
        let descriptor = sample_descriptor();
        assert_eq!(descriptor.schema_version, "1.0");
    }

    #[test]
    fn test_tool_modes_serialization() {
        // Tools with modes should include them
        let tools = default_mcp_tools();
        let query_tool = &tools[0];
        assert!(query_tool.modes.is_some());
        assert_eq!(query_tool.modes.as_ref().unwrap().len(), 5);

        // Tools without modes should skip the field
        let search_tool = &tools[1];
        assert!(search_tool.modes.is_none());
        let json = serde_json::to_string(search_tool).unwrap();
        assert!(!json.contains("modes"));
    }

    #[test]
    fn test_namespace_info_description_skip_none() {
        let info = McpNamespaceInfo {
            slug: "test".to_string(),
            description: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("description"));
    }
}
