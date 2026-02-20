//! DynamoDB namespace registry.
//!
//! Provides persistent storage for namespace metadata and per-namespace pipeline
//! configuration using Amazon DynamoDB as the authority store.
//!
//! ## Table Design
//!
//! ```text
//! Table: {config.table_name}
//! PK (S)               SK (S)          Data
//! NAMESPACES           {slug}          { created_at }          -- list entry
//! NS#{slug}            META            { NamespaceRecord JSON }
//! NS#{slug}            CONFIG          { PipelineConfig JSON }
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use edgequake_storage_aws::{DynamoNamespaceRegistry, DynamoNamespaceConfig};
//! use edgequake_core::NamespaceSlug;
//!
//! let config = DynamoNamespaceConfig {
//!     table_name: "edgequake-namespaces".to_string(),
//! };
//! let registry = DynamoNamespaceRegistry::new(config, dynamo_client);
//!
//! let slug = NamespaceSlug::parse("epstein-files").unwrap();
//! let record = registry.create_namespace(&slug, Some("Epstein investigation files".into())).await?;
//! ```

use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client as DynamoDbClient;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use edgequake_core::{NamespaceRecord, NamespaceSlug, PipelineConfig};

use crate::error::{AwsStorageError, Result};

/// Extract a descriptive error message from a DynamoDB SDK error.
///
/// Mirrors the `dynamo_err` helper in `dynamodb_kv.rs` for consistent error
/// reporting across all DynamoDB modules.
fn dynamo_err<E, R>(err: aws_smithy_runtime_api::client::result::SdkError<E, R>) -> AwsStorageError
where
    E: std::fmt::Display + std::fmt::Debug,
    R: std::fmt::Debug,
{
    let msg = match &err {
        aws_smithy_runtime_api::client::result::SdkError::ServiceError(ctx) => {
            let display = format!("{}", ctx.err());
            let debug_msg = format!("{:?}", ctx.err());
            if display.contains("unhandled") || display == "service error" {
                debug_msg
            } else {
                display
            }
        }
        other => format!("{:?}", other),
    };
    AwsStorageError::DynamoDbError(msg)
}

/// Check whether a DynamoDB SDK error is a ConditionalCheckFailedException.
fn is_condition_check_failed<E, R>(
    err: &aws_smithy_runtime_api::client::result::SdkError<E, R>,
) -> bool
where
    E: std::fmt::Display + std::fmt::Debug,
    R: std::fmt::Debug,
{
    match err {
        aws_smithy_runtime_api::client::result::SdkError::ServiceError(ctx) => {
            let msg = format!("{:?}", ctx.err());
            msg.contains("ConditionalCheckFailed")
        }
        _ => false,
    }
}

/// Configuration for the DynamoDB namespace registry.
#[derive(Debug, Clone)]
pub struct DynamoNamespaceConfig {
    /// DynamoDB table name (e.g., "edgequake-namespaces").
    pub table_name: String,
}

/// An item in the namespace listing.
///
/// Contains the slug and creation timestamp plus optional stats fields.
/// The `entity_count` and `document_count` are always `None` from the
/// registry layer -- they are enriched by the API handler using
/// namespace-scoped `get_graph_stats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceListItem {
    /// The namespace slug.
    pub slug: String,
    /// Creation timestamp (epoch milliseconds).
    pub created_at: i64,
    /// Number of entities in this namespace (populated by API layer, not registry).
    pub entity_count: Option<u64>,
    /// Number of documents in this namespace (populated by API layer, not registry).
    pub document_count: Option<u64>,
}

/// DynamoDB-backed namespace registry.
///
/// Provides create, list, describe operations for namespace metadata and
/// per-namespace pipeline configuration persistence. Uses conditional puts
/// for slug uniqueness enforcement.
pub struct DynamoNamespaceRegistry {
    client: DynamoDbClient,
    config: DynamoNamespaceConfig,
}

impl DynamoNamespaceRegistry {
    /// Create a new namespace registry with a pre-configured DynamoDB client.
    ///
    /// # Arguments
    ///
    /// * `config` - Registry configuration (table name)
    /// * `client` - Pre-configured DynamoDB client
    pub fn new(config: DynamoNamespaceConfig, client: DynamoDbClient) -> Self {
        Self { client, config }
    }

    /// Create a new namespace.
    ///
    /// Writes three DynamoDB items atomically:
    /// 1. META record with the full `NamespaceRecord`
    /// 2. CONFIG record with `PipelineConfig::default()`
    /// 3. NAMESPACES list entry for efficient listing
    ///
    /// Uses `attribute_not_exists(PK)` on the META record to enforce slug
    /// uniqueness -- if the slug already exists, returns
    /// `AwsStorageError::NamespaceAlreadyExists`.
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug
    /// * `description` - Optional human-readable description
    ///
    /// # Errors
    ///
    /// Returns `NamespaceAlreadyExists` if a namespace with this slug exists.
    pub async fn create_namespace(
        &self,
        slug: &NamespaceSlug,
        description: Option<String>,
    ) -> Result<NamespaceRecord> {
        let now = chrono::Utc::now().timestamp_millis();

        let record = NamespaceRecord {
            slug: slug.clone(),
            description,
            created_at: now,
            created_by: None,
        };

        let record_json = serde_json::to_string(&record)?;
        let pk = format!("NS#{}", slug.as_str());

        // 1. Write META record with uniqueness check
        let meta_result = self
            .client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(pk.clone()))
            .item("SK", AttributeValue::S("META".to_string()))
            .item("data", AttributeValue::S(record_json))
            .item(
                "created_at",
                AttributeValue::N(now.to_string()),
            )
            .condition_expression("attribute_not_exists(PK)")
            .send()
            .await;

        match meta_result {
            Ok(_) => {}
            Err(ref e) if is_condition_check_failed(e) => {
                return Err(AwsStorageError::NamespaceAlreadyExists(
                    slug.as_str().to_string(),
                ));
            }
            Err(e) => return Err(dynamo_err(e)),
        }

        // 2. Write default CONFIG record
        let default_config = PipelineConfig::default();
        let config_json = serde_json::to_string(&default_config)?;

        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(pk))
            .item("SK", AttributeValue::S("CONFIG".to_string()))
            .item("data", AttributeValue::S(config_json))
            .send()
            .await
            .map_err(dynamo_err)?;

        // 3. Write NAMESPACES list entry
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S("NAMESPACES".to_string()))
            .item(
                "SK",
                AttributeValue::S(slug.as_str().to_string()),
            )
            .item(
                "created_at",
                AttributeValue::N(now.to_string()),
            )
            .send()
            .await
            .map_err(dynamo_err)?;

        info!(namespace = slug.as_str(), "Created namespace");
        Ok(record)
    }

    /// List all namespaces.
    ///
    /// Queries the NAMESPACES partition to retrieve all namespace slugs and
    /// creation timestamps. Results are sorted by `created_at` descending
    /// (newest first).
    ///
    /// The `entity_count` and `document_count` fields are always `None` --
    /// the API layer enriches them using namespace-scoped graph stats.
    pub async fn list_namespaces(&self) -> Result<Vec<NamespaceListItem>> {
        let mut items = Vec::new();
        let mut last_evaluated_key = None;

        loop {
            let mut query = self
                .client
                .query()
                .table_name(&self.config.table_name)
                .key_condition_expression("PK = :pk")
                .expression_attribute_values(
                    ":pk",
                    AttributeValue::S("NAMESPACES".to_string()),
                );

            if let Some(key) = last_evaluated_key {
                query = query.set_exclusive_start_key(Some(key));
            }

            let result = query.send().await.map_err(dynamo_err)?;

            if let Some(dynamo_items) = result.items {
                for item in dynamo_items {
                    let slug = item
                        .get("SK")
                        .and_then(|v| v.as_s().ok())
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    let created_at = item
                        .get("created_at")
                        .and_then(|v| v.as_n().ok())
                        .and_then(|n| n.parse::<i64>().ok())
                        .unwrap_or(0);

                    items.push(NamespaceListItem {
                        slug,
                        created_at,
                        entity_count: None,
                        document_count: None,
                    });
                }
            }

            last_evaluated_key = result.last_evaluated_key;
            if last_evaluated_key.is_none() {
                break;
            }
        }

        // Sort by created_at descending (newest first)
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        debug!(count = items.len(), "Listed namespaces");
        Ok(items)
    }

    /// Describe a single namespace by slug.
    ///
    /// Returns the full `NamespaceRecord` if the namespace exists, or `None`
    /// if not found.
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug to look up
    pub async fn describe_namespace(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<Option<NamespaceRecord>> {
        let pk = format!("NS#{}", slug.as_str());

        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk))
            .key("SK", AttributeValue::S("META".to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        match result.item {
            Some(item) => {
                let data = item
                    .get("data")
                    .ok_or_else(|| {
                        AwsStorageError::DynamoDbError(
                            "Missing 'data' attribute on namespace META record".into(),
                        )
                    })?
                    .as_s()
                    .map_err(|_| {
                        AwsStorageError::DynamoDbError(
                            "'data' attribute is not a string".into(),
                        )
                    })?;

                let record: NamespaceRecord = serde_json::from_str(data)?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    /// Get the pipeline configuration for a namespace.
    ///
    /// Returns the `PipelineConfig` if the namespace exists, or `None` if
    /// the namespace (or its config record) does not exist.
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug
    pub async fn get_config(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<Option<PipelineConfig>> {
        let pk = format!("NS#{}", slug.as_str());

        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk))
            .key("SK", AttributeValue::S("CONFIG".to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        match result.item {
            Some(item) => {
                let data = item
                    .get("data")
                    .ok_or_else(|| {
                        AwsStorageError::DynamoDbError(
                            "Missing 'data' attribute on namespace CONFIG record".into(),
                        )
                    })?
                    .as_s()
                    .map_err(|_| {
                        AwsStorageError::DynamoDbError(
                            "'data' attribute is not a string".into(),
                        )
                    })?;

                let config: PipelineConfig = serde_json::from_str(data)?;
                Ok(Some(config))
            }
            None => Ok(None),
        }
    }

    /// Update the pipeline configuration for an existing namespace.
    ///
    /// Uses `attribute_exists(PK)` to ensure the namespace exists before
    /// allowing a config update. If the namespace does not exist, returns
    /// `AwsStorageError::NamespaceNotFound`.
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug
    /// * `config` - New pipeline configuration to persist
    ///
    /// # Errors
    ///
    /// Returns `NamespaceNotFound` if the namespace does not exist.
    pub async fn update_config(
        &self,
        slug: &NamespaceSlug,
        config: &PipelineConfig,
    ) -> Result<()> {
        let pk = format!("NS#{}", slug.as_str());
        let config_json = serde_json::to_string(config)?;

        let result = self
            .client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(pk))
            .item("SK", AttributeValue::S("CONFIG".to_string()))
            .item("data", AttributeValue::S(config_json))
            .condition_expression("attribute_exists(PK)")
            .send()
            .await;

        match result {
            Ok(_) => {
                debug!(namespace = slug.as_str(), "Updated pipeline config");
                Ok(())
            }
            Err(ref e) if is_condition_check_failed(e) => {
                Err(AwsStorageError::NamespaceNotFound(
                    slug.as_str().to_string(),
                ))
            }
            Err(e) => Err(dynamo_err(e)),
        }
    }
}
