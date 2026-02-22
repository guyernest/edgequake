//! Phase 1: Prepare - Read input data, chunk documents, build JSONL files.
//!
//! Supports three input modes:
//! - **Parquet file** (`.parquet`): Legacy format, reads CSV-in-parquet documents
//! - **Text/Markdown file** (`.txt`, `.md`, `.markdown`): Single file input
//! - **Directory**: Scans for supported files with glob filtering

use crate::config::BatchConfig;
use crate::directory_scanner::DirectoryScanner;
use crate::document_reader::{self, FrontMatter};
use crate::domain_config::DomainConfig;
use crate::domain_prompts::DomainExtractionPrompts;
use crate::jsonl::{build_jsonl_files, make_custom_id, JsonlBuildResult, PreparedChunk};
use crate::parquet_reader::{read_parquet_documents, ReconstructedDocument};
use crate::progress::BatchProgress;
use crate::state::{DocumentState, DocumentStatus, JobState, LatestRunStatus, Phase, StateManager};
use edgequake_pipeline::chunker::{Chunker, ChunkerConfig, HeadingBoundaryChunking, TextChunk};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

/// Result of the prepare phase.
#[derive(Debug)]
pub struct PrepareResult {
    /// Reconstructed documents from input data.
    pub documents: Vec<ReconstructedDocument>,

    /// All chunks with custom IDs for batch processing.
    pub chunks: Vec<PreparedChunk>,

    /// JSONL file paths for batch submission.
    pub jsonl_result: JsonlBuildResult,
}

/// Describes the type of input data detected.
#[derive(Debug)]
enum InputType {
    Parquet,
    TextMarkdown,
    Directory,
}

/// Read input data based on the data_path in config.
///
/// Returns documents, their input type, and a front matter map for markdown files.
fn read_input_data(
    config: &BatchConfig,
) -> anyhow::Result<(Vec<ReconstructedDocument>, InputType, HashMap<String, FrontMatter>)> {
    let data_path = &config.data_path;
    let mut front_matter_map: HashMap<String, FrontMatter> = HashMap::new();

    if data_path.is_dir() {
        // Case 1: Directory path
        let scanner = DirectoryScanner::new(data_path)
            .recursive(!config.no_recurse)
            .include_patterns(&config.include_patterns)
            .exclude_patterns(&config.exclude_patterns);

        let scan_result = scanner.scan()?;
        scan_result.print_summary();

        let supported = scan_result.supported_files();
        if supported.is_empty() {
            anyhow::bail!(
                "No supported files (.txt, .md) found in directory: {}",
                data_path.display()
            );
        }

        // Collect front matter from markdown files
        for file_path in &supported {
            if matches!(
                file_path.extension().and_then(|e| e.to_str()),
                Some("md") | Some("markdown")
            ) {
                if let Ok((_, Some(fm))) = document_reader::read_markdown_file(file_path, data_path)
                {
                    let rel = file_path
                        .strip_prefix(data_path)
                        .unwrap_or(file_path)
                        .to_string_lossy()
                        .to_string();
                    front_matter_map.insert(rel, fm);
                }
            }
        }

        let documents = document_reader::read_all_documents(
            &supported,
            data_path,
            config.delimiter.as_deref(),
            config.no_split,
        )?;

        Ok((documents, InputType::Directory, front_matter_map))
    } else if data_path.is_file() {
        // Case 2: Single file
        match data_path.extension().and_then(|e| e.to_str()) {
            Some("parquet") => {
                let documents = read_parquet_documents(data_path)?;
                Ok((documents, InputType::Parquet, front_matter_map))
            }
            Some("txt") => {
                let base_dir = data_path.parent().unwrap_or(Path::new("."));
                let documents = document_reader::read_single_file(
                    data_path,
                    base_dir,
                    config.delimiter.as_deref(),
                    config.no_split,
                )?;
                Ok((documents, InputType::TextMarkdown, front_matter_map))
            }
            Some("md") | Some("markdown") => {
                let base_dir = data_path.parent().unwrap_or(Path::new("."));

                // Collect front matter before reading (read_single_file discards it)
                if let Ok((_, Some(fm))) =
                    document_reader::read_markdown_file(data_path, base_dir)
                {
                    let rel = data_path
                        .strip_prefix(base_dir)
                        .unwrap_or(data_path)
                        .to_string_lossy()
                        .to_string();
                    front_matter_map.insert(rel, fm);
                }

                let documents = document_reader::read_single_file(
                    data_path,
                    base_dir,
                    config.delimiter.as_deref(),
                    config.no_split,
                )?;
                Ok((documents, InputType::TextMarkdown, front_matter_map))
            }
            Some(ext) => anyhow::bail!(
                "Unsupported file type: .{} (supported: .parquet, .txt, .md)",
                ext
            ),
            None => anyhow::bail!(
                "Cannot determine file type (no extension): {}",
                data_path.display()
            ),
        }
    } else {
        anyhow::bail!(
            "Data path does not exist: {}",
            data_path.display()
        )
    }
}

/// Build front matter metadata map for a chunk.
fn build_front_matter_metadata(fm: &FrontMatter) -> serde_json::Map<String, serde_json::Value> {
    let mut meta = serde_json::Map::new();
    if let Some(ref title) = fm.title {
        meta.insert(
            "fm_title".to_string(),
            serde_json::Value::String(title.clone()),
        );
    }
    if let Some(ref author) = fm.author {
        meta.insert(
            "fm_author".to_string(),
            serde_json::Value::String(author.clone()),
        );
    }
    if let Some(ref date) = fm.date {
        meta.insert(
            "fm_date".to_string(),
            serde_json::Value::String(date.clone()),
        );
    }
    if let Some(ref tags) = fm.tags {
        meta.insert("fm_tags".to_string(), serde_json::json!(tags));
    }
    // Merge extra front matter fields
    for (k, v) in &fm.extra {
        meta.insert(format!("fm_{}", k), v.clone());
    }
    meta
}

/// Check if a filename indicates a markdown document.
fn is_markdown_file(filename: &str) -> bool {
    filename.ends_with(".md") || filename.ends_with(".markdown")
}

/// Look up front matter for a document filename.
///
/// For delimiter-split documents (filename#N), look up by the base filename.
fn lookup_front_matter<'a>(
    filename: &str,
    front_matter_map: &'a HashMap<String, FrontMatter>,
) -> Option<&'a FrontMatter> {
    // Direct match first
    if let Some(fm) = front_matter_map.get(filename) {
        return Some(fm);
    }
    // Try stripping delimiter index suffix (e.g., "file.md#2" -> "file.md")
    if let Some(base) = filename.rsplit_once('#') {
        front_matter_map.get(base.0)
    } else {
        None
    }
}

/// Run Phase 1: Prepare.
///
/// 1. Read input data (parquet, text/markdown file, or directory)
/// 2. Skip already-completed documents (idempotency)
/// 3. Chunk each document (HeadingBoundaryChunking for markdown)
/// 4. Merge front matter metadata into chunk metadata
/// 5. Build JSONL files for OpenAI Batch API
/// 6. Save document states to DynamoDB
pub async fn run_prepare(
    config: &BatchConfig,
    domain_config: &DomainConfig,
    state_mgr: &StateManager,
    job: &mut JobState,
    progress: &BatchProgress,
) -> anyhow::Result<PrepareResult> {
    info!("Phase 1: PREPARE starting");
    job.phase = Phase::Preparing;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    // Determine run_started_at: if UI triggered ingestion (status=requested),
    // preserve the original started_at timestamp. Otherwise use current time.
    let now_millis = chrono::Utc::now().timestamp_millis();
    let run_started_at = if job.run_started_at.is_none() {
        let started_at = match state_mgr.read_latest_run().await? {
            Some(existing) if existing.status == "requested" => {
                info!("Preserving started_at from UI-triggered 'requested' status");
                existing.started_at.unwrap_or(now_millis)
            }
            _ => now_millis,
        };
        job.run_started_at = Some(started_at);
        started_at
    } else {
        job.run_started_at.unwrap()
    };

    // Record phase start time for duration tracking
    job.phase_started_at
        .insert("preparing".to_string(), now_millis);

    // Write LATEST_RUN for UI visibility
    let latest = LatestRunStatus {
        status: "preparing".to_string(),
        phase: Some("preparing".to_string()),
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

    // Step 1: Read input data (dispatches by file type)
    let spinner = progress.spinner("Reading input data...");
    let (mut documents, input_type, front_matter_map) = read_input_data(config)?;
    let total_in_source = documents.len();

    let input_desc = match input_type {
        InputType::Parquet => "parquet",
        InputType::TextMarkdown => "text/markdown",
        InputType::Directory => "directory",
    };

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
        "Read {} documents from {} (offset={}, limit={}, total_in_source={})",
        documents.len(),
        input_desc,
        config.offset,
        if config.limit > 0 {
            config.limit.to_string()
        } else {
            "none".to_string()
        },
        total_in_source
    ));

    job.total_documents = documents.len();

    // Step 2: Chunk documents
    let chunker_config = ChunkerConfig {
        chunk_size: config.chunk_size,
        chunk_overlap: config.chunk_overlap,
        ..ChunkerConfig::default()
    };

    // Create two chunkers: standard for text/parquet, heading-based for markdown
    let text_chunker = Chunker::new(chunker_config.clone());
    let md_chunker = Chunker::with_strategy(
        chunker_config,
        Arc::new(HeadingBoundaryChunking::new()),
    );

    let bar = progress.document_bar(documents.len() as u64, "Chunking documents");
    let mut all_chunks: Vec<PreparedChunk> = Vec::new();
    let mut doc_states: Vec<DocumentState> = Vec::new();

    for doc in &documents {
        bar.inc(1);

        // Idempotency: skip completed documents
        if state_mgr.is_document_completed(&doc.content_hash).await {
            continue;
        }

        // Select chunker based on file type
        let chunker_to_use = if is_markdown_file(&doc.filename) {
            &md_chunker
        } else {
            &text_chunker
        };

        // Chunk the document
        // Use async chunking for markdown files (to invoke HeadingBoundaryChunking strategy)
        // and sync chunking for text/parquet files
        let chunks: Vec<TextChunk> = if is_markdown_file(&doc.filename) {
            match chunker_to_use
                .chunk_async(&doc.content, &doc.content_hash)
                .await
            {
                Ok(chunks) => chunks,
                Err(e) => {
                    warn!(
                        filename = %doc.filename,
                        error = %e,
                        "Failed to chunk markdown document, falling back to text chunker"
                    );
                    // Fall back to text chunker on error
                    match text_chunker.chunk(&doc.content, &doc.content_hash) {
                        Ok(chunks) => chunks,
                        Err(e2) => {
                            warn!(
                                filename = %doc.filename,
                                error = %e2,
                                "Failed to chunk document, skipping"
                            );
                            doc_states.push(DocumentState {
                                content_hash: doc.content_hash.clone(),
                                filename: doc.filename.clone(),
                                status: DocumentStatus::Failed,
                                chunk_ids: Vec::new(),
                                entity_count: 0,
                                relationship_count: 0,
                                error: Some(e2.to_string()),
                            });
                            continue;
                        }
                    }
                }
            }
        } else {
            match chunker_to_use.chunk(&doc.content, &doc.content_hash) {
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
            }
        };

        let chunk_ids: Vec<String> = chunks
            .iter()
            .enumerate()
            .map(|(idx, _)| make_custom_id(&doc.content_hash, idx))
            .collect();

        // Build front matter metadata for this document (if available)
        let fm_metadata = lookup_front_matter(&doc.filename, &front_matter_map)
            .map(build_front_matter_metadata);

        // Create prepared chunks with custom IDs and optional metadata
        for (idx, chunk) in chunks.into_iter().enumerate() {
            let custom_id = make_custom_id(&doc.content_hash, idx);
            all_chunks.push(PreparedChunk {
                custom_id,
                chunk,
                doc_hash: doc.content_hash.clone(),
                doc_filename: doc.filename.clone(),
                metadata: fm_metadata.clone(),
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

    // Track document-level errors in prepare phase
    let prepare_errors: usize = doc_states
        .iter()
        .filter(|d| d.status == DocumentStatus::Failed)
        .count();
    if prepare_errors > 0 {
        job.errors_per_phase
            .insert("preparing".to_string(), prepare_errors);
    }

    // Step 3: Build JSONL files
    let spinner = progress.spinner("Building JSONL files...");
    let prompts = DomainExtractionPrompts::new(domain_config.clone());
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
        input_type = input_desc,
        "Phase 1: PREPARE complete"
    );

    Ok(PrepareResult {
        documents,
        chunks: all_chunks,
        jsonl_result,
    })
}
