//! Phase 2: Extract - Submit batch jobs, poll for completion, download and parse results.
//!
//! Batches are processed sequentially to stay within OpenAI's enqueued token limit.

use crate::config::BatchConfig;
use crate::domain_config::DomainConfig;
use crate::jsonl::PreparedChunk;
use crate::progress::BatchProgress;
use crate::state::{JobState, Phase, StateManager};
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

    /// Number of successful extractions.
    pub success_count: usize,

    /// Number of failed extractions.
    pub error_count: usize,
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
    job.phase = Phase::Extracting;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    let client = OpenAIBatchClient::new(api_key);
    let parser = HybridExtractionParser::new(true);
    let resolver = build_resolver(domain_config);

    let mut results: HashMap<String, ExtractionResult> = HashMap::new();
    let mut success_count = 0;
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

        // Step 2: Create batch job
        let batch = client.create_batch(&file_id, &config.extraction_model).await?;
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
                            let extraction = resolver.resolve_extraction(extraction);
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
                let mut error_msg = format!("Batch {} failed with status {:?}", batch_id, status.status);
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
                    let resolved =
                        resolver.resolve_extraction(std::mem::take(extraction));
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
            info!("No cached extraction results found at {}", cache_path.display());
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
