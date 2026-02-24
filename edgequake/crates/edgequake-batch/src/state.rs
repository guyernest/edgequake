//! DynamoDB-backed job state management for crash recovery.
//!
//! Tracks:
//! - Job-level state: current phase, progress counters, batch IDs
//! - Document-level state: processing status per content hash
//!
//! State is stored in the existing DynamoDB table under special namespaces:
//! - `__batch_jobs__/{job_id}` for job state
//! - `__batch_docs__/{content_hash}` for per-document state

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use tracing::warn;

/// Job-level state persisted to DynamoDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    /// Unique job identifier.
    pub job_id: String,

    /// Current pipeline phase.
    pub phase: Phase,

    /// Total documents in the dataset.
    pub total_documents: usize,

    /// Documents successfully processed in current phase.
    pub processed_documents: usize,

    /// Total chunks generated.
    pub total_chunks: usize,

    /// OpenAI batch job IDs (Phase 2).
    pub batch_ids: Vec<String>,

    /// OpenAI file IDs for uploaded JSONL files.
    pub input_file_ids: Vec<String>,

    /// OpenAI file IDs for result JSONL files.
    pub output_file_ids: Vec<String>,

    /// Total entities extracted.
    pub total_entities: usize,

    /// Total relationships extracted.
    pub total_relationships: usize,

    /// Total embeddings generated.
    pub total_embeddings: usize,

    /// Timestamp of job creation.
    pub created_at: String,

    /// Timestamp of last update.
    pub updated_at: String,

    /// Error message if job failed.
    pub error: Option<String>,

    /// Run start timestamp (epoch millis). Set once at the beginning of a run
    /// and carried through all phases for LATEST_RUN tracking.
    #[serde(default)]
    pub run_started_at: Option<i64>,

    /// Per-phase error counts, accumulated during pipeline execution.
    /// Used to build the run report at completion.
    #[serde(default)]
    pub errors_per_phase: HashMap<String, usize>,

    /// Per-phase start timestamps (epoch millis) for duration computation.
    #[serde(default)]
    pub phase_started_at: HashMap<String, i64>,
}

/// Pipeline phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Not yet started.
    Pending,
    /// Phase 1: Parsing parquet, chunking, building JSONL.
    Preparing,
    /// Phase 1 complete.
    Prepared,
    /// Phase 2: Batch extraction in progress.
    Extracting,
    /// Phase 2 complete.
    Extracted,
    /// Phase 3: Generating embeddings.
    Embedding,
    /// Phase 3 complete.
    Embedded,
    /// Phase 4: Writing to storage backends.
    Storing,
    /// All phases complete.
    Completed,
    /// Job failed.
    Failed,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Phase::Pending => write!(f, "pending"),
            Phase::Preparing => write!(f, "preparing"),
            Phase::Prepared => write!(f, "prepared"),
            Phase::Extracting => write!(f, "extracting"),
            Phase::Extracted => write!(f, "extracted"),
            Phase::Embedding => write!(f, "embedding"),
            Phase::Embedded => write!(f, "embedded"),
            Phase::Storing => write!(f, "storing"),
            Phase::Completed => write!(f, "completed"),
            Phase::Failed => write!(f, "failed"),
        }
    }
}

/// Per-document processing state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentState {
    /// SHA-256 content hash of the document.
    pub content_hash: String,

    /// Original filename from the parquet file.
    pub filename: String,

    /// Processing status.
    pub status: DocumentStatus,

    /// Chunk IDs generated from this document.
    pub chunk_ids: Vec<String>,

    /// Number of entities extracted.
    pub entity_count: usize,

    /// Number of relationships extracted.
    pub relationship_count: usize,

    /// Error message if processing failed.
    pub error: Option<String>,
}

/// Per-document processing status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    Pending,
    Chunked,
    Extracted,
    Embedded,
    Stored,
    Completed,
    Failed,
}

/// Pipeline run status written to `NS#{slug}/LATEST_RUN` in the namespace
/// registry table so the management UI can display real-time pipeline progress.
///
/// This is a separate write path from the `__batch_jobs__/{job_id}` KVStorage
/// records -- the UI reads LATEST_RUN, not __batch_jobs__.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatestRunStatus {
    /// Pipeline status: "requested", "preparing", "extracting", "embedding",
    /// "storing", "completed", "completed_with_warnings", "failed".
    pub status: String,

    /// Current phase name (lowercase).
    pub phase: Option<String>,

    /// Batch job identifier.
    pub job_id: Option<String>,

    /// Total documents in the dataset.
    pub total_documents: Option<usize>,

    /// Documents successfully processed so far.
    pub processed_documents: Option<usize>,

    /// Total chunks generated.
    pub total_chunks: Option<usize>,

    /// Current batch file being processed (extract phase).
    pub current_batch: Option<usize>,

    /// Total batch files to process (extract phase).
    pub total_batches: Option<usize>,

    /// Run start timestamp (epoch millis). Set once at beginning of run.
    pub started_at: Option<i64>,

    /// Last update timestamp (epoch millis).
    pub updated_at: Option<i64>,

    /// Phase start timestamp (epoch millis) for computing per-phase duration.
    pub phase_started_at: Option<i64>,

    /// Human-readable error summary on failure or partial failure.
    pub error_summary: Option<String>,

    /// Run completion timestamp (epoch millis).
    pub completed_at: Option<i64>,

    /// Per-phase duration in milliseconds (populated on completion).
    pub per_phase_durations: Option<HashMap<String, i64>>,

    /// Total entities extracted across all documents.
    pub total_entities: Option<usize>,

    /// Total relationships extracted across all documents.
    pub total_relationships: Option<usize>,

    /// Error count per phase (populated on completion).
    pub error_count_per_phase: Option<HashMap<String, usize>>,
}

/// State manager backed by DynamoDB.
///
/// Manages job state in KVStorage (`__batch_jobs__/{job_id}`) and optionally
/// writes pipeline status to the namespace registry table (`NS#{slug}/LATEST_RUN`)
/// for UI visibility.
pub struct StateManager {
    kv: Box<dyn edgequake_storage::KVStorage>,
    /// Direct DynamoDB client for LATEST_RUN writes to the namespace registry table.
    dynamo_client: Option<aws_sdk_dynamodb::Client>,
    /// Namespace registry table name (e.g. `edgequake-namespaces` or shared table).
    registry_table: Option<String>,
    /// Namespace slug for PK construction (`NS#{slug}`).
    namespace: Option<String>,
}

impl StateManager {
    /// Create a new state manager with the given KV storage.
    ///
    /// LATEST_RUN support is disabled by default. Call [`with_latest_run_config`]
    /// to enable writes to the namespace registry table.
    pub fn new(kv: Box<dyn edgequake_storage::KVStorage>) -> Self {
        Self {
            kv,
            dynamo_client: None,
            registry_table: None,
            namespace: None,
        }
    }

    /// Enable LATEST_RUN writes to the namespace registry table.
    ///
    /// When configured, `write_latest_run()` writes to `PK=NS#{namespace},
    /// SK=LATEST_RUN` in the specified table. Without this config, writes
    /// are silently skipped (graceful degradation for tests / non-namespace runs).
    pub fn with_latest_run_config(
        mut self,
        dynamo_client: aws_sdk_dynamodb::Client,
        registry_table: String,
        namespace: String,
    ) -> Self {
        self.dynamo_client = Some(dynamo_client);
        self.registry_table = Some(registry_table);
        self.namespace = Some(namespace);
        self
    }

    /// Write pipeline status to `NS#{slug}/LATEST_RUN` in the namespace registry table.
    ///
    /// If LATEST_RUN config is not set (no dynamo_client/registry_table/namespace),
    /// logs a warning and returns Ok -- this allows tests and non-namespace runs
    /// to work without modification.
    pub async fn write_latest_run(&self, status: &LatestRunStatus) -> anyhow::Result<()> {
        let (client, table, ns) = match (&self.dynamo_client, &self.registry_table, &self.namespace)
        {
            (Some(c), Some(t), Some(n)) => (c, t, n),
            _ => {
                warn!("LATEST_RUN config not set, skipping write");
                return Ok(());
            }
        };

        let pk = format!("NS#{}", ns);
        let data = serde_json::to_string(status)?;

        client
            .put_item()
            .table_name(table)
            .item("PK", aws_sdk_dynamodb::types::AttributeValue::S(pk))
            .item(
                "SK",
                aws_sdk_dynamodb::types::AttributeValue::S("LATEST_RUN".to_string()),
            )
            .item("data", aws_sdk_dynamodb::types::AttributeValue::S(data))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to write LATEST_RUN: {}", e))?;

        Ok(())
    }

    /// Read the current LATEST_RUN record for this namespace, if it exists.
    ///
    /// Used by the Prepare phase to check if the UI set `status=requested`
    /// (preserving the `started_at` timestamp from the UI trigger).
    pub async fn read_latest_run(&self) -> anyhow::Result<Option<LatestRunStatus>> {
        let (client, table, ns) = match (&self.dynamo_client, &self.registry_table, &self.namespace)
        {
            (Some(c), Some(t), Some(n)) => (c, t, n),
            _ => return Ok(None),
        };

        let pk = format!("NS#{}", ns);

        let result = client
            .get_item()
            .table_name(table)
            .key("PK", aws_sdk_dynamodb::types::AttributeValue::S(pk))
            .key(
                "SK",
                aws_sdk_dynamodb::types::AttributeValue::S("LATEST_RUN".to_string()),
            )
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read LATEST_RUN: {}", e))?;

        match result.item() {
            Some(item) => {
                if let Some(aws_sdk_dynamodb::types::AttributeValue::S(data)) = item.get("data") {
                    let status: LatestRunStatus = serde_json::from_str(data)?;
                    Ok(Some(status))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Save job state.
    pub async fn save_job(&self, state: &JobState) -> anyhow::Result<()> {
        let key = format!("__batch_jobs__/{}", state.job_id);
        let value = serde_json::to_value(state)?;
        self.kv.upsert(&[(key, value)]).await?;
        Ok(())
    }

    /// Load job state.
    pub async fn load_job(&self, job_id: &str) -> anyhow::Result<Option<JobState>> {
        let key = format!("__batch_jobs__/{}", job_id);
        match self.kv.get_by_id(&key).await? {
            Some(value) => Ok(Some(serde_json::from_value(value)?)),
            None => Ok(None),
        }
    }

    /// Save document state.
    pub async fn save_document(&self, state: &DocumentState) -> anyhow::Result<()> {
        let key = format!("__batch_docs__/{}", state.content_hash);
        let value = serde_json::to_value(state)?;
        self.kv.upsert(&[(key, value)]).await?;
        Ok(())
    }

    /// Save multiple document states in batch.
    pub async fn save_documents(&self, states: &[DocumentState]) -> anyhow::Result<()> {
        let entries: Vec<(String, JsonValue)> = states
            .iter()
            .map(|s| {
                let key = format!("__batch_docs__/{}", s.content_hash);
                let value = serde_json::to_value(s).unwrap();
                (key, value)
            })
            .collect();
        self.kv.upsert(&entries).await?;
        Ok(())
    }

    /// Load document state by content hash.
    pub async fn load_document(&self, content_hash: &str) -> anyhow::Result<Option<DocumentState>> {
        let key = format!("__batch_docs__/{}", content_hash);
        match self.kv.get_by_id(&key).await? {
            Some(value) => Ok(Some(serde_json::from_value(value)?)),
            None => Ok(None),
        }
    }

    /// Check if a document has already been completed (for idempotency).
    pub async fn is_document_completed(&self, content_hash: &str) -> bool {
        match self.load_document(content_hash).await {
            Ok(Some(state)) => state.status == DocumentStatus::Completed,
            _ => false,
        }
    }

    /// Create initial job state.
    pub fn create_job(job_id: &str) -> JobState {
        let now = chrono::Utc::now().to_rfc3339();
        JobState {
            job_id: job_id.to_string(),
            phase: Phase::Pending,
            total_documents: 0,
            processed_documents: 0,
            total_chunks: 0,
            batch_ids: Vec::new(),
            input_file_ids: Vec::new(),
            output_file_ids: Vec::new(),
            total_entities: 0,
            total_relationships: 0,
            total_embeddings: 0,
            created_at: now.clone(),
            updated_at: now,
            error: None,
            run_started_at: None,
            errors_per_phase: HashMap::new(),
            phase_started_at: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_display() {
        assert_eq!(Phase::Pending.to_string(), "pending");
        assert_eq!(Phase::Preparing.to_string(), "preparing");
        assert_eq!(Phase::Completed.to_string(), "completed");
    }

    #[test]
    fn test_create_job() {
        let job = StateManager::create_job("test-job-1");
        assert_eq!(job.job_id, "test-job-1");
        assert_eq!(job.phase, Phase::Pending);
        assert_eq!(job.total_documents, 0);
    }

    #[test]
    fn test_job_state_serialization() {
        let job = StateManager::create_job("test-job-2");
        let json = serde_json::to_string(&job).unwrap();
        let deserialized: JobState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.job_id, "test-job-2");
        assert_eq!(deserialized.phase, Phase::Pending);
    }

    #[test]
    fn test_document_state_serialization() {
        let doc = DocumentState {
            content_hash: "abc123".to_string(),
            filename: "test.txt".to_string(),
            status: DocumentStatus::Pending,
            chunk_ids: vec!["chunk-0".to_string()],
            entity_count: 0,
            relationship_count: 0,
            error: None,
        };
        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: DocumentState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content_hash, "abc123");
        assert_eq!(deserialized.status, DocumentStatus::Pending);
    }
}
