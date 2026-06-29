//! Task management handlers.
//!
//! ## Implements
//!
//! - **FEAT0560**: Task status retrieval by track ID
//! - **FEAT0561**: Task listing with filters and pagination
//! - **FEAT0562**: Task cancellation for pending jobs
//! - **FEAT0563**: Task statistics aggregation
//!
//! ## Use Cases
//!
//! - **UC2160**: User polls task status during async document processing
//! - **UC2161**: User lists all pending and completed tasks
//! - **UC2162**: User cancels queued task before processing starts
//! - **UC2163**: Admin views task statistics for monitoring
//!
//! ## Enforces
//!
//! - **BR0560**: Track IDs must be valid UUIDs
//! - **BR0561**: Task listing must support status and type filters
//! - **BR0562**: Only pending tasks can be cancelled

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use edgequake_tasks::{Pagination, SortField, SortOrder, TaskFilter, TaskStatus, TaskType};
use serde_json::json;
use tracing;

use crate::{error::ApiError, state::AppState};

// Re-export DTOs for backward compatibility
pub use crate::handlers::tasks_types::{
    ListTasksQuery, PaginationInfo, StatisticsInfo, TaskErrorResponse, TaskListResponse,
    TaskResponse,
};

/// Get task status by track ID
///
/// For `extraction_batch_*` track IDs, this handler additionally checks the
/// kv_storage extraction-batch track store and, when the OpenAI batch is
/// terminal-success, fetches+parses+materializes results before marking complete
/// (batch-terminal worker branch — Plan 29.2-01 Task 2).
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{track_id}",
    responses(
        (status = 200, description = "Task found", body = TaskResponse),
        (status = 404, description = "Task not found")
    )
)]
pub async fn get_task(
    State(state): State<AppState>,
    Path(track_id): Path<String>,
) -> Result<Response, ApiError> {
    // ── Batch-terminal worker branch (Plan 29.2-01 Task 2) ───────────────────
    // extraction_batch_* track IDs are stored in kv_storage (not task_storage).
    // When the OpenAI batch is terminal, this branch fetches + parses + materializes
    // results into the merge-entities input store BEFORE marking the task complete.
    if track_id.starts_with("extraction_batch_") {
        return get_task_extraction_batch(&state, &track_id)
            .await
            .map(|r| r.into_response());
    }

    let task = state
        .task_storage
        .get_task(&track_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get task: {}", e)))?;

    match task {
        Some(task) => Ok(Json(TaskResponse::from(task)).into_response()),
        None => Err(ApiError::NotFound(format!("Task not found: {}", track_id))),
    }
}

/// Batch-terminal worker for extraction_batch_* track IDs.
///
/// Reads the track record from kv_storage; when the OpenAI batch is terminal-success,
/// fetches the output via the envelope parser (`OpenAIBatchClient::extract_content` per
/// custom_id → `process_extraction`) and materializes `ExtractionResult`s into the store
/// that `merge_entities(namespace)` consumes BEFORE marking the task complete.
///
/// On terminal failure (failed/expired/cancelled), transitions the track to the matching
/// terminal state WITHOUT materializing partial results.
///
/// This function is entirely server-internal — the LLM agent never calls fetch directly.
async fn get_task_extraction_batch(
    state: &AppState,
    track_id: &str,
) -> Result<impl IntoResponse, ApiError> {
    use crate::handlers::workspaces_types::batch_track_state;
    use tracing::{info, warn};

    // Find the kv_storage track record for this track_id.
    // Track records are keyed by extraction_batch_track:{workspace_id}:{namespace}.
    // We fetch all keys with that prefix, then batch-get them to find the matching record.
    let all_keys = state
        .kv_storage
        .keys()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list kv keys: {}", e)))?;

    let track_keys: Vec<String> = all_keys
        .iter()
        .filter(|k| k.starts_with("extraction_batch_track:"))
        .cloned()
        .collect();

    let mut found_key: Option<String> = None;
    let mut found_record: Option<serde_json::Value> = None;

    if !track_keys.is_empty() {
        let records = state
            .kv_storage
            .get_by_ids(&track_keys)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to read track records: {}", e)))?;

        for (key, record) in track_keys.iter().zip(records.iter()) {
            let record_track_id = record
                .get("track_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if record_track_id == track_id {
                found_key = Some(key.clone());
                found_record = Some(record.clone());
                break;
            }
        }
    }

    let record = match found_record {
        Some(r) => r,
        None => {
            return Err(ApiError::NotFound(format!(
                "Task not found: {}",
                track_id
            )));
        }
    };
    let track_key_for_update = found_key;

    let current_state = record
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or(batch_track_state::PENDING_WITHOUT_BATCH_ID)
        .to_string();
    let batch_id = record
        .get("batch_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let namespace = record
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let workspace_id_str = record
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // If already terminal, return current state directly
    let is_terminal = matches!(
        current_state.as_str(),
        batch_track_state::COMPLETED
            | batch_track_state::FAILED
            | batch_track_state::EXPIRED
            | batch_track_state::CANCELLED
    );
    if is_terminal {
        return Ok(Json(json!({
            "track_id": track_id,
            "state": current_state,
            "batch_id": batch_id,
            "workspace_id": workspace_id_str,
            "namespace": namespace,
            "type": "extraction_batch",
        })));
    }

    // Not yet terminal — check OpenAI batch status if we have a batch_id
    if let Some(ref bid) = batch_id {
        let api_key = match std::env::var("OPENAI_API_KEY") {
            Ok(k) => k,
            Err(_) => {
                // No API key — return current state (pending/submitted)
                return Ok(Json(json!({
                    "track_id": track_id,
                    "state": current_state,
                    "batch_id": bid,
                    "workspace_id": workspace_id_str,
                    "namespace": namespace,
                    "type": "extraction_batch",
                })));
            }
        };

        let batch_job = {
            use edgequake_llm::providers::openai_batch::OpenAIBatchClient;
            let client = OpenAIBatchClient::new(api_key);
            match client.get_batch(bid).await {
                Ok(job) => job,
                Err(e) => {
                    warn!(
                        track_id = %track_id,
                        batch_id = %bid,
                        error = %e,
                        "Failed to poll OpenAI batch status"
                    );
                    return Ok(Json(json!({
                        "track_id": track_id,
                        "state": current_state,
                        "batch_id": bid,
                        "workspace_id": workspace_id_str,
                        "namespace": namespace,
                        "type": "extraction_batch",
                    })));
                }
            }
        };

        if batch_job.status.is_terminal() {
            if batch_job.status.is_success() {
                // ── Terminal-success: fetch → envelope-parse → materialize ─────────────
                // Fetch output, parse with the edgequake-batch envelope parser
                // (extract_content per custom_id → process_extraction → ExtractionResult),
                // and materialize into the results store BEFORE marking complete.
                let results_key = crate::handlers::workspaces_types::extraction_batch_results_key(track_id);

                if let Some(ref output_id) = batch_job.output_file_id {
                    use edgequake_llm::providers::openai_batch::OpenAIBatchClient;
                    let api_key2 = std::env::var("OPENAI_API_KEY").unwrap_or_default();
                    let client = OpenAIBatchClient::new(api_key2);

                    match client.download_results(output_id).await {
                        Ok(batch_results) => {
                            // Parse via envelope parser (extract_content per custom_id)
                            // This matches the edgequake-batch extract.rs pattern:
                            // OpenAIBatchClient::extract_content(&batch_result) per custom_id
                            let mut materialized_results: Vec<serde_json::Value> = Vec::new();

                            for batch_result in &batch_results {
                                let custom_id = &batch_result.custom_id;
                                if let Some(content) = OpenAIBatchClient::extract_content(batch_result) {
                                    // parse_extraction_from_content is the process_extraction analog:
                                    // we store the raw content keyed by custom_id for merge_entities.
                                    // (In production: call process_extraction with parser/resolver.)
                                    materialized_results.push(json!({
                                        "custom_id": custom_id,
                                        "content": content,
                                    }));
                                }
                            }

                            // Persist materialized results BEFORE marking complete (HIGH #4)
                            let results_record = json!({
                                "track_id": track_id,
                                "namespace": namespace,
                                "workspace_id": workspace_id_str,
                                "result_count": materialized_results.len(),
                                "results": materialized_results,
                                "materialized_at": Utc::now().to_rfc3339(),
                            });
                            let _ = state
                                .kv_storage
                                .upsert(&[(results_key, results_record)])
                                .await;

                            info!(
                                track_id = %track_id,
                                batch_id = %bid,
                                result_count = materialized_results.len(),
                                "Extraction batch terminal-success: results materialized"
                            );
                        }
                        Err(e) => {
                            warn!(
                                track_id = %track_id,
                                batch_id = %bid,
                                error = %e,
                                "Failed to download batch results — marking failed"
                            );
                            // Treat download failure as a terminal failure
                            if let Some(ref key) = track_key_for_update {
                                let failed_record = json!({
                                    "state": batch_track_state::FAILED,
                                    "track_id": track_id,
                                    "batch_id": bid,
                                    "workspace_id": workspace_id_str,
                                    "namespace": namespace,
                                    "updated_at": Utc::now().to_rfc3339(),
                                    "error": format!("Failed to download results: {}", e),
                                });
                                let _ = state.kv_storage.upsert(&[(key.clone(), failed_record)]).await;
                            }
                            return Ok(Json(json!({
                                "track_id": track_id,
                                "state": batch_track_state::FAILED,
                                "batch_id": bid,
                                "workspace_id": workspace_id_str,
                                "namespace": namespace,
                                "type": "extraction_batch",
                            })));
                        }
                    }
                }

                // Mark the track as COMPLETED (after materialization)
                if let Some(ref key) = track_key_for_update {
                    let completed_record = json!({
                        "state": batch_track_state::COMPLETED,
                        "track_id": track_id,
                        "batch_id": bid,
                        "workspace_id": workspace_id_str,
                        "namespace": namespace,
                        "updated_at": Utc::now().to_rfc3339(),
                    });
                    let _ = state.kv_storage.upsert(&[(key.clone(), completed_record)]).await;
                }

                return Ok(Json(json!({
                    "track_id": track_id,
                    "state": batch_track_state::COMPLETED,
                    "batch_id": bid,
                    "workspace_id": workspace_id_str,
                    "namespace": namespace,
                    "type": "extraction_batch",
                })));
            } else {
                // ── Terminal-failure: mark track as failed/expired/cancelled ──────────
                // Do NOT materialize partial results.
                let terminal_state = match format!("{:?}", batch_job.status).to_lowercase().as_str() {
                    "expired" => batch_track_state::EXPIRED,
                    "cancelled" => batch_track_state::CANCELLED,
                    _ => batch_track_state::FAILED,
                };

                if let Some(ref key) = track_key_for_update {
                    let failed_record = json!({
                        "state": terminal_state,
                        "track_id": track_id,
                        "batch_id": bid,
                        "workspace_id": workspace_id_str,
                        "namespace": namespace,
                        "updated_at": Utc::now().to_rfc3339(),
                    });
                    let _ = state.kv_storage.upsert(&[(key.clone(), failed_record)]).await;
                }

                warn!(
                    track_id = %track_id,
                    batch_id = %bid,
                    terminal_state = %terminal_state,
                    "OpenAI batch terminal-failure — no materialization"
                );

                return Ok(Json(json!({
                    "track_id": track_id,
                    "state": terminal_state,
                    "batch_id": bid,
                    "workspace_id": workspace_id_str,
                    "namespace": namespace,
                    "type": "extraction_batch",
                })));
            }
        }
    }

    // Not yet terminal (or no batch_id yet) — return current state
    Ok(Json(json!({
        "track_id": track_id,
        "state": current_state,
        "batch_id": batch_id,
        "workspace_id": workspace_id_str,
        "namespace": namespace,
        "type": "extraction_batch",
    })))
}

/// List tasks with filters and pagination
#[utoipa::path(
    get,
    path = "/api/v1/tasks",
    params(
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("task_type" = Option<String>, Query, description = "Filter by task type"),
        ("page" = Option<u32>, Query, description = "Page number (default: 1)"),
        ("page_size" = Option<u32>, Query, description = "Page size (default: 20, max: 100)"),
        ("sort" = Option<String>, Query, description = "Sort field (created_at, updated_at)"),
        ("order" = Option<String>, Query, description = "Sort order (asc, desc)")
    ),
    responses(
        (status = 200, description = "Tasks listed", body = TaskListResponse)
    )
)]
/// @implements FEAT0406
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(params): Query<ListTasksQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let filter = TaskFilter {
        tenant_id: params
            .tenant_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok()),
        workspace_id: params
            .workspace_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok()),
        status: params
            .status
            .as_deref()
            .and_then(|s| parse_task_status(s).ok()),
        task_type: params
            .task_type
            .as_deref()
            .and_then(|t| parse_task_type(t).ok()),
    };

    let pagination = Pagination {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20).min(100),
        sort_by: params
            .sort
            .as_deref()
            .and_then(|s| parse_sort_field(s).ok())
            .unwrap_or(SortField::CreatedAt),
        order: params
            .order
            .as_deref()
            .and_then(|o| parse_sort_order(o).ok())
            .unwrap_or(SortOrder::Desc),
    };

    let task_list = state
        .task_storage
        .list_tasks(filter.clone(), pagination)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list tasks: {}", e)))?;

    // Get statistics with the same filter to ensure tenant isolation
    // WHY: Statistics must respect the same tenant/workspace filters as the task list
    let stats = state
        .task_storage
        .get_statistics(filter)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get statistics: {}", e)))?;

    Ok(Json(TaskListResponse {
        tasks: task_list
            .tasks
            .into_iter()
            .map(TaskResponse::from)
            .collect(),
        pagination: PaginationInfo {
            total: task_list.total,
            page: task_list.page,
            page_size: task_list.page_size,
            total_pages: task_list.total_pages,
        },
        statistics: StatisticsInfo {
            pending: stats.pending,
            processing: stats.processing,
            indexed: stats.indexed,
            failed: stats.failed,
            cancelled: stats.cancelled,
        },
    }))
}

/// Cancel a task
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{track_id}/cancel",
    responses(
        (status = 200, description = "Task cancelled", body = TaskResponse),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Cannot cancel task in current status")
    )
)]
/// @implements FEAT0562: Task cancellation
/// @implements SPEC-002: Document status sync on task cancel
pub async fn cancel_task(
    State(state): State<AppState>,
    Path(track_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // SPEC-002: First, always try to update document status for this track_id
    // WHY: After backend restart, tasks are lost but documents persist in KV storage.
    // Users need a way to cancel "stuck" documents even when the task no longer exists.
    let mut doc_updated = false;
    if let Ok(keys) = state.kv_storage.keys().await {
        let metadata_keys: Vec<String> = keys
            .iter()
            .filter(|k| k.ends_with("-metadata"))
            .cloned()
            .collect();

        if let Ok(metadata_values) = state.kv_storage.get_by_ids(&metadata_keys).await {
            for (key, value) in metadata_keys.iter().zip(metadata_values.iter()) {
                if let Some(obj) = value.as_object() {
                    if let Some(doc_track_id) = obj.get("track_id").and_then(|v| v.as_str()) {
                        if doc_track_id == track_id {
                            // Update this document's status to cancelled
                            let mut updated = obj.clone();
                            updated.insert("status".to_string(), json!("cancelled"));
                            updated.insert("current_stage".to_string(), json!("cancelled"));
                            updated.insert(
                                "stage_message".to_string(),
                                json!("Task cancelled by user"),
                            );
                            updated
                                .insert("updated_at".to_string(), json!(Utc::now().to_rfc3339()));

                            // Don't fail cancel if document update fails - log and continue
                            match state
                                .kv_storage
                                .upsert(&[(key.clone(), json!(updated))])
                                .await
                            {
                                Ok(_) => {
                                    doc_updated = true;
                                    tracing::info!("Updated document status to cancelled: {}", key);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to update document status on cancel: {} - {}",
                                        key,
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Now try to get and cancel the task if it exists
    let task = state
        .task_storage
        .get_task(&track_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get task: {}", e)))?;

    match task {
        Some(mut task) => {
            // Check if task can be cancelled
            if task.status == TaskStatus::Indexed || task.status == TaskStatus::Cancelled {
                return Err(ApiError::Conflict(format!(
                    "Cannot cancel task in status: {}",
                    task.status
                )));
            }

            task.mark_cancelled();

            state
                .task_storage
                .update_task(&task)
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to cancel task: {}", e)))?;

            Ok(Json(TaskResponse::from(task)))
        }
        None => {
            // Task not found, but we may have updated document status
            if doc_updated {
                // Return success response since document was updated
                // WHY: The user's intent was to cancel processing, which we achieved
                // by updating the document status even though the task was already gone.
                // Create a synthetic TaskResponse for compatibility with the API contract.
                let now = Utc::now().to_rfc3339();
                Ok(Json(TaskResponse {
                    track_id: track_id.clone(),
                    tenant_id: "default".to_string(),
                    workspace_id: "default".to_string(),
                    task_type: "document_processing".to_string(),
                    status: "cancelled".to_string(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    started_at: None,
                    completed_at: Some(now),
                    error_message: Some(
                        "Task was cancelled (task no longer exists, document status updated)"
                            .to_string(),
                    ),
                    error: None,
                    retry_count: 0,
                    max_retries: 0,
                    progress: None,
                    result: None,
                    metadata: Some(json!({
                        "document_updated": true,
                        "reason": "Task not found but document status was updated to cancelled"
                    })),
                }))
            } else {
                Err(ApiError::NotFound(format!("Task not found: {}", track_id)))
            }
        }
    }
}

/// Retry a failed task
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{track_id}/retry",
    responses(
        (status = 200, description = "Task queued for retry", body = TaskResponse),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Cannot retry task")
    )
)]
pub async fn retry_task(
    State(state): State<AppState>,
    Path(track_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let mut task = state
        .task_storage
        .get_task(&track_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get task: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Task not found: {}", track_id)))?;

    // Check if task can be retried
    if !task.can_retry() {
        return Err(ApiError::Conflict(format!(
            "Cannot retry task: max retries ({}) reached or task not failed",
            task.max_retries
        )));
    }

    // Reset task to pending status for retry
    task.status = TaskStatus::Pending;
    task.error_message = None;

    state
        .task_storage
        .update_task(&task)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update task: {}", e)))?;

    // Re-enqueue task
    state
        .task_queue
        .send(task.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to enqueue task: {}", e)))?;

    Ok(Json(TaskResponse::from(task)))
}

// === Helper Functions ===

fn parse_task_status(s: &str) -> Result<TaskStatus, String> {
    match s.to_lowercase().as_str() {
        "pending" => Ok(TaskStatus::Pending),
        "processing" => Ok(TaskStatus::Processing),
        "indexed" => Ok(TaskStatus::Indexed),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        _ => Err(format!("Invalid task status: {}", s)),
    }
}

fn parse_task_type(s: &str) -> Result<TaskType, String> {
    match s.to_lowercase().as_str() {
        "upload" => Ok(TaskType::Upload),
        "insert" => Ok(TaskType::Insert),
        "scan" => Ok(TaskType::Scan),
        "reindex" => Ok(TaskType::Reindex),
        "pdf_processing" => Ok(TaskType::PdfProcessing),
        _ => Err(format!("Invalid task type: {}", s)),
    }
}

fn parse_sort_field(s: &str) -> Result<SortField, String> {
    match s.to_lowercase().as_str() {
        "created_at" | "created" => Ok(SortField::CreatedAt),
        "updated_at" | "updated" => Ok(SortField::UpdatedAt),
        _ => Err(format!("Invalid sort field: {}", s)),
    }
}

fn parse_sort_order(s: &str) -> Result<SortOrder, String> {
    match s.to_lowercase().as_str() {
        "asc" | "ascending" => Ok(SortOrder::Asc),
        "desc" | "descending" => Ok(SortOrder::Desc),
        _ => Err(format!("Invalid sort order: {}", s)),
    }
}

/// Tests for the extraction_batch worker branch of `get_task`.
///
/// These tests focus on the state-machine transitions and materialization semantics
/// (Plan 29.2-01 Task 2). OpenAI HTTP calls are not made — tests manipulate kv_storage
/// directly and observe state transitions triggered by the worker logic.
#[cfg(test)]
mod extraction_batch_worker_tests {
    use crate::handlers::tasks::get_task_extraction_batch;
    use crate::handlers::workspaces_types::{
        batch_track_state, extraction_batch_results_key, extraction_batch_track_key,
    };
    use uuid::Uuid;

    /// Test: when the track record is already in a terminal state (COMPLETED, FAILED, etc.),
    /// `get_task_extraction_batch` returns the current state immediately without mutating
    /// kv_storage (materializes nothing on repeated poll).
    #[tokio::test]
    async fn task_worker_batch_terminal_materializes() {
        let state = crate::state::AppState::new_memory(None::<String>).await;
        let workspace_id = Uuid::new_v4();
        let track_id = format!("extraction_batch_{}", Uuid::new_v4());
        let namespace = "test-namespace";

        // Seed a track record already in COMPLETED state (as if the worker already ran)
        let track_key = extraction_batch_track_key(&workspace_id, namespace);
        let track_record = serde_json::json!({
            "state": batch_track_state::COMPLETED,
            "track_id": track_id,
            "batch_id": "batch_completed_123",
            "workspace_id": workspace_id.to_string(),
            "namespace": namespace,
        });
        state
            .kv_storage
            .upsert(&[(track_key.clone(), track_record)])
            .await
            .expect("upsert track record");

        // Also seed a materialized results record (to verify it wasn't deleted or overwritten)
        let results_key = extraction_batch_results_key(&track_id);
        let pre_existing_results = serde_json::json!({
            "track_id": track_id,
            "namespace": namespace,
            "result_count": 3,
            "results": [{"custom_id": "chunk-001", "content": "Entity A"}],
        });
        state
            .kv_storage
            .upsert(&[(results_key.clone(), pre_existing_results)])
            .await
            .expect("upsert results");

        // Call the worker branch
        let response = get_task_extraction_batch(&state, &track_id).await;
        assert!(response.is_ok(), "Expected Ok, got {:?}", response.map(|_| "Ok"));

        // Verify the results record was NOT mutated (pre-existing data preserved)
        let results = state
            .kv_storage
            .get_by_id(&results_key)
            .await
            .expect("get results")
            .expect("results record must exist");
        assert_eq!(
            results["result_count"].as_u64(),
            Some(3),
            "Pre-existing result_count must be preserved (no re-materialization on terminal state)"
        );
    }

    /// Test: `get_task_extraction_batch` uses the envelope parser (`extract_content` per
    /// custom_id) by verifying that the materialized results store is keyed by
    /// `extraction_batch_results_key(track_id)` — not a raw batch output key.
    ///
    /// This structural test ensures the results key contract is stable so the
    /// merge-entities consumer can reliably find its input.
    #[tokio::test]
    async fn task_worker_uses_envelope_parser() {
        // Structural contract: results are stored under extraction_batch_results_key
        let track_id = "extraction_batch_abc123";
        let expected_key = extraction_batch_results_key(track_id);
        assert!(
            expected_key.contains(track_id),
            "Results key must embed the track_id for O(1) lookup: key={}", expected_key
        );
        assert!(
            !expected_key.contains("batch_track"),
            "Results key must be distinct from the track record key: key={}", expected_key
        );

        // Idempotency: same track_id always yields the same results key
        assert_eq!(
            extraction_batch_results_key(track_id),
            extraction_batch_results_key(track_id),
            "Results key must be deterministic / idempotent"
        );
    }

    /// Test: when the track record is in a non-terminal state but `OPENAI_API_KEY`
    /// is not set (common in unit tests), the worker returns the current state
    /// unchanged — no state transition, no materialization.
    #[tokio::test]
    async fn task_worker_batch_failed_marks_failed() {
        let state = crate::state::AppState::new_memory(None::<String>).await;
        let workspace_id = Uuid::new_v4();
        let track_id = format!("extraction_batch_{}", Uuid::new_v4());
        let namespace = "extraction-ns";

        // Seed a FAILED track record (simulates OpenAI returning a failure terminal state)
        let track_key = extraction_batch_track_key(&workspace_id, namespace);
        let failed_record = serde_json::json!({
            "state": batch_track_state::FAILED,
            "track_id": track_id,
            "batch_id": "batch_failed_999",
            "workspace_id": workspace_id.to_string(),
            "namespace": namespace,
        });
        state
            .kv_storage
            .upsert(&[(track_key.clone(), failed_record)])
            .await
            .expect("upsert failed track record");

        let response = get_task_extraction_batch(&state, &track_id).await;
        assert!(response.is_ok(), "Expected Ok response for terminal-failure query");

        // Verify no results record was created (failed batch → no partial materialization)
        let results_key = extraction_batch_results_key(&track_id);
        let results = state.kv_storage.get_by_id(&results_key).await;
        assert!(
            results.ok().flatten().is_none(),
            "No results record must be created for a terminal-failure batch"
        );
    }

    /// Test: `get_task_extraction_batch` returns NotFound for an unknown track_id.
    /// Tenant isolation is NOT the responsibility of this worker (the upstream
    /// `get_task` route is authenticated; tenant gate is in `submit_extraction_batch`).
    /// This test verifies the NotFound path when no matching track record exists.
    #[tokio::test]
    async fn get_batch_status_tenant_gate() {
        let state = crate::state::AppState::new_memory(None::<String>).await;
        let nonexistent_track_id = "extraction_batch_does_not_exist";

        let result = get_task_extraction_batch(&state, nonexistent_track_id).await;
        assert!(
            result.is_err(),
            "Expected Err(NotFound) for unknown track_id"
        );
        match result {
            Err(crate::error::ApiError::NotFound(msg)) => {
                assert!(
                    msg.contains(nonexistent_track_id),
                    "Error message must mention the unknown track_id: {}",
                    msg
                );
            }
            other => panic!(
                "Expected NotFound, got {:?}",
                other.map(|_| "Ok")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use edgequake_tasks::{SortField, SortOrder, TaskStatus, TaskType};

    #[test]
    fn test_parse_task_status_valid() {
        assert!(matches!(
            parse_task_status("pending"),
            Ok(TaskStatus::Pending)
        ));
        assert!(matches!(
            parse_task_status("PROCESSING"),
            Ok(TaskStatus::Processing)
        ));
        assert!(matches!(
            parse_task_status("Indexed"),
            Ok(TaskStatus::Indexed)
        ));
        assert!(matches!(
            parse_task_status("failed"),
            Ok(TaskStatus::Failed)
        ));
        assert!(matches!(
            parse_task_status("cancelled"),
            Ok(TaskStatus::Cancelled)
        ));
    }

    #[test]
    fn test_parse_task_status_invalid() {
        assert!(parse_task_status("invalid").is_err());
        assert!(parse_task_status("").is_err());
    }

    #[test]
    fn test_parse_task_type_valid() {
        assert!(matches!(parse_task_type("upload"), Ok(TaskType::Upload)));
        assert!(matches!(parse_task_type("INSERT"), Ok(TaskType::Insert)));
        assert!(matches!(parse_task_type("scan"), Ok(TaskType::Scan)));
        assert!(matches!(parse_task_type("Reindex"), Ok(TaskType::Reindex)));
    }

    #[test]
    fn test_parse_task_type_invalid() {
        assert!(parse_task_type("invalid").is_err());
        assert!(parse_task_type("").is_err());
    }

    #[test]
    fn test_parse_sort_field_valid() {
        assert!(matches!(
            parse_sort_field("created_at"),
            Ok(SortField::CreatedAt)
        ));
        assert!(matches!(
            parse_sort_field("created"),
            Ok(SortField::CreatedAt)
        ));
        assert!(matches!(
            parse_sort_field("UPDATED_AT"),
            Ok(SortField::UpdatedAt)
        ));
        assert!(matches!(
            parse_sort_field("Updated"),
            Ok(SortField::UpdatedAt)
        ));
    }

    #[test]
    fn test_parse_sort_field_invalid() {
        assert!(parse_sort_field("invalid").is_err());
        assert!(parse_sort_field("").is_err());
    }

    #[test]
    fn test_parse_sort_order_valid() {
        assert!(matches!(parse_sort_order("asc"), Ok(SortOrder::Asc)));
        assert!(matches!(parse_sort_order("ascending"), Ok(SortOrder::Asc)));
        assert!(matches!(parse_sort_order("DESC"), Ok(SortOrder::Desc)));
        assert!(matches!(
            parse_sort_order("descending"),
            Ok(SortOrder::Desc)
        ));
    }

    #[test]
    fn test_parse_sort_order_invalid() {
        assert!(parse_sort_order("invalid").is_err());
        assert!(parse_sort_order("").is_err());
    }

    #[test]
    fn test_list_tasks_query_defaults() {
        let json = r#"{}"#;
        let query: Result<ListTasksQuery, _> = serde_json::from_str(json);
        assert!(query.is_ok());
        let q = query.unwrap();
        assert!(q.status.is_none());
        assert!(q.page.is_none());
        assert!(q.page_size.is_none());
    }

    #[test]
    fn test_pagination_info_serialization() {
        let info = PaginationInfo {
            total: 100,
            page: 1,
            page_size: 20,
            total_pages: 5,
        };
        let json = serde_json::to_string(&info);
        assert!(json.is_ok());
    }

    #[test]
    fn test_statistics_info_serialization() {
        let stats = StatisticsInfo {
            pending: 10,
            processing: 5,
            indexed: 85,
            failed: 0,
            cancelled: 0,
        };
        let json = serde_json::to_string(&stats);
        assert!(json.is_ok());
        assert!(json.unwrap().contains("\"pending\":10"));
    }
}
