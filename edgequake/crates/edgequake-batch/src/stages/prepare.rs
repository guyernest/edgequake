//! Phase 1: Prepare - Parse parquet, chunk documents, build JSONL files.

use crate::config::BatchConfig;
use crate::jsonl::{build_jsonl_files, make_custom_id, JsonlBuildResult, PreparedChunk};
use crate::parquet_reader::{read_parquet_documents, ReconstructedDocument};
use crate::progress::BatchProgress;
use crate::prompts::EpsteinExtractionPrompts;
use crate::state::{DocumentState, DocumentStatus, JobState, Phase, StateManager};
use edgequake_pipeline::chunker::{Chunker, ChunkerConfig, TextChunk};
use tracing::{info, warn};

/// Result of the prepare phase.
#[derive(Debug)]
pub struct PrepareResult {
    /// Reconstructed documents from parquet.
    pub documents: Vec<ReconstructedDocument>,

    /// All chunks with custom IDs for batch processing.
    pub chunks: Vec<PreparedChunk>,

    /// JSONL file paths for batch submission.
    pub jsonl_result: JsonlBuildResult,
}

/// Run Phase 1: Prepare.
///
/// 1. Read parquet file and reconstruct documents
/// 2. Skip already-completed documents (idempotency)
/// 3. Chunk each document
/// 4. Build JSONL files for OpenAI Batch API
/// 5. Save document states to DynamoDB
pub async fn run_prepare(
    config: &BatchConfig,
    state_mgr: &StateManager,
    job: &mut JobState,
    progress: &BatchProgress,
) -> anyhow::Result<PrepareResult> {
    info!("Phase 1: PREPARE starting");
    job.phase = Phase::Preparing;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    // Step 1: Read parquet file
    let spinner = progress.spinner("Reading parquet file...");
    let mut documents = read_parquet_documents(&config.data_path)?;
    let total_in_file = documents.len();

    // Apply offset: skip the first N documents
    if config.offset > 0 {
        if config.offset >= documents.len() {
            info!(
                offset = config.offset,
                total = documents.len(),
                "Offset exceeds document count, nothing to process"
            );
            documents.clear();
        } else {
            info!(
                offset = config.offset,
                total = documents.len(),
                "Skipping first {} documents",
                config.offset
            );
            documents = documents.split_off(config.offset);
        }
    }

    // Apply limit: take at most N documents after offset
    if config.limit > 0 && documents.len() > config.limit {
        info!(
            limit = config.limit,
            available = documents.len(),
            offset = config.offset,
            "Limiting to {} documents (from offset {})",
            config.limit,
            config.offset
        );
        documents.truncate(config.limit);
    }
    spinner.finish_with_message(format!(
        "Read {} documents from parquet (offset={}, limit={}, total_in_file={})",
        documents.len(),
        config.offset,
        if config.limit > 0 {
            config.limit.to_string()
        } else {
            "none".to_string()
        },
        total_in_file
    ));

    job.total_documents = documents.len();

    // Step 2: Chunk documents
    let chunker_config = ChunkerConfig {
        chunk_size: config.chunk_size,
        chunk_overlap: config.chunk_overlap,
        ..ChunkerConfig::default()
    };
    let chunker = Chunker::new(chunker_config);

    let bar = progress.document_bar(documents.len() as u64, "Chunking documents");
    let mut all_chunks: Vec<PreparedChunk> = Vec::new();
    let mut doc_states: Vec<DocumentState> = Vec::new();

    for doc in &documents {
        bar.inc(1);

        // Idempotency: skip completed documents
        if state_mgr.is_document_completed(&doc.content_hash).await {
            continue;
        }

        // Chunk the document
        let chunks = match chunker.chunk(&doc.content, &doc.content_hash) {
            Ok(chunks) => chunks,
            Err(e) => {
                warn!(
                    filename = %doc.filename,
                    error = %e,
                    "Failed to chunk document, skipping"
                );
                doc_states.push(DocumentState {
                    content_hash: doc.content_hash.clone(),
                    filename: doc.filename.clone(),
                    status: DocumentStatus::Failed,
                    chunk_ids: Vec::new(),
                    entity_count: 0,
                    relationship_count: 0,
                    error: Some(e.to_string()),
                });
                continue;
            }
        };

        let chunk_ids: Vec<String> = chunks
            .iter()
            .enumerate()
            .map(|(idx, _)| make_custom_id(&doc.content_hash, idx))
            .collect();

        // Create prepared chunks with custom IDs
        for (idx, chunk) in chunks.into_iter().enumerate() {
            let custom_id = make_custom_id(&doc.content_hash, idx);
            all_chunks.push(PreparedChunk {
                custom_id,
                chunk,
                doc_hash: doc.content_hash.clone(),
                doc_filename: doc.filename.clone(),
            });
        }

        doc_states.push(DocumentState {
            content_hash: doc.content_hash.clone(),
            filename: doc.filename.clone(),
            status: DocumentStatus::Chunked,
            chunk_ids,
            entity_count: 0,
            relationship_count: 0,
            error: None,
        });
    }

    bar.finish_with_message(format!(
        "Chunked {} documents into {} chunks",
        documents.len(),
        all_chunks.len()
    ));

    // Step 3: Build JSONL files
    let spinner = progress.spinner("Building JSONL files...");
    let prompts = EpsteinExtractionPrompts::new();
    let system_prompt = prompts.system_prompt();

    let jsonl_result = build_jsonl_files(
        &all_chunks,
        &system_prompt,
        &|chunk_text| prompts.user_prompt(chunk_text),
        config,
    )?;

    spinner.finish_with_message(format!(
        "Built {} JSONL files with {} total requests",
        jsonl_result.batch_count, jsonl_result.total_requests
    ));

    // Step 4: Save state
    state_mgr.save_documents(&doc_states).await?;

    job.total_chunks = all_chunks.len();
    job.phase = Phase::Prepared;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    info!(
        documents = documents.len(),
        chunks = all_chunks.len(),
        jsonl_files = jsonl_result.batch_count,
        "Phase 1: PREPARE complete"
    );

    Ok(PrepareResult {
        documents,
        chunks: all_chunks,
        jsonl_result,
    })
}
