//! DynamoDB key-value storage.
//!
//! This module provides KV storage using Amazon DynamoDB, a fully managed
//! NoSQL database service with single-digit millisecond latency.
//!
//! ## Features
//!
//! - **Low latency**: Single-digit millisecond response times
//! - **Scalable**: Auto-scales with demand (on-demand mode)
//! - **Cost-effective**: Pay per request, no idle costs
//! - **Durable**: 99.999999999% (11 nines) durability
//! - **Point-in-time recovery**: Continuous backups
//! - **Encryption**: At rest and in transit
//!
//! ## Table Design
//!
//! ```text
//! Primary Key:
//! - Partition Key: namespace (String)
//! - Sort Key: id (String)
//!
//! Attributes:
//! - data (String): JSON-serialized value
//! - status (String, optional): For status tracking
//! - created_at (Number): Creation timestamp
//! - updated_at (Number): Last update timestamp
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use edgequake_storage_aws::{DynamoKVConfig, DynamoKVStorage};
//! use edgequake_storage::KVStorage;
//!
//! let config = DynamoKVConfig::new("edgequake-kv", "documents");
//! let storage = DynamoKVStorage::new(config).await?;
//!
//! storage.initialize().await?;
//!
//! // Upsert data
//! let data = vec![
//!     ("doc-001".to_string(), json!({"title": "Document 1"}))
//! ];
//! storage.upsert(&data).await?;
//!
//! // Get data
//! let result = storage.get_by_id("doc-001").await?;
//! ```

use std::collections::HashSet;

use async_trait::async_trait;
use aws_sdk_dynamodb::types::{
    AttributeValue, KeysAndAttributes, PutRequest, ReturnValue, WriteRequest,
};
use aws_sdk_dynamodb::Client as DynamoDbClient;
use serde_json::Value as JsonValue;
use tracing::{debug, info, warn};

use edgequake_storage::{KVStorage, StorageError};

use crate::error::AwsStorageError;

/// Extract a descriptive error message from an AWS SDK error.
///
/// `SdkError::to_string()` often returns just "service error" which is useless
/// for debugging. This helper extracts the actual service error message.
fn dynamo_err<E, R>(err: aws_smithy_runtime_api::client::result::SdkError<E, R>) -> AwsStorageError
where
    E: std::fmt::Display + std::fmt::Debug,
    R: std::fmt::Debug,
{
    let msg = match &err {
        aws_smithy_runtime_api::client::result::SdkError::ServiceError(ctx) => {
            // Use both Display and Debug to capture full error detail;
            // Display on unhandled errors often says just "unhandled error"
            let display = format!("{}", ctx.err());
            let debug = format!("{:?}", ctx.err());
            if display.contains("unhandled") || display == "service error" {
                debug
            } else {
                display
            }
        }
        other => format!("{:?}", other),
    };
    AwsStorageError::DynamoDbError(msg)
}

/// Configuration for DynamoDB KV storage.
#[derive(Debug, Clone)]
pub struct DynamoKVConfig {
    /// DynamoDB table name
    pub table_name: String,
    /// Storage namespace (workspace ID)
    pub namespace: String,
    /// AWS region (optional, uses default if not set)
    pub region: Option<String>,
    /// Enable on-demand billing (vs provisioned capacity)
    pub on_demand: bool,
    /// Read capacity units (if not on-demand)
    pub read_capacity: i64,
    /// Write capacity units (if not on-demand)
    pub write_capacity: i64,
    /// Enable point-in-time recovery
    pub enable_pitr: bool,
}

impl DynamoKVConfig {
    /// Create a new DynamoDB configuration.
    ///
    /// # Arguments
    ///
    /// * `table_name` - DynamoDB table name
    /// * `namespace` - Storage namespace (typically workspace ID)
    pub fn new(table_name: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            table_name: table_name.into(),
            namespace: namespace.into(),
            region: None,
            on_demand: true, // Pay-per-request by default
            read_capacity: 5,
            write_capacity: 5,
            enable_pitr: true,
        }
    }

    /// Set the AWS region.
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Enable or disable on-demand billing.
    pub fn with_on_demand(mut self, on_demand: bool) -> Self {
        self.on_demand = on_demand;
        self
    }

    /// Set provisioned capacity (only used if on_demand is false).
    pub fn with_capacity(mut self, read: i64, write: i64) -> Self {
        self.read_capacity = read;
        self.write_capacity = write;
        self
    }

    /// Enable or disable point-in-time recovery.
    pub fn with_pitr(mut self, enable: bool) -> Self {
        self.enable_pitr = enable;
        self
    }
}

/// DynamoDB key-value storage.
///
/// Provides low-latency key-value storage using Amazon DynamoDB.
/// Suitable for hot data that requires fast access (< 10ms).
pub struct DynamoKVStorage {
    config: DynamoKVConfig,
    client: DynamoDbClient,
}

impl DynamoKVStorage {
    /// Create a new DynamoDB KV storage.
    ///
    /// # Arguments
    ///
    /// * `config` - DynamoDB configuration
    pub async fn new(config: DynamoKVConfig) -> crate::error::Result<Self> {
        // Load AWS configuration
        let aws_config = if let Some(region) = &config.region {
            aws_config::from_env()
                .region(aws_sdk_dynamodb::config::Region::new(region.clone()))
                .load()
                .await
        } else {
            aws_config::load_from_env().await
        };

        let client = DynamoDbClient::new(&aws_config);

        Ok(Self { config, client })
    }

    /// Create a new DynamoDB KV storage with a pre-configured client.
    ///
    /// Use this when you already have an AWS SDK client (e.g., in batch
    /// processing where the caller manages AWS configuration).
    pub fn new_with_client(config: DynamoKVConfig, client: DynamoDbClient) -> Self {
        Self { config, client }
    }

    /// Convert namespace and id to DynamoDB key.
    fn make_key(&self, id: &str) -> std::collections::HashMap<String, AttributeValue> {
        let mut key = std::collections::HashMap::new();
        key.insert(
            "namespace".to_string(),
            AttributeValue::S(self.config.namespace.clone()),
        );
        key.insert("id".to_string(), AttributeValue::S(id.to_string()));
        key
    }

    /// Parse DynamoDB item to JSON value.
    fn parse_item(
        &self,
        item: &std::collections::HashMap<String, AttributeValue>,
    ) -> crate::error::Result<JsonValue> {
        let data_attr = item
            .get("data")
            .ok_or_else(|| AwsStorageError::DynamoDbError("Missing 'data' attribute".into()))?;

        let json_str = data_attr
            .as_s()
            .map_err(|_| AwsStorageError::DynamoDbError("'data' is not a string".into()))?;

        let value: JsonValue = serde_json::from_str(json_str)?;
        Ok(value)
    }

    /// Get current timestamp in seconds.
    fn timestamp() -> i64 {
        chrono::Utc::now().timestamp()
    }
}

#[async_trait]
impl KVStorage for DynamoKVStorage {
    fn namespace(&self) -> &str {
        &self.config.namespace
    }

    async fn initialize(&self) -> edgequake_storage::error::Result<()> {
        info!(
            "Initializing DynamoDB KV storage: table={}, namespace={}",
            self.config.table_name, self.config.namespace
        );

        // Check if table exists
        match self
            .client
            .describe_table()
            .table_name(&self.config.table_name)
            .send()
            .await
        {
            Ok(output) => {
                let status = output
                    .table()
                    .and_then(|t| t.table_status())
                    .map(|s| format!("{:?}", s))
                    .unwrap_or_else(|| "Unknown".to_string());

                info!(
                    "DynamoDB table '{}' exists with status: {}",
                    self.config.table_name, status
                );
            }
            Err(e) => {
                warn!(
                    "Table '{}' does not exist or cannot be accessed: {}",
                    self.config.table_name, e
                );
                return Err(AwsStorageError::DynamoDbError(format!(
                    "Table '{}' not found. Create it first with: aws dynamodb create-table",
                    self.config.table_name
                ))
                .into());
            }
        }

        info!("DynamoDB KV storage initialized successfully");
        Ok(())
    }

    async fn finalize(&self) -> edgequake_storage::error::Result<()> {
        info!("Finalizing DynamoDB KV storage");
        // DynamoDB doesn't require explicit finalization
        Ok(())
    }

    async fn get_by_id(&self, id: &str) -> edgequake_storage::error::Result<Option<JsonValue>> {
        let key = self.make_key(id);

        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .set_key(Some(key))
            .send()
            .await
            .map_err(dynamo_err)?;

        if let Some(item) = result.item {
            let value = self.parse_item(&item)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    async fn get_by_ids(&self, ids: &[String]) -> edgequake_storage::error::Result<Vec<JsonValue>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        // DynamoDB BatchGetItem has a limit of 100 items
        for chunk in ids.chunks(100) {
            let keys: Vec<_> = chunk.iter().map(|id| self.make_key(id)).collect();

            let keys_and_attrs = KeysAndAttributes::builder()
                .set_keys(Some(keys))
                .build()
                .map_err(|e| AwsStorageError::DynamoDbError(e.to_string()))?;

            let response = self
                .client
                .batch_get_item()
                .request_items(&self.config.table_name, keys_and_attrs)
                .send()
                .await
                .map_err(dynamo_err)?;

            if let Some(responses) = response.responses {
                if let Some(items) = responses.get(&self.config.table_name) {
                    for item in items {
                        if let Ok(value) = self.parse_item(item) {
                            results.push(value);
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    async fn filter_keys(
        &self,
        keys: HashSet<String>,
    ) -> edgequake_storage::error::Result<HashSet<String>> {
        if keys.is_empty() {
            return Ok(HashSet::new());
        }

        let mut missing_keys = keys.clone();

        // Check which keys exist
        for chunk in keys.iter().collect::<Vec<_>>().chunks(100) {
            let dynamo_keys: Vec<_> = chunk.iter().map(|id| self.make_key(id)).collect();

            let keys_and_attrs = KeysAndAttributes::builder()
                .set_keys(Some(dynamo_keys))
                .build()
                .map_err(|e| AwsStorageError::DynamoDbError(e.to_string()))?;

            let response = self
                .client
                .batch_get_item()
                .request_items(&self.config.table_name, keys_and_attrs)
                .send()
                .await
                .map_err(dynamo_err)?;

            if let Some(responses) = response.responses {
                if let Some(items) = responses.get(&self.config.table_name) {
                    for item in items {
                        if let Some(AttributeValue::S(id)) = item.get("id") {
                            missing_keys.remove(id.as_str());
                        }
                    }
                }
            }
        }

        Ok(missing_keys)
    }

    async fn upsert(&self, data: &[(String, JsonValue)]) -> edgequake_storage::error::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        debug!("Upserting {} items to DynamoDB", data.len());

        let timestamp = Self::timestamp();
        const MAX_RETRIES: u32 = 8;

        // DynamoDB BatchWriteItem has a limit of 25 items
        for chunk in data.chunks(25) {
            let mut write_requests: Vec<WriteRequest> = Vec::new();

            for (id, value) in chunk {
                let json_str = serde_json::to_string(value)?;

                let mut item = std::collections::HashMap::new();
                item.insert(
                    "namespace".to_string(),
                    AttributeValue::S(self.config.namespace.clone()),
                );
                item.insert("id".to_string(), AttributeValue::S(id.clone()));
                item.insert("data".to_string(), AttributeValue::S(json_str));
                item.insert(
                    "updated_at".to_string(),
                    AttributeValue::N(timestamp.to_string()),
                );

                // Add created_at only if it doesn't exist (handled by update expression in production)
                item.insert(
                    "created_at".to_string(),
                    AttributeValue::N(timestamp.to_string()),
                );

                let put_request = PutRequest::builder()
                    .set_item(Some(item))
                    .build()
                    .map_err(|e| AwsStorageError::DynamoDbError(e.to_string()))?;

                let write_request = WriteRequest::builder().put_request(put_request).build();

                write_requests.push(write_request);
            }

            // Retry loop for unprocessed items with exponential backoff
            let mut attempt = 0u32;
            loop {
                let result = self
                    .client
                    .batch_write_item()
                    .request_items(&self.config.table_name, write_requests.clone())
                    .send()
                    .await
                    .map_err(dynamo_err)?;

                // Check for unprocessed items
                let unprocessed = result
                    .unprocessed_items()
                    .and_then(|items| items.get(&self.config.table_name))
                    .map(|items| items.to_vec())
                    .unwrap_or_default();

                if unprocessed.is_empty() {
                    break;
                }

                attempt += 1;
                if attempt >= MAX_RETRIES {
                    warn!(
                        unprocessed = unprocessed.len(),
                        attempts = attempt,
                        "Giving up on unprocessed items after max retries"
                    );
                    return Err(AwsStorageError::DynamoDbError(format!(
                        "{} items still unprocessed after {} retries",
                        unprocessed.len(),
                        MAX_RETRIES
                    ))
                    .into());
                }

                debug!(
                    unprocessed = unprocessed.len(),
                    attempt = attempt,
                    "Retrying unprocessed items with backoff"
                );

                // Exponential backoff: 50ms, 100ms, 200ms, 400ms, 800ms, 1.6s, 3.2s, 6.4s
                let delay_ms = 50 * (1u64 << attempt);
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                write_requests = unprocessed;
            }
        }

        debug!("Upserted {} items successfully", data.len());
        Ok(())
    }

    async fn delete(&self, ids: &[String]) -> edgequake_storage::error::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        info!("Deleting {} items from DynamoDB", ids.len());

        // DynamoDB BatchWriteItem has a limit of 25 items
        for chunk in ids.chunks(25) {
            let mut write_requests = Vec::new();

            for id in chunk {
                let key = self.make_key(id);

                let delete_request = aws_sdk_dynamodb::types::DeleteRequest::builder()
                    .set_key(Some(key))
                    .build()
                    .map_err(|e| AwsStorageError::DynamoDbError(e.to_string()))?;

                let write_request = WriteRequest::builder()
                    .delete_request(delete_request)
                    .build();

                write_requests.push(write_request);
            }

            self.client
                .batch_write_item()
                .request_items(&self.config.table_name, write_requests)
                .send()
                .await
                .map_err(dynamo_err)?;
        }

        debug!("Deleted {} items successfully", ids.len());
        Ok(())
    }

    async fn is_empty(&self) -> edgequake_storage::error::Result<bool> {
        let count = self.count().await?;
        Ok(count == 0)
    }

    async fn count(&self) -> edgequake_storage::error::Result<usize> {
        // Query with count only
        let result = self
            .client
            .query()
            .table_name(&self.config.table_name)
            .key_condition_expression("#ns = :namespace")
            .expression_attribute_names("#ns", "namespace")
            .expression_attribute_values(
                ":namespace",
                AttributeValue::S(self.config.namespace.clone()),
            )
            .select(aws_sdk_dynamodb::types::Select::Count)
            .send()
            .await
            .map_err(dynamo_err)?;

        Ok(result.count() as usize)
    }

    async fn count_by_prefix(&self, prefix: &str) -> edgequake_storage::error::Result<usize> {
        let result = self
            .client
            .query()
            .table_name(&self.config.table_name)
            .key_condition_expression("#ns = :namespace AND begins_with(#id, :prefix)")
            .expression_attribute_names("#ns", "namespace")
            .expression_attribute_names("#id", "id")
            .expression_attribute_values(
                ":namespace",
                AttributeValue::S(self.config.namespace.clone()),
            )
            .expression_attribute_values(":prefix", AttributeValue::S(prefix.to_string()))
            .select(aws_sdk_dynamodb::types::Select::Count)
            .send()
            .await
            .map_err(dynamo_err)?;

        Ok(result.count() as usize)
    }

    async fn keys(&self) -> edgequake_storage::error::Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut last_evaluated_key = None;

        loop {
            let mut query = self
                .client
                .query()
                .table_name(&self.config.table_name)
                .key_condition_expression("#ns = :namespace")
                .expression_attribute_names("#ns", "namespace")
                .expression_attribute_values(
                    ":namespace",
                    AttributeValue::S(self.config.namespace.clone()),
                )
                .projection_expression("id");

            if let Some(key) = last_evaluated_key {
                query = query.set_exclusive_start_key(Some(key));
            }

            let result = query.send().await.map_err(dynamo_err)?;

            if let Some(items) = result.items {
                for item in items {
                    if let Some(AttributeValue::S(id)) = item.get("id") {
                        keys.push(id.clone());
                    }
                }
            }

            last_evaluated_key = result.last_evaluated_key;
            if last_evaluated_key.is_none() {
                break;
            }
        }

        Ok(keys)
    }

    async fn clear(&self) -> edgequake_storage::error::Result<()> {
        warn!(
            "Clearing all items for namespace: {}",
            self.config.namespace
        );

        // Get all keys
        let keys = self.keys().await?;

        // Delete in batches
        self.delete(&keys).await?;

        info!("Cleared {} items", keys.len());
        Ok(())
    }

    async fn transition_if_status(
        &self,
        key: &str,
        expected_status: &str,
        new_status: &str,
    ) -> edgequake_storage::error::Result<bool> {
        let dynamo_key = self.make_key(key);

        // Use conditional update to atomically check and set status
        let result = self
            .client
            .update_item()
            .table_name(&self.config.table_name)
            .set_key(Some(dynamo_key))
            .update_expression("SET #status = :new_status, updated_at = :timestamp")
            .condition_expression("#status = :expected_status")
            .expression_attribute_names("#status", "status")
            .expression_attribute_values(
                ":expected_status",
                AttributeValue::S(expected_status.to_string()),
            )
            .expression_attribute_values(":new_status", AttributeValue::S(new_status.to_string()))
            .expression_attribute_values(
                ":timestamp",
                AttributeValue::N(Self::timestamp().to_string()),
            )
            .return_values(ReturnValue::AllNew)
            .send()
            .await;

        match result {
            Ok(_) => {
                debug!(
                    "Status transition successful: {} -> {}",
                    expected_status, new_status
                );
                Ok(true)
            }
            Err(e) => {
                // Check for conditional check failure (expected case)
                let is_condition_failure = match &e {
                    aws_smithy_runtime_api::client::result::SdkError::ServiceError(ctx) => {
                        format!("{}", ctx.err()).contains("ConditionalCheckFailed")
                    }
                    _ => false,
                };
                if is_condition_failure {
                    debug!("Status transition failed: condition not met");
                    Ok(false)
                } else {
                    Err(dynamo_err(e).into())
                }
            }
        }
    }
}
