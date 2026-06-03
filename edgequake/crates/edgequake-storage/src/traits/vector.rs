//! Vector storage trait for similarity search.
//!
//! # Implements
//!
//! - **FEAT0201**: Vector Similarity Search
//!
//! # Enforces
//!
//! - **BR0201**: Namespace-based tenant isolation
//! - **BR0010**: Embedding dimension validated on insert
//!
//! # WHY: Separate Vector Storage
//!
//! Vector similarity search is specialized:
//! - Requires optimized index structures (HNSW, IVF)
//! - Benefits from GPU acceleration
//! - Different scaling characteristics than graph/KV
//!
//! Abstracting as a trait allows using:
//! - pgvector (PostgreSQL extension)
//! - Pinecone, Weaviate, Qdrant (managed services)
//! - In-memory brute-force (testing)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Vector similarity search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorSearchResult {
    /// Record identifier
    pub id: String,
    /// Similarity score (higher is more similar)
    pub score: f32,
    /// Associated metadata
    pub metadata: serde_json::Value,
}

/// Vector storage interface for similarity search.
///
/// Provides storage and retrieval of vector embeddings with
/// support for similarity search operations.
///
/// # Implementations
///
/// - `MemoryVectorStorage` - In-memory brute-force search (testing)
/// - `PgVectorStorage` - PostgreSQL with pgvector extension
/// - `SurrealDBVectorStorage` - SurrealDB native vector support
#[async_trait]
pub trait VectorStorage: Send + Sync {
    /// Get the storage namespace.
    fn namespace(&self) -> &str;

    /// Get the expected embedding dimension.
    fn dimension(&self) -> usize;

    /// Initialize the vector storage.
    ///
    /// Creates necessary indices and tables.
    async fn initialize(&self) -> Result<()>;

    /// Flush pending changes.
    async fn finalize(&self) -> Result<()>;

    /// Perform similarity search.
    ///
    /// # Arguments
    ///
    /// * `query_embedding` - The query vector
    /// * `top_k` - Maximum number of results to return
    /// * `filter_ids` - Optional list of IDs to restrict search to
    ///
    /// # Returns
    ///
    /// Vector of search results ordered by similarity (highest first).
    async fn query(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        filter_ids: Option<&[String]>,
    ) -> Result<Vec<VectorSearchResult>>;

    /// Insert or update vectors with metadata.
    ///
    /// # Arguments
    ///
    /// * `data` - Vector of (id, embedding, metadata) tuples
    async fn upsert(&self, data: &[(String, Vec<f32>, serde_json::Value)]) -> Result<()>;

    /// Delete vectors by IDs.
    async fn delete(&self, ids: &[String]) -> Result<()>;

    /// Delete all vectors associated with an entity.
    ///
    /// This is used when deleting an entity to clean up its embeddings.
    async fn delete_entity(&self, entity_name: &str) -> Result<()>;

    /// Delete all relationship vectors involving an entity.
    ///
    /// Used when cascading entity deletion.
    async fn delete_entity_relations(&self, entity_name: &str) -> Result<()>;

    /// Get a single vector by ID.
    async fn get_by_id(&self, id: &str) -> Result<Option<Vec<f32>>>;

    /// Get multiple vectors by IDs.
    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<(String, Vec<f32>)>>;

    /// Check if storage is empty.
    async fn is_empty(&self) -> Result<bool>;

    /// Get count of stored vectors.
    async fn count(&self) -> Result<usize>;

    /// Clear all vectors.
    async fn clear(&self) -> Result<()>;

    /// Perform similarity search filtered by vector type.
    ///
    /// Uses server-side metadata filtering when the backend supports it
    /// (e.g., S3 Vectors native `filter` parameter), otherwise falls back
    /// to client-side filtering after retrieval.
    ///
    /// # Why This Method Exists
    ///
    /// All vector types (chunks, entities, relationships) share a single index
    /// per namespace. Entity descriptions are semantically denser than chunk text,
    /// so they dominate cosine similarity rankings. A naive `query()` call with
    /// `top_k=100` can return ~26 entities + ~74 relationships and **zero chunks**.
    ///
    /// Server-side filtering guarantees the correct type fills the full `top_k`.
    ///
    /// # Arguments
    ///
    /// * `query_embedding` - The query vector
    /// * `top_k` - Maximum number of results to return
    /// * `vector_type` - Metadata type to filter for: `"chunk"`, `"entity"`, or `"relationship"`
    /// * `filter_ids` - Optional list of IDs to restrict search to
    ///
    /// # Default Implementation
    ///
    /// Calls `query()` with 3x oversampling, then filters client-side by metadata `type` field.
    /// Backends with native metadata filtering should override this for better recall.
    async fn query_by_type(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        vector_type: &str,
        filter_ids: Option<&[String]>,
    ) -> Result<Vec<VectorSearchResult>> {
        // Default: oversample to compensate for mixed types, then filter client-side.
        let oversample = if filter_ids.is_some() {
            // When filtering by IDs, the ID filter already narrows results
            top_k
        } else {
            (top_k * 3).min(100)
        };
        let results = self.query(query_embedding, oversample, filter_ids).await?;
        Ok(results
            .into_iter()
            .filter(|r| {
                r.metadata
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| t == vector_type)
                    .unwrap_or(false)
            })
            .take(top_k)
            .collect())
    }

    /// Clear vectors for a specific workspace.
    ///
    /// This is used when rebuilding embeddings for a single workspace
    /// without affecting other workspaces.
    ///
    /// # Arguments
    ///
    /// * `workspace_id` - The UUID of the workspace to clear vectors for
    ///
    /// # Returns
    ///
    /// Number of vectors deleted.
    ///
    /// # Default Implementation
    ///
    /// Returns 0 by default. Implementations should override this for
    /// workspace-scoped clearing.
    async fn clear_workspace(&self, workspace_id: &uuid::Uuid) -> Result<usize> {
        // Default implementation does nothing - clear() clears all
        // Implementations should override this for workspace-scoped clearing
        let _ = workspace_id;
        Ok(0)
    }
}
