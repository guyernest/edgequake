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

    // Batch upsert vectors
    for batch in vector_entries.chunks(100) {
        vectors.upsert(batch).await.ok();
    }
    bar.finish_with_message(format!("Stored {} chunks", chunks.len()));

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

    let mut entity_vectors: Vec<(String, Vec<f32>, serde_json::Value)> = Vec::new();
    for result in extractions.values() {
        for entity in &result.entities {
            if let Some(embedding) = embeddings.entity_embeddings.get(&entity.name) {
                let source_chunk_ids = entity.source_chunk_ids.join("|");
                let metadata = serde_json::json!({
                    "entity_name": entity.name,
                    "entity_type": entity.entity_type,
                    "type": "entity",
                    "source_chunk_ids": source_chunk_ids,
                    "source_document_id": entity.source_document_id,
                    "source_file_path": entity.source_file_path,
                });
                entity_vectors.push((
                    format!("entity:{}", entity.name),
                    embedding.clone(),
                    metadata,
                ));
            }
            bar.inc(1);
        }
    }

    for batch in entity_vectors.chunks(100) {
        vectors.upsert(batch).await.ok();
    }
    bar.finish_with_message(format!("Stored {} entity vectors", entity_vectors.len()));

    let total_rels: usize = extractions.values().map(|r| r.relationships.len()).sum();
    let bar = progress.document_bar(total_rels as u64, "Storing relationship vectors");

    let mut rel_vectors: Vec<(String, Vec<f32>, serde_json::Value)> = Vec::new();
    for result in extractions.values() {
        for rel in &result.relationships {
            let rel_key = (rel.source.clone(), rel.target.clone());
            if let Some(embedding) = embeddings.relationship_embeddings.get(&rel_key) {
                let metadata = serde_json::json!({
                    "source": rel.source,
                    "target": rel.target,
                    "type": "relationship",
                    "keywords": rel.keywords.join(", "),
                    "source_chunk_id": rel.source_chunk_id,
                    "source_document_id": rel.source_document_id,
                    "source_file_path": rel.source_file_path,
                });
                let vector_id = format!("rel:{}:{}", rel.source, rel.target);
                rel_vectors.push((vector_id, embedding.clone(), metadata));
            }
            bar.inc(1);
        }
    }

    for batch in rel_vectors.chunks(100) {
        vectors.upsert(batch).await.ok();
    }
    bar.finish_with_message(format!("Stored {} relationship vectors", rel_vectors.len()));

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
