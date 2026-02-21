//! Phase 2: Extract - Submit batch jobs, poll for completion, download and parse results.
//!
//! Batches are processed sequentially to stay within OpenAI's enqueued token limit.

use crate::config::BatchConfig;
use crate::domain_config::DomainConfig;
use crate::jsonl::PreparedChunk;
use crate::progress::BatchProgress;
use crate::state::{JobState, LatestRunStatus, Phase, StateManager};
use edgequake_llm::providers::openai_batch::OpenAIBatchClient;
use edgequake_pipeline::extractor::ExtractionResult;
use edgequake_pipeline::prompts::HybridExtractionParser;
use edgequake_pipeline::{EntityResolutionConfig, EntityResolver};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

/// Result of the extract phase.
pub struct ExtractResult {
    /// Extraction results keyed by custom_id.
    pub results: HashMap<String, ExtractionResult>,

    /// Number of successful extractions (with entities or relationships).
    pub success_count: usize,

    /// Number of empty extractions (no entities/relationships).
    pub empty_count: usize,

    /// Number of failed extractions (parsing errors).
    pub error_count: usize,
}

/// Create a batch job with automatic retry on token limit errors.
///
/// This function implements exponential backoff retry logic specifically for
/// the "token_limit_exceeded" error, which occurs when too many batches are
/// already queued in OpenAI's system. It will automatically wait and retry
/// up to max_attempts times.
async fn create_batch_with_retry(
    client: &OpenAIBatchClient,
    file_id: &str,
    model: &str,
    max_attempts: u32,
    initial_delay_secs: u64,
) -> anyhow::Result<edgequake_llm::providers::openai_batch::BatchJob> {
    let mut attempt = 0;
    let mut delay_secs = initial_delay_secs;

    loop {
        attempt += 1;

        match client.create_batch(file_id, model).await {
            Ok(batch) => return Ok(batch),
            Err(e) => {
                let error_str = e.to_string();
                let is_token_limit = error_str.contains("token_limit_exceeded")
                    || error_str.contains("Enqueued token limit reached");

                if is_token_limit && attempt < max_attempts {
                    warn!(
                        attempt = attempt,
                        max_attempts = max_attempts,
                        retry_in_secs = delay_secs,
                        "Token limit exceeded - waiting for in-progress batches to complete"
                    );

                    info!(
                        "⏳ Automatic retry {}/{} - sleeping for {} seconds...",
                        attempt, max_attempts, delay_secs
                    );

                    tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;

                    // Exponential backoff: double the delay each time, up to 30 minutes
                    delay_secs = (delay_secs * 2).min(1800);

                    info!(
                        "🔄 Retrying batch creation (attempt {}/{})",
                        attempt + 1,
                        max_attempts
                    );
                } else if is_token_limit {
                    return Err(anyhow::anyhow!(
                        "Token limit exceeded after {} retry attempts. In-progress batches are still consuming the 2M token limit. \
                        Use 'make batch-list' to check status. Wait longer or reduce batch size.",
                        max_attempts
                    ));
                } else {
                    // Non-token-limit error - fail immediately
                    return Err(e.into());
                }
            }
        }
    }
}

/// Run Phase 2: Extract.
///
/// Processes batch jobs sequentially (upload → create → poll → download → next)
/// to stay within OpenAI's enqueued token limit.
pub async fn run_extract(
    config: &BatchConfig,
    api_key: &str,
    domain_config: &DomainConfig,
    state_mgr: &StateManager,
    job: &mut JobState,
    jsonl_paths: &[PathBuf],
    _chunks: &[PreparedChunk],
    progress: &BatchProgress,
) -> anyhow::Result<ExtractResult> {
    info!("Phase 2: EXTRACT starting");
    info!(
        "Automatic retry enabled: Will retry up to {} times if token limits are hit (exponential backoff: {}s -> {}s -> ...)",
        config.max_retries,
        config.retry_delay_secs,
        config.retry_delay_secs * 2
    );
    job.phase = Phase::Extracting;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    // Record phase start time and write LATEST_RUN for UI visibility
    let now_millis = chrono::Utc::now().timestamp_millis();
    job.phase_started_at
        .insert("extracting".to_string(), now_millis);
    let run_started_at = job.run_started_at.unwrap_or(now_millis);

    let latest = LatestRunStatus {
        status: "extracting".to_string(),
        phase: Some("extracting".to_string()),
        job_id: Some(job.job_id.clone()),
        total_documents: Some(job.total_documents),
        processed_documents: Some(job.processed_documents),
        total_chunks: Some(job.total_chunks),
        total_batches: Some(jsonl_paths.len()),
        started_at: Some(run_started_at),
        updated_at: Some(now_millis),
        phase_started_at: Some(now_millis),
        ..Default::default()
    };
    state_mgr.write_latest_run(&latest).await?;

    let client = OpenAIBatchClient::new(api_key);
    let parser = HybridExtractionParser::new(true);
    let resolver = build_resolver(domain_config);

    let mut results: HashMap<String, ExtractionResult> = HashMap::new();
    let mut success_count = 0;
    let mut empty_count = 0;
    let mut error_count = 0;

    let bar = progress.document_bar(jsonl_paths.len() as u64, "Batch files");

    for (idx, path) in jsonl_paths.iter().enumerate() {
        info!(
            file = idx + 1,
            total = jsonl_paths.len(),
            path = %path.display(),
            "Processing batch file"
        );

        // Step 1: Upload JSONL file
        let file_id = client.upload_jsonl(path).await?;
        job.input_file_ids.push(file_id.clone());

        // Step 2: Create batch job (with automatic retry on token limit)
        let batch = create_batch_with_retry(
            &client,
            &file_id,
            &config.extraction_model,
            config.max_retries,
            config.retry_delay_secs,
        )
        .await?;
        let batch_id = batch.id.clone();
        job.batch_ids.push(batch.id);

        job.updated_at = chrono::Utc::now().to_rfc3339();
        state_mgr.save_job(job).await?;

        info!(batch_id = %batch_id, "Batch job created, polling for completion");

        // Step 3: Poll until this batch completes
        let output_file_id = poll_single_batch(&client, &batch_id).await?;

        // Step 4: Download and parse results for this batch
        if let Some(ref output_id) = output_file_id {
            job.output_file_ids.push(output_id.clone());

            let batch_results = client.download_results(output_id).await?;

            for batch_result in batch_results {
                let custom_id = &batch_result.custom_id;

                match OpenAIBatchClient::extract_content(&batch_result) {
                    Some(content) => match parser.parse(&content, custom_id) {
                        Ok(extraction) => {
                            // Apply entity resolution (from remote)
                            let extraction = resolver.resolve_extraction(extraction);

                            // Check if extraction is empty or has actual data (our changes)
                            let is_empty = extraction.entities.is_empty()
                                && extraction.relationships.is_empty();

                            let is_marked_empty = extraction
                                .metadata
                                .get("empty_response")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            if is_empty || is_marked_empty {
                                empty_count += 1;
                                if is_marked_empty {
                                    tracing::debug!(
                                        custom_id = %custom_id,
                                        "Empty response detected (LLM returned minimal content)"
                                    );
                                }
                            } else {
                                success_count += 1;
                            }

                            results.insert(custom_id.clone(), extraction);
                        }
                        Err(e) => {
                            warn!(
                                custom_id = %custom_id,
                                error = %e,
                                "Failed to parse extraction result"
                            );
                            error_count += 1;
                        }
                    },
                    None => {
                        warn!(
                            custom_id = %custom_id,
                            "No content in batch result (error or non-200 response)"
                        );
                        error_count += 1;
                    }
                }
            }

            info!(
                file = idx + 1,
                results_so_far = results.len(),
                "Batch file completed"
            );
        } else {
            warn!(
                batch_id = %batch_id,
                "Batch completed without output file"
            );
        }

        // Save state checkpoint after each batch
        job.updated_at = chrono::Utc::now().to_rfc3339();
        state_mgr.save_job(job).await?;

        bar.inc(1);
    }

    bar.finish_with_message(format!(
        "Processed {} batch files ({} results)",
        jsonl_paths.len(),
        results.len()
    ));

    // Update job state with extraction counts
    let total_entities: usize = results.values().map(|r| r.entities.len()).sum();
    let total_relationships: usize = results.values().map(|r| r.relationships.len()).sum();

    job.total_entities = total_entities;
    job.total_relationships = total_relationships;
    job.phase = Phase::Extracted;
    job.updated_at = chrono::Utc::now().to_rfc3339();

    // Track extraction errors per phase
    if error_count > 0 {
        job.errors_per_phase
            .insert("extracting".to_string(), error_count);
    }

    state_mgr.save_job(job).await?;

    info!(
        results = results.len(),
        entities = total_entities,
        relationships = total_relationships,
        success = success_count,
        empty = empty_count,
        errors = error_count,
        "Phase 2: EXTRACT complete"
    );

    // Log summary statistics
    if empty_count > 0 {
        info!(
            empty_count = empty_count,
            empty_pct = format!(
                "{:.1}%",
                (empty_count as f64 / results.len() as f64) * 100.0
            ),
            "Note: Some chunks produced empty extractions (no entities/relationships found)"
        );
    }

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
        empty_count,
        error_count,
    })
}

/// Poll a single batch job until it reaches a terminal state.
///
/// Returns the output_file_id if the batch completed successfully.
async fn poll_single_batch(
    client: &OpenAIBatchClient,
    batch_id: &str,
) -> anyhow::Result<Option<String>> {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

        let status = client.get_batch(batch_id).await?;

        if status.status.is_terminal() {
            if status.status.is_success() {
                info!(batch_id = %batch_id, "Batch completed successfully");
                return Ok(status.output_file_id);
            } else {
                let mut error_msg =
                    format!("Batch {} failed with status {:?}", batch_id, status.status);
                if let Some(errors) = &status.errors {
                    for err in &errors.data {
                        warn!(
                            code = ?err.code,
                            message = ?err.message,
                            "Batch error"
                        );
                        if let Some(msg) = &err.message {
                            error_msg = format!("{}: {}", error_msg, msg);
                        }
                    }
                }
                anyhow::bail!(error_msg);
            }
        }

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

/// Load cached extraction results from disk.
///
/// Returns None if the cache file doesn't exist or can't be parsed.
/// Applies entity resolution to normalize names and deduplicate entities.
pub fn load_cached_results(
    config: &BatchConfig,
    domain_config: &DomainConfig,
) -> Option<HashMap<String, ExtractionResult>> {
    let cache_path = config.work_dir.join("extract_results.json");
    match std::fs::read_to_string(&cache_path) {
        Ok(json) => match serde_json::from_str::<HashMap<String, ExtractionResult>>(&json) {
            Ok(mut results) => {
                info!(path = %cache_path.display(), "Loaded cached extraction results");
                let resolver = build_resolver(domain_config);
                for extraction in results.values_mut() {
                    let resolved = resolver.resolve_extraction(std::mem::take(extraction));
                    *extraction = resolved;
                }
                info!("Applied entity resolution to cached results");
                Some(results)
            }
            Err(e) => {
                warn!(error = %e, "Failed to parse cached extraction results");
                None
            }
        },
        Err(_) => {
            info!(
                "No cached extraction results found at {}",
                cache_path.display()
            );
            None
        }
    }
}

/// Build an EntityResolver from domain config aliases.
fn build_resolver(domain_config: &DomainConfig) -> EntityResolver {
    let mut resolver = EntityResolver::new(EntityResolutionConfig::default());
    resolver.add_aliases(domain_config.alias_pairs());
    resolver
}
