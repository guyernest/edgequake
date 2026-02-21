//! Batch job configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for a batch ingestion job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchConfig {
    /// Unique job identifier.
    pub job_id: String,

    /// Path to the source parquet file.
    pub data_path: PathBuf,

    /// Working directory for intermediate files.
    pub work_dir: PathBuf,

    /// OpenAI model for entity extraction.
    pub extraction_model: String,

    /// OpenAI model for embeddings.
    pub embedding_model: String,

    /// Maximum output tokens for extraction.
    pub max_tokens: u32,

    /// Chunk size in tokens.
    pub chunk_size: usize,

    /// Chunk overlap in tokens.
    pub chunk_overlap: usize,

    /// Embedding batch size.
    pub embedding_batch_size: usize,

    /// Maximum concurrent embedding requests.
    pub embedding_concurrency: usize,

    /// DynamoDB table for state.
    pub dynamo_table: String,

    /// Storage namespace.
    pub namespace: String,

    /// Neptune endpoint (optional).
    pub neptune_endpoint: Option<String>,

    /// S3 bucket for Neptune bulk load staging (optional).
    pub s3_bucket: Option<String>,

    /// IAM role ARN for Neptune S3 bulk load (optional).
    pub neptune_role_arn: Option<String>,

    /// S3 Vectors bucket name (optional).
    pub vector_bucket: Option<String>,

    /// S3 Vectors index name (optional).
    pub vector_index: Option<String>,

    /// Maximum number of documents to process (0 = no limit).
    pub limit: usize,

    /// Number of documents to skip before processing (0 = start from beginning).
    pub offset: usize,

    /// Maximum retry attempts for token limit errors.
    pub max_retries: u32,

    /// Initial retry delay in seconds (doubles with each attempt).
    pub retry_delay_secs: u64,

    /// Athena database for BM25 index (optional -- when set, BM25 indexing is enabled).
    pub athena_bm25_database: Option<String>,

    /// S3 bucket for BM25 Iceberg table data (required when athena_bm25_database is set).
    pub bm25_s3_bucket: Option<String>,

    /// Athena workgroup for BM25 queries.
    pub athena_workgroup: String,

    /// S3 location for Athena query results (required when athena_bm25_database is set).
    pub athena_output_location: Option<String>,
}

impl BatchConfig {
    /// Maximum requests per OpenAI batch job (API limit).
    pub const MAX_BATCH_REQUESTS: usize = 50_000;

    /// Maximum JSONL file size in bytes (5MB for reliable uploads; API limit is 100MB).
    pub const MAX_JSONL_SIZE: usize = 5 * 1024 * 1024;

    /// Get the JSONL output directory.
    pub fn jsonl_dir(&self) -> PathBuf {
        self.work_dir.join("jsonl")
    }

    /// Get the results output directory.
    pub fn results_dir(&self) -> PathBuf {
        self.work_dir.join("results")
    }

    /// Get the embeddings output directory.
    pub fn embeddings_dir(&self) -> PathBuf {
        self.work_dir.join("embeddings")
    }

    /// Get the bulk load staging directory.
    pub fn bulk_load_dir(&self) -> PathBuf {
        self.work_dir.join("bulk_load")
    }
}
