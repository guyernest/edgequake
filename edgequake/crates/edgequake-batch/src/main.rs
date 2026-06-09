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
mod report;
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
    // Load .env file if present (before clap parses CLI args, so env vars are available)
    dotenvy::dotenv().ok();

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
            anyhow::anyhow!(
                "API key is required for list-batches (set OPENAI_API_KEY or ANTHROPIC_API_KEY)"
            )
        })?;
        return list_openai_batches(api_key, limit).await;
    }

    if let Command::Descriptor = cli.command {
        let ns_table = cli.registry_table.as_deref().unwrap_or(&cli.state_table);
        return generate_descriptor_command(
            &cli.namespace,
            ns_table,
            cli.neptune_endpoint.as_deref(),
            cli.vector_bucket.as_deref(),
            cli.athena_bm25_database.as_deref(),
            cli.bm25_s3_bucket.as_deref(),
            Some(cli.athena_workgroup.as_str()),
        )
        .await;
    }

    if let Command::SuggestSchema {
        sample_percentage: _,
        ref domain_hint,
        ref domain_description,
        ref expected_entity_types,
        ref expected_relationship_types,
        ref sample_budget,
    } = cli.command
    {
        let api_key = cli
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY is required for suggest-schema"))?;
        let suggest_model = cli.model.as_deref().unwrap_or("gpt-4.1-mini");
        let ns_table = cli.registry_table.as_deref().unwrap_or(&cli.state_table);

        let input = edgequake_schema::SuggestSchemaInput {
            domain_description: domain_description.clone(),
            expected_entity_types: expected_entity_types.clone(),
            expected_relationship_types: expected_relationship_types.clone(),
            domain_hint: domain_hint.clone(),
            sample_budget: *sample_budget,
            delimiter: cli.delimiter.clone(),
            skip_positional_extraction: false,
        };

        return handle_suggest_schema(
            &cli.data,
            api_key,
            suggest_model,
            &cli.namespace,
            ns_table,
            &input,
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
            | Command::Preview
    );

    // Initialize AWS config early (needed by resolver and state manager)
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

    // Resolve the namespace registry table: explicit --namespace-table, or fall back to --dynamo-table
    let registry_table = cli
        .registry_table
        .clone()
        .unwrap_or_else(|| cli.state_table.clone());

    // Resolve namespace config for commands that need it via NamespaceConfigResolver
    let (resolved_config, domain_config) = if needs_config_resolution {
        let namespace_slug = edgequake_core::NamespaceSlug::parse(&cli.namespace)?;
        let resolver = namespace_resolver::NamespaceConfigResolver::new(
            namespace_slug,
            aws_config.clone(),
            registry_table.clone(),
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
            let prompts =
                domain_prompts::DomainExtractionPrompts::new(resolved.domain_config.clone());
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
            state_table: cli.state_table,
            registry_table: registry_table.clone(),
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
            anthropic_api_key: cli.anthropic_key,
            embedding_dimension: resolved_config
                .as_ref()
                .map(|rc| rc.embedding_dimension)
                .unwrap_or(1536),
            snapshot_uri: cli.snapshot_uri.clone(),
            snapshot_mode: cli.snapshot_mode,
        };
        return dry_run::run_dry_run(
            &dry_run_config,
            resolved_config
                .as_ref()
                .expect("DryRun requires resolved config"),
        )
        .await;
    }

    // Handle Preview command early: after config resolution (needs schema) but before
    // state manager / job ID (doesn't need full pipeline infra).
    if let Command::Preview = cli.command {
        let resolved = resolved_config
            .as_ref()
            .expect("Preview requires resolved config");
        let preview_api_key = cli
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY is required for preview extraction"))?;

        return handle_preview_command(
            &cli.namespace,
            &cli.data,
            preview_api_key,
            &resolved.extraction_model,
            &domain_config,
            &registry_table,
            &aws_config,
            cli.delimiter.as_deref(),
        )
        .await;
    }

    // Determine which provider we're using based on extraction model
    let use_anthropic = {
        let model = if let Some(ref rc) = resolved_config {
            &rc.extraction_model
        } else {
            cli.model.as_deref().unwrap_or("gpt-4o-mini")
        };
        edgequake_llm::providers::anthropic_batch::is_anthropic_model(model)
    };

    // Pipeline commands require the appropriate API key
    let api_key = if use_anthropic {
        cli.anthropic_key
            .as_deref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "ANTHROPIC_API_KEY is required when using Claude models for extraction"
                )
            })?
            .to_string()
    } else {
        cli.api_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY is required for pipeline commands"))?
            .to_string()
    };

    // Generate or use provided job ID
    let job_id = cli
        .job_id
        .unwrap_or_else(|| format!("batch-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")));

    info!(job_id = %job_id, "EdgeQuake Batch Ingestion starting");

    // Resolve models from namespace config (or defaults for non-ingestion commands)
    let extraction_model = if let Some(ref rc) = resolved_config {
        rc.extraction_model.clone()
    } else {
        cli.model
            .clone()
            .unwrap_or_else(|| "gpt-4o-mini".to_string())
    };
    let embedding_model = if let Some(ref rc) = resolved_config {
        rc.embedding_model.clone()
    } else {
        cli.embedding_model
            .clone()
            .unwrap_or_else(|| "text-embedding-3-small".to_string())
    };
    let embedding_dimension = resolved_config
        .as_ref()
        .map(|rc| rc.embedding_dimension)
        .unwrap_or(1536);

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
        state_table: cli.state_table,
        registry_table: registry_table.clone(),
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
        anthropic_api_key: cli.anthropic_key.clone(),
        embedding_dimension,
        snapshot_uri: cli.snapshot_uri.clone(),
        snapshot_mode: cli.snapshot_mode,
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
        edgequake_storage_aws::DynamoKVConfig::new(&config.state_table, &config.namespace);
    let dynamo_kv = edgequake_storage_aws::DynamoKVStorage::new_with_client(
        dynamo_config,
        dynamo_client.clone(),
    );

    // Verify table exists before proceeding (gives clear error on misconfiguration)
    use edgequake_storage::KVStorage;
    dynamo_kv.initialize().await?;

    // LATEST_RUN writes go to the namespace registry table (PK=NS#{slug}, SK=LATEST_RUN),
    // which is separate from the KV state table.
    let state_mgr = StateManager::new(Box::new(dynamo_kv)).with_latest_run_config(
        dynamo_client.clone(),
        config.registry_table.clone(),
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
                let extract_results = stages::extract::load_cached_results(&config, &domain_config)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No cached extraction results found. Run 'extract' first, then 'embed'."
                        )
                    })?;

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
                let extract_results = stages::extract::load_cached_results(&config, &domain_config)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No cached extraction results found. Run 'extract' first, then 'store'."
                        )
                    })?;

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
            Command::Descriptor => {
                // Handled earlier before DynamoDB state initialization
                unreachable!()
            }
            Command::Preview => {
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

    // Phase 3: Embed (always uses OpenAI for embeddings)
    info!("[Embed] Starting...");
    let embed_api_key = if edgequake_llm::providers::anthropic_batch::is_anthropic_model(
        &config.extraction_model,
    ) {
        // When using Anthropic for extraction, embeddings still need OpenAI key
        // The `api_key` passed here is the Anthropic key, so we need the OpenAI key separately
        // Fall back to the extraction api_key if it looks like an OpenAI key, or error
        std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| api_key.to_string())
    } else {
        api_key.to_string()
    };
    let embedding_provider = create_embedding_provider(&embed_api_key, &config.embedding_model)?;
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

    // Post-store verification: wait for eventual consistency, then query counts
    info!("Waiting 3s for storage eventual consistency before verification...");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let verification = report::verify_stored_data(config, aws_config).await;

    // Generate and store MCP descriptor (best-effort -- failure doesn't fail pipeline)
    let descriptor = {
        let namespace_slug = edgequake_core::NamespaceSlug::parse(&config.namespace).ok();
        if let Some(ref slug) = namespace_slug {
            let dynamo_client = aws_sdk_dynamodb::Client::new(aws_config);
            let registry_config = edgequake_storage_aws::DynamoNamespaceConfig {
                table_name: config.registry_table.clone(),
            };
            let registry =
                edgequake_storage_aws::DynamoNamespaceRegistry::new(registry_config, dynamo_client);

            // Build InfrastructureConfig from batch config + AWS identity
            let account_id = match aws_sdk_sts::Client::new(aws_config)
                .get_caller_identity()
                .send()
                .await
            {
                Ok(identity) => identity.account().unwrap_or("unknown").to_string(),
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to get AWS account ID for descriptor");
                    "unknown".to_string()
                }
            };

            let infra = edgequake_core::InfrastructureConfig {
                neptune_endpoint: config.neptune_endpoint.clone().unwrap_or_default(),
                vector_bucket_name: config.vector_bucket.clone().unwrap_or_default(),
                dynamodb_table_name: config.registry_table.clone(),
                account_id,
                region: aws_config
                    .region()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "us-east-1".to_string()),
                environment: "dev".to_string(),
                external_id_suffix: {
                    use sha2::Digest;
                    let hash = sha2::Sha256::digest(config.namespace.as_bytes());
                    format!("{:02x}{:02x}{:02x}", hash[0], hash[1], hash[2])
                },
                athena_bm25_database: config.athena_bm25_database.clone(),
                bm25_s3_bucket: config.bm25_s3_bucket.clone(),
                athena_workgroup: Some(config.athena_workgroup.clone()),
            };

            match registry.generate_descriptor(slug, &infra).await {
                Ok(descriptor) => {
                    match registry.store_descriptor(slug, &descriptor).await {
                        Ok(()) => {
                            info!(
                                namespace = slug.as_str(),
                                "Generated and stored MCP descriptor"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to store MCP descriptor (non-fatal)");
                        }
                    }
                    Some(descriptor)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to generate MCP descriptor (non-fatal)");
                    None
                }
            }
        } else {
            None
        }
    };

    let pipeline_duration = pipeline_start.elapsed();

    // Print detailed final report
    report::print_final_report(
        job,
        &config.namespace,
        &verification,
        descriptor.as_ref(),
        pipeline_duration,
    );

    // Write success LATEST_RUN status
    let completion_millis = chrono::Utc::now().timestamp_millis();
    let total_error_count: usize = job.errors_per_phase.values().sum();
    let completed_status = if total_error_count > 0 {
        "completed_with_warnings"
    } else {
        "completed"
    };

    let success_status = state::LatestRunStatus {
        status: completed_status.to_string(),
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
        error_count_per_phase: if !job.errors_per_phase.is_empty() {
            Some(job.errors_per_phase.clone())
        } else {
            None
        },
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
    state_mgr.write_latest_run(&success_status).await.ok(); // best-effort

    info!(
        duration = ?pipeline_duration,
        status = completed_status,
        "Full batch pipeline completed!"
    );

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
                dimension: config.embedding_dimension as usize,
                namespace: config.namespace.clone(),
            };
            let s3v_client = edgequake_storage_aws::aws_sdk_s3vectors::Client::new(&aws_config);
            let storage: std::sync::Arc<dyn edgequake_storage::traits::VectorStorage> =
                std::sync::Arc::new(edgequake_storage_aws::S3VectorsStorage::new_with_client(
                    vectors_config,
                    s3v_client,
                ));
            // Initialize creates the bucket/index if they don't exist
            storage.initialize().await?;
            storage
        } else {
            info!("No vector bucket configured, using in-memory vector storage");
            std::sync::Arc::new(edgequake_storage::MemoryVectorStorage::new(
                &config.namespace,
                config.embedding_dimension as usize,
            ))
        };

    // KV storage: DynamoDB
    let dynamo_client = aws_sdk_dynamodb::Client::new(&aws_config);
    let dynamo_config =
        edgequake_storage_aws::DynamoKVConfig::new(&config.state_table, &config.namespace);
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
    let athena_config =
        edgequake_storage_aws::AthenaConfig::new(database.clone(), output_location.clone())
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

/// Generate and store the MCP descriptor for a namespace.
///
/// Lightweight command that only reads namespace config from DynamoDB,
/// builds the descriptor from infrastructure config, and writes it back.
/// No pipeline phases are executed.
async fn generate_descriptor_command(
    namespace: &str,
    registry_table: &str,
    neptune_endpoint: Option<&str>,
    vector_bucket: Option<&str>,
    athena_bm25_database: Option<&str>,
    bm25_s3_bucket: Option<&str>,
    athena_workgroup: Option<&str>,
) -> anyhow::Result<()> {
    let slug = edgequake_core::NamespaceSlug::parse(namespace)
        .map_err(|e| anyhow::anyhow!("Invalid namespace: {}", e))?;

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

    // Get AWS account ID
    let sts_client = aws_sdk_sts::Client::new(&aws_config);
    let identity = sts_client
        .get_caller_identity()
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get AWS identity: {}", e))?;
    let account_id = identity
        .account()
        .ok_or_else(|| anyhow::anyhow!("Could not determine AWS account ID"))?
        .to_string();

    let region = aws_config
        .region()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "us-east-1".to_string());

    let neptune = neptune_endpoint.ok_or_else(|| {
        anyhow::anyhow!("Neptune endpoint required (set NEPTUNE_ENDPOINT or --neptune-endpoint)")
    })?;

    let vectors = vector_bucket.ok_or_else(|| {
        anyhow::anyhow!("Vector bucket required (set VECTOR_BUCKET or --vector-bucket)")
    })?;

    let infra = edgequake_core::InfrastructureConfig {
        neptune_endpoint: neptune.to_string(),
        vector_bucket_name: vectors.to_string(),
        dynamodb_table_name: registry_table.to_string(),
        account_id,
        region: region.clone(),
        environment: "dev".to_string(),
        external_id_suffix: {
            use sha2::Digest;
            let hash = sha2::Sha256::digest(namespace.as_bytes());
            format!("{:02x}{:02x}{:02x}", hash[0], hash[1], hash[2])
        },
        athena_bm25_database: athena_bm25_database.map(|s| s.to_string()),
        bm25_s3_bucket: bm25_s3_bucket.map(|s| s.to_string()),
        athena_workgroup: athena_workgroup.map(|s| s.to_string()),
    };

    let dynamo_client = aws_sdk_dynamodb::Client::new(&aws_config);
    let registry_config = edgequake_storage_aws::DynamoNamespaceConfig {
        table_name: registry_table.to_string(),
    };
    let registry =
        edgequake_storage_aws::DynamoNamespaceRegistry::new(registry_config, dynamo_client);

    let descriptor = registry
        .generate_descriptor(&slug, &infra)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to generate descriptor: {}", e))?;

    registry
        .store_descriptor(&slug, &descriptor)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to store descriptor: {}", e))?;

    println!(
        "MCP descriptor generated and stored for namespace '{}'",
        namespace
    );
    println!();
    println!("--- Storage Endpoints ---");
    println!("Neptune:      {}", descriptor.storage.neptune.endpoint);
    println!("Label prefix: {}", descriptor.storage.neptune.label_prefix);
    println!(
        "S3 Vectors:   {}/{}",
        descriptor.storage.s3_vectors.bucket_name, descriptor.storage.s3_vectors.index_name
    );
    println!("DynamoDB:     {}", descriptor.storage.dynamodb.table_name);
    if let Some(ref bm25) = descriptor.storage.bm25 {
        println!("BM25 DB:      {}", bm25.database);
        println!("BM25 Bucket:  {}", bm25.s3_bucket);
        println!("BM25 WG:      {}", bm25.workgroup);
    }
    println!();
    println!("--- Authentication ---");
    println!("Role ARN:     {}", descriptor.auth.role_arn);
    println!("External ID:  {}", descriptor.auth.external_id);
    println!("Region:       {}", descriptor.auth.region);
    println!();
    println!("--- Tools ({}) ---", descriptor.tools.len());
    for tool in &descriptor.tools {
        println!("  - {}: {}", tool.name, tool.description);
    }

    Ok(())
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
/// Samples documents from the dataset using stratified sampling, analyzes them
/// with OpenAI to propose entity/relation types, and stores the proposal in DynamoDB.
async fn handle_suggest_schema(
    data: &std::path::Path,
    api_key: &str,
    model: &str,
    namespace: &str,
    registry_table: &str,
    input: &edgequake_schema::SuggestSchemaInput,
) -> anyhow::Result<()> {
    info!(
        data = %data.display(),
        namespace = namespace,
        domain_description = ?input.domain_description,
        domain_hint = ?input.domain_hint,
        sample_budget = ?input.sample_budget,
        "Starting schema suggestion"
    );

    // Determine location string: if it looks like s3://, pass as-is; otherwise use local path
    let location = data.to_string_lossy().to_string();

    // Build OpenAI config
    let openai_config = edgequake_schema::OpenAiConfig::new(api_key).with_model(model);

    // Run schema suggestion
    let proposal = edgequake_schema::suggest_schema(&location, input, &openai_config).await?;

    // Store in DynamoDB
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let dynamo_client = aws_sdk_dynamodb::Client::new(&aws_config);
    let registry_config = edgequake_storage_aws::DynamoNamespaceConfig {
        table_name: registry_table.to_string(),
    };
    let registry =
        edgequake_storage_aws::DynamoNamespaceRegistry::new(registry_config, dynamo_client);

    let slug = edgequake_core::NamespaceSlug::parse(namespace)?;

    // Use the inherent method directly (not the trait method) to avoid needing the trait import.
    // DynamoNamespaceRegistry has store_schema as both an inherent method and a trait implementation.
    registry
        .store_schema(&slug, &proposal)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to store schema proposal: {}", e))?;

    info!(
        namespace = namespace,
        entity_types = proposal.entity_types.len(),
        relation_types = proposal.relation_types.len(),
        sample_size = proposal.sample_size,
        total_documents = proposal.total_documents,
        "Schema proposed for namespace"
    );

    // Print proposal as formatted JSON for partner review
    println!("{}", serde_json::to_string_pretty(&proposal)?);

    Ok(())
}

/// Handle the preview extraction subcommand.
///
/// Reads the PREVIEW_REQUEST from DynamoDB, samples documents using stratified
/// sampling, runs extraction preview, and writes results back to DynamoDB.
/// Writes progress updates during processing (status="processing", documents_completed=N).
async fn handle_preview_command(
    namespace: &str,
    data: &std::path::Path,
    api_key: &str,
    extraction_model: &str,
    domain_config: &std::sync::Arc<domain_config::DomainConfig>,
    registry_table: &str,
    aws_config: &aws_config::SdkConfig,
    delimiter: Option<&str>,
) -> anyhow::Result<()> {
    info!(namespace = namespace, "Starting preview extraction");

    let dynamo_client = aws_sdk_dynamodb::Client::new(aws_config);
    let pk = format!("NS#{}", namespace);

    // Read the PREVIEW_REQUEST record to verify it exists and is "requested"
    let request_result = dynamo_client
        .get_item()
        .table_name(registry_table)
        .key("PK", aws_sdk_dynamodb::types::AttributeValue::S(pk.clone()))
        .key(
            "SK",
            aws_sdk_dynamodb::types::AttributeValue::S("PREVIEW_REQUEST".to_string()),
        )
        .send()
        .await?;

    if let Some(item) = request_result.item() {
        if let Some(data_attr) = item.get("data") {
            if let Ok(data_str) = data_attr.as_s() {
                let request: serde_json::Value = serde_json::from_str(data_str)?;
                let status = request["status"].as_str().unwrap_or("");
                if status != "requested" {
                    anyhow::bail!(
                        "Preview request status is '{}', expected 'requested'. Nothing to do.",
                        status
                    );
                }
            }
        }
    } else {
        anyhow::bail!("No PREVIEW_REQUEST found for namespace '{}'. Trigger a preview from the console first.", namespace);
    }

    // Helper to write progress updates to DynamoDB
    let write_preview_request = |client: aws_sdk_dynamodb::Client,
                                 table: String,
                                 pk_val: String,
                                 status: String,
                                 docs_completed: usize,
                                 docs_total: usize| {
        async move {
            let data = serde_json::json!({
                "namespace": namespace,
                "status": status,
                "documents_completed": docs_completed,
                "documents_total": docs_total,
            });
            client
                .put_item()
                .table_name(&table)
                .item("PK", aws_sdk_dynamodb::types::AttributeValue::S(pk_val))
                .item(
                    "SK",
                    aws_sdk_dynamodb::types::AttributeValue::S("PREVIEW_REQUEST".to_string()),
                )
                .item(
                    "data",
                    aws_sdk_dynamodb::types::AttributeValue::S(data.to_string()),
                )
                .send()
                .await
        }
    };

    // Update status to "processing"
    write_preview_request(
        dynamo_client.clone(),
        registry_table.to_string(),
        pk.clone(),
        "processing".to_string(),
        0,
        3,
    )
    .await
    .ok();

    // Sample documents using stratified sampling
    let location = data.to_string_lossy().to_string();
    let input = edgequake_schema::SuggestSchemaInput {
        sample_budget: Some(6),
        delimiter: delimiter.map(|s| s.to_string()),
        skip_positional_extraction: true, // Preview: let chunker + 20-chunk cap handle truncation
        ..Default::default()
    };

    let (dataset, _sampler_stats) =
        edgequake_schema::sampler::sample_documents_stratified(&location, &input).await?;

    let documents: Vec<(String, String, String)> = dataset
        .documents
        .into_iter()
        .take(6)
        .map(|d| (d.id, d.source, d.content))
        .collect();

    if documents.is_empty() {
        anyhow::bail!("No documents found in data path: {}", location);
    }

    info!(documents = documents.len(), "Sampled documents for preview");

    // Build entity types list from domain config
    let entity_types: Vec<String> = domain_config.entity_types.keys().cloned().collect();
    let relation_types: Vec<String> = domain_config
        .relationship_keywords
        .values()
        .flatten()
        .map(|k| k.keyword.clone())
        .collect();
    let language = &domain_config.domain.language;

    // Create LLM provider for synchronous (standard rate) extraction
    let llm_provider = std::sync::Arc::new(
        edgequake_llm::providers::openai::OpenAIProvider::new(api_key).with_model(extraction_model),
    );

    // Use smaller chunks for preview to get more diversity across the document.
    // Default is 1200 tokens; 512 tokens yields ~10 chunks per long doc,
    // which with 3 docs and the 20-chunk cap gives better coverage.
    let chunk_config = edgequake_pipeline::chunker::ChunkerConfig {
        chunk_size: 512,
        chunk_overlap: 50,
        min_chunk_size: 50,
        ..edgequake_pipeline::chunker::ChunkerConfig::default()
    };

    // Set up progress callback that writes to DynamoDB
    let progress_client = dynamo_client.clone();
    let progress_table = registry_table.to_string();
    let progress_pk = pk.clone();
    let progress_namespace = namespace.to_string();
    let on_document_complete: Option<Box<dyn Fn(usize, usize) + Send>> =
        Some(Box::new(move |completed, total| {
            let client = progress_client.clone();
            let table = progress_table.clone();
            let pk_val = progress_pk.clone();
            let ns = progress_namespace.clone();
            // Spawn a task to write progress (fire-and-forget from sync callback)
            tokio::spawn(async move {
                let data = serde_json::json!({
                    "namespace": ns,
                    "status": "processing",
                    "documents_completed": completed,
                    "documents_total": total,
                });
                let _ = client
                    .put_item()
                    .table_name(&table)
                    .item("PK", aws_sdk_dynamodb::types::AttributeValue::S(pk_val))
                    .item(
                        "SK",
                        aws_sdk_dynamodb::types::AttributeValue::S("PREVIEW_REQUEST".to_string()),
                    )
                    .item(
                        "data",
                        aws_sdk_dynamodb::types::AttributeValue::S(data.to_string()),
                    )
                    .send()
                    .await;
            });
        }));

    // Run preview extraction
    let result = match edgequake_pipeline::preview::preview_extraction(
        documents.clone(),
        &entity_types,
        &relation_types,
        language,
        llm_provider,
        &chunk_config,
        on_document_complete,
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            // Write failed status
            write_preview_request(
                dynamo_client.clone(),
                registry_table.to_string(),
                pk.clone(),
                "failed".to_string(),
                0,
                3,
            )
            .await
            .ok();
            return Err(anyhow::anyhow!("Preview extraction failed: {}", e));
        }
    };

    // Build the PREVIEW_RESULT document for DynamoDB
    // Shape matches the PreviewResult GraphQL type
    let entity_type_counts: Vec<serde_json::Value> = result
        .entity_type_counts
        .iter()
        .map(|(type_name, count)| serde_json::json!({ "typeName": type_name, "count": count }))
        .collect();

    let relation_type_counts: Vec<serde_json::Value> = result
        .relation_type_counts
        .iter()
        .map(|(type_name, count)| serde_json::json!({ "typeName": type_name, "count": count }))
        .collect();

    let coverage_rows: Vec<serde_json::Value> = result
        .coverage_matrix
        .iter()
        .map(|row| {
            serde_json::json!({
                "entityType": row.entity_type,
                "counts": row.counts,
            })
        })
        .collect();

    let document_columns: Vec<serde_json::Value> = result
        .documents
        .iter()
        .map(|doc| {
            let truncated = if doc.document_name.len() > 20 {
                format!("{}...", &doc.document_name[..17])
            } else {
                doc.document_name.clone()
            };
            serde_json::json!({
                "id": doc.document_id,
                "name": doc.document_name,
                "truncatedName": truncated,
            })
        })
        .collect();

    let preview_result_data = serde_json::json!({
        "status": "completed",
        "entityTypeCounts": entity_type_counts,
        "relationTypeCounts": relation_type_counts,
        "coverageRows": coverage_rows,
        "documentColumns": document_columns,
        "totalChunks": result.total_chunks,
        "totalEntities": result.total_entities,
        "totalRelationships": result.total_relationships,
        "cost": {
            "inputTokens": result.cost.input_tokens,
            "outputTokens": result.cost.output_tokens,
            "totalCostUsd": result.cost.total_cost_usd,
            "model": result.cost.model,
        },
        "processingTimeMs": result.processing_time_ms,
        "documentsCompleted": result.documents.len(),
        "documentsTotal": result.documents.len(),
    });

    // Write PREVIEW_RESULT to DynamoDB
    dynamo_client
        .put_item()
        .table_name(registry_table)
        .item("PK", aws_sdk_dynamodb::types::AttributeValue::S(pk.clone()))
        .item(
            "SK",
            aws_sdk_dynamodb::types::AttributeValue::S("PREVIEW_RESULT".to_string()),
        )
        .item(
            "data",
            aws_sdk_dynamodb::types::AttributeValue::S(preview_result_data.to_string()),
        )
        .send()
        .await?;

    // Update PREVIEW_REQUEST to "completed"
    write_preview_request(
        dynamo_client,
        registry_table.to_string(),
        pk,
        "completed".to_string(),
        result.documents.len(),
        result.documents.len(),
    )
    .await
    .ok();

    // Print summary
    println!("\n=== Preview Extraction Complete ===");
    println!("Documents:     {}", result.documents.len());
    println!("Total chunks:  {}", result.total_chunks);
    println!("Entities:      {}", result.total_entities);
    println!("Relationships: {}", result.total_relationships);
    println!(
        "Cost:          ${:.4} ({} input, {} output tokens)",
        result.cost.total_cost_usd, result.cost.input_tokens, result.cost.output_tokens
    );
    println!(
        "Time:          {:.1}s",
        result.processing_time_ms as f64 / 1000.0
    );
    println!();

    println!("Entity type counts:");
    for (type_name, count) in &result.entity_type_counts {
        let indicator = if *count == 0 { " (zero!)" } else { "" };
        println!("  {:30} {}{}", type_name, count, indicator);
    }
    println!();

    println!("Relationship type counts:");
    for (type_name, count) in &result.relation_type_counts {
        let indicator = if *count == 0 { " (zero!)" } else { "" };
        println!("  {:30} {}{}", type_name, count, indicator);
    }
    println!();

    println!(
        "Results written to DynamoDB (PREVIEW_RESULT). The console will show them automatically."
    );

    Ok(())
}
