//! CLI argument definitions for the batch ingestion tool.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// EdgeQuake Batch Ingestion CLI
///
/// Build a knowledge graph from large document datasets using OpenAI Batch API
/// for 50% cost savings on entity extraction.
#[derive(Parser, Debug)]
#[command(name = "edgequake-batch", version, about)]
pub struct Cli {
    /// Path to input data (parquet file, text/markdown file, or directory)
    #[arg(short, long, env = "BATCH_DATA_PATH")]
    pub data: PathBuf,

    /// Document delimiter for splitting a single file into multiple documents (literal string match)
    #[arg(long, env = "BATCH_DELIMITER")]
    pub delimiter: Option<String>,

    /// Disable automatic document splitting (treat each file as a single document)
    #[arg(long, default_value = "false")]
    pub no_split: bool,

    /// Disable recursive directory traversal (process only top-level files)
    #[arg(long, default_value = "false")]
    pub no_recurse: bool,

    /// Include only files matching these glob patterns (e.g., '*.md')
    #[arg(long)]
    pub include: Vec<String>,

    /// Exclude files matching these glob patterns (e.g., 'drafts/**')
    #[arg(long)]
    pub exclude: Vec<String>,

    /// OpenAI API key (required for OpenAI models, not needed for dry-run)
    #[arg(long, env = "OPENAI_API_KEY")]
    pub api_key: Option<String>,

    /// Anthropic API key (required for Claude models, not needed for dry-run)
    #[arg(long, env = "ANTHROPIC_API_KEY")]
    pub anthropic_key: Option<String>,

    /// Maximum number of document failures before aborting pipeline (default: 0 = fail on first error)
    #[arg(long, default_value = "0", env = "BATCH_MAX_FAILURES")]
    pub max_failures: usize,

    /// Override extraction model (reads from namespace config if not set)
    #[arg(long, env = "BATCH_MODEL")]
    pub model: Option<String>,

    /// Override embedding model (reads from namespace config if not set)
    #[arg(long, env = "BATCH_EMBEDDING_MODEL")]
    pub embedding_model: Option<String>,

    /// DynamoDB table for pipeline execution state (key schema: namespace + id)
    #[arg(long, default_value = "edgequake-kv", env = "PIPELINE_STATE_TABLE")]
    pub state_table: String,

    /// DynamoDB table for namespace registry: config, schema, run status (key schema: PK + SK)
    #[arg(long, env = "NAMESPACE_REGISTRY_TABLE")]
    pub registry_table: Option<String>,

    /// Job ID for resume/status (auto-generated if not provided)
    #[arg(long, env = "BATCH_JOB_ID")]
    pub job_id: Option<String>,

    /// Working directory for intermediate files (JSONL, results)
    #[arg(long, default_value = "./batch_work", env = "BATCH_WORK_DIR")]
    pub work_dir: PathBuf,

    /// Neptune graph endpoint
    #[arg(long, env = "NEPTUNE_ENDPOINT")]
    pub neptune_endpoint: Option<String>,

    /// S3 Vectors bucket name
    #[arg(long, env = "VECTOR_BUCKET")]
    pub vector_bucket: Option<String>,

    /// S3 Vectors index name (default: {namespace}-embeddings)
    #[arg(long, env = "VECTOR_INDEX")]
    pub vector_index: Option<String>,

    /// S3 bucket for Neptune bulk load staging
    #[arg(long, env = "S3_BUCKET")]
    pub s3_bucket: Option<String>,

    /// IAM role ARN for Neptune S3 bulk load (Neptune assumes this to read S3)
    #[arg(long, env = "NEPTUNE_ROLE_ARN")]
    pub neptune_role_arn: Option<String>,

    /// Path to domain configuration TOML file
    #[arg(long, env = "EDGEQUAKE_DOMAIN_CONFIG")]
    pub domain_config: Option<PathBuf>,

    /// Path to pipeline config TOML file (alternative to DynamoDB namespace config)
    #[arg(long)]
    pub config_file: Option<PathBuf>,

    /// Storage namespace (required -- identifies config and storage target)
    #[arg(long)]
    pub namespace: String,

    /// Maximum tokens for extraction output
    #[arg(long, default_value = "4096")]
    pub max_tokens: u32,

    /// Chunk size in tokens
    #[arg(long, default_value = "1200")]
    pub chunk_size: usize,

    /// Chunk overlap in tokens
    #[arg(long, default_value = "100")]
    pub chunk_overlap: usize,

    /// Embedding batch size (requests per batch call)
    #[arg(long, default_value = "100")]
    pub embedding_batch_size: usize,

    /// Maximum concurrent embedding requests
    #[arg(long, default_value = "5")]
    pub embedding_concurrency: usize,

    /// Limit number of documents to process (0 = no limit)
    #[arg(long, default_value = "0", env = "BATCH_LIMIT")]
    pub limit: usize,

    /// Skip the first N documents before processing (default 0).
    /// Use with --limit to process documents in batches:
    ///   First run:  --limit 100             (processes docs 0-99)
    ///   Second run: --offset 100 --limit 200 (processes docs 100-299)
    #[arg(long, default_value = "0", env = "BATCH_OFFSET")]
    pub offset: usize,

    /// Maximum retry attempts for token limit errors (default: 10)
    #[arg(long, default_value = "10", env = "BATCH_MAX_RETRIES")]
    pub max_retries: u32,

    /// Initial retry delay in seconds for token limit errors (default: 60, doubles each attempt)
    #[arg(long, default_value = "60", env = "BATCH_RETRY_DELAY")]
    pub retry_delay_secs: u64,

    /// Athena database name for BM25 index (enables BM25 indexing when set)
    #[arg(long, env = "ATHENA_BM25_DATABASE")]
    pub athena_bm25_database: Option<String>,

    /// S3 bucket for BM25 Iceberg table data
    #[arg(long, env = "BM25_S3_BUCKET")]
    pub bm25_s3_bucket: Option<String>,

    /// Athena workgroup (should be v3 for Iceberg support)
    #[arg(long, default_value = "primary", env = "ATHENA_WORKGROUP")]
    pub athena_workgroup: String,

    /// S3 location for Athena query results
    #[arg(long, env = "ATHENA_OUTPUT_LOCATION")]
    pub athena_output_location: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

/// Batch pipeline commands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the full pipeline (all 4 phases)
    Run,

    /// Phase 1: Parse parquet, reconstruct documents, chunk, build JSONL
    Prepare,

    /// Phase 2: Submit batch extraction jobs, poll, download results
    Extract,

    /// Phase 3: Generate embeddings for chunks, entities, and relationships
    Embed,

    /// Phase 4: Store results in Neptune, S3 Vectors, and DynamoDB
    Store,

    /// Show current job status and progress
    Status,

    /// Resume from last checkpoint
    Resume,

    /// List all batch jobs in OpenAI (shows what's consuming token limits)
    ListBatches {
        /// Number of batches to show (default: 20, max: 100)
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Generate and store the MCP descriptor for this namespace (no pipeline re-run)
    Descriptor,

    /// Preview pipeline: show document count, chunk estimate, schema summary, and estimated cost (no API calls)
    DryRun,

    /// Analyze a dataset sample and propose entity/relation types for the namespace
    SuggestSchema {
        /// Sample percentage of documents to analyze (default: 10)
        #[arg(long, default_value = "10.0")]
        sample_percentage: f64,

        /// Optional domain hint to guide schema extraction (e.g. "legal", "healthcare", "finance")
        #[arg(long)]
        domain_hint: Option<String>,
    },
}
