//! Anthropic Message Batches API client for cost-effective bulk extraction.
//!
//! Uses the Anthropic `/v1/messages/batches` endpoint for 50% cost savings on
//! large-scale entity extraction jobs. Batch jobs complete within 24 hours.
//!
//! ## Key Differences from OpenAI Batch API
//!
//! - Requests are sent inline in the POST body (no separate file upload)
//! - Maximum 10,000 requests per batch (vs 50,000 for OpenAI)
//! - Results are streamed from a `results_url` (vs downloading a file)
//! - Auth uses `x-api-key` header + `anthropic-version` header
//!
//! ## Workflow
//!
//! 1. Read JSONL file and parse into request objects
//! 2. Create batch with inline requests via POST `/v1/messages/batches`
//! 3. Poll batch status until completion
//! 4. Stream results from `results_url`

use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info, warn};

// Re-use the BatchError type from openai_batch for consistency.
pub use super::openai_batch::{BatchError, Result};

/// Anthropic Message Batches API client.
pub struct AnthropicBatchClient {
    api_key: String,
    client: reqwest::Client,
    base_url: String,
}

/// Batch job status from Anthropic API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicBatchJob {
    pub id: String,
    pub processing_status: AnthropicBatchStatus,
    pub request_counts: AnthropicRequestCounts,
    pub created_at: String,
    pub ended_at: Option<String>,
    pub results_url: Option<String>,
}

/// Anthropic batch processing status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnthropicBatchStatus {
    InProgress,
    Ended,
    Canceling,
    /// Catch-all for unknown statuses.
    #[serde(other)]
    Unknown,
}

impl AnthropicBatchStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, AnthropicBatchStatus::Ended)
    }

    pub fn is_success(&self) -> bool {
        matches!(self, AnthropicBatchStatus::Ended)
    }
}

/// Request counts within an Anthropic batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicRequestCounts {
    pub processing: usize,
    pub succeeded: usize,
    pub errored: usize,
    pub canceled: usize,
    pub expired: usize,
}

/// A single request in the Anthropic batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicBatchRequest {
    pub custom_id: String,
    pub params: AnthropicBatchParams,
}

/// Parameters for a single Anthropic batch request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicBatchParams {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

/// Anthropic message format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

/// A single result line from Anthropic batch results JSONL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicBatchResult {
    pub custom_id: String,
    pub result: AnthropicResultBody,
}

/// Result body (tagged union on `type` field).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicResultBody {
    #[serde(rename = "succeeded")]
    Succeeded { message: AnthropicResultMessage },
    #[serde(rename = "errored")]
    Errored { error: AnthropicResultError },
    #[serde(rename = "canceled")]
    Canceled,
    #[serde(rename = "expired")]
    Expired,
}

/// Successful message response within a batch result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicResultMessage {
    pub id: String,
    pub role: String,
    pub content: Vec<AnthropicContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<serde_json::Value>,
}

/// Content block in an Anthropic response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(other)]
    Other,
}

/// Error within a batch result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicResultError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

/// Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Beta header for Message Batches API.
const ANTHROPIC_BETA: &str = "message-batches-2024-09-24";

impl AnthropicBatchClient {
    /// Create a new batch client with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: reqwest::Client::new(),
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Create with a custom base URL (for testing or proxy).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Add Anthropic auth headers to a request builder.
    fn auth_headers(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        builder
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", ANTHROPIC_BETA)
    }

    /// Create a batch job from a JSONL file.
    ///
    /// Reads the JSONL file, parses each line into an `AnthropicBatchRequest`,
    /// and sends them inline in the POST body. Anthropic does not use separate
    /// file uploads like OpenAI.
    ///
    /// Returns the batch job with initial status.
    pub async fn create_batch(&self, jsonl_path: &Path) -> Result<AnthropicBatchJob> {
        let content = tokio::fs::read_to_string(jsonl_path).await?;

        let requests: Vec<AnthropicBatchRequest> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
            .filter_map(|(i, line)| {
                match serde_json::from_str::<AnthropicBatchRequest>(line) {
                    Ok(req) => Some(req),
                    Err(e) => {
                        warn!(line = i + 1, error = %e, "Failed to parse JSONL line, skipping");
                        None
                    }
                }
            })
            .collect();

        if requests.is_empty() {
            return Err(BatchError::JobFailed(
                "No valid requests found in JSONL file".to_string(),
            ));
        }

        info!(
            requests = requests.len(),
            path = %jsonl_path.display(),
            "Creating Anthropic batch with inline requests"
        );

        let body = serde_json::json!({
            "requests": requests,
        });

        let resp = self
            .auth_headers(
                self.client
                    .post(format!("{}/v1/messages/batches", self.base_url)),
            )
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

        let batch: AnthropicBatchJob = resp.json().await?;
        info!(batch_id = %batch.id, status = ?batch.processing_status, "Anthropic batch created");
        Ok(batch)
    }

    /// Get the current status of a batch job.
    pub async fn get_batch(&self, batch_id: &str) -> Result<AnthropicBatchJob> {
        debug!(batch_id = %batch_id, "Polling Anthropic batch status");

        let resp = self
            .auth_headers(
                self.client.get(format!(
                    "{}/v1/messages/batches/{}",
                    self.base_url, batch_id
                )),
            )
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

        let batch: AnthropicBatchJob = resp.json().await?;
        Ok(batch)
    }

    /// Download and parse batch results from the results URL.
    ///
    /// Anthropic returns results as JSONL streamed from `results_url`.
    pub async fn download_results(
        &self,
        results_url: &str,
    ) -> Result<Vec<AnthropicBatchResult>> {
        info!(url = %results_url, "Downloading Anthropic batch results");

        let resp = self
            .auth_headers(self.client.get(results_url))
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
            match serde_json::from_str::<AnthropicBatchResult>(line) {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!(
                        line = line_num + 1,
                        error = %e,
                        "Failed to parse Anthropic batch result line, skipping"
                    );
                }
            }
        }

        info!(
            total_results = results.len(),
            "Anthropic batch results downloaded and parsed"
        );
        Ok(results)
    }

    /// List batch jobs.
    pub async fn list_batches(&self, limit: Option<usize>) -> Result<Vec<AnthropicBatchJob>> {
        debug!(limit = ?limit, "Listing Anthropic batch jobs");

        let limit = limit.unwrap_or(20).min(100);
        let resp = self
            .auth_headers(
                self.client
                    .get(format!("{}/v1/messages/batches", self.base_url)),
            )
            .query(&[("limit", limit)])
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

        #[derive(Deserialize)]
        struct BatchListResponse {
            data: Vec<AnthropicBatchJob>,
        }

        let list_resp: BatchListResponse = resp.json().await?;
        Ok(list_resp.data)
    }

    /// Cancel a batch job.
    pub async fn cancel_batch(&self, batch_id: &str) -> Result<()> {
        info!(batch_id = %batch_id, "Cancelling Anthropic batch job");

        let resp = self
            .auth_headers(
                self.client.post(format!(
                    "{}/v1/messages/batches/{}/cancel",
                    self.base_url, batch_id
                )),
            )
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

        info!(batch_id = %batch_id, "Anthropic batch cancellation requested");
        Ok(())
    }

    /// Extract the LLM response content from an Anthropic batch result.
    ///
    /// Navigates `result.message.content[0].text`.
    pub fn extract_content(result: &AnthropicBatchResult) -> Option<String> {
        match &result.result {
            AnthropicResultBody::Succeeded { message } => {
                for block in &message.content {
                    if let AnthropicContentBlock::Text { text } = block {
                        return Some(text.clone());
                    }
                }
                None
            }
            _ => None,
        }
    }
}

/// Build a single JSONL request line for the Anthropic Message Batches API.
///
/// # Arguments
/// * `custom_id` - Unique ID for correlating results (e.g., "chunk-{hash}-{idx}")
/// * `model` - Anthropic model name (e.g., "claude-haiku-4-5-20251001")
/// * `system_prompt` - System prompt (sent as top-level `system` parameter)
/// * `user_prompt` - User prompt with chunk text
/// * `max_tokens` - Maximum output tokens
/// * `temperature` - Sampling temperature (0.0 for deterministic extraction)
pub fn build_anthropic_jsonl_request(
    custom_id: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> serde_json::Value {
    serde_json::json!({
        "custom_id": custom_id,
        "params": {
            "model": model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "system": system_prompt,
            "messages": [
                {"role": "user", "content": user_prompt}
            ]
        }
    })
}

/// Check if a model name indicates an Anthropic (Claude) model.
pub fn is_anthropic_model(model: &str) -> bool {
    model.starts_with("claude")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_batch_status_terminal() {
        assert!(AnthropicBatchStatus::Ended.is_terminal());
        assert!(!AnthropicBatchStatus::InProgress.is_terminal());
        assert!(!AnthropicBatchStatus::Canceling.is_terminal());
    }

    #[test]
    fn test_build_anthropic_jsonl_request() {
        let req = build_anthropic_jsonl_request(
            "chunk-abc12345-0",
            "claude-haiku-4-5-20251001",
            "You are a KG specialist.",
            "Extract entities from: test text",
            4096,
            0.0,
        );

        assert_eq!(req["custom_id"], "chunk-abc12345-0");
        assert_eq!(req["params"]["model"], "claude-haiku-4-5-20251001");
        assert_eq!(req["params"]["max_tokens"], 4096);
        assert_eq!(req["params"]["temperature"], 0.0);
        assert_eq!(req["params"]["system"], "You are a KG specialist.");
        assert_eq!(req["params"]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(req["params"]["messages"][0]["role"], "user");
    }

    #[test]
    fn test_is_anthropic_model() {
        assert!(is_anthropic_model("claude-haiku-4-5-20251001"));
        assert!(is_anthropic_model("claude-sonnet-4-20250514"));
        assert!(is_anthropic_model("claude-opus-4-6"));
        assert!(!is_anthropic_model("gpt-4o"));
        assert!(!is_anthropic_model("gpt-4.1-mini"));
    }

    #[test]
    fn test_extract_content_succeeded() {
        let result = AnthropicBatchResult {
            custom_id: "chunk-abc-0".to_string(),
            result: AnthropicResultBody::Succeeded {
                message: AnthropicResultMessage {
                    id: "msg_123".to_string(),
                    role: "assistant".to_string(),
                    content: vec![AnthropicContentBlock::Text {
                        text: "entity<|#|>Test<|#|>PERSON<|#|>A test person.".to_string(),
                    }],
                    stop_reason: Some("end_turn".to_string()),
                    usage: None,
                },
            },
        };

        let content = AnthropicBatchClient::extract_content(&result);
        assert_eq!(
            content,
            Some("entity<|#|>Test<|#|>PERSON<|#|>A test person.".to_string())
        );
    }

    #[test]
    fn test_extract_content_errored() {
        let result = AnthropicBatchResult {
            custom_id: "chunk-abc-1".to_string(),
            result: AnthropicResultBody::Errored {
                error: AnthropicResultError {
                    error_type: "server_error".to_string(),
                    message: "Internal error".to_string(),
                },
            },
        };

        assert!(AnthropicBatchClient::extract_content(&result).is_none());
    }

    #[test]
    fn test_extract_content_expired() {
        let result = AnthropicBatchResult {
            custom_id: "chunk-abc-2".to_string(),
            result: AnthropicResultBody::Expired,
        };

        assert!(AnthropicBatchClient::extract_content(&result).is_none());
    }

    #[test]
    fn test_batch_result_deserialization_succeeded() {
        let json = r#"{
            "custom_id": "chunk-abc12345-0",
            "result": {
                "type": "succeeded",
                "message": {
                    "id": "msg_abc",
                    "role": "assistant",
                    "content": [{
                        "type": "text",
                        "text": "entity<|#|>Test<|#|>PERSON<|#|>desc"
                    }],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 100, "output_tokens": 50}
                }
            }
        }"#;

        let result: AnthropicBatchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.custom_id, "chunk-abc12345-0");
        assert!(matches!(
            result.result,
            AnthropicResultBody::Succeeded { .. }
        ));
    }

    #[test]
    fn test_batch_result_deserialization_errored() {
        let json = r#"{
            "custom_id": "chunk-abc12345-1",
            "result": {
                "type": "errored",
                "error": {
                    "type": "server_error",
                    "message": "Internal server error"
                }
            }
        }"#;

        let result: AnthropicBatchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.custom_id, "chunk-abc12345-1");
        assert!(matches!(
            result.result,
            AnthropicResultBody::Errored { .. }
        ));
    }

    #[test]
    fn test_anthropic_batch_job_deserialization() {
        let json = r#"{
            "id": "msgbatch_abc123",
            "processing_status": "in_progress",
            "request_counts": {
                "processing": 50,
                "succeeded": 0,
                "errored": 0,
                "canceled": 0,
                "expired": 0
            },
            "created_at": "2024-01-01T00:00:00Z",
            "ended_at": null,
            "results_url": null
        }"#;

        let batch: AnthropicBatchJob = serde_json::from_str(json).unwrap();
        assert_eq!(batch.id, "msgbatch_abc123");
        assert_eq!(batch.processing_status, AnthropicBatchStatus::InProgress);
        assert_eq!(batch.request_counts.processing, 50);
        assert!(batch.results_url.is_none());
    }
}
