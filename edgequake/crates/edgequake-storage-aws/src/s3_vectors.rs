//! Amazon S3 Vectors storage backend.
//!
//! Uses the native S3 Vectors service (`aws-sdk-s3vectors`) for vector storage
//! with built-in similarity search. Replaces the custom HNSW + S3 JSON approach.
//!
//! ## Benefits
//!
//! - Native cosine similarity search via `query_vectors`
//! - No custom index management
//! - Automatic indexing and scaling
//! - Pay-per-request pricing

use async_trait::async_trait;
use aws_sdk_s3vectors::types::{DataType, DistanceMetric, PutInputVector, VectorData};
use aws_sdk_s3vectors::Client;
use aws_smithy_types::Document;
use edgequake_storage::traits::{VectorSearchResult, VectorStorage};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use crate::error::AwsStorageError;

/// Extract a descriptive error message from an AWS SDK error.
///
/// `SdkError::to_string()` often returns just "service error" which is useless
/// for debugging. This helper extracts the actual service error message.
fn s3v_err<E, R>(err: aws_smithy_runtime_api::client::result::SdkError<E, R>) -> AwsStorageError
where
    E: std::fmt::Display + std::fmt::Debug,
    R: std::fmt::Debug,
{
    let msg = match &err {
        aws_smithy_runtime_api::client::result::SdkError::ServiceError(ctx) => {
            let display = format!("{}", ctx.err());
            let debug = format!("{:?}", ctx.err());
            if display.contains("unhandled") || display == "service error" {
                debug
            } else {
                display
            }
        }
        other => format!("{:?}", other),
    };
    AwsStorageError::S3VectorsError(msg)
}

/// Maximum vectors per `put_vectors` / `delete_vectors` call.
const PUT_BATCH_SIZE: usize = 500;

/// Maximum keys per `get_vectors` call.
const GET_BATCH_SIZE: usize = 100;

/// Configuration for S3 Vectors storage.
///
/// ## Namespace Isolation Strategy
///
/// S3 Vectors does not have built-in namespace filtering. Isolation is achieved
/// via **one index per namespace** within a shared vector bucket:
///
/// - `vector_bucket_name` — shared across all namespaces (one bucket per deployment)
/// - `index_name` — namespace-scoped (convention: `{namespace}-embeddings`), one
///   index per namespace containing all vector types (chunks, entities, relationships)
/// - `namespace` — the namespace slug this storage instance belongs to
///
/// All vector types (chunks, entities, relationships) share the same namespace
/// index, distinguished by a `type` metadata field (`"chunk"`, `"entity"`,
/// `"relationship"`). This avoids multiplying indexes per namespace.
///
/// The `initialize()` method creates the index if it doesn't exist, so new
/// namespaces get their index auto-created on first use.
#[derive(Debug, Clone)]
pub struct S3VectorsConfig {
    /// Name of the S3 vector bucket (shared across all namespaces).
    pub vector_bucket_name: String,
    /// Name of the vector index within the bucket.
    ///
    /// Convention: `{namespace}-embeddings` (e.g., `epstein-files-embeddings`).
    /// One index per namespace, containing all vector types (chunks, entities,
    /// relationships) distinguished by the metadata `type` field.
    pub index_name: String,
    /// Expected embedding dimension.
    pub dimension: usize,
    /// The namespace slug this storage instance belongs to.
    ///
    /// Used by `VectorStorage::namespace()` to identify which namespace's data
    /// this instance manages. Must match the slug used in the index name.
    pub namespace: String,
}

/// S3 Vectors-backed VectorStorage implementation.
pub struct S3VectorsStorage {
    config: S3VectorsConfig,
    client: Client,
}

impl S3VectorsStorage {
    /// Create a new S3VectorsStorage, constructing its own AWS client.
    pub async fn new(config: S3VectorsConfig) -> crate::error::Result<Self> {
        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = Client::new(&aws_config);
        Ok(Self { config, client })
    }

    /// Create a new S3VectorsStorage with a pre-configured client.
    pub fn new_with_client(config: S3VectorsConfig, client: Client) -> Self {
        Self { config, client }
    }

    /// Convert a `serde_json::Value` into an `aws_smithy_types::Document`.
    fn json_to_document(value: &serde_json::Value) -> Document {
        match value {
            serde_json::Value::Null => Document::Null,
            serde_json::Value::Bool(b) => Document::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Document::Number(aws_smithy_types::Number::NegInt(i))
                } else if let Some(f) = n.as_f64() {
                    Document::Number(aws_smithy_types::Number::Float(f))
                } else {
                    Document::Null
                }
            }
            serde_json::Value::String(s) => Document::String(s.clone()),
            serde_json::Value::Array(arr) => {
                Document::Array(arr.iter().map(Self::json_to_document).collect())
            }
            serde_json::Value::Object(obj) => {
                let map: HashMap<String, Document> = obj
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::json_to_document(v)))
                    .collect();
                Document::Object(map)
            }
        }
    }

    /// Convert an `aws_smithy_types::Document` back into a `serde_json::Value`.
    fn document_to_json(doc: &Document) -> serde_json::Value {
        match doc {
            Document::Null => serde_json::Value::Null,
            Document::Bool(b) => serde_json::Value::Bool(*b),
            Document::Number(n) => match *n {
                aws_smithy_types::Number::PosInt(i) => {
                    serde_json::Value::Number(serde_json::Number::from(i))
                }
                aws_smithy_types::Number::NegInt(i) => {
                    serde_json::Value::Number(serde_json::Number::from(i))
                }
                aws_smithy_types::Number::Float(f) => serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
            },
            Document::String(s) => serde_json::Value::String(s.clone()),
            Document::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::document_to_json).collect())
            }
            Document::Object(map) => {
                let obj: serde_json::Map<String, serde_json::Value> = map
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::document_to_json(v)))
                    .collect();
                serde_json::Value::Object(obj)
            }
        }
    }
}

#[async_trait]
impl VectorStorage for S3VectorsStorage {
    fn namespace(&self) -> &str {
        &self.config.namespace
    }

    fn dimension(&self) -> usize {
        self.config.dimension
    }

    async fn initialize(&self) -> edgequake_storage::error::Result<()> {
        // Verify bucket exists
        match self
            .client
            .get_vector_bucket()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .send()
            .await
        {
            Ok(_) => {
                debug!(
                    bucket = %self.config.vector_bucket_name,
                    "S3 Vector bucket exists"
                );
            }
            Err(e) => {
                info!(
                    bucket = %self.config.vector_bucket_name,
                    "Vector bucket not found, creating: {}", e
                );
                self.client
                    .create_vector_bucket()
                    .vector_bucket_name(&self.config.vector_bucket_name)
                    .send()
                    .await
                    .map_err(s3v_err)?;
            }
        }

        // Verify index exists and has the correct dimension
        match self
            .client
            .get_index()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .index_name(&self.config.index_name)
            .send()
            .await
        {
            Ok(resp) => {
                let existing_dim = resp.index().map(|i| i.dimension()).unwrap_or(0);
                if existing_dim > 0 && existing_dim != self.config.dimension as i32 {
                    return Err(edgequake_storage::error::StorageError::InvalidConfig(
                        format!(
                            "S3 Vectors index '{}' has dimension {} but pipeline expects {}. \
                             Delete the index or use a different index name (e.g. '{{namespace}}-embeddings').",
                            self.config.index_name, existing_dim, self.config.dimension
                        ),
                    ));
                }
                debug!(
                    index = %self.config.index_name,
                    dimension = existing_dim,
                    "S3 Vector index exists with matching dimension"
                );
            }
            Err(e) => {
                info!(
                    index = %self.config.index_name,
                    "Vector index not found, creating: {}", e
                );
                self.client
                    .create_index()
                    .vector_bucket_name(&self.config.vector_bucket_name)
                    .index_name(&self.config.index_name)
                    .data_type(DataType::Float32)
                    .dimension(self.config.dimension as i32)
                    .distance_metric(DistanceMetric::Cosine)
                    .send()
                    .await
                    .map_err(s3v_err)?;
            }
        }

        Ok(())
    }

    async fn finalize(&self) -> edgequake_storage::error::Result<()> {
        // S3 Vectors handles indexing automatically — nothing to do.
        Ok(())
    }

    async fn upsert(
        &self,
        data: &[(String, Vec<f32>, serde_json::Value)],
    ) -> edgequake_storage::error::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        for chunk in data.chunks(PUT_BATCH_SIZE) {
            let mut builder = self
                .client
                .put_vectors()
                .vector_bucket_name(&self.config.vector_bucket_name)
                .index_name(&self.config.index_name);

            for (id, embedding, metadata) in chunk {
                let doc = Self::json_to_document(metadata);
                let vector = PutInputVector::builder()
                    .key(id)
                    .data(VectorData::Float32(embedding.clone()))
                    .metadata(doc)
                    .build()
                    .map_err(|e| AwsStorageError::S3VectorsError(e.to_string()))?;
                builder = builder.vectors(vector);
            }

            builder.send().await.map_err(s3v_err)?;
        }

        debug!(count = data.len(), "Put vectors to S3 Vectors");
        Ok(())
    }

    async fn query(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        filter_ids: Option<&[String]>,
    ) -> edgequake_storage::error::Result<Vec<VectorSearchResult>> {
        // If filtering by IDs, request more results to account for filtering.
        // S3 Vectors API limits top_k to 100.
        let request_top_k = if filter_ids.is_some() {
            (top_k * 3).min(100) as i32
        } else {
            (top_k).min(100) as i32
        };

        let resp = self
            .client
            .query_vectors()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .index_name(&self.config.index_name)
            .query_vector(VectorData::Float32(query_embedding.to_vec()))
            .top_k(request_top_k)
            .return_distance(true)
            .return_metadata(true)
            .send()
            .await
            .map_err(s3v_err)?;

        let mut results: Vec<VectorSearchResult> = resp
            .vectors()
            .iter()
            .filter_map(|v| {
                let key = v.key().to_string();

                // Apply ID filter if provided
                if let Some(ids) = filter_ids {
                    if !ids.iter().any(|id| id == &key) {
                        return None;
                    }
                }

                // S3 Vectors cosine distance: lower = more similar
                // Convert to score: score = 1.0 - distance
                let score = v.distance().map(|d| 1.0 - d).unwrap_or(0.0);
                let metadata = v
                    .metadata()
                    .map(Self::document_to_json)
                    .unwrap_or(serde_json::Value::Null);

                Some(VectorSearchResult {
                    id: key,
                    score,
                    metadata,
                })
            })
            .collect();

        results.truncate(top_k);
        Ok(results)
    }

    async fn query_by_type(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        type_filter: &str,
        filter_ids: Option<&[String]>,
    ) -> edgequake_storage::error::Result<Vec<VectorSearchResult>> {
        let request_top_k = if filter_ids.is_some() {
            (top_k * 3).min(100) as i32
        } else {
            (top_k).min(100) as i32
        };

        // Build native metadata filter: {"type": "<type_filter>"}
        let filter = {
            let mut map = HashMap::new();
            map.insert(
                "type".to_string(),
                Document::String(type_filter.to_string()),
            );
            Document::Object(map)
        };

        let resp = self
            .client
            .query_vectors()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .index_name(&self.config.index_name)
            .query_vector(VectorData::Float32(query_embedding.to_vec()))
            .top_k(request_top_k)
            .filter(filter)
            .return_distance(true)
            .return_metadata(true)
            .send()
            .await
            .map_err(s3v_err)?;

        let mut results: Vec<VectorSearchResult> = resp
            .vectors()
            .iter()
            .filter_map(|v| {
                let key = v.key().to_string();

                if let Some(ids) = filter_ids {
                    if !ids.iter().any(|id| id == &key) {
                        return None;
                    }
                }

                let score = v.distance().map(|d| 1.0 - d).unwrap_or(0.0);
                let metadata = v
                    .metadata()
                    .map(Self::document_to_json)
                    .unwrap_or(serde_json::Value::Null);

                Some(VectorSearchResult {
                    id: key,
                    score,
                    metadata,
                })
            })
            .collect();

        results.truncate(top_k);
        debug!(
            type_filter = type_filter,
            results = results.len(),
            "S3 Vectors query_by_type with native metadata filter"
        );
        Ok(results)
    }

    async fn delete(&self, ids: &[String]) -> edgequake_storage::error::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        for chunk in ids.chunks(PUT_BATCH_SIZE) {
            let mut builder = self
                .client
                .delete_vectors()
                .vector_bucket_name(&self.config.vector_bucket_name)
                .index_name(&self.config.index_name);

            for id in chunk {
                builder = builder.keys(id);
            }

            builder.send().await.map_err(s3v_err)?;
        }

        debug!(count = ids.len(), "Deleted vectors from S3 Vectors");
        Ok(())
    }

    async fn delete_entity(&self, entity_name: &str) -> edgequake_storage::error::Result<()> {
        // Query for vectors with metadata matching this entity, then delete them.
        // Use a metadata filter for type=entity and entity_name=X.
        let filter = {
            let mut map = HashMap::new();
            map.insert("type".to_string(), Document::String("entity".to_string()));
            map.insert(
                "entity_name".to_string(),
                Document::String(entity_name.to_string()),
            );
            Document::Object(map)
        };

        let resp = self
            .client
            .query_vectors()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .index_name(&self.config.index_name)
            // Use a dummy zero vector for the query — we're filtering by metadata
            .query_vector(VectorData::Float32(vec![0.0; self.config.dimension]))
            .top_k(10_000)
            .filter(filter)
            .return_metadata(false)
            .send()
            .await
            .map_err(s3v_err)?;

        let keys: Vec<String> = resp.vectors().iter().map(|v| v.key().to_string()).collect();
        if !keys.is_empty() {
            self.delete(&keys).await?;
            debug!(
                entity = entity_name,
                count = keys.len(),
                "Deleted entity vectors"
            );
        }

        Ok(())
    }

    async fn delete_entity_relations(
        &self,
        entity_name: &str,
    ) -> edgequake_storage::error::Result<()> {
        // Delete vectors where metadata source=entity_name or target=entity_name.
        // S3 Vectors filter doesn't support OR, so we do two queries.
        for field in ["source", "target"] {
            let filter = {
                let mut map = HashMap::new();
                map.insert(field.to_string(), Document::String(entity_name.to_string()));
                Document::Object(map)
            };

            let resp = self
                .client
                .query_vectors()
                .vector_bucket_name(&self.config.vector_bucket_name)
                .index_name(&self.config.index_name)
                .query_vector(VectorData::Float32(vec![0.0; self.config.dimension]))
                .top_k(10_000)
                .filter(filter)
                .return_metadata(false)
                .send()
                .await
                .map_err(s3v_err)?;

            let keys: Vec<String> = resp.vectors().iter().map(|v| v.key().to_string()).collect();
            if !keys.is_empty() {
                self.delete(&keys).await?;
                debug!(
                    entity = entity_name,
                    field = field,
                    count = keys.len(),
                    "Deleted entity relation vectors"
                );
            }
        }

        Ok(())
    }

    async fn get_by_id(&self, id: &str) -> edgequake_storage::error::Result<Option<Vec<f32>>> {
        let resp = self
            .client
            .get_vectors()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .index_name(&self.config.index_name)
            .keys(id)
            .return_data(true)
            .send()
            .await
            .map_err(s3v_err)?;

        let vector = resp.vectors().first().and_then(|v| {
            v.data().and_then(|d| match d {
                VectorData::Float32(data) => Some(data.clone()),
                _ => None,
            })
        });

        Ok(vector)
    }

    async fn get_by_ids(
        &self,
        ids: &[String],
    ) -> edgequake_storage::error::Result<Vec<(String, Vec<f32>)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for chunk in ids.chunks(GET_BATCH_SIZE) {
            let mut builder = self
                .client
                .get_vectors()
                .vector_bucket_name(&self.config.vector_bucket_name)
                .index_name(&self.config.index_name)
                .return_data(true);

            for id in chunk {
                builder = builder.keys(id);
            }

            let resp = builder.send().await.map_err(s3v_err)?;

            for v in resp.vectors() {
                if let Some(VectorData::Float32(data)) = v.data() {
                    results.push((v.key().to_string(), data.clone()));
                }
            }
        }

        Ok(results)
    }

    async fn is_empty(&self) -> edgequake_storage::error::Result<bool> {
        let resp = self
            .client
            .list_vectors()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .index_name(&self.config.index_name)
            .max_results(1)
            .send()
            .await
            .map_err(s3v_err)?;

        Ok(resp.vectors().is_empty())
    }

    async fn count(&self) -> edgequake_storage::error::Result<usize> {
        let mut total = 0usize;
        let mut next_token: Option<String> = None;

        loop {
            let mut builder = self
                .client
                .list_vectors()
                .vector_bucket_name(&self.config.vector_bucket_name)
                .index_name(&self.config.index_name)
                .max_results(500);

            if let Some(ref token) = next_token {
                builder = builder.next_token(token);
            }

            let resp = builder.send().await.map_err(s3v_err)?;

            total += resp.vectors().len();
            next_token = resp.next_token().map(|s| s.to_string());

            if next_token.is_none() {
                break;
            }

            // Safety limit: don't iterate forever for very large indexes
            if total > 1_000_000 {
                warn!("count() exceeded 1M vectors, returning approximate count");
                break;
            }
        }

        Ok(total)
    }

    async fn clear(&self) -> edgequake_storage::error::Result<()> {
        // Fastest way to clear: delete and recreate the index
        info!(
            index = %self.config.index_name,
            "Clearing S3 Vectors index (delete + recreate)"
        );

        // Delete the index
        self.client
            .delete_index()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .index_name(&self.config.index_name)
            .send()
            .await
            .map_err(s3v_err)?;

        // Recreate the index
        self.client
            .create_index()
            .vector_bucket_name(&self.config.vector_bucket_name)
            .index_name(&self.config.index_name)
            .data_type(DataType::Float32)
            .dimension(self.config.dimension as i32)
            .distance_metric(DistanceMetric::Cosine)
            .send()
            .await
            .map_err(s3v_err)?;

        Ok(())
    }
}
