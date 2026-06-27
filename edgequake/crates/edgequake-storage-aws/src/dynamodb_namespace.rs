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
//! NS#{slug}            DESCRIPTOR      { McpDescriptor JSON }  -- MCP descriptor
//! NS#{slug}            SCHEMA          { SchemaProposal JSON } -- schema proposal
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
use tracing::{debug, info, warn};

use edgequake_core::{
    default_mcp_tools, InfrastructureConfig, McpAuthConfig, McpBm25Config, McpDescriptor,
    McpDynamoDbConfig, McpNamespaceInfo, McpNeptuneConfig, McpPipelineConfig, McpS3VectorsConfig,
    McpStorageConfig,
    NamespaceListItem as CoreNamespaceListItem, NamespaceRecord, NamespaceRegistry,
    NamespaceRegistryError, NamespaceSlug, PipelineConfig, SchemaProposal, SchemaStatus,
};

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
///
/// When `infra` is set via [`with_infra`](Self::with_infra), MCP descriptors
/// are auto-generated and stored on namespace creation.
pub struct DynamoNamespaceRegistry {
    client: DynamoDbClient,
    config: DynamoNamespaceConfig,
    infra: Option<InfrastructureConfig>,
}

impl DynamoNamespaceRegistry {
    /// Create a new namespace registry with a pre-configured DynamoDB client.
    ///
    /// # Arguments
    ///
    /// * `config` - Registry configuration (table name)
    /// * `client` - Pre-configured DynamoDB client
    pub fn new(config: DynamoNamespaceConfig, client: DynamoDbClient) -> Self {
        Self {
            client,
            config,
            infra: None,
        }
    }

    /// Set the infrastructure config for auto-generating MCP descriptors
    /// on namespace creation.
    ///
    /// When set, `create_namespace` will automatically generate and store
    /// an MCP descriptor after writing the namespace record and config.
    pub fn with_infra(mut self, infra: InfrastructureConfig) -> Self {
        self.infra = Some(infra);
        self
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

        // 4. Auto-generate and store MCP descriptor if infra config is available
        if let Some(ref infra) = self.infra {
            match self.generate_descriptor(slug, infra).await {
                Ok(descriptor) => match self.store_descriptor(slug, &descriptor).await {
                    Ok(()) => {
                        info!(
                            namespace = slug.as_str(),
                            "Auto-generated and stored MCP descriptor"
                        );
                    }
                    Err(e) => {
                        warn!(
                            namespace = slug.as_str(),
                            error = %e,
                            "Failed to store MCP descriptor (namespace created successfully)"
                        );
                    }
                },
                Err(e) => {
                    warn!(
                        namespace = slug.as_str(),
                        error = %e,
                        "Failed to generate MCP descriptor (namespace created successfully)"
                    );
                }
            }
        }

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

    /// Generate an MCP descriptor for a namespace.
    ///
    /// Reads the namespace record and pipeline config from DynamoDB,
    /// combines them with the infrastructure config to produce a
    /// self-contained [`McpDescriptor`].
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug
    /// * `infra` - Infrastructure configuration (endpoints, account, region)
    ///
    /// # Errors
    ///
    /// Returns `NamespaceNotFound` if the namespace does not exist.
    pub async fn generate_descriptor(
        &self,
        slug: &NamespaceSlug,
        infra: &InfrastructureConfig,
    ) -> Result<McpDescriptor> {
        // Fetch namespace record (must exist)
        let record = self
            .describe_namespace(slug)
            .await?
            .ok_or_else(|| AwsStorageError::NamespaceNotFound(slug.as_str().to_string()))?;

        // Fetch pipeline config (use defaults if not set)
        let config = self
            .get_config(slug)
            .await?
            .unwrap_or_default();

        let descriptor = McpDescriptor {
            schema_version: "1.0".to_string(),
            namespace: McpNamespaceInfo {
                slug: slug.as_str().to_string(),
                description: record.description,
            },
            storage: McpStorageConfig {
                neptune: McpNeptuneConfig {
                    endpoint: infra.neptune_endpoint.clone(),
                    label_prefix: slug.as_str().to_string(),
                },
                s3_vectors: McpS3VectorsConfig {
                    bucket_name: infra.vector_bucket_name.clone(),
                    index_name: format!("{}-embeddings", slug.as_str()),
                },
                dynamodb: McpDynamoDbConfig {
                    table_name: infra.dynamodb_table_name.clone(),
                    namespace_key: slug.as_str().to_string(),
                },
                bm25: infra.athena_bm25_database.as_ref().map(|db| McpBm25Config {
                    database: db.clone(),
                    s3_bucket: infra.bm25_s3_bucket.clone().unwrap_or_default(),
                    workgroup: infra.athena_workgroup.clone().unwrap_or_else(|| "primary".to_string()),
                }),
            },
            auth: McpAuthConfig {
                role_arn: format!(
                    "arn:aws:iam::{}:role/edgequake-mcp-{}-{}",
                    infra.account_id,
                    slug.as_str(),
                    infra.environment
                ),
                external_id: format!("eq-{}-{}", slug.as_str(), infra.external_id_suffix),
                region: infra.region.clone(),
            },
            pipeline_config: McpPipelineConfig {
                embedding_model: config.embedding_model,
                embedding_dimension: config.embedding_dimension,
                llm_model: config.llm_model,
            },
            tools: default_mcp_tools(),
            generated_at: chrono::Utc::now().timestamp_millis(),
        };

        Ok(descriptor)
    }

    /// Store an MCP descriptor in DynamoDB.
    ///
    /// Writes the descriptor as SK=DESCRIPTOR alongside the namespace's
    /// META and CONFIG records. Overwrites any existing descriptor.
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug
    /// * `descriptor` - The MCP descriptor to store
    pub async fn store_descriptor(
        &self,
        slug: &NamespaceSlug,
        descriptor: &McpDescriptor,
    ) -> Result<()> {
        let pk = format!("NS#{}", slug.as_str());
        let descriptor_json = serde_json::to_string(descriptor)?;

        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(pk))
            .item("SK", AttributeValue::S("DESCRIPTOR".to_string()))
            .item("data", AttributeValue::S(descriptor_json))
            .item(
                "generated_at",
                AttributeValue::N(descriptor.generated_at.to_string()),
            )
            .send()
            .await
            .map_err(dynamo_err)?;

        debug!(namespace = slug.as_str(), "Stored MCP descriptor");
        Ok(())
    }

    /// Get the MCP descriptor for a namespace.
    ///
    /// Returns the stored [`McpDescriptor`] if one exists, or `None` if
    /// the descriptor has not been generated yet.
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug
    pub async fn get_descriptor(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<Option<McpDescriptor>> {
        let pk = format!("NS#{}", slug.as_str());

        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk))
            .key("SK", AttributeValue::S("DESCRIPTOR".to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        match result.item {
            Some(item) => {
                let data = item
                    .get("data")
                    .ok_or_else(|| {
                        AwsStorageError::DynamoDbError(
                            "Missing 'data' attribute on namespace DESCRIPTOR record".into(),
                        )
                    })?
                    .as_s()
                    .map_err(|_| {
                        AwsStorageError::DynamoDbError(
                            "'data' attribute is not a string".into(),
                        )
                    })?;

                let descriptor: McpDescriptor = serde_json::from_str(data)?;
                Ok(Some(descriptor))
            }
            None => Ok(None),
        }
    }

    /// Store a schema proposal in DynamoDB.
    ///
    /// Writes the proposal as SK=SCHEMA alongside the namespace's META, CONFIG,
    /// and DESCRIPTOR records. Overwrites any existing proposal.
    ///
    /// Also clears `entity_types` and `relation_types` from PipelineConfig to
    /// prevent stale approved types from being used after a re-run.
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug
    /// * `proposal` - The schema proposal to store
    pub async fn store_schema(
        &self,
        slug: &NamespaceSlug,
        proposal: &SchemaProposal,
    ) -> Result<()> {
        let pk = format!("NS#{}", slug.as_str());
        let proposal_json = serde_json::to_string(proposal)?;

        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(pk))
            .item("SK", AttributeValue::S("SCHEMA".to_string()))
            .item("data", AttributeValue::S(proposal_json))
            .item(
                "proposed_at",
                AttributeValue::N(proposal.proposed_at.to_string()),
            )
            .send()
            .await
            .map_err(dynamo_err)?;

        // Clear entity_types and relation_types from PipelineConfig to force re-approval
        if let Some(mut config) = self.get_config(slug).await? {
            if !config.entity_types.is_empty() || !config.relation_types.is_empty() {
                config.entity_types.clear();
                config.relation_types.clear();
                config.updated_at = chrono::Utc::now().timestamp_millis();
                self.update_config(slug, &config).await?;
                debug!(
                    namespace = slug.as_str(),
                    "Cleared PipelineConfig entity/relation types on schema re-proposal"
                );
            }
        }

        debug!(namespace = slug.as_str(), "Stored schema proposal");
        Ok(())
    }

    /// Get the schema proposal for a namespace.
    ///
    /// Returns the stored [`SchemaProposal`] if one exists, or `None` if
    /// no schema has been proposed yet.
    ///
    /// # Arguments
    ///
    /// * `slug` - Validated namespace slug
    pub async fn get_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<Option<SchemaProposal>> {
        let pk = format!("NS#{}", slug.as_str());

        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk))
            // Strongly-consistent: the admin UI refetches the schema immediately
            // after a write (start-from-defaults / propose / resample / approve),
            // so an eventually-consistent read races the write and returns the
            // stale (empty) replica — forcing a manual page refresh. (Phase 33
            // wave-8 live-verify finding; mirrors the workspace-store consistent
            // reads from 33-08.)
            .consistent_read(true)
            .key("SK", AttributeValue::S("SCHEMA".to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        match result.item {
            Some(item) => {
                let data = item
                    .get("data")
                    .ok_or_else(|| {
                        AwsStorageError::DynamoDbError(
                            "Missing 'data' attribute on namespace SCHEMA record".into(),
                        )
                    })?
                    .as_s()
                    .map_err(|_| {
                        AwsStorageError::DynamoDbError(
                            "'data' attribute is not a string".into(),
                        )
                    })?;

                let proposal: SchemaProposal = serde_json::from_str(data)?;
                Ok(Some(proposal))
            }
            None => Ok(None),
        }
    }

    /// Approve the current schema proposal.
    ///
    /// 1. Read current schema, error if not found or not in `Proposed` state
    /// 2. Set status to `Approved`, `reviewed_at` to now
    /// 3. Store updated schema
    /// 4. Copy entity/relation type names into PipelineConfig
    /// 5. Return updated proposal
    pub async fn approve_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<SchemaProposal> {
        let mut proposal = self
            .get_schema(slug)
            .await?
            .ok_or_else(|| AwsStorageError::SchemaNotFound(slug.as_str().to_string()))?;

        // WR-03: the in-process suggest flow stores a pending marker with
        // status=Proposed + domain_hint="__pending__" and EMPTY type lists
        // (see edgequake-api handlers/schema.rs PENDING_SENTINEL). Approving
        // it would mark an empty schema Approved and clobber PipelineConfig
        // entity/relation types with empty vectors.
        if proposal.domain_hint.as_deref() == Some("__pending__") {
            return Err(AwsStorageError::InvalidSchemaState(
                "Cannot approve schema: a schema suggestion is still in progress".to_string(),
            ));
        }

        if proposal.status != SchemaStatus::Proposed {
            return Err(AwsStorageError::InvalidSchemaState(format!(
                "Cannot approve schema in state {:?}",
                proposal.status
            )));
        }

        let now = chrono::Utc::now().timestamp_millis();
        proposal.status = SchemaStatus::Approved;
        proposal.reviewed_at = Some(now);
        self.store_schema_raw(slug, &proposal).await?;

        // Copy entity/relation type names into PipelineConfig
        let mut config = self.get_config(slug).await?.unwrap_or_default();
        config.entity_types = proposal
            .entity_types
            .iter()
            .map(|e| e.name.clone())
            .collect();
        config.relation_types = proposal
            .relation_types
            .iter()
            .map(|r| r.name.clone())
            .collect();
        config.updated_at = now;
        self.update_config(slug, &config).await?;

        info!(
            namespace = slug.as_str(),
            entity_types = config.entity_types.len(),
            relation_types = config.relation_types.len(),
            "Schema approved, PipelineConfig updated"
        );

        Ok(proposal)
    }

    /// Reject the current schema proposal.
    ///
    /// 1. Read current schema, error if not found or not in `Proposed` state
    /// 2. Set status to `Rejected`, `reviewed_at` to now
    /// 3. Store updated schema
    /// 4. Return updated proposal
    pub async fn reject_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> Result<SchemaProposal> {
        let mut proposal = self
            .get_schema(slug)
            .await?
            .ok_or_else(|| AwsStorageError::SchemaNotFound(slug.as_str().to_string()))?;

        if proposal.status != SchemaStatus::Proposed {
            return Err(AwsStorageError::InvalidSchemaState(format!(
                "Cannot reject schema in state {:?}",
                proposal.status
            )));
        }

        let now = chrono::Utc::now().timestamp_millis();
        proposal.status = SchemaStatus::Rejected;
        proposal.reviewed_at = Some(now);
        self.store_schema_raw(slug, &proposal).await?;

        info!(
            namespace = slug.as_str(),
            "Schema rejected"
        );

        Ok(proposal)
    }

    // -----------------------------------------------------------------------
    // Preview request / result methods (Phase 23 Plan 02)
    // -----------------------------------------------------------------------

    /// Write a PREVIEW_REQUEST item with status="requested".
    ///
    /// Deletes any stale PREVIEW_RESULT from a previous run first, so the
    /// GET /preview poll cannot return the previous run's completed result
    /// for the new request (CR-03: stale-result early-advance bug).
    ///
    /// Exact item shape (matches handle_preview_command reader):
    ///   PK = "NS#{slug}", SK = "PREVIEW_REQUEST",
    ///   data = JSON { namespace, status:"requested", documents_completed:0, documents_total:0 }
    pub async fn put_preview_request(&self, slug: &NamespaceSlug) -> Result<()> {
        let pk = format!("NS#{}", slug.as_str());

        // Remove any stale result from a previous run BEFORE recording the
        // new request. DeleteItem is idempotent (no error when absent).
        self.client
            .delete_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk.clone()))
            .key("SK", AttributeValue::S("PREVIEW_RESULT".to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        let data = serde_json::json!({
            "namespace": slug.as_str(),
            "status": "requested",
            "documents_completed": 0,
            "documents_total": 0,
        });

        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(pk))
            .item("SK", AttributeValue::S("PREVIEW_REQUEST".to_string()))
            .item("data", AttributeValue::S(data.to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        debug!(
            namespace = slug.as_str(),
            "Cleared stale PREVIEW_RESULT and wrote PREVIEW_REQUEST with status=requested"
        );
        Ok(())
    }

    /// Read the PREVIEW_REQUEST status string.
    ///
    /// Returns the `status` field of the PREVIEW_REQUEST `data` JSON,
    /// or `None` if the item does not exist.
    pub async fn get_preview_status(&self, slug: &NamespaceSlug) -> Result<Option<String>> {
        let pk = format!("NS#{}", slug.as_str());

        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk))
            .key("SK", AttributeValue::S("PREVIEW_REQUEST".to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        match result.item {
            Some(item) => {
                let data_str = item
                    .get("data")
                    .ok_or_else(|| {
                        AwsStorageError::DynamoDbError(
                            "Missing 'data' attribute on PREVIEW_REQUEST record".into(),
                        )
                    })?
                    .as_s()
                    .map_err(|_| {
                        AwsStorageError::DynamoDbError(
                            "'data' attribute is not a string".into(),
                        )
                    })?;

                let parsed: serde_json::Value = serde_json::from_str(data_str)
                    .map_err(|e| AwsStorageError::DynamoDbError(format!("JSON parse error: {}", e)))?;

                Ok(parsed["status"].as_str().map(|s| s.to_string()))
            }
            None => Ok(None),
        }
    }

    /// Read the PREVIEW_RESULT item.
    ///
    /// Returns the parsed `data` JSON of the PREVIEW_RESULT record, or `None`
    /// if the batch worker has not yet written the result.
    pub async fn get_preview_result(&self, slug: &NamespaceSlug) -> Result<Option<serde_json::Value>> {
        let pk = format!("NS#{}", slug.as_str());

        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk))
            .key("SK", AttributeValue::S("PREVIEW_RESULT".to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        match result.item {
            Some(item) => {
                let data_str = item
                    .get("data")
                    .ok_or_else(|| {
                        AwsStorageError::DynamoDbError(
                            "Missing 'data' attribute on PREVIEW_RESULT record".into(),
                        )
                    })?
                    .as_s()
                    .map_err(|_| {
                        AwsStorageError::DynamoDbError(
                            "'data' attribute is not a string".into(),
                        )
                    })?;

                let parsed: serde_json::Value = serde_json::from_str(data_str)
                    .map_err(|e| AwsStorageError::DynamoDbError(format!("JSON parse error: {}", e)))?;

                Ok(Some(parsed))
            }
            None => Ok(None),
        }
    }

    /// Internal: store schema without clearing PipelineConfig types.
    ///
    /// Used by approve/reject to update the schema without triggering
    /// the re-proposal config clearing logic.
    async fn store_schema_raw(
        &self,
        slug: &NamespaceSlug,
        proposal: &SchemaProposal,
    ) -> Result<()> {
        let pk = format!("NS#{}", slug.as_str());
        let proposal_json = serde_json::to_string(proposal)?;

        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(pk))
            .item("SK", AttributeValue::S("SCHEMA".to_string()))
            .item("data", AttributeValue::S(proposal_json))
            .item(
                "proposed_at",
                AttributeValue::N(proposal.proposed_at.to_string()),
            )
            .send()
            .await
            .map_err(dynamo_err)?;

        Ok(())
    }
}

/// Map AwsStorageError to NamespaceRegistryError.
fn to_registry_error(err: AwsStorageError) -> NamespaceRegistryError {
    match err {
        AwsStorageError::NamespaceAlreadyExists(slug) => {
            NamespaceRegistryError::AlreadyExists(slug)
        }
        AwsStorageError::NamespaceNotFound(slug) => NamespaceRegistryError::NotFound(slug),
        AwsStorageError::SchemaNotFound(slug) => NamespaceRegistryError::SchemaNotFound(slug),
        AwsStorageError::InvalidSchemaState(msg) => {
            NamespaceRegistryError::InvalidSchemaState(msg)
        }
        other => NamespaceRegistryError::Internal(other.to_string()),
    }
}

#[async_trait::async_trait]
impl NamespaceRegistry for DynamoNamespaceRegistry {
    async fn create_namespace(
        &self,
        slug: &NamespaceSlug,
        description: Option<String>,
    ) -> std::result::Result<NamespaceRecord, NamespaceRegistryError> {
        self.create_namespace(slug, description)
            .await
            .map_err(to_registry_error)
    }

    async fn list_namespaces(
        &self,
    ) -> std::result::Result<Vec<CoreNamespaceListItem>, NamespaceRegistryError> {
        let items = self.list_namespaces().await.map_err(to_registry_error)?;
        Ok(items
            .into_iter()
            .map(|item| CoreNamespaceListItem {
                slug: item.slug,
                created_at: item.created_at,
                entity_count: item.entity_count,
                document_count: item.document_count,
            })
            .collect())
    }

    async fn describe_namespace(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<Option<NamespaceRecord>, NamespaceRegistryError> {
        self.describe_namespace(slug)
            .await
            .map_err(to_registry_error)
    }

    async fn get_config(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<Option<PipelineConfig>, NamespaceRegistryError> {
        self.get_config(slug).await.map_err(to_registry_error)
    }

    async fn update_config(
        &self,
        slug: &NamespaceSlug,
        config: &PipelineConfig,
    ) -> std::result::Result<(), NamespaceRegistryError> {
        self.update_config(slug, config)
            .await
            .map_err(to_registry_error)
    }

    async fn get_descriptor(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<Option<McpDescriptor>, NamespaceRegistryError> {
        self.get_descriptor(slug)
            .await
            .map_err(to_registry_error)
    }

    async fn get_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<Option<SchemaProposal>, NamespaceRegistryError> {
        self.get_schema(slug).await.map_err(to_registry_error)
    }

    async fn store_schema(
        &self,
        slug: &NamespaceSlug,
        proposal: &SchemaProposal,
    ) -> std::result::Result<(), NamespaceRegistryError> {
        self.store_schema(slug, proposal)
            .await
            .map_err(to_registry_error)
    }

    async fn approve_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<SchemaProposal, NamespaceRegistryError> {
        self.approve_schema(slug)
            .await
            .map_err(to_registry_error)
    }

    async fn reject_schema(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<SchemaProposal, NamespaceRegistryError> {
        self.reject_schema(slug)
            .await
            .map_err(to_registry_error)
    }

    async fn put_preview_request(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<(), NamespaceRegistryError> {
        self.put_preview_request(slug)
            .await
            .map_err(to_registry_error)
    }

    async fn get_preview_status(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<Option<String>, NamespaceRegistryError> {
        self.get_preview_status(slug)
            .await
            .map_err(to_registry_error)
    }

    async fn get_preview_result(
        &self,
        slug: &NamespaceSlug,
    ) -> std::result::Result<Option<serde_json::Value>, NamespaceRegistryError> {
        self.get_preview_result(slug)
            .await
            .map_err(to_registry_error)
    }
}
