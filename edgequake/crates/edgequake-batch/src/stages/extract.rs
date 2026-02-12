//! Phase 2: Extract - Submit batch jobs, poll for completion, download and parse results.

use crate::config::BatchConfig;
use crate::jsonl::PreparedChunk;
use crate::progress::BatchProgress;
use crate::state::{JobState, Phase, StateManager};
use edgequake_llm::providers::openai_batch::{BatchStatus, OpenAIBatchClient};
use edgequake_pipeline::extractor::ExtractionResult;
use edgequake_pipeline::prompts::HybridExtractionParser;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

/// Result of the extract phase.
pub struct ExtractResult {
    /// Extraction results keyed by custom_id.
    pub results: HashMap<String, ExtractionResult>,

    /// Number of successful extractions.
    pub success_count: usize,

    /// Number of failed extractions.
    pub error_count: usize,
}

/// Run Phase 2: Extract.
///
/// 1. Upload JSONL files to OpenAI
/// 2. Create batch jobs
/// 3. Poll until all batches complete
/// 4. Download results
/// 5. Parse extraction tuples
pub async fn run_extract(
    config: &BatchConfig,
    api_key: &str,
    state_mgr: &StateManager,
    job: &mut JobState,
    jsonl_paths: &[PathBuf],
    chunks: &[PreparedChunk],
    progress: &BatchProgress,
) -> anyhow::Result<ExtractResult> {
    info!("Phase 2: EXTRACT starting");
    job.phase = Phase::Extracting;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    let client = OpenAIBatchClient::new(api_key);
    let parser = HybridExtractionParser::new(true);

    // Build chunk lookup map
    let chunk_map: HashMap<&str, &PreparedChunk> = chunks
        .iter()
        .map(|c| (c.custom_id.as_str(), c))
        .collect();

    // Step 1: Upload JSONL files and create batch jobs
    let bar = progress.document_bar(jsonl_paths.len() as u64, "Uploading batch files");
    let mut batch_ids: Vec<String> = Vec::new();

    for path in jsonl_paths {
        // Skip if we already have batch IDs from a previous run
        if batch_ids.len() >= jsonl_paths.len() {
            break;
        }

        let file_id = client.upload_jsonl(path).await?;
        job.input_file_ids.push(file_id.clone());

        let batch = client.create_batch(&file_id, &config.extraction_model).await?;
        batch_ids.push(batch.id.clone());
        job.batch_ids.push(batch.id);

        bar.inc(1);

        // Save state after each upload for crash recovery
        job.updated_at = chrono::Utc::now().to_rfc3339();
        state_mgr.save_job(job).await?;
    }
    bar.finish_with_message(format!("Submitted {} batch jobs", batch_ids.len()));

    // Step 2: Poll for completion
    let poll_bar = progress.batch_bar(batch_ids.len() as u64);
    let mut completed_count = 0;

    loop {
        let mut all_done = true;

        for batch_id in &batch_ids {
            let status = client.get_batch(batch_id).await?;

            if status.status.is_terminal() {
                if status.status.is_success() {
                    // Only count newly completed
                    if !job.output_file_ids.contains(
                        &status.output_file_id.clone().unwrap_or_default(),
                    ) {
                        if let Some(ref output_id) = status.output_file_id {
                            job.output_file_ids.push(output_id.clone());
                            completed_count += 1;
                            poll_bar.set_position(completed_count);
                        }
                    }
                } else {
                    warn!(
                        batch_id = %batch_id,
                        status = ?status.status,
                        "Batch job did not complete successfully"
                    );
                    if let Some(errors) = &status.errors {
                        for err in &errors.data {
                            warn!(
                                code = ?err.code,
                                message = ?err.message,
                                "Batch error"
                            );
                        }
                    }
                }
            } else {
                all_done = false;
                if let Some(counts) = &status.request_counts {
                    info!(
                        batch_id = %batch_id,
                        completed = counts.completed,
                        total = counts.total,
                        failed = counts.failed,
                        status = ?status.status,
                        "Batch progress"
                    );
                }
            }
        }

        if all_done {
            break;
        }

        // Poll interval: 30 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
    }

    poll_bar.finish_with_message("All batch jobs completed");

    // Save state checkpoint
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    // Step 3: Download and parse results
    let download_bar =
        progress.document_bar(job.output_file_ids.len() as u64, "Downloading results");
    let mut results: HashMap<String, ExtractionResult> = HashMap::new();
    let mut success_count = 0;
    let mut error_count = 0;

    for output_file_id in &job.output_file_ids {
        let batch_results = client.download_results(output_file_id).await?;

        for batch_result in batch_results {
            let custom_id = &batch_result.custom_id;

            match OpenAIBatchClient::extract_content(&batch_result) {
                Some(content) => {
                    // Parse the extraction tuples using HybridExtractionParser
                    match parser.parse(&content, custom_id) {
                        Ok(extraction) => {
                            results.insert(custom_id.clone(), extraction);
                            success_count += 1;
                        }
                        Err(e) => {
                            warn!(
                                custom_id = %custom_id,
                                error = %e,
                                "Failed to parse extraction result"
                            );
                            error_count += 1;
                        }
                    }
                }
                None => {
                    warn!(
                        custom_id = %custom_id,
                        "No content in batch result (error or non-200 response)"
                    );
                    error_count += 1;
                }
            }
        }

        download_bar.inc(1);
    }

    download_bar.finish_with_message(format!(
        "Parsed {} results ({} success, {} errors)",
        results.len(),
        success_count,
        error_count
    ));

    // Update job state with extraction counts
    let total_entities: usize = results.values().map(|r| r.entities.len()).sum();
    let total_relationships: usize = results.values().map(|r| r.relationships.len()).sum();

    job.total_entities = total_entities;
    job.total_relationships = total_relationships;
    job.phase = Phase::Extracted;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    info!(
        results = results.len(),
        entities = total_entities,
        relationships = total_relationships,
        success = success_count,
        errors = error_count,
        "Phase 2: EXTRACT complete"
    );

    // Cache results to disk for standalone embed/store commands
    let cache_path = config.work_dir.join("extract_results.json");
    if let Ok(json) = serde_json::to_string(&results) {
        if let Err(e) = std::fs::write(&cache_path, json) {
            warn!(error = %e, "Failed to cache extraction results to disk");
        } else {
            info!(path = %cache_path.display(), "Extraction results cached to disk");
        }
    }

    Ok(ExtractResult {
        results,
        success_count,
        error_count,
    })
}

/// Load cached extraction results from disk.
///
/// Returns None if the cache file doesn't exist or can't be parsed.
pub fn load_cached_results(
    config: &BatchConfig,
) -> Option<HashMap<String, ExtractionResult>> {
    let cache_path = config.work_dir.join("extract_results.json");
    match std::fs::read_to_string(&cache_path) {
        Ok(json) => match serde_json::from_str(&json) {
            Ok(results) => {
                info!(path = %cache_path.display(), "Loaded cached extraction results");
                Some(results)
            }
            Err(e) => {
                warn!(error = %e, "Failed to parse cached extraction results");
                None
            }
        },
        Err(_) => {
            info!("No cached extraction results found at {}", cache_path.display());
            None
        }
    }
}
