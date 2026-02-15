//! OpenAI Batch API client for cost-effective bulk extraction.
//!
//! Uses the OpenAI `/v1/batches` endpoint for 50% cost savings on large-scale
//! entity extraction jobs. Batch jobs complete within 24 hours.
//!
//! This module uses `reqwest` directly rather than `async-openai` because the
//! latter doesn't support the Batch API.
//!
//! ## Workflow
//!
//! 1. Upload JSONL file via `/v1/files`
//! 2. Create batch job referencing the file
//! 3. Poll batch status until completion
//! 4. Download results from output file

use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info, warn};

/// OpenAI Batch API client.
pub struct OpenAIBatchClient {
    api_key: String,
    client: reqwest::Client,
    base_url: String,
}

/// Batch job status from OpenAI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchJob {
    pub id: String,
    pub status: BatchStatus,
    pub input_file_id: String,
    pub output_file_id: Option<String>,
    pub error_file_id: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub expired_at: Option<i64>,
    pub request_counts: Option<BatchRequestCounts>,
    pub errors: Option<BatchErrors>,
}

/// Batch job status enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Validating,
    InProgress,
    Completed,
    Failed,
    Expired,
    Cancelling,
    Cancelled,
    Finalizing,
    /// Catch-all for unknown statuses.
    #[serde(other)]
    Unknown,
}

impl BatchStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            BatchStatus::Completed
                | BatchStatus::Failed
                | BatchStatus::Expired
                | BatchStatus::Cancelled
        )
    }

    pub fn is_success(&self) -> bool {
        matches!(self, BatchStatus::Completed)
    }
}

/// Request counts within a batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRequestCounts {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
}

/// Batch-level errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchErrors {
    pub object: Option<String>,
    pub data: Vec<BatchErrorItem>,
}

/// A single error item from a batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchErrorItem {
    pub code: Option<String>,
    pub message: Option<String>,
    pub param: Option<String>,
    pub line: Option<usize>,
}

/// A single result line from batch output JSONL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub id: String,
    pub custom_id: String,
    pub response: Option<BatchResultResponse>,
    pub error: Option<BatchResultError>,
}

/// Response body within a batch result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResultResponse {
    pub status_code: u16,
    pub body: serde_json::Value,
}

/// Error within a batch result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResultError {
    pub code: Option<String>,
    pub message: Option<String>,
}

/// File upload response from OpenAI.
#[derive(Debug, Deserialize)]
struct FileUploadResponse {
    id: String,
    #[allow(dead_code)]
    filename: String,
}

/// Batch creation response from OpenAI.
#[derive(Debug, Deserialize)]
struct BatchCreateResponse {
    id: String,
    status: BatchStatus,
    input_file_id: String,
    output_file_id: Option<String>,
    error_file_id: Option<String>,
    created_at: i64,
    completed_at: Option<i64>,
    failed_at: Option<i64>,
    expired_at: Option<i64>,
    request_counts: Option<BatchRequestCounts>,
    errors: Option<BatchErrors>,
}

impl From<BatchCreateResponse> for BatchJob {
    fn from(r: BatchCreateResponse) -> Self {
        BatchJob {
            id: r.id,
            status: r.status,
            input_file_id: r.input_file_id,
            output_file_id: r.output_file_id,
            error_file_id: r.error_file_id,
            created_at: r.created_at,
            completed_at: r.completed_at,
            failed_at: r.failed_at,
            expired_at: r.expired_at,
            request_counts: r.request_counts,
            errors: r.errors,
        }
    }
}

/// Errors from the batch client.
#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Batch job failed: {0}")]
    JobFailed(String),
}

pub type Result<T> = std::result::Result<T, BatchError>;

impl OpenAIBatchClient {
    /// Create a new batch client with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: reqwest::Client::new(),
            base_url: "https://api.openai.com".to_string(),
        }
    }

    /// Create with a custom base URL (for testing or proxy).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Upload a JSONL file for batch processing.
    ///
    /// Returns the file ID for use in `create_batch`.
    pub async fn upload_jsonl(&self, path: &Path) -> Result<String> {
        let file_bytes = tokio::fs::read(path).await?;
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("batch.jsonl")
            .to_string();

        let file_size_mb = file_bytes.len() as f64 / 1_048_576.0;
        info!(
            file = %path.display(),
            size_mb = format!("{:.1}", file_size_mb),
            "Uploading JSONL file to OpenAI"
        );

        let max_retries = 3;
        let mut last_err = None;

        for attempt in 0..max_retries {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(2u64.pow(attempt as u32));
                warn!(
                    attempt = attempt + 1,
                    delay_secs = delay.as_secs(),
                    "Retrying JSONL upload after error"
                );
                tokio::time::sleep(delay).await;
            }

            let part = reqwest::multipart::Part::bytes(file_bytes.clone())
                .file_name(file_name.clone())
                .mime_str("application/jsonl")
                .map_err(|e| BatchError::Api {
                    status: 0,
                    message: format!("Failed to set MIME type: {}", e),
                })?;

            let form = reqwest::multipart::Form::new()
                .text("purpose", "batch")
                .part("file", part);

            let result = self
                .client
                .post(format!("{}/v1/files", self.base_url))
                .bearer_auth(&self.api_key)
                .multipart(form)
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        return Err(BatchError::Api {
                            status: status.as_u16(),
                            message: body,
                        });
                    }

                    let upload_resp: FileUploadResponse = resp.json().await?;
                    info!(file_id = %upload_resp.id, "JSONL file uploaded successfully");
                    return Ok(upload_resp.id);
                }
                Err(e) => {
                    warn!(
                        attempt = attempt + 1,
                        error = %e,
                        "JSONL upload failed"
                    );
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap().into())
    }

    /// Create a batch job from an uploaded file.
    ///
    /// Returns the batch job with initial status.
    pub async fn create_batch(&self, file_id: &str, model: &str) -> Result<BatchJob> {
        info!(file_id = %file_id, model = %model, "Creating batch job");

        let body = serde_json::json!({
            "input_file_id": file_id,
            "endpoint": "/v1/chat/completions",
            "completion_window": "24h",
            "metadata": {
                "tool": "edgequake-batch",
                "model": model,
            }
        });

        let resp = self
            .client
            .post(format!("{}/v1/batches", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(BatchError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        let batch_resp: BatchCreateResponse = resp.json().await?;
        info!(batch_id = %batch_resp.id, status = ?batch_resp.status, "Batch job created");
        Ok(batch_resp.into())
    }

    /// Get the current status of a batch job.
    pub async fn get_batch(&self, batch_id: &str) -> Result<BatchJob> {
        debug!(batch_id = %batch_id, "Polling batch status");

        let resp = self
            .client
            .get(format!("{}/v1/batches/{}", self.base_url, batch_id))
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(BatchError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        let batch_resp: BatchCreateResponse = resp.json().await?;
        Ok(batch_resp.into())
    }

    /// Download and parse batch results from the output file.
    ///
    /// Each line in the output JSONL is a `BatchResult` with the original
    /// `custom_id` for correlation back to input chunks.
    pub async fn download_results(&self, output_file_id: &str) -> Result<Vec<BatchResult>> {
        info!(file_id = %output_file_id, "Downloading batch results");

        let resp = self
            .client
            .get(format!(
                "{}/v1/files/{}/content",
                self.base_url, output_file_id
            ))
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(BatchError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        let body = resp.text().await?;
        let mut results = Vec::new();

        for (line_num, line) in body.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<BatchResult>(line) {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!(
                        line = line_num + 1,
                        error = %e,
                        "Failed to parse batch result line, skipping"
                    );
                }
            }
        }

        info!(
            total_results = results.len(),
            "Batch results downloaded and parsed"
        );
        Ok(results)
    }

    /// Download error results from the error file (if any).
    pub async fn download_errors(&self, error_file_id: &str) -> Result<Vec<BatchResult>> {
        self.download_results(error_file_id).await
    }

    /// Cancel a batch job.
    pub async fn cancel_batch(&self, batch_id: &str) -> Result<()> {
        info!(batch_id = %batch_id, "Cancelling batch job");

        let resp = self
            .client
            .post(format!("{}/v1/batches/{}/cancel", self.base_url, batch_id))
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(BatchError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        info!(batch_id = %batch_id, "Batch job cancelled");
        Ok(())
    }

    /// Extract the LLM response content from a batch result.
    ///
    /// Navigates the OpenAI response structure to get the assistant's message content.
    pub fn extract_content(result: &BatchResult) -> Option<String> {
        let response = result.response.as_ref()?;
        if response.status_code != 200 {
            return None;
        }

        response
            .body
            .get("choices")?
            .as_array()?
            .first()?
            .get("message")?
            .get("content")?
            .as_str()
            .map(|s| s.to_string())
    }
}

/// Build a single JSONL request line for the OpenAI Batch API.
///
/// # Arguments
/// * `custom_id` - Unique ID for correlating results (e.g., "chunk-{hash}-{idx}")
/// * `model` - OpenAI model name (e.g., "gpt-4.1-mini")
/// * `system_prompt` - Static system prompt (cached across requests)
/// * `user_prompt` - Variable user prompt with chunk text
/// * `max_tokens` - Maximum output tokens
/// * `temperature` - Sampling temperature (0.0 for deterministic extraction)
pub fn build_jsonl_request(
    custom_id: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> serde_json::Value {
    serde_json::json!({
        "custom_id": custom_id,
        "method": "POST",
        "url": "/v1/chat/completions",
        "body": {
            "model": model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "max_tokens": max_tokens,
            "temperature": temperature
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_status_terminal() {
        assert!(BatchStatus::Completed.is_terminal());
        assert!(BatchStatus::Failed.is_terminal());
        assert!(BatchStatus::Expired.is_terminal());
        assert!(BatchStatus::Cancelled.is_terminal());
        assert!(!BatchStatus::InProgress.is_terminal());
        assert!(!BatchStatus::Validating.is_terminal());
        assert!(!BatchStatus::Finalizing.is_terminal());
    }

    #[test]
    fn test_batch_status_success() {
        assert!(BatchStatus::Completed.is_success());
        assert!(!BatchStatus::Failed.is_success());
        assert!(!BatchStatus::InProgress.is_success());
    }

    #[test]
    fn test_build_jsonl_request() {
        let req = build_jsonl_request(
            "chunk-abc12345-0",
            "gpt-4.1-mini",
            "You are a KG specialist.",
            "Extract entities from: test text",
            4096,
            0.0,
        );

        assert_eq!(req["custom_id"], "chunk-abc12345-0");
        assert_eq!(req["method"], "POST");
        assert_eq!(req["url"], "/v1/chat/completions");
        assert_eq!(req["body"]["model"], "gpt-4.1-mini");
        assert_eq!(req["body"]["messages"].as_array().unwrap().len(), 2);
        assert_eq!(req["body"]["max_tokens"], 4096);
        assert_eq!(req["body"]["temperature"], 0.0);
    }

    #[test]
    fn test_extract_content() {
        let result = BatchResult {
            id: "resp-1".to_string(),
            custom_id: "chunk-abc-0".to_string(),
            response: Some(BatchResultResponse {
                status_code: 200,
                body: serde_json::json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "entity<|#|>Test<|#|>PERSON<|#|>A test person."
                        }
                    }]
                }),
            }),
            error: None,
        };

        let content = OpenAIBatchClient::extract_content(&result);
        assert_eq!(
            content,
            Some("entity<|#|>Test<|#|>PERSON<|#|>A test person.".to_string())
        );
    }

    #[test]
    fn test_extract_content_error_response() {
        let result = BatchResult {
            id: "resp-2".to_string(),
            custom_id: "chunk-abc-1".to_string(),
            response: Some(BatchResultResponse {
                status_code: 429,
                body: serde_json::json!({"error": "rate limited"}),
            }),
            error: None,
        };

        assert!(OpenAIBatchClient::extract_content(&result).is_none());
    }

    #[test]
    fn test_extract_content_no_response() {
        let result = BatchResult {
            id: "resp-3".to_string(),
            custom_id: "chunk-abc-2".to_string(),
            response: None,
            error: Some(BatchResultError {
                code: Some("server_error".to_string()),
                message: Some("Internal error".to_string()),
            }),
        };

        assert!(OpenAIBatchClient::extract_content(&result).is_none());
    }

    #[test]
    fn test_batch_result_deserialization() {
        let json = r#"{
            "id": "batch_req_123",
            "custom_id": "chunk-abc12345-0",
            "response": {
                "status_code": 200,
                "body": {
                    "id": "chatcmpl-xyz",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "entity<|#|>Test<|#|>PERSON<|#|>desc"
                        },
                        "finish_reason": "stop"
                    }]
                }
            },
            "error": null
        }"#;

        let result: BatchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.custom_id, "chunk-abc12345-0");
        assert!(result.response.is_some());
        assert!(result.error.is_none());
    }
}
