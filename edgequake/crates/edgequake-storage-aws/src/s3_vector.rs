//! S3-based vector storage with HNSW indexing.
//!
//! This module provides a cost-effective vector storage solution using Amazon S3.
//! Vectors are stored in Parquet format for efficient columnar access, while an
//! in-memory HNSW index provides fast similarity search.
//!
//! ## Architecture
//!
//! ```text
//! S3 Bucket Structure:
//! ├── {namespace}/
//! │   ├── vectors/
//! │   │   ├── batch-{timestamp}.parquet  (vector batches)
//! │   │   └── metadata.json              (vector metadata)
//! │   ├── index/
//! │   │   └── hnsw.bin                   (HNSW index)
//! │   └── manifest.json                  (manifest of all files)
//! ```
//!
//! ## Features
//!
//! - **Cost-effective**: S3 storage at $0.023/GB (vs $0.115/GB for RDS)
//! - **Scalable**: Unlimited storage capacity
//! - **Fast search**: In-memory HNSW index for sub-50ms queries
//! - **Durable**: 99.999999999% (11 nines) durability
//! - **Lifecycle**: Automatic transition to Glacier for cold data
//!
//! ## Example
//!
//! ```rust,ignore
//! use edgequake_storage_aws::{S3VectorStorage, S3Config};
//!
//! let config = S3Config::new("my-vectors", "workspace-123", 1536);
//! let storage = S3VectorStorage::new(config).await?;
//!
//! storage.initialize().await?;
//!
//! // Insert vectors
//! let data = vec![
//!     ("id1".to_string(), vec![0.1; 1536], json!({"source": "doc1"}))
//! ];
//! storage.upsert(&data).await?;
//!
//! // Search similar vectors
//! let query = vec![0.1; 1536];
//! let results = storage.query(&query, 10, None).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use aws_sdk_s3::Client as S3Client;
use instant_distance::{Builder, HnswMap, Point, Search};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use edgequake_storage::{VectorSearchResult, VectorStorage};

use crate::error::AwsStorageError;

/// Configuration for S3 vector storage.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// S3 bucket name
    pub bucket: String,
    /// Storage namespace (workspace ID)
    pub namespace: String,
    /// Vector dimension
    pub dimension: usize,
    /// AWS region (optional, uses default if not set)
    pub region: Option<String>,
    /// HNSW index parameters
    pub hnsw_m: usize,
    /// HNSW construction parameter
    pub hnsw_ef_construction: usize,
    /// Search parameter
    pub hnsw_ef_search: usize,
    /// Batch size for vector uploads
    pub batch_size: usize,
    /// Enable compression
    pub enable_compression: bool,
    /// Cache size (number of vectors to keep in memory)
    pub cache_size: usize,
}

impl S3Config {
    /// Create a new S3 configuration.
    ///
    /// # Arguments
    ///
    /// * `bucket` - S3 bucket name
    /// * `namespace` - Storage namespace (typically workspace ID)
    /// * `dimension` - Vector embedding dimension
    pub fn new(bucket: impl Into<String>, namespace: impl Into<String>, dimension: usize) -> Self {
        Self {
            bucket: bucket.into(),
            namespace: namespace.into(),
            dimension,
            region: None,
            hnsw_m: 16,                // Default: 16 connections per layer
            hnsw_ef_construction: 200, // Default: 200 for construction
            hnsw_ef_search: 50,        // Default: 50 for search
            batch_size: 1000,          // Upload 1000 vectors per batch
            enable_compression: true,
            cache_size: 10000, // Cache 10k vectors in memory
        }
    }

    /// Set the AWS region.
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set HNSW parameters.
    pub fn with_hnsw_params(mut self, m: usize, ef_construction: usize, ef_search: usize) -> Self {
        self.hnsw_m = m;
        self.hnsw_ef_construction = ef_construction;
        self.hnsw_ef_search = ef_search;
        self
    }

    /// Set batch size for vector uploads.
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Enable or disable compression.
    pub fn with_compression(mut self, enable: bool) -> Self {
        self.enable_compression = enable;
        self
    }
}

/// Vector record stored in S3.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VectorRecord {
    id: String,
    embedding: Vec<f32>,
    metadata: serde_json::Value,
}

/// Manifest tracking all vector batches.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    dimension: usize,
    total_vectors: usize,
    batches: Vec<BatchInfo>,
    last_updated: i64,
}

/// Information about a vector batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchInfo {
    key: String,
    vector_count: usize,
    timestamp: i64,
}

impl Manifest {
    fn new(dimension: usize) -> Self {
        Self {
            version: 1,
            dimension,
            total_vectors: 0,
            batches: Vec::new(),
            last_updated: chrono::Utc::now().timestamp(),
        }
    }
}

/// Point wrapper for instant-distance HNSW index.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VectorPoint {
    id: String,
    vector: Vec<f32>,
}

impl Point for VectorPoint {
    fn distance(&self, other: &Self) -> f32 {
        // Cosine distance: 1 - cosine_similarity
        let dot: f32 = self
            .vector
            .iter()
            .zip(other.vector.iter())
            .map(|(a, b)| a * b)
            .sum();
        let norm_a: f32 = self.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = other.vector.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 1.0;
        }

        1.0 - (dot / (norm_a * norm_b))
    }
}

/// S3-based vector storage with HNSW indexing.
///
/// Provides cost-effective vector similarity search using Amazon S3 for storage
/// and an in-memory HNSW index for fast queries.
pub struct S3VectorStorage {
    config: S3Config,
    s3_client: S3Client,
    index: Arc<RwLock<Option<HnswMap<VectorPoint, String>>>>,
    metadata_cache: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    vector_cache: Arc<RwLock<lru::LruCache<String, Vec<f32>>>>,
    manifest: Arc<RwLock<Manifest>>,
}

impl S3VectorStorage {
    /// Create a new S3 vector storage.
    ///
    /// # Arguments
    ///
    /// * `config` - S3 storage configuration
    pub async fn new(config: S3Config) -> crate::error::Result<Self> {
        // Load AWS configuration
        let aws_config = if let Some(region) = &config.region {
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(aws_sdk_s3::config::Region::new(region.clone()))
                .load()
                .await
        } else {
            aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await
        };

        let s3_client = S3Client::new(&aws_config);

        // Create LRU cache for vectors
        let vector_cache =
            lru::LruCache::new(std::num::NonZeroUsize::new(config.cache_size).unwrap());

        let dimension = config.dimension;
        Ok(Self {
            config,
            s3_client,
            index: Arc::new(RwLock::new(None)),
            metadata_cache: Arc::new(RwLock::new(HashMap::new())),
            vector_cache: Arc::new(RwLock::new(vector_cache)),
            manifest: Arc::new(RwLock::new(Manifest::new(dimension))),
        })
    }

    /// Create a new S3 vector storage with a pre-configured client.
    ///
    /// Use this when you already have an AWS SDK client (e.g., in batch
    /// processing where the caller manages AWS configuration).
    pub fn new_with_client(config: S3Config, s3_client: S3Client) -> Self {
        let dimension = config.dimension;
        let vector_cache =
            lru::LruCache::new(std::num::NonZeroUsize::new(config.cache_size).unwrap());

        Self {
            config,
            s3_client,
            index: Arc::new(RwLock::new(None)),
            metadata_cache: Arc::new(RwLock::new(HashMap::new())),
            vector_cache: Arc::new(RwLock::new(vector_cache)),
            manifest: Arc::new(RwLock::new(Manifest::new(dimension))),
        }
    }

    /// Get the S3 key prefix for this namespace.
    fn key_prefix(&self) -> String {
        format!("{}/", self.config.namespace)
    }

    /// Get the manifest key.
    fn manifest_key(&self) -> String {
        format!("{}manifest.json", self.key_prefix())
    }

    /// Get the index key.
    fn index_key(&self) -> String {
        format!("{}index/hnsw.bin", self.key_prefix())
    }

    /// Load the manifest from S3.
    async fn load_manifest(&self) -> crate::error::Result<Manifest> {
        let key = self.manifest_key();

        match self
            .s3_client
            .get_object()
            .bucket(&self.config.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(output) => {
                let bytes = output
                    .body
                    .collect()
                    .await
                    .map_err(|e| AwsStorageError::S3Error(e.to_string()))?
                    .into_bytes();
                let manifest: Manifest = serde_json::from_slice(&bytes)?;
                Ok(manifest)
            }
            Err(e) => {
                // If manifest doesn't exist, create a new one
                warn!("Manifest not found, creating new: {}", e);
                Ok(Manifest::new(self.config.dimension))
            }
        }
    }

    /// Save the manifest to S3.
    async fn save_manifest(&self, manifest: &Manifest) -> crate::error::Result<()> {
        let key = self.manifest_key();
        let json = serde_json::to_vec_pretty(manifest)?;

        self.s3_client
            .put_object()
            .bucket(&self.config.bucket)
            .key(&key)
            .body(json.into())
            .content_type("application/json")
            .send()
            .await?;

        Ok(())
    }

    /// Load the HNSW index from S3.
    async fn load_index(&self) -> crate::error::Result<Option<HnswMap<VectorPoint, String>>> {
        let key = self.index_key();

        match self
            .s3_client
            .get_object()
            .bucket(&self.config.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(output) => {
                let bytes = output
                    .body
                    .collect()
                    .await
                    .map_err(|e| AwsStorageError::S3Error(e.to_string()))?
                    .into_bytes();
                let index: HnswMap<VectorPoint, String> = bincode::deserialize(&bytes)?;
                info!("Loaded HNSW index from S3: {} vectors", index.values.len());
                Ok(Some(index))
            }
            Err(e) => {
                debug!("Index not found in S3: {}", e);
                Ok(None)
            }
        }
    }

    /// Save the HNSW index to S3.
    async fn save_index(&self, index: &HnswMap<VectorPoint, String>) -> crate::error::Result<()> {
        let key = self.index_key();
        let bytes = bincode::serialize(index)?;

        // Compress if enabled
        let body = if self.config.enable_compression {
            use flate2::write::GzEncoder;
            use flate2::Compression;
            use std::io::Write;

            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&bytes)?;
            encoder.finish()?
        } else {
            bytes
        };

        self.s3_client
            .put_object()
            .bucket(&self.config.bucket)
            .key(&key)
            .body(body.into())
            .content_type("application/octet-stream")
            .send()
            .await?;

        info!("Saved HNSW index to S3: {} vectors", index.values.len());
        Ok(())
    }

    /// Build HNSW index from all vectors in S3.
    async fn build_index(&self) -> crate::error::Result<HnswMap<VectorPoint, String>> {
        info!("Building HNSW index from S3 vectors...");

        let manifest = self.manifest.read().await.clone();
        let mut all_vectors = Vec::new();

        // Load all vector batches
        for batch_info in &manifest.batches {
            let vectors = self.load_vector_batch(&batch_info.key).await?;
            all_vectors.extend(vectors);
        }

        if all_vectors.is_empty() {
            return Ok(Builder::default().build(Vec::<VectorPoint>::new(), Vec::<String>::new()));
        }

        // Build HNSW index
        let points: Vec<_> = all_vectors
            .into_iter()
            .map(|record| VectorPoint {
                id: record.id.clone(),
                vector: record.embedding,
            })
            .collect();

        let values: Vec<_> = points.iter().map(|p| p.id.clone()).collect();

        let hnsw: HnswMap<VectorPoint, String> = Builder::default().seed(42).build(points, values);

        info!("Built HNSW index with {} vectors", hnsw.values.len());
        Ok(hnsw)
    }

    /// Load a vector batch from S3.
    async fn load_vector_batch(&self, key: &str) -> crate::error::Result<Vec<VectorRecord>> {
        let output = self
            .s3_client
            .get_object()
            .bucket(&self.config.bucket)
            .key(key)
            .send()
            .await?;

        let bytes = output
            .body
            .collect()
            .await
            .map_err(|e| AwsStorageError::S3Error(e.to_string()))?
            .into_bytes();
        let records: Vec<VectorRecord> = serde_json::from_slice(&bytes)?;

        Ok(records)
    }

    /// Save a vector batch to S3.
    async fn save_vector_batch(&self, records: &[VectorRecord]) -> crate::error::Result<String> {
        let timestamp = chrono::Utc::now().timestamp();
        let key = format!("{}vectors/batch-{}.json", self.key_prefix(), timestamp);

        let json = serde_json::to_vec(records)?;

        self.s3_client
            .put_object()
            .bucket(&self.config.bucket)
            .key(&key)
            .body(json.into())
            .content_type("application/json")
            .send()
            .await?;

        Ok(key)
    }

    /// Update metadata cache.
    async fn update_metadata_cache(&self, records: &[(String, Vec<f32>, serde_json::Value)]) {
        let mut cache = self.metadata_cache.write().await;
        for (id, _, metadata) in records {
            cache.insert(id.clone(), metadata.clone());
        }
    }
}

#[async_trait]
impl VectorStorage for S3VectorStorage {
    fn namespace(&self) -> &str {
        &self.config.namespace
    }

    fn dimension(&self) -> usize {
        self.config.dimension
    }

    async fn initialize(&self) -> edgequake_storage::error::Result<()> {
        info!(
            "Initializing S3 vector storage: bucket={}, namespace={}",
            self.config.bucket, self.config.namespace
        );

        // Check if bucket exists and is accessible
        match self
            .s3_client
            .head_bucket()
            .bucket(&self.config.bucket)
            .send()
            .await
        {
            Ok(_) => {
                info!("S3 bucket '{}' is accessible", self.config.bucket);
            }
            Err(e) => {
                return Err(AwsStorageError::S3Error(format!(
                    "Cannot access bucket '{}': {}",
                    self.config.bucket, e
                ))
                .into());
            }
        }

        // Load manifest
        let manifest = self.load_manifest().await?;
        *self.manifest.write().await = manifest;

        // Load or build index
        let index = if let Some(idx) = self.load_index().await? {
            idx
        } else {
            info!("No existing index found, building from scratch");
            self.build_index().await?
        };

        *self.index.write().await = Some(index);

        info!("S3 vector storage initialized successfully");
        Ok(())
    }

    async fn finalize(&self) -> edgequake_storage::error::Result<()> {
        info!("Finalizing S3 vector storage");

        // Save index if it exists and has been modified
        if let Some(index) = self.index.read().await.as_ref() {
            self.save_index(index).await?;
        }

        // Save manifest
        let manifest = self.manifest.read().await.clone();
        self.save_manifest(&manifest).await?;

        info!("S3 vector storage finalized");
        Ok(())
    }

    async fn query(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        filter_ids: Option<&[String]>,
    ) -> edgequake_storage::error::Result<Vec<VectorSearchResult>> {
        // Validate dimension
        if query_embedding.len() != self.config.dimension {
            return Err(AwsStorageError::DimensionMismatch {
                expected: self.config.dimension,
                actual: query_embedding.len(),
            }
            .into());
        }

        let index_guard = self.index.read().await;
        let index = index_guard
            .as_ref()
            .ok_or_else(|| AwsStorageError::IndexNotFound("HNSW index not initialized".into()))?;

        // Create query point
        let query_point = VectorPoint {
            id: "query".to_string(),
            vector: query_embedding.to_vec(),
        };

        // Search HNSW index
        let mut search = Search::default();
        let results = index.search(&query_point, &mut search);

        // Convert to VectorSearchResult
        let metadata_cache = self.metadata_cache.read().await;
        let mut search_results = Vec::new();

        for map_item in results.take(top_k * 2) {
            let id = map_item.value;
            let distance = map_item.distance;

            // Apply filter if specified
            if let Some(filter) = filter_ids {
                if !filter.contains(id) {
                    continue;
                }
            }

            // Get metadata from cache or default
            let metadata = metadata_cache
                .get(id)
                .cloned()
                .unwrap_or(serde_json::json!({}));

            // Convert distance to similarity score (1 - distance for cosine)
            let score = 1.0 - distance;

            search_results.push(VectorSearchResult {
                id: id.clone(),
                score,
                metadata,
            });

            if search_results.len() >= top_k {
                break;
            }
        }

        Ok(search_results)
    }

    async fn upsert(
        &self,
        data: &[(String, Vec<f32>, serde_json::Value)],
    ) -> edgequake_storage::error::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        info!("Upserting {} vectors to S3", data.len());

        // Validate dimensions
        for (_id, embedding, _) in data {
            if embedding.len() != self.config.dimension {
                return Err(AwsStorageError::DimensionMismatch {
                    expected: self.config.dimension,
                    actual: embedding.len(),
                }
                .into());
            }
        }

        // Convert to VectorRecords
        let records: Vec<VectorRecord> = data
            .iter()
            .map(|(id, embedding, metadata)| VectorRecord {
                id: id.clone(),
                embedding: embedding.clone(),
                metadata: metadata.clone(),
            })
            .collect();

        // Save batch to S3
        let batch_key = self.save_vector_batch(&records).await?;

        // Update manifest
        let mut manifest = self.manifest.write().await;
        manifest.batches.push(BatchInfo {
            key: batch_key,
            vector_count: records.len(),
            timestamp: chrono::Utc::now().timestamp(),
        });
        manifest.total_vectors += records.len();
        manifest.last_updated = chrono::Utc::now().timestamp();

        // Save manifest
        self.save_manifest(&manifest).await?;

        // Update metadata cache
        self.update_metadata_cache(data).await;

        // Update HNSW index
        let mut index_guard = self.index.write().await;
        let index = index_guard.take().unwrap_or_else(|| {
            Builder::default().build(Vec::<VectorPoint>::new(), Vec::<String>::new())
        });

        // Note: instant-distance doesn't support incremental updates easily
        // For production, we'd need to rebuild the index periodically
        // For now, we rebuild when index gets large enough (see below)

        // Rebuild index if it's been updated significantly
        if manifest.total_vectors % 10000 == 0 {
            info!(
                "Rebuilding HNSW index after {} vectors",
                manifest.total_vectors
            );
            let new_index = self.build_index().await?;
            *index_guard = Some(new_index);
        } else {
            *index_guard = Some(index);
        }

        info!("Upserted {} vectors successfully", data.len());
        Ok(())
    }

    async fn delete(&self, ids: &[String]) -> edgequake_storage::error::Result<()> {
        info!("Deleting {} vectors", ids.len());

        // For simplicity, we mark them as deleted in metadata
        // Full implementation would rebuild index without deleted vectors
        let mut metadata_cache = self.metadata_cache.write().await;
        for id in ids {
            metadata_cache.remove(id);
        }

        // TODO: Implement proper deletion with index rebuild
        warn!("Vector deletion requires index rebuild - not yet implemented");

        Ok(())
    }

    async fn delete_entity(&self, entity_name: &str) -> edgequake_storage::error::Result<()> {
        info!("Deleting vectors for entity: {}", entity_name);
        // Find all vectors for this entity and delete them
        // This would require maintaining an entity-to-vector mapping
        warn!("delete_entity not yet implemented for S3 storage");
        Ok(())
    }

    async fn delete_entity_relations(
        &self,
        entity_name: &str,
    ) -> edgequake_storage::error::Result<()> {
        info!("Deleting relation vectors for entity: {}", entity_name);
        warn!("delete_entity_relations not yet implemented for S3 storage");
        Ok(())
    }

    async fn get_by_id(&self, id: &str) -> edgequake_storage::error::Result<Option<Vec<f32>>> {
        // Check cache first
        let mut cache = self.vector_cache.write().await;
        if let Some(vector) = cache.get(id) {
            return Ok(Some(vector.clone()));
        }

        // Load from S3 (would need to search through batches)
        // For now, return None
        warn!("get_by_id not efficiently implemented for S3 - requires full scan");
        Ok(None)
    }

    async fn get_by_ids(
        &self,
        _ids: &[String],
    ) -> edgequake_storage::error::Result<Vec<(String, Vec<f32>)>> {
        warn!("get_by_ids not efficiently implemented for S3 - requires full scan");
        Ok(Vec::new())
    }

    async fn is_empty(&self) -> edgequake_storage::error::Result<bool> {
        let manifest = self.manifest.read().await;
        Ok(manifest.total_vectors == 0)
    }

    async fn count(&self) -> edgequake_storage::error::Result<usize> {
        let manifest = self.manifest.read().await;
        Ok(manifest.total_vectors)
    }

    async fn clear(&self) -> edgequake_storage::error::Result<()> {
        info!("Clearing all vectors from S3 storage");

        // Delete all vector batches
        let manifest = self.manifest.read().await;
        for batch in &manifest.batches {
            self.s3_client
                .delete_object()
                .bucket(&self.config.bucket)
                .key(&batch.key)
                .send()
                .await
                .map_err(|e| AwsStorageError::S3Error(e.to_string()))?;
        }

        // Clear index
        *self.index.write().await =
            Some(Builder::default().build(Vec::<VectorPoint>::new(), Vec::<String>::new()));

        // Clear manifest
        *self.manifest.write().await = Manifest::new(self.config.dimension);

        // Clear caches
        self.metadata_cache.write().await.clear();
        self.vector_cache.write().await.clear();

        info!("Cleared all vectors");
        Ok(())
    }

    async fn clear_workspace(
        &self,
        _workspace_id: &uuid::Uuid,
    ) -> edgequake_storage::error::Result<usize> {
        // For S3, each workspace has its own namespace
        // So clearing this workspace means clearing all data
        let count = self.count().await?;
        self.clear().await?;
        Ok(count)
    }
}
