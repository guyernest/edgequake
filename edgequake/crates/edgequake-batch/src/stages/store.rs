//! Phase 4: Store - Write extracted data to Neptune, S3, and DynamoDB.

use crate::config::BatchConfig;
use crate::jsonl::PreparedChunk;
use crate::parquet_reader::ReconstructedDocument;
use crate::progress::BatchProgress;
use crate::stages::embed::EmbedResult;
use crate::state::{JobState, Phase, StateManager};
use edgequake_pipeline::extractor::ExtractionResult;
use edgequake_storage::traits::{KVStorage, VectorStorage};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// Run Phase 4: Store.
///
/// Writes to three storage backends:
/// 1. Neptune graph: entities as nodes, relationships as edges (via bulk load)
/// 2. S3 vectors: chunk, entity, and relationship embeddings with HNSW index
/// 3. DynamoDB KV: document metadata, chunk content, processing stats
pub async fn run_store(
    config: &BatchConfig,
    aws_config: &aws_config::SdkConfig,
    vectors: Arc<dyn VectorStorage>,
    kv: Arc<dyn KVStorage>,
    state_mgr: &StateManager,
    job: &mut JobState,
    documents: &[ReconstructedDocument],
    chunks: &[PreparedChunk],
    extractions: &HashMap<String, ExtractionResult>,
    embeddings: &EmbedResult,
    progress: &BatchProgress,
) -> anyhow::Result<()> {
    info!("Phase 4: STORE starting");
    job.phase = Phase::Storing;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    // Step 1: Store document metadata in KV
    let bar = progress.document_bar(documents.len() as u64, "Storing documents");
    let mut doc_entries: Vec<(String, serde_json::Value)> = Vec::new();

    for doc in documents {
        // Store preview only — full content is in chunks.
        // DynamoDB has a 400KB item size limit.
        let content_preview: String = doc.content.chars().take(1000).collect();
        let doc_meta = serde_json::json!({
            "id": doc.content_hash,
            "filename": doc.filename,
            "content_preview": content_preview,
            "content_length": doc.content.len(),
            "content_hash": doc.content_hash,
            "source_row_count": doc.source_row_count,
            "status": "completed",
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        doc_entries.push((format!("doc:{}", doc.content_hash), doc_meta));
        bar.inc(1);
    }

    // Batch upsert documents
    for batch in doc_entries.chunks(25) {
        if let Err(e) = kv.upsert(batch).await {
            warn!(error = %e, "Failed to upsert document batch to KV");
        }
    }
    bar.finish_with_message(format!("Stored {} documents", documents.len()));

    // Step 2: Store chunks in KV and vectors in S3
    let bar = progress.document_bar(chunks.len() as u64, "Storing chunks & vectors");
    let mut chunk_kv_entries: Vec<(String, serde_json::Value)> = Vec::new();
    let mut vector_entries: Vec<(String, Vec<f32>, serde_json::Value)> = Vec::new();

    for chunk in chunks {
        // KV entry for chunk content
        let chunk_meta = serde_json::json!({
            "id": chunk.custom_id,
            "document_id": chunk.doc_hash,
            "filename": chunk.doc_filename,
            "content": chunk.chunk.content,
            "index": chunk.chunk.index,
            "token_count": chunk.chunk.token_count,
        });
        chunk_kv_entries.push((format!("chunk:{}", chunk.custom_id), chunk_meta));

        // Vector entry if embedding exists
        if let Some(embedding) = embeddings.chunk_embeddings.get(&chunk.custom_id) {
            let metadata = serde_json::json!({
                "chunk_id": chunk.custom_id,
                "document_id": chunk.doc_hash,
                "filename": chunk.doc_filename,
                "type": "chunk",
            });
            vector_entries.push((chunk.custom_id.clone(), embedding.clone(), metadata));
        }

        bar.inc(1);
    }

    // Batch upsert chunks to KV
    for batch in chunk_kv_entries.chunks(25) {
        if let Err(e) = kv.upsert(batch).await {
            warn!(error = %e, "Failed to upsert chunk batch to KV");
        }
    }

    // Batch upsert chunk vectors to S3 Vectors
    let mut chunk_vectors_stored = 0;
    for batch in vector_entries.chunks(100) {
        match vectors.upsert(batch).await {
            Ok(_) => {
                chunk_vectors_stored += batch.len();
            }
            Err(e) => {
                warn!(error = %e, batch_size = batch.len(), "Failed to store chunk vectors to S3 Vectors");
            }
        }
    }
    info!(
        total_chunks = chunks.len(),
        vectors_stored = chunk_vectors_stored,
        "Stored chunk vectors to S3 Vectors"
    );
    bar.finish_with_message(format!("Stored {} chunks ({} vectors)", chunks.len(), chunk_vectors_stored));

    // Step 3: Store entities and relationships in Neptune via bulk load
    if config.neptune_endpoint.is_some()
        && config.s3_bucket.is_some()
        && config.neptune_role_arn.is_some()
    {
        info!("Using Neptune bulk load for graph storage");
        super::bulk_load::run_bulk_load(config, aws_config, extractions, progress).await?;
    } else if config.neptune_endpoint.is_some() {
        warn!(
            "Neptune endpoint configured but --s3-bucket and --neptune-role-arn are required \
             for bulk load. Skipping graph storage."
        );
    } else {
        info!("No Neptune endpoint configured, skipping graph storage");
    }

    // Step 4: Store entity and relationship vectors in S3 Vectors
    let total_entities: usize = extractions.values().map(|r| r.entities.len()).sum();
    let bar = progress.document_bar(total_entities as u64, "Storing entity vectors");

    info!(
        total_entities = total_entities,
        embeddings_available = embeddings.entity_embeddings.len(),
        "Preparing entity vectors for S3 Vectors storage"
    );

    // Use HashMap to deduplicate entities by name (same entity may appear in multiple chunks)
    let mut entity_map: std::collections::HashMap<String, (Vec<f32>, serde_json::Value)> = std::collections::HashMap::new();
    let mut missing_embeddings = 0;

    for result in extractions.values() {
        for entity in &result.entities {
            // Skip if we've already processed this entity
            if entity_map.contains_key(&entity.name) {
                bar.inc(1);
                continue;
            }

            if let Some(embedding) = embeddings.entity_embeddings.get(&entity.name) {
                let source_chunk_ids = entity.source_chunk_ids.join("|");

                // S3 Vectors metadata constraints:
                // - Only strings, numbers, booleans, or arrays of these types
                // - No nested objects or null values
                let mut metadata = serde_json::json!({
                    "entity_name": entity.name.clone(),
                    "entity_type": entity.entity_type.clone(),
                    "type": "entity",
                    "source_chunk_ids": source_chunk_ids,
                });

                // Only add optional fields if they exist
                if let Some(ref doc_id) = entity.source_document_id {
                    metadata.as_object_mut().unwrap().insert(
                        "source_document_id".to_string(),
                        serde_json::Value::String(doc_id.clone())
                    );
                }
                if let Some(ref file_path) = entity.source_file_path {
                    metadata.as_object_mut().unwrap().insert(
                        "source_file_path".to_string(),
                        serde_json::Value::String(file_path.clone())
                    );
                }

                entity_map.insert(
                    entity.name.clone(),
                    (embedding.clone(), metadata)
                );
            } else {
                missing_embeddings += 1;
                if missing_embeddings <= 5 {
                    warn!(entity_name = %entity.name, "Entity missing embedding");
                }
            }
            bar.inc(1);
        }
    }

    if missing_embeddings > 0 {
        warn!(
            total_entities = total_entities,
            missing_embeddings = missing_embeddings,
            "Some entities are missing embeddings and will not be stored to S3 Vectors"
        );
    }

    // Convert deduplicated map to vector list
    let entity_vectors: Vec<(String, Vec<f32>, serde_json::Value)> = entity_map
        .into_iter()
        .map(|(name, (embedding, metadata))| {
            (format!("entity:{}", name), embedding, metadata)
        })
        .collect();

    info!(
        unique_entities = entity_vectors.len(),
        "Deduplicated entity vectors ready for S3 Vectors storage"
    );

    // Batch upsert entity vectors to S3 Vectors (same destination as chunks)
    let mut entity_vectors_stored = 0;
    for batch in entity_vectors.chunks(100) {
        match vectors.upsert(batch).await {
            Ok(_) => {
                entity_vectors_stored += batch.len();
            }
            Err(e) => {
                warn!(
                    error = %e,
                    batch_size = batch.len(),
                    "Failed to store entity vectors to S3 Vectors"
                );
            }
        }
    }
    info!(
        total_entities = total_entities,
        vectors_prepared = entity_vectors.len(),
        vectors_stored = entity_vectors_stored,
        "Stored entity vectors to S3 Vectors"
    );
    bar.finish_with_message(format!(
        "Stored {} entity vectors to S3 Vectors",
        entity_vectors_stored
    ));

    let total_rels: usize = extractions.values().map(|r| r.relationships.len()).sum();
    let bar = progress.document_bar(total_rels as u64, "Storing relationship vectors");

    info!(
        total_relationships = total_rels,
        embeddings_available = embeddings.relationship_embeddings.len(),
        "Preparing relationship vectors for S3 Vectors storage"
    );

    // Use HashMap to deduplicate relationships by (source, target) pair
    let mut rel_map: std::collections::HashMap<(String, String), (Vec<f32>, serde_json::Value)> = std::collections::HashMap::new();

    for result in extractions.values() {
        for rel in &result.relationships {
            let rel_key = (rel.source.clone(), rel.target.clone());

            // Skip if we've already processed this relationship
            if rel_map.contains_key(&rel_key) {
                bar.inc(1);
                continue;
            }

            if let Some(embedding) = embeddings.relationship_embeddings.get(&rel_key) {
                // S3 Vectors metadata constraints: only strings, numbers, booleans, or arrays
                let mut metadata = serde_json::json!({
                    "source": rel.source.clone(),
                    "target": rel.target.clone(),
                    "type": "relationship",
                    "keywords": rel.keywords.join(", "),
                });

                // Only add optional fields if they exist
                if let Some(ref chunk_id) = rel.source_chunk_id {
                    metadata.as_object_mut().unwrap().insert(
                        "source_chunk_id".to_string(),
                        serde_json::Value::String(chunk_id.clone())
                    );
                }
                if let Some(ref doc_id) = rel.source_document_id {
                    metadata.as_object_mut().unwrap().insert(
                        "source_document_id".to_string(),
                        serde_json::Value::String(doc_id.clone())
                    );
                }
                if let Some(ref file_path) = rel.source_file_path {
                    metadata.as_object_mut().unwrap().insert(
                        "source_file_path".to_string(),
                        serde_json::Value::String(file_path.clone())
                    );
                }

                rel_map.insert(rel_key.clone(), (embedding.clone(), metadata));
            }
            bar.inc(1);
        }
    }

    // Convert deduplicated map to vector list
    let rel_vectors: Vec<(String, Vec<f32>, serde_json::Value)> = rel_map
        .into_iter()
        .map(|((source, target), (embedding, metadata))| {
            let vector_id = format!("rel:{}:{}", source, target);
            (vector_id, embedding, metadata)
        })
        .collect();

    info!(
        unique_relationships = rel_vectors.len(),
        "Deduplicated relationship vectors ready for S3 Vectors storage"
    );

    // Batch upsert relationship vectors to S3 Vectors (same destination as chunks)
    let mut rel_vectors_stored = 0;
    for batch in rel_vectors.chunks(100) {
        match vectors.upsert(batch).await {
            Ok(_) => {
                rel_vectors_stored += batch.len();
            }
            Err(e) => {
                warn!(
                    error = %e,
                    batch_size = batch.len(),
                    "Failed to store relationship vectors to S3 Vectors"
                );
            }
        }
    }
    info!(
        total_relationships = total_rels,
        vectors_prepared = rel_vectors.len(),
        vectors_stored = rel_vectors_stored,
        "Stored relationship vectors to S3 Vectors"
    );
    bar.finish_with_message(format!(
        "Stored {} relationship vectors to S3 Vectors",
        rel_vectors_stored
    ));

    // Finalize
    job.phase = Phase::Completed;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    info!(
        documents = documents.len(),
        entities = total_entities,
        relationships = total_rels,
        "Phase 4: STORE complete - pipeline finished!"
    );

    Ok(())
}
