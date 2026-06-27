//! # EdgeQuake AWS Storage
//!
//! AWS cloud storage adapters for the EdgeQuake RAG system.
//!
//! This crate provides cost-effective, scalable storage implementations using:
//! - Amazon S3 for vector embeddings (with HNSW indexing)
//! - Amazon Neptune for graph storage
//! - Amazon DynamoDB for key-value storage
//! - Amazon Athena for SQL queries over S3
//!
//! ## Cost Benefits
//!
//! - **95% cost reduction** vs PostgreSQL RDS for typical workloads
//! - **Pay-per-use pricing** - no idle costs
//! - **Auto-scaling** - from zero to massive scale
//! - **S3 storage**: $0.023/GB vs $0.115/GB for RDS
//!
//! ## Security Features
//!
//! - IAM-based fine-grained access control
//! - Encryption at rest (S3, Neptune, DynamoDB)
//! - VPC isolation for Neptune
//! - CloudTrail audit logging
//! - Compliance ready (HIPAA, SOC 2, ISO 27001)
//!
//! ## Example
//!
//! ```rust,ignore
//! use edgequake_storage_aws::{S3VectorStorage, S3Config};
//! use edgequake_storage::VectorStorage;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let config = S3Config::new("my-bucket", "workspace-123", 1536);
//!     let storage = S3VectorStorage::new(config).await?;
//!
//!     storage.initialize().await?;
//!
//!     // Upsert vectors
//!     let data = vec![
//!         ("doc1".to_string(), vec![0.1; 1536], serde_json::json!({"title": "Document 1"}))
//!     ];
//!     storage.upsert(&data).await?;
//!
//!     // Query similar vectors
//!     let query = vec![0.1; 1536];
//!     let results = storage.query(&query, 10, None).await?;
//!
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod raw_docs;
pub mod s3_vector;

#[cfg(feature = "s3vectors")]
pub mod s3_vectors;

#[cfg(feature = "neptune")]
pub mod gremlin_helpers;

#[cfg(feature = "neptune")]
pub mod neptune_graph;

#[cfg(feature = "dynamodb")]
pub mod dynamodb_kv;

#[cfg(feature = "dynamodb")]
pub mod dynamodb_namespace;

#[cfg(feature = "dynamodb")]
pub mod dynamodb_workspace;

#[cfg(feature = "athena")]
pub mod athena_bm25;

#[cfg(feature = "athena")]
pub mod athena_sql;

#[cfg(feature = "athena")]
pub mod bm25_parquet;

// Re-export main types
pub use error::{AwsStorageError, Result};
pub use raw_docs::RawDocsStorage;
pub use s3_vector::{S3Config, S3VectorStorage};

#[cfg(feature = "s3vectors")]
pub use s3_vectors::{S3VectorsConfig, S3VectorsStorage};

#[cfg(feature = "neptune")]
pub use neptune_graph::{NamespaceMode, NeptuneConfig, NeptuneGraphStorage};

#[cfg(feature = "dynamodb")]
pub use dynamodb_kv::{DynamoKVConfig, DynamoKVStorage};

#[cfg(feature = "dynamodb")]
pub use dynamodb_namespace::{DynamoNamespaceConfig, DynamoNamespaceRegistry, NamespaceListItem};

#[cfg(feature = "dynamodb")]
pub use dynamodb_workspace::{DynamoWorkspaceConfig, DynamoWorkspaceService};

#[cfg(feature = "athena")]
pub use athena_bm25::{AthenaBm25Config, AthenaBm25Storage};

#[cfg(feature = "athena")]
pub use athena_sql::{AthenaConfig, AthenaQueryEngine, AthenaRow, QueryBuilder, QueryStats};

// Re-export AWS SDK types for convenience
pub use aws_config;
pub use aws_sdk_s3;

#[cfg(feature = "s3vectors")]
pub use aws_sdk_s3vectors;

#[cfg(feature = "dynamodb")]
pub use aws_sdk_dynamodb;

#[cfg(feature = "athena")]
pub use aws_sdk_athena;
