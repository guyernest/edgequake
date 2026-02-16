//! Amazon Athena SQL query engine for EdgeQuake.
//!
//! Provides serverless SQL analytics over S3-stored data using Amazon Athena.
//! Athena is suitable for:
//! - Analytical queries over large datasets
//! - Metadata searches and filtering
//! - Reporting and batch processing
//! - Ad-hoc data exploration
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │   S3 Data   │  (Parquet/JSON files)
//! └──────┬──────┘
//!        │
//!        ▼
//! ┌─────────────┐
//! │   Athena    │  (Serverless SQL)
//! └──────┬──────┘
//!        │
//!        ▼
//! ┌─────────────┐
//! │  Results    │  (JSON)
//! └─────────────┘
//! ```
//!
//! # Cost Model
//!
//! - $5 per TB of data scanned
//! - No infrastructure costs
//! - Columnar format (Parquet) reduces costs by 90%+
//! - Partitioning further reduces scan costs
//!
//! # Example
//!
//! ```rust,ignore
//! use edgequake_storage_aws::{AthenaConfig, AthenaQueryEngine};
//!
//! let config = AthenaConfig::new(
//!     "my-database",
//!     "s3://my-bucket/athena-results/",
//! );
//!
//! let engine = AthenaQueryEngine::new(config).await?;
//!
//! // Query documents
//! let results = engine.query(
//!     "SELECT * FROM documents WHERE workspace_id = 'ws-123' LIMIT 10"
//! ).await?;
//!
//! // Parse results
//! for row in results {
//!     let id: String = row.get("id")?;
//!     let title: String = row.get("title")?;
//!     println!("{}: {}", id, title);
//! }
//! ```

use crate::error::{AwsStorageError, Result};
use aws_sdk_athena::types::{QueryExecutionContext, QueryExecutionState, ResultConfiguration};
use aws_sdk_athena::Client as AthenaClient;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Configuration for Athena SQL query engine.
#[derive(Debug, Clone)]
pub struct AthenaConfig {
    /// Athena database name (Glue Catalog database)
    pub database: String,

    /// S3 location for query results (e.g., "s3://bucket/athena-results/")
    pub output_location: String,

    /// AWS region (optional, uses default if not set)
    pub region: Option<String>,

    /// Workgroup name (default: "primary")
    pub workgroup: String,

    /// Maximum time to wait for query completion (default: 300s)
    pub query_timeout_secs: u64,

    /// Polling interval for query status (default: 1s)
    pub poll_interval_secs: u64,

    /// Enable query result caching (default: true)
    pub enable_result_cache: bool,
}

impl AthenaConfig {
    /// Create a new Athena configuration.
    ///
    /// # Arguments
    ///
    /// * `database` - Glue Catalog database name
    /// * `output_location` - S3 path for query results (must start with "s3://")
    pub fn new(database: impl Into<String>, output_location: impl Into<String>) -> Self {
        Self {
            database: database.into(),
            output_location: output_location.into(),
            region: None,
            workgroup: "primary".to_string(),
            query_timeout_secs: 300,
            poll_interval_secs: 1,
            enable_result_cache: true,
        }
    }

    /// Set AWS region.
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set Athena workgroup.
    pub fn with_workgroup(mut self, workgroup: impl Into<String>) -> Self {
        self.workgroup = workgroup.into();
        self
    }

    /// Set query timeout in seconds.
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.query_timeout_secs = timeout_secs;
        self
    }

    /// Set polling interval in seconds.
    pub fn with_poll_interval(mut self, interval_secs: u64) -> Self {
        self.poll_interval_secs = interval_secs;
        self
    }

    /// Enable or disable result caching.
    pub fn with_result_cache(mut self, enable: bool) -> Self {
        self.enable_result_cache = enable;
        self
    }
}

/// A row from an Athena query result.
#[derive(Debug, Clone)]
pub struct AthenaRow {
    columns: HashMap<String, String>,
}

impl AthenaRow {
    /// Create a new row from column data.
    pub fn new(columns: HashMap<String, String>) -> Self {
        Self { columns }
    }

    /// Get a column value as a string.
    pub fn get(&self, column: &str) -> Result<String> {
        self.columns
            .get(column)
            .cloned()
            .ok_or_else(|| AwsStorageError::Other(format!("Column not found: {}", column)))
    }

    /// Get a column value as a JSON value.
    pub fn get_json(&self, column: &str) -> Result<JsonValue> {
        let value = self.get(column)?;
        serde_json::from_str(&value).map_err(|e| {
            AwsStorageError::Other(format!(
                "Failed to parse JSON from column {}: {}",
                column, e
            ))
        })
    }

    /// Get a column value as an integer.
    pub fn get_i64(&self, column: &str) -> Result<i64> {
        let value = self.get(column)?;
        value.parse().map_err(|e| {
            AwsStorageError::Other(format!("Failed to parse i64 from column {}: {}", column, e))
        })
    }

    /// Get a column value as a float.
    pub fn get_f64(&self, column: &str) -> Result<f64> {
        let value = self.get(column)?;
        value.parse().map_err(|e| {
            AwsStorageError::Other(format!("Failed to parse f64 from column {}: {}", column, e))
        })
    }

    /// Get a column value as a boolean.
    pub fn get_bool(&self, column: &str) -> Result<bool> {
        let value = self.get(column)?;
        match value.to_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            _ => Err(AwsStorageError::Other(format!(
                "Failed to parse bool from column {}: {}",
                column, value
            ))),
        }
    }

    /// Get all column names.
    pub fn columns(&self) -> Vec<String> {
        self.columns.keys().cloned().collect()
    }

    /// Get all column values as a map.
    pub fn as_map(&self) -> &HashMap<String, String> {
        &self.columns
    }

    /// Convert row to JSON object.
    pub fn to_json(&self) -> JsonValue {
        let mut obj = serde_json::Map::new();
        for (k, v) in &self.columns {
            obj.insert(k.clone(), json!(v));
        }
        JsonValue::Object(obj)
    }
}

/// Query execution statistics.
#[derive(Debug, Clone)]
pub struct QueryStats {
    /// Query execution ID
    pub execution_id: String,

    /// Query execution state
    pub state: String,

    /// Data scanned in bytes
    pub data_scanned_bytes: i64,

    /// Query execution time in milliseconds
    pub execution_time_ms: i64,

    /// Query queue time in milliseconds
    pub queue_time_ms: i64,

    /// Engine execution time in milliseconds
    pub engine_execution_time_ms: i64,
}

impl QueryStats {
    /// Calculate estimated cost based on data scanned.
    /// Athena charges $5 per TB scanned.
    pub fn estimated_cost_usd(&self) -> f64 {
        let tb_scanned = self.data_scanned_bytes as f64 / 1_000_000_000_000.0;
        tb_scanned * 5.0
    }
}

/// Athena SQL query engine.
///
/// Provides serverless SQL analytics over S3-stored data.
pub struct AthenaQueryEngine {
    config: AthenaConfig,
    client: Arc<AthenaClient>,
}

impl AthenaQueryEngine {
    /// Create a new Athena query engine.
    pub async fn new(config: AthenaConfig) -> Result<Self> {
        let aws_config = if let Some(region) = &config.region {
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(aws_config::Region::new(region.clone()))
                .load()
                .await
        } else {
            aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await
        };

        let client = Arc::new(AthenaClient::new(&aws_config));

        Ok(Self { config, client })
    }

    /// Execute a SQL query and wait for results.
    ///
    /// # Arguments
    ///
    /// * `query` - SQL query string
    ///
    /// # Returns
    ///
    /// Vector of result rows with column data.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let results = engine.query("SELECT * FROM documents WHERE status = 'completed'").await?;
    /// for row in results {
    ///     println!("{:?}", row.to_json());
    /// }
    /// ```
    pub async fn query(&self, query: &str) -> Result<Vec<AthenaRow>> {
        let execution_id = self.start_query(query).await?;
        self.wait_for_query(&execution_id).await?;
        self.get_query_results(&execution_id).await
    }

    /// Execute a SQL query and return execution ID immediately (async execution).
    ///
    /// Use `wait_for_query()` and `get_query_results()` to retrieve results later.
    pub async fn start_query(&self, query: &str) -> Result<String> {
        info!("Starting Athena query: {}", query);

        let result_config = ResultConfiguration::builder()
            .output_location(&self.config.output_location)
            .build();

        let query_context = QueryExecutionContext::builder()
            .database(&self.config.database)
            .build();

        let response = self
            .client
            .start_query_execution()
            .query_string(query)
            .result_configuration(result_config)
            .query_execution_context(query_context)
            .work_group(&self.config.workgroup)
            .send()
            .await
            .map_err(|e| {
                AwsStorageError::AthenaError(format!("Failed to start query execution: {}", e))
            })?;

        let execution_id = response.query_execution_id().ok_or_else(|| {
            AwsStorageError::AthenaError("No query execution ID returned".to_string())
        })?;

        debug!("Query started with execution ID: {}", execution_id);
        Ok(execution_id.to_string())
    }

    /// Wait for a query to complete.
    ///
    /// # Arguments
    ///
    /// * `execution_id` - Query execution ID from `start_query()`
    pub async fn wait_for_query(&self, execution_id: &str) -> Result<QueryStats> {
        let timeout = Duration::from_secs(self.config.query_timeout_secs);
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);
        let start_time = std::time::Instant::now();

        loop {
            let response = self
                .client
                .get_query_execution()
                .query_execution_id(execution_id)
                .send()
                .await
                .map_err(|e| {
                    AwsStorageError::AthenaError(format!("Failed to get query execution: {}", e))
                })?;

            let query_execution = response.query_execution().ok_or_else(|| {
                AwsStorageError::AthenaError("No query execution data".to_string())
            })?;

            let status = query_execution.status().ok_or_else(|| {
                AwsStorageError::AthenaError("No query execution status".to_string())
            })?;

            let state = status.state().ok_or_else(|| {
                AwsStorageError::AthenaError("No query execution state".to_string())
            })?;

            match state {
                QueryExecutionState::Succeeded => {
                    let stats = query_execution.statistics().ok_or_else(|| {
                        AwsStorageError::AthenaError("No query execution statistics".to_string())
                    })?;

                    return Ok(QueryStats {
                        execution_id: execution_id.to_string(),
                        state: format!("{:?}", state),
                        data_scanned_bytes: stats.data_scanned_in_bytes().unwrap_or(0),
                        execution_time_ms: stats.total_execution_time_in_millis().unwrap_or(0),
                        queue_time_ms: stats.query_queue_time_in_millis().unwrap_or(0),
                        engine_execution_time_ms: stats
                            .engine_execution_time_in_millis()
                            .unwrap_or(0),
                    });
                }
                QueryExecutionState::Failed => {
                    let reason = status
                        .state_change_reason()
                        .unwrap_or("Unknown error")
                        .to_string();
                    return Err(AwsStorageError::AthenaError(format!(
                        "Query failed: {}",
                        reason
                    )));
                }
                QueryExecutionState::Cancelled => {
                    return Err(AwsStorageError::AthenaError(
                        "Query was cancelled".to_string(),
                    ));
                }
                QueryExecutionState::Queued | QueryExecutionState::Running => {
                    if start_time.elapsed() > timeout {
                        return Err(AwsStorageError::AthenaError(format!(
                            "Query timeout after {} seconds",
                            self.config.query_timeout_secs
                        )));
                    }

                    debug!("Query state: {:?}, waiting...", state);
                    sleep(poll_interval).await;
                }
                _ => {
                    warn!("Unknown query state: {:?}", state);
                    sleep(poll_interval).await;
                }
            }
        }
    }

    /// Get query results after execution completes.
    ///
    /// # Arguments
    ///
    /// * `execution_id` - Query execution ID from `start_query()`
    pub async fn get_query_results(&self, execution_id: &str) -> Result<Vec<AthenaRow>> {
        let mut rows = Vec::new();

        // Get column names from first page
        let first_response = self
            .client
            .get_query_results()
            .query_execution_id(execution_id)
            .send()
            .await
            .map_err(|e| {
                AwsStorageError::AthenaError(format!("Failed to get query results: {}", e))
            })?;

        let result_set = first_response.result_set().ok_or_else(|| {
            AwsStorageError::AthenaError("No result set in query results".to_string())
        })?;

        let metadata = result_set
            .result_set_metadata()
            .ok_or_else(|| AwsStorageError::AthenaError("No metadata in result set".to_string()))?;

        let column_info = metadata.column_info();
        let column_names: Vec<String> = column_info
            .iter()
            .map(|col| col.name().to_string())
            .collect();

        // Parse rows from first page (skip header row)
        let result_rows = result_set.rows();
        for (i, row) in result_rows.iter().enumerate() {
            if i == 0 {
                continue; // Skip header row
            }

            let data = row.data();
            let mut column_map = HashMap::new();

            for (j, col_name) in column_names.iter().enumerate() {
                if let Some(datum) = data.get(j) {
                    if let Some(value) = datum.var_char_value() {
                        column_map.insert(col_name.clone(), value.to_string());
                    }
                }
            }

            rows.push(AthenaRow::new(column_map));
        }

        let mut next_token = first_response.next_token().map(|s| s.to_string());

        // Fetch remaining pages
        while let Some(token) = next_token {
            let response = self
                .client
                .get_query_results()
                .query_execution_id(execution_id)
                .next_token(token)
                .send()
                .await
                .map_err(|e| {
                    AwsStorageError::AthenaError(format!("Failed to get query results: {}", e))
                })?;

            let result_set = response.result_set().ok_or_else(|| {
                AwsStorageError::AthenaError("No result set in query results".to_string())
            })?;

            let result_rows = result_set.rows();
            for row in result_rows {
                let data = row.data();
                let mut column_map = HashMap::new();

                for (j, col_name) in column_names.iter().enumerate() {
                    if let Some(datum) = data.get(j) {
                        if let Some(value) = datum.var_char_value() {
                            column_map.insert(col_name.clone(), value.to_string());
                        }
                    }
                }

                rows.push(AthenaRow::new(column_map));
            }

            next_token = response.next_token().map(|s| s.to_string());
        }

        info!("Retrieved {} rows from query {}", rows.len(), execution_id);
        Ok(rows)
    }

    /// Cancel a running query.
    pub async fn cancel_query(&self, execution_id: &str) -> Result<()> {
        self.client
            .stop_query_execution()
            .query_execution_id(execution_id)
            .send()
            .await
            .map_err(|e| AwsStorageError::AthenaError(format!("Failed to cancel query: {}", e)))?;

        info!("Cancelled query: {}", execution_id);
        Ok(())
    }

    /// Get query execution statistics without fetching results.
    pub async fn get_query_stats(&self, execution_id: &str) -> Result<QueryStats> {
        let response = self
            .client
            .get_query_execution()
            .query_execution_id(execution_id)
            .send()
            .await
            .map_err(|e| {
                AwsStorageError::AthenaError(format!("Failed to get query execution: {}", e))
            })?;

        let query_execution = response
            .query_execution()
            .ok_or_else(|| AwsStorageError::AthenaError("No query execution data".to_string()))?;

        let status = query_execution
            .status()
            .ok_or_else(|| AwsStorageError::AthenaError("No query execution status".to_string()))?;

        let state = status
            .state()
            .ok_or_else(|| AwsStorageError::AthenaError("No query execution state".to_string()))?;

        let stats = query_execution.statistics().ok_or_else(|| {
            AwsStorageError::AthenaError("No query execution statistics".to_string())
        })?;

        Ok(QueryStats {
            execution_id: execution_id.to_string(),
            state: format!("{:?}", state),
            data_scanned_bytes: stats.data_scanned_in_bytes().unwrap_or(0),
            execution_time_ms: stats.total_execution_time_in_millis().unwrap_or(0),
            queue_time_ms: stats.query_queue_time_in_millis().unwrap_or(0),
            engine_execution_time_ms: stats.engine_execution_time_in_millis().unwrap_or(0),
        })
    }
}

/// Builder for common SQL queries.
pub struct QueryBuilder {
    database: String,
}

impl QueryBuilder {
    /// Create a new query builder for a database.
    pub fn new(database: impl Into<String>) -> Self {
        Self {
            database: database.into(),
        }
    }

    /// Build a query to list all documents in a workspace.
    pub fn list_documents(&self, workspace_id: &str, limit: usize) -> String {
        format!(
            "SELECT * FROM {}.documents WHERE workspace_id = '{}' LIMIT {}",
            self.database, workspace_id, limit
        )
    }

    /// Build a query to search documents by status.
    pub fn search_by_status(&self, workspace_id: &str, status: &str) -> String {
        format!(
            "SELECT * FROM {}.documents WHERE workspace_id = '{}' AND status = '{}'",
            self.database, workspace_id, status
        )
    }

    /// Build a query to count documents.
    pub fn count_documents(&self, workspace_id: &str) -> String {
        format!(
            "SELECT COUNT(*) as count FROM {}.documents WHERE workspace_id = '{}'",
            self.database, workspace_id
        )
    }

    /// Build a query to get document statistics by workspace.
    pub fn workspace_stats(&self) -> String {
        format!(
            "SELECT workspace_id, COUNT(*) as doc_count, \
             SUM(size_bytes) as total_bytes \
             FROM {}.documents \
             GROUP BY workspace_id",
            self.database
        )
    }

    /// Build a query to search vector metadata.
    pub fn search_vectors(&self, namespace: &str, filter_expr: Option<&str>) -> String {
        let base = format!(
            "SELECT * FROM {}.vectors WHERE namespace = '{}'",
            self.database, namespace
        );

        if let Some(filter) = filter_expr {
            format!("{} AND {}", base, filter)
        } else {
            base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_athena_config() {
        let config = AthenaConfig::new("test-db", "s3://bucket/results/")
            .with_region("us-east-1")
            .with_workgroup("custom")
            .with_timeout(120)
            .with_poll_interval(2)
            .with_result_cache(false);

        assert_eq!(config.database, "test-db");
        assert_eq!(config.output_location, "s3://bucket/results/");
        assert_eq!(config.region, Some("us-east-1".to_string()));
        assert_eq!(config.workgroup, "custom");
        assert_eq!(config.query_timeout_secs, 120);
        assert_eq!(config.poll_interval_secs, 2);
        assert!(!config.enable_result_cache);
    }

    #[test]
    fn test_athena_row() {
        let mut columns = HashMap::new();
        columns.insert("id".to_string(), "123".to_string());
        columns.insert("name".to_string(), "test".to_string());
        columns.insert("count".to_string(), "42".to_string());
        columns.insert("active".to_string(), "true".to_string());

        let row = AthenaRow::new(columns);

        assert_eq!(row.get("id").unwrap(), "123");
        assert_eq!(row.get("name").unwrap(), "test");
        assert_eq!(row.get_i64("count").unwrap(), 42);
        assert_eq!(row.get_bool("active").unwrap(), true);
    }

    #[test]
    fn test_query_builder() {
        let builder = QueryBuilder::new("edgequake");

        let query = builder.list_documents("ws-123", 10);
        assert!(query.contains("workspace_id = 'ws-123'"));
        assert!(query.contains("LIMIT 10"));

        let query = builder.search_by_status("ws-123", "completed");
        assert!(query.contains("status = 'completed'"));

        let query = builder.count_documents("ws-123");
        assert!(query.contains("COUNT(*)"));

        let query = builder.workspace_stats();
        assert!(query.contains("GROUP BY workspace_id"));
    }

    #[test]
    fn test_query_stats_cost() {
        let stats = QueryStats {
            execution_id: "test".to_string(),
            state: "SUCCEEDED".to_string(),
            data_scanned_bytes: 1_000_000_000_000, // 1 TB
            execution_time_ms: 1000,
            queue_time_ms: 100,
            engine_execution_time_ms: 900,
        };

        assert_eq!(stats.estimated_cost_usd(), 5.0);
    }
}
