//! Phase 4: Store - Write extracted data to Neptune, S3, DynamoDB, and BM25 index.

use crate::config::BatchConfig;
use crate::jsonl::PreparedChunk;
use crate::parquet_reader::ReconstructedDocument;
use crate::progress::BatchProgress;
use crate::stages::embed::EmbedResult;
use crate::state::{JobState, LatestRunStatus, Phase, StateManager};
use edgequake_pipeline::extractor::ExtractionResult;
use edgequake_storage::traits::{KVStorage, VectorStorage};
use edgequake_storage::Bm25Storage;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// Run Phase 4: Store.
///
/// Writes to up to four storage backends:
/// 1. DynamoDB KV: document metadata, chunk content, processing stats
/// 2. S3 vectors: chunk, entity, and relationship embeddings with HNSW index
/// 3. Neptune graph: entities as nodes, relationships as edges (via bulk load)
/// 4. BM25 inverted index: keyword search data in Iceberg tables (if configured)
///
/// BM25 indexing is optional (`bm25` parameter is `Option`). When `None`,
/// the pipeline behaves identically to pre-BM25 behavior for backward
/// compatibility with deployments that do not have Athena configured.
pub async fn run_store(
    config: &BatchConfig,
    aws_config: &aws_config::SdkConfig,
    vectors: Arc<dyn VectorStorage>,
    kv: Arc<dyn KVStorage>,
    bm25: Option<Arc<dyn Bm25Storage>>,
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

    // Record phase start time and write LATEST_RUN for UI visibility
    let now_millis = chrono::Utc::now().timestamp_millis();
    job.phase_started_at
        .insert("storing".to_string(), now_millis);
    let run_started_at = job.run_started_at.unwrap_or(now_millis);

    let latest = LatestRunStatus {
        status: "storing".to_string(),
        phase: Some("storing".to_string()),
        job_id: Some(job.job_id.clone()),
        total_documents: Some(job.total_documents),
        processed_documents: Some(job.processed_documents),
        total_chunks: Some(job.total_chunks),
        started_at: Some(run_started_at),
        updated_at: Some(now_millis),
        phase_started_at: Some(now_millis),
        ..Default::default()
    };
    state_mgr.write_latest_run(&latest).await?;

    // Optional portable snapshot output for embedded MCP runtimes.
    let snapshot_counts = if let Some(manifest) =
        super::snapshot::write_snapshot(config, documents, chunks, extractions, embeddings).await?
    {
        let counts = manifest.counts.clone();
        info!(
            snapshot_uri = ?config.snapshot_uri,
            documents = manifest.counts.documents,
            chunks = manifest.counts.chunks,
            entities = manifest.counts.entities,
            relationships = manifest.counts.relationships,
            vectors = manifest.counts.vectors,
            bm25 = manifest.counts.bm25,
            "Portable Graph-RAG snapshot written"
        );
        Some(counts)
    } else {
        None
    };

    if config.snapshot_mode == crate::config::SnapshotMode::SnapshotOnly {
        info!("Snapshot-only mode enabled; skipping managed index writes");
        if let Some(counts) = snapshot_counts {
            job.total_entities = counts.entities;
            job.total_relationships = counts.relationships;
            job.total_embeddings = counts.vectors;
        }
        job.processed_documents = documents.len();
        job.total_chunks = chunks.len();
        job.phase = Phase::Completed;
        job.updated_at = chrono::Utc::now().to_rfc3339();
        state_mgr.save_job(job).await?;

        let completion_millis = chrono::Utc::now().timestamp_millis();
        let run_report = LatestRunStatus {
            status: "completed".to_string(),
            phase: Some("completed".to_string()),
            job_id: Some(job.job_id.clone()),
            total_documents: Some(job.total_documents),
            processed_documents: Some(job.processed_documents),
            total_chunks: Some(job.total_chunks),
            started_at: job.run_started_at,
            updated_at: Some(completion_millis),
            completed_at: Some(completion_millis),
            total_entities: Some(job.total_entities),
            total_relationships: Some(job.total_relationships),
            ..Default::default()
        };
        state_mgr.write_latest_run(&run_report).await?;
        return Ok(());
    }

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
        kv.upsert(batch).await?;
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
        kv.upsert(batch).await?;
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
    bar.finish_with_message(format!(
        "Stored {} chunks ({} vectors)",
        chunks.len(),
        chunk_vectors_stored
    ));

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
    let mut entity_map: std::collections::HashMap<String, (Vec<f32>, serde_json::Value)> =
        std::collections::HashMap::new();
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
                        serde_json::Value::String(doc_id.clone()),
                    );
                }
                if let Some(ref file_path) = entity.source_file_path {
                    metadata.as_object_mut().unwrap().insert(
                        "source_file_path".to_string(),
                        serde_json::Value::String(file_path.clone()),
                    );
                }

                entity_map.insert(entity.name.clone(), (embedding.clone(), metadata));
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
        .map(|(name, (embedding, metadata))| (format!("entity:{}", name), embedding, metadata))
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
    let mut rel_map: std::collections::HashMap<(String, String), (Vec<f32>, serde_json::Value)> =
        std::collections::HashMap::new();

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
                        serde_json::Value::String(chunk_id.clone()),
                    );
                }
                if let Some(ref doc_id) = rel.source_document_id {
                    metadata.as_object_mut().unwrap().insert(
                        "source_document_id".to_string(),
                        serde_json::Value::String(doc_id.clone()),
                    );
                }
                if let Some(ref file_path) = rel.source_file_path {
                    metadata.as_object_mut().unwrap().insert(
                        "source_file_path".to_string(),
                        serde_json::Value::String(file_path.clone()),
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

    // Step 5: Populate BM25 inverted index (if configured)
    if let Some(bm25_storage) = &bm25 {
        info!("Populating BM25 inverted index");

        // Ensure Iceberg tables exist (lazy DDL -- creates on first ingestion)
        bm25_storage.ensure_tables().await?;

        // Preprocess chunks into BM25 documents using shared tantivy analyzer
        let bm25_docs = super::bm25_index::build_bm25_documents(chunks);
        info!(
            chunks = chunks.len(),
            bm25_documents = bm25_docs.len(),
            "BM25 preprocessed chunks into documents"
        );

        // Index the batch into Iceberg tables
        bm25_storage.index_batch(&bm25_docs).await?;

        // Recalculate corpus statistics (N, avgdl) -- MUST be last BM25 step
        // so scoring uses up-to-date document counts and average length.
        bm25_storage.update_corpus_stats().await?;
        info!("BM25 corpus stats updated");
    } else {
        info!("BM25 storage not configured, skipping BM25 indexing");
    }

    // Finalize
    job.phase = Phase::Completed;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    // Compute per-phase durations from phase_started_at timestamps
    let completion_millis = chrono::Utc::now().timestamp_millis();
    let phase_order = ["preparing", "extracting", "embedding", "storing"];
    let mut per_phase_durations = std::collections::HashMap::new();
    for (i, phase_name) in phase_order.iter().enumerate() {
        if let Some(&start) = job.phase_started_at.get(*phase_name) {
            // Duration = next phase start - this phase start, or completion time for last phase
            let end = if i + 1 < phase_order.len() {
                job.phase_started_at
                    .get(phase_order[i + 1])
                    .copied()
                    .unwrap_or(completion_millis)
            } else {
                completion_millis
            };
            per_phase_durations.insert(phase_name.to_string(), end - start);
        }
    }

    // Compute total error count across all phases
    let total_error_count: usize = job.errors_per_phase.values().sum();

    // Determine completion status: amber for partial failures, green for clean runs
    let completed_status = if total_error_count > 0 {
        "completed_with_warnings"
    } else {
        "completed"
    };

    // Build error_count_per_phase only if there were errors
    let error_count_per_phase = if !job.errors_per_phase.is_empty() {
        Some(job.errors_per_phase.clone())
    } else {
        None
    };

    // Write final run report to LATEST_RUN
    let run_report = LatestRunStatus {
        status: completed_status.to_string(),
        phase: None,
        job_id: Some(job.job_id.clone()),
        total_documents: Some(job.total_documents),
        processed_documents: Some(job.processed_documents),
        total_chunks: Some(job.total_chunks),
        started_at: job.run_started_at,
        updated_at: Some(completion_millis),
        completed_at: Some(completion_millis),
        total_entities: Some(job.total_entities),
        total_relationships: Some(job.total_relationships),
        per_phase_durations: Some(per_phase_durations),
        error_count_per_phase,
        error_summary: if total_error_count > 0 {
            Some(format!(
                "{} document errors across phases",
                total_error_count
            ))
        } else {
            None
        },
        ..Default::default()
    };
    state_mgr.write_latest_run(&run_report).await?;

    info!(
        documents = documents.len(),
        entities = total_entities,
        relationships = total_rels,
        status = completed_status,
        "Phase 4: STORE complete - pipeline finished!"
    );

    Ok(())
}
