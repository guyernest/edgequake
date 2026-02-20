//! AWS-backed namespace storage factory.
//!
//! Creates namespace-scoped storage backends (Neptune, S3 Vectors, DynamoDB)
//! for per-namespace data isolation. This module lives in the binary crate
//! because it depends on concrete AWS types from `edgequake-storage-aws`,
//! while the API crate only depends on the abstract `NamespaceStorageFactory` trait.

use std::sync::Arc;

use edgequake_api::state::{NamespaceStorageFactory, NamespaceStorageSet};
use edgequake_storage::traits::VectorStorage;
use edgequake_storage_aws::{
    DynamoKVConfig, DynamoKVStorage, NeptuneConfig, NeptuneGraphStorage, S3VectorsConfig,
    S3VectorsStorage,
};
use tracing::info;

/// Factory for creating namespace-scoped AWS storage instances.
///
/// Holds the base configuration for each storage backend. When `create_storage`
/// is called with a namespace slug, it creates new storage instances configured
/// with namespace-specific isolation:
///
/// - **NeptuneGraphStorage**: label-prefix set to namespace slug (e.g., `epstein-files:Entity`)
/// - **S3VectorsStorage**: index name set to `{namespace}-embeddings`
/// - **DynamoKVStorage**: namespace composite key set to namespace slug
pub struct AwsNamespaceStorageFactory {
    /// Neptune cluster endpoint (e.g., "host:port").
    neptune_endpoint: String,
    /// S3 Vectors bucket name (shared across all namespaces).
    vector_bucket_name: String,
    /// Default embedding dimension.
    embedding_dimension: usize,
    /// DynamoDB KV storage table name.
    dynamodb_table_name: String,
}

impl AwsNamespaceStorageFactory {
    /// Create a new factory with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `neptune_endpoint` - Neptune cluster endpoint (host:port)
    /// * `vector_bucket_name` - S3 Vectors bucket name (shared bucket)
    /// * `embedding_dimension` - Default embedding dimension (typically 1536)
    /// * `dynamodb_table_name` - DynamoDB table for KV storage
    pub fn new(
        neptune_endpoint: String,
        vector_bucket_name: String,
        embedding_dimension: usize,
        dynamodb_table_name: String,
    ) -> Self {
        Self {
            neptune_endpoint,
            vector_bucket_name,
            embedding_dimension,
            dynamodb_table_name,
        }
    }
}

#[async_trait::async_trait]
impl NamespaceStorageFactory for AwsNamespaceStorageFactory {
    async fn create_storage(
        &self,
        namespace: &str,
    ) -> Result<NamespaceStorageSet, Box<dyn std::error::Error + Send + Sync>> {
        info!(
            namespace = %namespace,
            neptune_endpoint = %self.neptune_endpoint,
            vector_bucket = %self.vector_bucket_name,
            dynamodb_table = %self.dynamodb_table_name,
            "Creating namespace-scoped storage backends"
        );

        // 1. Create NeptuneGraphStorage with namespace-specific label-prefix
        let neptune_config = NeptuneConfig::new(&self.neptune_endpoint)
            .with_namespace(namespace);
        let neptune_storage = NeptuneGraphStorage::new(neptune_config).await?;

        // 2. Create S3VectorsStorage with namespace-scoped index
        let vector_config = S3VectorsConfig {
            vector_bucket_name: self.vector_bucket_name.clone(),
            index_name: format!("{}-embeddings", namespace),
            dimension: self.embedding_dimension,
            namespace: namespace.to_string(),
        };
        let vector_storage = S3VectorsStorage::new(vector_config).await?;
        // Initialize creates the index if it doesn't exist (idempotent)
        vector_storage.initialize().await.map_err(|e| {
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;

        // 3. Create DynamoKVStorage with namespace composite key
        let kv_config = DynamoKVConfig::new(&self.dynamodb_table_name, namespace);
        let kv_storage = DynamoKVStorage::new(kv_config).await?;

        info!(
            namespace = %namespace,
            "Namespace storage backends created successfully"
        );

        Ok(NamespaceStorageSet {
            graph_storage: Arc::new(neptune_storage),
            vector_storage: Arc::new(vector_storage),
            kv_storage: Arc::new(kv_storage),
        })
    }
}
