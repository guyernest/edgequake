//! EdgeQuake Batch Ingestion CLI
//!
//! Builds a knowledge graph from large document datasets using the OpenAI
//! Batch API for 50% cost savings on entity extraction.
//!
//! ## Usage
//!
//! ```sh
//! # Full pipeline
//! edgequake-batch --namespace my-ns --data ./documents --api-key $OPENAI_API_KEY run
//!
//! # Individual phases
//! edgequake-batch --namespace my-ns --data ./documents --api-key $OPENAI_API_KEY prepare
//! edgequake-batch --namespace my-ns --data ./documents --api-key $OPENAI_API_KEY extract
//! edgequake-batch --namespace my-ns --data ./documents --api-key $OPENAI_API_KEY embed
//! edgequake-batch --namespace my-ns --data ./documents --api-key $OPENAI_API_KEY store
//!
//! # Check status
//! edgequake-batch --namespace my-ns --data ./documents --api-key $OPENAI_API_KEY --job-id <id> status
//! ```

mod cli;
mod config;
mod config_file;
pub mod directory_scanner;
pub mod document_reader;
pub mod document_splitter;
pub mod domain_config;
mod domain_prompts;
mod dry_run;
mod jsonl;
mod namespace_resolver;
mod parquet_reader;
mod progress;
mod stages;
mod state;
mod validation;

use clap::Parser;
use cli::{Cli, Command};
use config::BatchConfig;
use progress::BatchProgress;
use state::StateManager;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Handle commands that don't need full pipeline infrastructure early
    // (before BatchConfig consumes the CLI fields by move)
    if let Command::ListBatches { limit } = cli.command {
        let api_key = cli.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!("OPENAI_API_KEY is required for list-batches")
        })?;
        return list_openai_batches(api_key, limit).await;
    }

    if let Command::SuggestSchema {
        sample_percentage,
        ref domain_hint,
    } = cli.command
    {
        let api_key = cli.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!("OPENAI_API_KEY is required for suggest-schema")
        })?;
        let suggest_model = cli.model.as_deref().unwrap_or("gpt-4.1-mini");
        return handle_suggest_schema(
            &cli.data,
            api_key,
            suggest_model,
            &cli.namespace,
            &cli.dynamo_table,
            sample_percentage,
            domain_hint.as_deref(),
        )
        .await;
    }

    // Determine if this is a command that needs config resolution (schema/model info)
    let needs_config_resolution = matches!(
        cli.command,
        Command::Run
            | Command::Prepare
            | Command::Extract
            | Command::Embed
            | Command::Store
            | Command::Resume
            | Command::DryRun
    );

    // Initialize AWS config early (needed by resolver and state manager)
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

    // Resolve namespace config for commands that need it via NamespaceConfigResolver
    let (resolved_config, domain_config) = if needs_config_resolution {
        let namespace_slug = edgequake_core::NamespaceSlug::parse(&cli.namespace)?;
        let resolver = namespace_resolver::NamespaceConfigResolver::new(
            namespace_slug,
            aws_config.clone(),
            cli.dynamo_table.clone(),
        );
        let resolved = resolver.resolve(&cli).await?;

        // Print startup summary
        info!(
            namespace = %cli.namespace,
            model = %resolved.extraction_model,
            embedding_model = %resolved.embedding_model,
            "{}",
            resolved.schema_summary,
        );

        // In verbose mode, print full generated prompt
        if tracing::event_enabled!(tracing::Level::DEBUG) {
            let prompts = domain_prompts::DomainExtractionPrompts::new(resolved.domain_config.clone());
            tracing::debug!(prompt = %prompts.system_prompt(), "Generated extraction system prompt");
        }

        let dc = std::sync::Arc::new(resolved.domain_config.clone());
        (Some(resolved), dc)
    } else {
        // Non-ingestion commands (Status) don't need domain config.
        // Create a minimal placeholder that won't be used by the pipeline.
        let mut entity_types = indexmap::IndexMap::new();
        entity_types.insert("PLACEHOLDER".to_string(), "Unused".to_string());
        let dc = std::sync::Arc::new(domain_config::DomainConfig {
            domain: domain_config::DomainMetadata {
                name: "none".to_string(),
                description: "Placeholder for non-ingestion commands".to_string(),
                language: "English".to_string(),
            },
            entity_types,
            aliases: indexmap::IndexMap::new(),
            prompts: domain_config::PromptConfig {
                role_description: "Unused".to_string(),
                canonicalization_examples: vec![],
                extra_instructions: vec![],
                user_instructions: vec![],
            },
            relationship_keywords: indexmap::IndexMap::new(),
            examples: vec![],
        });
        (None, dc)
    };

    // Handle DryRun command early: after config resolution (needs schema) but before
    // state manager / job ID (doesn't need DynamoDB state or API key).
    if let Command::DryRun = cli.command {
        // Build a minimal BatchConfig for dry-run (no job_id, no work_dir needed)
        let dry_run_config = BatchConfig {
            job_id: "dry-run".to_string(),
            data_path: cli.data,
            work_dir: cli.work_dir,
            extraction_model: resolved_config
                .as_ref()
                .map(|rc| rc.extraction_model.clone())
                .unwrap_or_else(|| "gpt-4o-mini".to_string()),
            embedding_model: resolved_config
                .as_ref()
                .map(|rc| rc.embedding_model.clone())
                .unwrap_or_else(|| "text-embedding-3-small".to_string()),
            max_tokens: cli.max_tokens,
            chunk_size: cli.chunk_size,
            chunk_overlap: cli.chunk_overlap,
            embedding_batch_size: cli.embedding_batch_size,
            embedding_concurrency: cli.embedding_concurrency,
            dynamo_table: cli.dynamo_table,
            namespace: cli.namespace.clone(),
            neptune_endpoint: cli.neptune_endpoint,
            s3_bucket: cli.s3_bucket,
            neptune_role_arn: cli.neptune_role_arn,
            vector_bucket: cli.vector_bucket,
            vector_index: cli.vector_index,
            limit: cli.limit,
            offset: cli.offset,
            max_retries: cli.max_retries,
            retry_delay_secs: cli.retry_delay_secs,
            athena_bm25_database: cli.athena_bm25_database,
            bm25_s3_bucket: cli.bm25_s3_bucket,
            athena_workgroup: cli.athena_workgroup,
            athena_output_location: cli.athena_output_location,
            delimiter: cli.delimiter,
            no_split: cli.no_split,
            no_recurse: cli.no_recurse,
            include_patterns: cli.include,
            exclude_patterns: cli.exclude,
            max_failures: cli.max_failures,
        };
        return dry_run::run_dry_run(
            &dry_run_config,
            resolved_config
                .as_ref()
                .expect("DryRun requires resolved config"),
        )
        .await;
    }

    // Pipeline commands require an API key
    let api_key = cli
        .api_key
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY is required for pipeline commands"))?
        .to_string();

    // Generate or use provided job ID
    let job_id = cli
        .job_id
        .unwrap_or_else(|| format!("batch-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")));

    info!(job_id = %job_id, "EdgeQuake Batch Ingestion starting");

    // Resolve models from namespace config (or defaults for non-ingestion commands)
    let extraction_model = if let Some(ref rc) = resolved_config {
        rc.extraction_model.clone()
    } else {
        cli.model.clone().unwrap_or_else(|| "gpt-4o-mini".to_string())
    };
    let embedding_model = if let Some(ref rc) = resolved_config {
        rc.embedding_model.clone()
    } else {
        cli.embedding_model.clone().unwrap_or_else(|| "text-embedding-3-small".to_string())
    };

    // Build config from CLI args + resolved namespace config
    let config = BatchConfig {
        job_id: job_id.clone(),
        data_path: cli.data,
        work_dir: cli.work_dir,
        extraction_model,
        embedding_model,
        max_tokens: cli.max_tokens,
        chunk_size: cli.chunk_size,
        chunk_overlap: cli.chunk_overlap,
        embedding_batch_size: cli.embedding_batch_size,
        embedding_concurrency: cli.embedding_concurrency,
        dynamo_table: cli.dynamo_table,
        namespace: cli.namespace.clone(),
        neptune_endpoint: cli.neptune_endpoint.clone(),
        s3_bucket: cli.s3_bucket.clone(),
        neptune_role_arn: cli.neptune_role_arn.clone(),
        vector_bucket: cli.vector_bucket.clone(),
        vector_index: cli.vector_index.clone(),
        limit: cli.limit,
        offset: cli.offset,
        max_retries: cli.max_retries,
        retry_delay_secs: cli.retry_delay_secs,
        athena_bm25_database: cli.athena_bm25_database.clone(),
        bm25_s3_bucket: cli.bm25_s3_bucket.clone(),
        athena_workgroup: cli.athena_workgroup.clone(),
        athena_output_location: cli.athena_output_location.clone(),
        delimiter: cli.delimiter.clone(),
        no_split: cli.no_split,
        no_recurse: cli.no_recurse,
        include_patterns: cli.include.clone(),
        exclude_patterns: cli.exclude.clone(),
        max_failures: cli.max_failures,
    };

    // Upfront validation: check all preconditions before any processing
    let is_ingestion_command = matches!(
        cli.command,
        Command::Run
            | Command::Prepare
            | Command::Extract
            | Command::Embed
            | Command::Store
            | Command::Resume
    );
    if is_ingestion_command {
        validation::validate_upfront(&config, &api_key).await?;
    }

    // Create work directory
    std::fs::create_dir_all(&config.work_dir)?;

    // Initialize state manager with DynamoDB
    let dynamo_client = aws_sdk_dynamodb::Client::new(&aws_config);

    let dynamo_config =
        edgequake_storage_aws::DynamoKVConfig::new(&config.dynamo_table, &config.namespace);
    let dynamo_kv =
        edgequake_storage_aws::DynamoKVStorage::new_with_client(dynamo_config, dynamo_client.clone());

    // Verify table exists before proceeding (gives clear error on misconfiguration)
    use edgequake_storage::KVStorage;
    dynamo_kv.initialize().await?;

    // The namespace registry table is the same DynamoDB table used for KV storage
    // (single-table design). LATEST_RUN writes go to PK=NS#{slug}, SK=LATEST_RUN.
    let state_mgr = StateManager::new(Box::new(dynamo_kv)).with_latest_run_config(
        dynamo_client.clone(),
        config.dynamo_table.clone(),
        cli.namespace.clone(),
    );
    let progress = BatchProgress::new();

    // Load or create job state
    let mut job = match state_mgr.load_job(&job_id).await? {
        Some(existing) => {
            info!(phase = %existing.phase, "Resuming existing job");
            existing
        }
        None => {
            let job = StateManager::create_job(&job_id);
            state_mgr.save_job(&job).await?;
            job
        }
    };

    // Execute pipeline command, catching fatal errors to write LATEST_RUN "failed" status.
    let pipeline_result: anyhow::Result<()> = async {
        match cli.command {
            Command::Run => {
                run_full_pipeline(
                    &config,
                    &api_key,
                    &aws_config,
                    &domain_config,
                    &state_mgr,
                    &mut job,
                    &progress,
                )
                .await?;
            }
            Command::Prepare => {
                stages::prepare::run_prepare(
                    &config,
                    &domain_config,
                    &state_mgr,
                    &mut job,
                    &progress,
                )
                .await?;
            }
            Command::Extract => {
                // Extract requires prepare results; load from state
                let prepare_result = stages::prepare::run_prepare(
                    &config,
                    &domain_config,
                    &state_mgr,
                    &mut job,
                    &progress,
                )
                .await?;
                stages::extract::run_extract(
                    &config,
                    &api_key,
                    &domain_config,
                    &state_mgr,
                    &mut job,
                    &prepare_result.jsonl_result.file_paths,
                    &prepare_result.chunks,
                    &progress,
                )
                .await?;
            }
            Command::Embed => {
                // Re-run prepare (fast, local only) to get chunks in memory
                let prepare_result = stages::prepare::run_prepare(
                    &config,
                    &domain_config,
                    &state_mgr,
                    &mut job,
                    &progress,
                )
                .await?;

                // Load cached extraction results from disk (avoids re-submitting to OpenAI)
                let extract_results =
                    stages::extract::load_cached_results(&config, &domain_config).ok_or_else(
                        || {
                            anyhow::anyhow!(
                            "No cached extraction results found. Run 'extract' first, then 'embed'."
                        )
                        },
                    )?;

                let embedding_provider =
                    create_embedding_provider(&api_key, &config.embedding_model)?;
                stages::embed::run_embed(
                    &config,
                    embedding_provider,
                    &state_mgr,
                    &mut job,
                    &prepare_result.chunks,
                    &extract_results,
                    &progress,
                )
                .await?;
            }
            Command::Store => {
                // Re-run prepare (fast, local only) to get chunks/documents in memory
                let prepare_result = stages::prepare::run_prepare(
                    &config,
                    &domain_config,
                    &state_mgr,
                    &mut job,
                    &progress,
                )
                .await?;

                // Load cached extraction results from disk
                let extract_results =
                    stages::extract::load_cached_results(&config, &domain_config).ok_or_else(
                        || {
                            anyhow::anyhow!(
                            "No cached extraction results found. Run 'extract' first, then 'store'."
                        )
                        },
                    )?;

                // Load cached embed results, or re-run embed if not cached
                let embed_result =
                    if let Some(cached) = stages::embed::load_cached_embed_results(&config) {
                        info!("Using cached embed results");
                        cached
                    } else {
                        info!("No cached embed results, running embed phase");
                        let embedding_provider =
                            create_embedding_provider(&api_key, &config.embedding_model)?;
                        stages::embed::run_embed(
                            &config,
                            embedding_provider,
                            &state_mgr,
                            &mut job,
                            &prepare_result.chunks,
                            &extract_results,
                            &progress,
                        )
                        .await?
                    };

                // Run store to persist to Neptune/S3/DynamoDB/BM25
                let (vectors, kv) = create_storage_backends(&config).await?;
                let bm25 = create_bm25_storage(&config).await?;
                stages::store::run_store(
                    &config,
                    &aws_config,
                    vectors,
                    kv,
                    bm25,
                    &state_mgr,
                    &mut job,
                    &prepare_result.documents,
                    &prepare_result.chunks,
                    &extract_results,
                    &embed_result,
                    &progress,
                )
                .await?;
            }
            Command::Status => {
                print_status(&job);
            }
            Command::Resume => {
                info!(phase = %job.phase, "Resuming from checkpoint");
                run_full_pipeline(
                    &config,
                    &api_key,
                    &aws_config,
                    &domain_config,
                    &state_mgr,
                    &mut job,
                    &progress,
                )
                .await?;
            }
            Command::ListBatches { .. } => {
                // Handled earlier before DynamoDB initialization
                unreachable!()
            }
            Command::SuggestSchema { .. } => {
                // Handled earlier before DynamoDB state initialization
                unreachable!()
            }
            Command::DryRun => {
                // Handled earlier after config resolution
                unreachable!()
            }
        }
        Ok(())
    }
    .await;

    // On fatal pipeline failure, write LATEST_RUN with status "failed" so the UI
    // shows a red status badge with the top-level error message.
    if let Err(ref e) = pipeline_result {
        let error_msg = format!("{}", e);
        tracing::error!(error = %error_msg, "Pipeline failed");

        let failed_status = state::LatestRunStatus {
            status: "failed".to_string(),
            phase: Some(job.phase.to_string()),
            job_id: Some(job.job_id.clone()),
            total_documents: Some(job.total_documents),
            processed_documents: Some(job.processed_documents),
            total_chunks: Some(job.total_chunks),
            started_at: job.run_started_at,
            updated_at: Some(chrono::Utc::now().timestamp_millis()),
            error_summary: Some(error_msg),
            error_count_per_phase: if !job.errors_per_phase.is_empty() {
                Some(job.errors_per_phase.clone())
            } else {
                None
            },
            ..Default::default()
        };

        // Best-effort write: if this fails too, we still want the original error
        if let Err(write_err) = state_mgr.write_latest_run(&failed_status).await {
            tracing::error!(error = %write_err, "Failed to write LATEST_RUN failure status");
        }
    }

    pipeline_result
}

/// Run the full 4-phase pipeline.
async fn run_full_pipeline(
    config: &BatchConfig,
    api_key: &str,
    aws_config: &aws_config::SdkConfig,
    domain_config: &std::sync::Arc<domain_config::DomainConfig>,
    state_mgr: &StateManager,
    job: &mut state::JobState,
    progress: &BatchProgress,
) -> anyhow::Result<()> {
    let pipeline_start = std::time::Instant::now();
    info!("Running full batch pipeline");

    // Helper: check cumulative failure threshold after each stage
    let check_failures = |job: &state::JobState, stage_name: &str| -> anyhow::Result<()> {
        let total_failures: usize = job.errors_per_phase.values().sum();
        if config.max_failures > 0 && total_failures > config.max_failures {
            anyhow::bail!(
                "Failure threshold exceeded: {} total failures (max: {}). Pipeline aborted after {} stage.",
                total_failures,
                config.max_failures,
                stage_name
            );
        }
        Ok(())
    };

    // Phase 1: Prepare
    info!("[Prepare] Starting...");
    let prepare_result = if matches!(job.phase, state::Phase::Pending | state::Phase::Preparing) {
        stages::prepare::run_prepare(config, domain_config, state_mgr, job, progress).await?
    } else {
        info!("Phase 1 already complete, re-running for data");
        stages::prepare::run_prepare(config, domain_config, state_mgr, job, progress).await?
    };
    check_failures(job, "prepare")?;

    // Phase 2: Extract
    info!("[Extract] Starting...");
    let extract_result = if matches!(job.phase, state::Phase::Prepared | state::Phase::Extracting) {
        stages::extract::run_extract(
            config,
            api_key,
            domain_config,
            state_mgr,
            job,
            &prepare_result.jsonl_result.file_paths,
            &prepare_result.chunks,
            progress,
        )
        .await?
    } else if matches!(
        job.phase,
        state::Phase::Extracted
            | state::Phase::Embedding
            | state::Phase::Embedded
            | state::Phase::Storing
    ) {
        info!("Phase 2 already complete, skipping extract (results need to be re-derived)");
        // For resume, we'd need to re-derive from stored state
        // For now, re-run extract
        stages::extract::run_extract(
            config,
            api_key,
            domain_config,
            state_mgr,
            job,
            &prepare_result.jsonl_result.file_paths,
            &prepare_result.chunks,
            progress,
        )
        .await?
    } else {
        stages::extract::run_extract(
            config,
            api_key,
            domain_config,
            state_mgr,
            job,
            &prepare_result.jsonl_result.file_paths,
            &prepare_result.chunks,
            progress,
        )
        .await?
    };
    check_failures(job, "extract")?;

    // Phase 3: Embed
    info!("[Embed] Starting...");
    let embedding_provider = create_embedding_provider(api_key, &config.embedding_model)?;
    let embed_result = stages::embed::run_embed(
        config,
        embedding_provider,
        state_mgr,
        job,
        &prepare_result.chunks,
        &extract_result.results,
        progress,
    )
    .await?;
    check_failures(job, "embed")?;

    // Phase 4: Store
    info!("[Store] Starting...");
    let (vectors, kv) = create_storage_backends(config).await?;
    let bm25 = create_bm25_storage(config).await?;
    stages::store::run_store(
        config,
        aws_config,
        vectors,
        kv,
        bm25,
        state_mgr,
        job,
        &prepare_result.documents,
        &prepare_result.chunks,
        &extract_result.results,
        &embed_result,
        progress,
    )
    .await?;

    let pipeline_duration = pipeline_start.elapsed();
    info!(
        duration = ?pipeline_duration,
        "Full batch pipeline completed successfully!"
    );
    print_status(job);

    Ok(())
}

/// Create an embedding provider from the API key and model.
fn create_embedding_provider(
    api_key: &str,
    model: &str,
) -> anyhow::Result<std::sync::Arc<dyn edgequake_llm::traits::EmbeddingProvider>> {
    let provider =
        edgequake_llm::providers::openai::OpenAIProvider::new(api_key).with_embedding_model(model);
    Ok(std::sync::Arc::new(provider))
}

/// Create storage backends based on configuration.
///
/// Graph storage (Neptune) is now handled by bulk_load directly, so this only
/// creates vector and KV storage backends.
async fn create_storage_backends(
    config: &BatchConfig,
) -> anyhow::Result<(
    std::sync::Arc<dyn edgequake_storage::traits::VectorStorage>,
    std::sync::Arc<dyn edgequake_storage::traits::KVStorage>,
)> {
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

    // Vector storage: S3 Vectors if configured, otherwise in-memory
    let vectors: std::sync::Arc<dyn edgequake_storage::traits::VectorStorage> =
        if let Some(ref bucket) = config.vector_bucket {
            let index_name = config
                .vector_index
                .clone()
                .unwrap_or_else(|| format!("{}-embeddings", config.namespace));
            let vectors_config = edgequake_storage_aws::S3VectorsConfig {
                vector_bucket_name: bucket.clone(),
                index_name,
                dimension: 1536,
                namespace: config.namespace.clone(),
            };
            let s3v_client = edgequake_storage_aws::aws_sdk_s3vectors::Client::new(&aws_config);
            std::sync::Arc::new(edgequake_storage_aws::S3VectorsStorage::new_with_client(
                vectors_config,
                s3v_client,
            ))
        } else {
            info!("No vector bucket configured, using in-memory vector storage");
            std::sync::Arc::new(edgequake_storage::MemoryVectorStorage::new(
                &config.namespace,
                1536,
            ))
        };

    // KV storage: DynamoDB
    let dynamo_client = aws_sdk_dynamodb::Client::new(&aws_config);
    let dynamo_config =
        edgequake_storage_aws::DynamoKVConfig::new(&config.dynamo_table, &config.namespace);
    let kv: std::sync::Arc<dyn edgequake_storage::traits::KVStorage> = std::sync::Arc::new(
        edgequake_storage_aws::DynamoKVStorage::new_with_client(dynamo_config, dynamo_client),
    );

    Ok((vectors, kv))
}

/// Create BM25 storage backend if Athena BM25 configuration is present.
///
/// Returns None when env vars are absent -- BM25 indexing is skipped.
/// Returns Some when ATHENA_BM25_DATABASE is set.
async fn create_bm25_storage(
    config: &BatchConfig,
) -> anyhow::Result<Option<std::sync::Arc<dyn edgequake_storage::Bm25Storage>>> {
    let Some(ref database) = config.athena_bm25_database else {
        info!("ATHENA_BM25_DATABASE not set, BM25 indexing disabled");
        return Ok(None);
    };

    let bm25_bucket = config.bm25_s3_bucket.as_ref().ok_or_else(|| {
        anyhow::anyhow!("BM25_S3_BUCKET is required when ATHENA_BM25_DATABASE is set")
    })?;

    let output_location = config.athena_output_location.as_ref().ok_or_else(|| {
        anyhow::anyhow!("ATHENA_OUTPUT_LOCATION is required when ATHENA_BM25_DATABASE is set")
    })?;

    // Create AthenaQueryEngine (reuses existing athena_sql module)
    let athena_config = edgequake_storage_aws::AthenaConfig::new(
        database.clone(),
        output_location.clone(),
    )
    .with_workgroup(config.athena_workgroup.clone());

    let athena_engine = edgequake_storage_aws::AthenaQueryEngine::new(athena_config).await?;

    // Create S3 client for Parquet staging
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let s3_client = aws_sdk_s3::Client::new(&aws_config);

    // Create AthenaBm25Config
    let bm25_config = edgequake_storage_aws::AthenaBm25Config {
        database: database.clone(),
        s3_bucket: bm25_bucket.clone(),
        s3_prefix: "bm25".to_string(),
        workgroup: config.athena_workgroup.clone(),
        output_location: output_location.clone(),
    };

    let bm25_storage = edgequake_storage_aws::AthenaBm25Storage::new(
        bm25_config,
        config.namespace.clone(),
        std::sync::Arc::new(athena_engine),
        s3_client,
    );

    info!(database = %database, bucket = %bm25_bucket, "BM25 storage configured");
    Ok(Some(
        std::sync::Arc::new(bm25_storage) as std::sync::Arc<dyn edgequake_storage::Bm25Storage>
    ))
}

/// Print job status to terminal.
fn print_status(job: &state::JobState) {
    println!("\n=== Batch Job Status ===");
    println!("Job ID:          {}", job.job_id);
    println!("Phase:           {}", job.phase);
    println!("Documents:       {}", job.total_documents);
    println!("Chunks:          {}", job.total_chunks);
    println!("Batch jobs:      {}", job.batch_ids.len());
    println!("Entities:        {}", job.total_entities);
    println!("Relationships:   {}", job.total_relationships);
    println!("Embeddings:      {}", job.total_embeddings);
    println!("Created:         {}", job.created_at);
    println!("Updated:         {}", job.updated_at);
    if let Some(ref error) = job.error {
        println!("Error:           {}", error);
    }
    println!("========================\n");
}

/// List all OpenAI batch jobs to identify what's consuming token limits.
async fn list_openai_batches(api_key: &str, limit: usize) -> anyhow::Result<()> {
    use edgequake_llm::providers::openai_batch::{BatchStatus, OpenAIBatchClient};

    let client = OpenAIBatchClient::new(api_key);
    let batches = client.list_batches(Some(limit)).await?;

    if batches.is_empty() {
        println!("\n=== No batch jobs found in your OpenAI organization ===\n");
        return Ok(());
    }

    println!(
        "\n=== OpenAI Batch Jobs (showing {} most recent) ===",
        batches.len()
    );
    println!();

    let mut in_progress_count = 0;
    let mut validating_count = 0;
    let mut total_in_progress_requests = 0;

    for batch in &batches {
        // Count in-progress batches for summary
        match batch.status {
            BatchStatus::InProgress | BatchStatus::Finalizing => {
                in_progress_count += 1;
                if let Some(ref counts) = batch.request_counts {
                    total_in_progress_requests += counts.total - counts.completed;
                }
            }
            BatchStatus::Validating => {
                validating_count += 1;
                if let Some(ref counts) = batch.request_counts {
                    total_in_progress_requests += counts.total;
                }
            }
            _ => {}
        }

        // Format status with emoji
        let status_display = match batch.status {
            BatchStatus::InProgress => "🔄 In Progress",
            BatchStatus::Validating => "⏳ Validating",
            BatchStatus::Completed => "✅ Completed",
            BatchStatus::Failed => "❌ Failed",
            BatchStatus::Expired => "⏰ Expired",
            BatchStatus::Cancelled => "🚫 Cancelled",
            BatchStatus::Finalizing => "🔄 Finalizing",
            _ => "❓ Unknown",
        };

        println!("Batch ID:     {}", batch.id);
        println!("Status:       {}", status_display);

        if let Some(ref counts) = batch.request_counts {
            let progress = if counts.total > 0 {
                counts.completed as f64 / counts.total as f64 * 100.0
            } else {
                0.0
            };
            println!(
                "Progress:     {}/{} ({:.1}%)",
                counts.completed, counts.total, progress
            );
            if counts.failed > 0 {
                println!("Failed:       {}", counts.failed);
            }
        }

        // Show creation time
        let created = chrono::DateTime::from_timestamp(batch.created_at, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        println!("Created:      {}", created);

        // Show completion/failure time if available
        if let Some(completed_at) = batch.completed_at {
            let completed = chrono::DateTime::from_timestamp(completed_at, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            println!("Completed:    {}", completed);
        }
        if let Some(failed_at) = batch.failed_at {
            let failed = chrono::DateTime::from_timestamp(failed_at, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            println!("Failed:       {}", failed);
        }

        // Show errors if present
        if let Some(ref errors) = batch.errors {
            for error in &errors.data {
                if let Some(ref msg) = error.message {
                    println!("Error:        {}", msg);
                }
            }
        }

        println!();
    }

    // Print summary
    println!("=== Summary ===");
    println!("Total batches shown:     {}", batches.len());
    println!("In progress/finalizing:  {}", in_progress_count);
    println!("Validating:              {}", validating_count);
    if total_in_progress_requests > 0 {
        println!("Pending requests:        {}", total_in_progress_requests);
        println!();
        println!("💡 Tip: These pending requests are consuming your 2M token limit.");
        println!("   Wait for them to complete before submitting new batches.");
    }
    println!("====================\n");

    Ok(())
}

/// Handle the suggest-schema subcommand.
///
/// Samples documents from the dataset, analyzes them with OpenAI to propose
/// entity/relation types, and stores the proposal in DynamoDB.
async fn handle_suggest_schema(
    data: &std::path::Path,
    api_key: &str,
    model: &str,
    namespace: &str,
    dynamo_table: &str,
    sample_percentage: f64,
    domain_hint: Option<&str>,
) -> anyhow::Result<()> {
    info!(
        data = %data.display(),
        namespace = namespace,
        sample_percentage = sample_percentage,
        domain_hint = domain_hint,
        "Starting schema suggestion"
    );

    // Determine location string: if it looks like s3://, pass as-is; otherwise use local path
    let location = data.to_string_lossy().to_string();

    // Build OpenAI config
    let openai_config = edgequake_schema::OpenAiConfig::new(api_key).with_model(model);

    // Run schema suggestion
    let proposal = edgequake_schema::suggest_schema(
        &location,
        sample_percentage,
        domain_hint,
        &openai_config,
    )
    .await?;

    // Store in DynamoDB
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let dynamo_client = aws_sdk_dynamodb::Client::new(&aws_config);
    let registry_config = edgequake_storage_aws::DynamoNamespaceConfig {
        table_name: dynamo_table.to_string(),
    };
    let registry = edgequake_storage_aws::DynamoNamespaceRegistry::new(registry_config, dynamo_client);

    let slug = edgequake_core::NamespaceSlug::parse(namespace)?;

    // Use the inherent method directly (not the trait method) to avoid needing the trait import.
    // DynamoNamespaceRegistry has store_schema as both an inherent method and a trait implementation.
    registry.store_schema(&slug, &proposal).await.map_err(|e| {
        anyhow::anyhow!("Failed to store schema proposal: {}", e)
    })?;

    info!(
        namespace = namespace,
        entity_types = proposal.entity_types.len(),
        relation_types = proposal.relation_types.len(),
        sample_size = proposal.sample_size,
        total_documents = proposal.total_documents,
        "Schema proposed for namespace"
    );

    // Print proposal as formatted JSON for partner review
    println!(
        "{}",
        serde_json::to_string_pretty(&proposal)?
    );

    Ok(())
}
