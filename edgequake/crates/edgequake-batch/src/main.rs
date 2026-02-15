//! EdgeQuake Batch Ingestion CLI
//!
//! Builds a knowledge graph from large document datasets using the OpenAI
//! Batch API for 50% cost savings on entity extraction.
//!
//! ## Usage
//!
//! ```sh
//! # Full pipeline
//! edgequake-batch --data data/epstein/0000.parquet --api-key $OPENAI_API_KEY run
//!
//! # Individual phases
//! edgequake-batch --data data/epstein/0000.parquet --api-key $OPENAI_API_KEY prepare
//! edgequake-batch --data data/epstein/0000.parquet --api-key $OPENAI_API_KEY extract
//! edgequake-batch --data data/epstein/0000.parquet --api-key $OPENAI_API_KEY embed
//! edgequake-batch --data data/epstein/0000.parquet --api-key $OPENAI_API_KEY store
//!
//! # Check status
//! edgequake-batch --data data/epstein/0000.parquet --api-key $OPENAI_API_KEY --job-id <id> status
//! ```

mod cli;
mod config;
pub mod domain_config;
mod domain_prompts;
mod jsonl;
mod parquet_reader;
mod progress;
mod stages;
mod state;

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

    // Load domain configuration (CLI flag → env → ./domain.toml → ~/.edgequake/ → built-in)
    let domain_config = domain_config::DomainConfig::load(cli.domain_config.as_deref())?;
    info!(domain = %domain_config.domain.name, "Loaded domain config");
    let domain_config = std::sync::Arc::new(domain_config);

    // Generate or use provided job ID
    let job_id = cli
        .job_id
        .unwrap_or_else(|| format!("batch-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")));

    info!(job_id = %job_id, "EdgeQuake Batch Ingestion starting");

    // Build config from CLI args
    let config = BatchConfig {
        job_id: job_id.clone(),
        data_path: cli.data,
        work_dir: cli.work_dir,
        extraction_model: cli.model,
        embedding_model: cli.embedding_model,
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
    };

    // Create work directory
    std::fs::create_dir_all(&config.work_dir)?;

    // Initialize state manager with DynamoDB
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let dynamo_client = aws_sdk_dynamodb::Client::new(&aws_config);

    let dynamo_config =
        edgequake_storage_aws::DynamoKVConfig::new(&config.dynamo_table, &config.namespace);
    let dynamo_kv =
        edgequake_storage_aws::DynamoKVStorage::new_with_client(dynamo_config, dynamo_client);

    // Verify table exists before proceeding (gives clear error on misconfiguration)
    use edgequake_storage::KVStorage;
    dynamo_kv.initialize().await?;

    let state_mgr = StateManager::new(Box::new(dynamo_kv));
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

    match cli.command {
        Command::Run => {
            run_full_pipeline(
                &config,
                &cli.api_key,
                &aws_config,
                &domain_config,
                &state_mgr,
                &mut job,
                &progress,
            )
            .await?;
        }
        Command::Prepare => {
            stages::prepare::run_prepare(&config, &domain_config, &state_mgr, &mut job, &progress)
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
                &cli.api_key,
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
                create_embedding_provider(&cli.api_key, &config.embedding_model)?;
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
                        create_embedding_provider(&cli.api_key, &config.embedding_model)?;
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

            // Run store to persist to Neptune/S3/DynamoDB
            let (vectors, kv) = create_storage_backends(&config).await?;
            stages::store::run_store(
                &config,
                &aws_config,
                vectors,
                kv,
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
                &cli.api_key,
                &aws_config,
                &domain_config,
                &state_mgr,
                &mut job,
                &progress,
            )
            .await?;
        }
    }

    Ok(())
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
    info!("Running full batch pipeline");

    // Phase 1: Prepare
    let prepare_result = if matches!(job.phase, state::Phase::Pending | state::Phase::Preparing) {
        stages::prepare::run_prepare(config, domain_config, state_mgr, job, progress).await?
    } else {
        info!("Phase 1 already complete, re-running for data");
        stages::prepare::run_prepare(config, domain_config, state_mgr, job, progress).await?
    };

    // Phase 2: Extract
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

    // Phase 3: Embed
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

    // Phase 4: Store
    let (vectors, kv) = create_storage_backends(config).await?;
    stages::store::run_store(
        config,
        aws_config,
        vectors,
        kv,
        state_mgr,
        job,
        &prepare_result.documents,
        &prepare_result.chunks,
        &extract_result.results,
        &embed_result,
        progress,
    )
    .await?;

    info!("Full batch pipeline completed successfully!");
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
