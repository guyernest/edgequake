//! DynamoDB workspace service.
//!
//! Provides persistent storage for tenants and workspaces (knowledge bases)
//! using Amazon DynamoDB as the authority store, so the live admin "My Instance"
//! backend stops losing tenants/workspaces on every Lambda cold start.
//!
//! Implements the 8 [`WorkspaceService`] methods the live admin read/write paths
//! actually exercise (tenant get/create/list, workspace create/insert/get/get-by-slug/list)
//! and stubs the remaining ~17 verbatim from
//! [`InMemoryWorkspaceService`](edgequake_core::InMemoryWorkspaceService) (they are off the
//! live path and return zeros/empty/Ok exactly as the in-memory impl does).
//!
//! ## Table Design (single PK/SK String table — mirrors `dynamodb_namespace.rs`)
//!
//! ```text
//! Table: {config.table_name}
//! PK (S)                SK (S)          data
//! TENANTS               {tenant_id}     { Tenant JSON }      -- list_tenants partition
//! TENANT#{tenant_id}    META            { Tenant JSON }      -- get_tenant
//! WS#{workspace_id}     META            { Workspace JSON }   -- get_workspace(id) one GetItem (Pitfall 5)
//! WSLIST#{tenant_id}    {workspace_id}  { workspace_id }     -- list_workspaces(tenant) = Query PK
//! WSBYSLUG#{tenant_id}  {slug}          { workspace_id }     -- get_workspace_by_slug O(1), no Scan
//! ```
//!
//! Only GetItem/PutItem/Query are used — NO GSI, NO Scan. Attribute names are
//! `PK`/`SK` (same as the namespace table).
//!
//! ## Consistent reads
//!
//! The admin create-instance flow lists/reads immediately after the write
//! (`create_tenant` -> `list_tenants`; `create_workspace` -> `get_workspace_by_slug`/
//! `get_workspace`), so the five read methods (`get_tenant`, `list_tenants`,
//! `get_workspace`, `get_workspace_by_slug`, `list_workspaces`) set
//! `.consistent_read(true)` — default eventual consistency would intermittently 404
//! the just-created row.
//!
//! ## Slug-uniqueness conditional put
//!
//! `create_workspace`'s `WSBYSLUG#{tenant}/{slug}` put uses a conditional
//! `attribute_not_exists(PK)` expression so a duplicate slug fails cleanly
//! (`ConditionalCheckFailed` -> a validation error) instead of silently overwriting an
//! existing workspace's slug index. Full `TransactWriteItems` across the 2-3 items is
//! intentionally OUT of scope for this single-tenant low-concurrency admin — the
//! conditional put on the unique key is the required item.
//!
//! ## Example
//!
//! ```rust,ignore
//! use edgequake_storage_aws::{DynamoWorkspaceService, DynamoWorkspaceConfig};
//!
//! let config = DynamoWorkspaceConfig {
//!     table_name: "pmcp-graphrag-workspaces".to_string(),
//! };
//! let service = DynamoWorkspaceService::new(config, dynamo_client);
//! ```

use async_trait::async_trait;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client as DynamoDbClient;
use tracing::{debug, info};
use uuid::Uuid;

use edgequake_core::error::{Error, Result};
use edgequake_core::types::{
    CreateWorkspaceRequest, Membership, MembershipRole, MetricsSnapshot, MetricsTriggerType,
    Tenant, TenantContext, UpdateWorkspaceRequest, Workspace, WorkspaceStats,
};
use edgequake_core::WorkspaceService;

// ---------------------------------------------------------------------------
// Error mapping helpers (mirror dynamodb_namespace.rs)
// ---------------------------------------------------------------------------

/// Extract a descriptive error message from a DynamoDB SDK error.
///
/// Mirrors the `dynamo_err` helper in `dynamodb_namespace.rs` / `dynamodb_kv.rs`
/// for consistent error reporting across all DynamoDB modules.
fn dynamo_err<E, R>(err: aws_smithy_runtime_api::client::result::SdkError<E, R>) -> Error
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
    Error::internal(format!("DynamoDB error: {}", msg))
}

/// Check whether a DynamoDB SDK error is a ConditionalCheckFailedException.
///
/// Mirrors the `is_condition_check_failed` helper in `dynamodb_namespace.rs`.
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

// ---------------------------------------------------------------------------
// Key builders (pure free functions — unit-tested AppState-free)
// ---------------------------------------------------------------------------

/// PK/SK for the `TENANTS` list partition entry: `(TENANTS, {tenant_id})`.
fn tenants_list_key(tenant_id: Uuid) -> (String, String) {
    ("TENANTS".to_string(), tenant_id.to_string())
}

/// PK/SK for the per-tenant META record: `(TENANT#{tenant_id}, META)`.
fn tenant_meta_key(tenant_id: Uuid) -> (String, String) {
    (format!("TENANT#{}", tenant_id), "META".to_string())
}

/// PK/SK for the per-workspace META record: `(WS#{workspace_id}, META)`.
///
/// This is the single-GetItem key for `get_workspace(workspace_id)` (no tenant
/// scope — Pitfall 5).
fn workspace_meta_key(workspace_id: Uuid) -> (String, String) {
    (format!("WS#{}", workspace_id), "META".to_string())
}

/// PK/SK for a per-tenant workspace-list entry: `(WSLIST#{tenant_id}, {workspace_id})`.
///
/// `list_workspaces(tenant)` queries `PK = WSLIST#{tenant_id}`.
fn workspace_list_key(tenant_id: Uuid, workspace_id: Uuid) -> (String, String) {
    (format!("WSLIST#{}", tenant_id), workspace_id.to_string())
}

/// PK for the per-tenant workspace-list partition: `WSLIST#{tenant_id}`.
fn workspace_list_pk(tenant_id: Uuid) -> String {
    format!("WSLIST#{}", tenant_id)
}

/// PK/SK for the slug-uniqueness index entry: `(WSBYSLUG#{tenant_id}, {slug})`.
///
/// `get_workspace_by_slug(tenant, slug)` reads this O(1), then dereferences the
/// stored workspace_id to the WS#{id}/META record.
fn workspace_by_slug_key(tenant_id: Uuid, slug: &str) -> (String, String) {
    (format!("WSBYSLUG#{}", tenant_id), slug.to_string())
}

// ---------------------------------------------------------------------------
// Serde round-trip helpers (pure — unit-tested AppState-free)
// ---------------------------------------------------------------------------

/// Serialize a [`Tenant`] to the `data` attribute string.
fn tenant_to_data(tenant: &Tenant) -> Result<String> {
    serde_json::to_string(tenant)
        .map_err(|e| Error::internal(format!("Tenant serialize error: {}", e)))
}

/// Deserialize a [`Tenant`] from a `data` attribute string.
fn tenant_from_data(data: &str) -> Result<Tenant> {
    serde_json::from_str(data)
        .map_err(|e| Error::internal(format!("Tenant deserialize error: {}", e)))
}

/// Serialize a [`Workspace`] to the `data` attribute string.
fn workspace_to_data(workspace: &Workspace) -> Result<String> {
    serde_json::to_string(workspace)
        .map_err(|e| Error::internal(format!("Workspace serialize error: {}", e)))
}

/// Deserialize a [`Workspace`] from a `data` attribute string.
fn workspace_from_data(data: &str) -> Result<Workspace> {
    serde_json::from_str(data)
        .map_err(|e| Error::internal(format!("Workspace deserialize error: {}", e)))
}

/// Pull the string `data` attribute out of a DynamoDB item map.
fn read_data_attr(
    item: &std::collections::HashMap<String, AttributeValue>,
    record_kind: &str,
) -> Result<String> {
    let data = item
        .get("data")
        .ok_or_else(|| {
            Error::internal(format!("Missing 'data' attribute on {} record", record_kind))
        })?
        .as_s()
        .map_err(|_| {
            Error::internal(format!(
                "'data' attribute is not a string on {} record",
                record_kind
            ))
        })?;
    Ok(data.clone())
}

/// Generate a URL-safe slug from a name (mirrors
/// `InMemoryWorkspaceService::generate_slug`).
fn generate_slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Configuration for the DynamoDB workspace service.
#[derive(Debug, Clone)]
pub struct DynamoWorkspaceConfig {
    /// DynamoDB table name (e.g., "pmcp-graphrag-workspaces").
    pub table_name: String,
}

/// DynamoDB-backed workspace service.
///
/// Persists tenants and workspaces to a single PK/SK DynamoDB table. The 8 live
/// admin methods are real; the remaining ~17 are stubbed verbatim from
/// [`InMemoryWorkspaceService`](edgequake_core::InMemoryWorkspaceService).
pub struct DynamoWorkspaceService {
    client: DynamoDbClient,
    config: DynamoWorkspaceConfig,
}

impl DynamoWorkspaceService {
    /// Create a new workspace service with a pre-configured DynamoDB client.
    ///
    /// Synchronous — mirrors `DynamoNamespaceRegistry::new`. The caller is
    /// responsible for building the `aws_sdk_dynamodb::Client` via an async
    /// `aws_config::load_defaults(...).await` (NEVER `block_on`, per the
    /// documented cold-start panic).
    ///
    /// # Arguments
    ///
    /// * `config` - Service configuration (table name)
    /// * `client` - Pre-configured DynamoDB client
    pub fn new(config: DynamoWorkspaceConfig, client: DynamoDbClient) -> Self {
        Self { client, config }
    }

    // -- internal item readers -------------------------------------------------

    /// Read a single [`Tenant`] by its META key (strongly consistent).
    async fn read_tenant(&self, tenant_id: Uuid) -> Result<Option<Tenant>> {
        let (pk, sk) = tenant_meta_key(tenant_id);
        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk))
            .key("SK", AttributeValue::S(sk))
            .consistent_read(true)
            .send()
            .await
            .map_err(dynamo_err)?;

        match result.item {
            Some(item) => Ok(Some(tenant_from_data(&read_data_attr(&item, "tenant META")?)?)),
            None => Ok(None),
        }
    }

    /// Read a single [`Workspace`] by its META key (strongly consistent).
    async fn read_workspace(&self, workspace_id: Uuid) -> Result<Option<Workspace>> {
        let (pk, sk) = workspace_meta_key(workspace_id);
        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(pk))
            .key("SK", AttributeValue::S(sk))
            .consistent_read(true)
            .send()
            .await
            .map_err(dynamo_err)?;

        match result.item {
            Some(item) => Ok(Some(workspace_from_data(&read_data_attr(
                &item,
                "workspace META",
            )?)?)),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl WorkspaceService for DynamoWorkspaceService {
    // ===================== Tenant Operations (LIVE) =====================

    async fn create_tenant(&self, tenant: Tenant) -> Result<Tenant> {
        let tenant_json = tenant_to_data(&tenant)?;
        let (meta_pk, meta_sk) = tenant_meta_key(tenant.tenant_id);

        // Write the TENANT#{id}/META record with a uniqueness guard on the
        // tenant id. (Slug-uniqueness across tenants is not enforced at the
        // store level here — the single-tenant admin creates one tenant; the
        // in-memory impl's slug check is a best-effort guard not relied on by
        // the live path.)
        let meta_result = self
            .client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(meta_pk))
            .item("SK", AttributeValue::S(meta_sk))
            .item("data", AttributeValue::S(tenant_json.clone()))
            .condition_expression("attribute_not_exists(PK)")
            .send()
            .await;

        match meta_result {
            Ok(_) => {}
            Err(ref e) if is_condition_check_failed(e) => {
                return Err(Error::validation(format!(
                    "Tenant {} already exists",
                    tenant.tenant_id
                )));
            }
            Err(e) => return Err(dynamo_err(e)),
        }

        // Write the TENANTS/{id} list entry (best-effort; no condition).
        let (list_pk, list_sk) = tenants_list_key(tenant.tenant_id);
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(list_pk))
            .item("SK", AttributeValue::S(list_sk))
            .item("data", AttributeValue::S(tenant_json))
            .send()
            .await
            .map_err(dynamo_err)?;

        info!(tenant_id = %tenant.tenant_id, "Created tenant (DynamoDB)");
        Ok(tenant)
    }

    async fn get_tenant(&self, tenant_id: Uuid) -> Result<Option<Tenant>> {
        self.read_tenant(tenant_id).await
    }

    async fn list_tenants(&self, limit: usize, offset: usize) -> Result<Vec<Tenant>> {
        let mut tenants = Vec::new();
        let mut last_evaluated_key = None;

        loop {
            let mut query = self
                .client
                .query()
                .table_name(&self.config.table_name)
                .key_condition_expression("PK = :pk")
                .expression_attribute_values(":pk", AttributeValue::S("TENANTS".to_string()))
                .consistent_read(true);

            if let Some(key) = last_evaluated_key {
                query = query.set_exclusive_start_key(Some(key));
            }

            let result = query.send().await.map_err(dynamo_err)?;

            if let Some(items) = result.items {
                for item in items {
                    let data = read_data_attr(&item, "tenant list")?;
                    tenants.push(tenant_from_data(&data)?);
                }
            }

            last_evaluated_key = result.last_evaluated_key;
            if last_evaluated_key.is_none() {
                break;
            }
        }

        // Apply offset/limit after gathering (small admin partition).
        Ok(tenants.into_iter().skip(offset).take(limit).collect())
    }

    // ===================== Workspace Operations (LIVE) =====================

    async fn create_workspace(
        &self,
        tenant_id: Uuid,
        request: CreateWorkspaceRequest,
    ) -> Result<Workspace> {
        // 1. Tenant must exist.
        let tenant = self
            .read_tenant(tenant_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("Tenant {} not found", tenant_id)))?;

        // 2. Enforce max_workspaces (count existing via the WSLIST partition).
        let existing = self.list_workspaces(tenant_id).await?;
        if existing.len() >= tenant.max_workspaces {
            return Err(Error::validation(format!(
                "Tenant has reached maximum workspace limit ({})",
                tenant.max_workspaces
            )));
        }

        let slug = request
            .slug
            .clone()
            .unwrap_or_else(|| generate_slug(&request.name));

        // 3. Build the workspace, inheriting tenant defaults then applying the
        //    request overrides (mirrors InMemoryWorkspaceService::create_workspace).
        let mut workspace = Workspace::new(tenant_id, &request.name, &slug).with_llm_config(
            tenant.default_llm_model.clone(),
            tenant.default_llm_provider.clone(),
        );
        workspace = workspace.with_embedding_config(
            tenant.default_embedding_model.clone(),
            tenant.default_embedding_provider.clone(),
            tenant.default_embedding_dimension,
        );

        if let Some(desc) = request.description.clone() {
            workspace = workspace.with_description(desc);
        }
        if let Some(max_docs) = request.max_documents {
            workspace = workspace.with_max_documents(max_docs);
        }

        // SPEC-032: apply LLM configuration from request (overrides tenant default).
        if let Some(model) = request.llm_model.clone() {
            workspace = workspace.with_llm_model(&model);
            if let Some(provider) = request.llm_provider.clone() {
                workspace = workspace.with_llm_provider(&provider);
            }
        } else if let Some(provider) = request.llm_provider.clone() {
            workspace = workspace.with_llm_provider(&provider);
        }

        // SPEC-032: apply embedding configuration from request (overrides tenant default).
        if let Some(model) = request.embedding_model.clone() {
            workspace = workspace.with_embedding_model(&model);
            if let Some(provider) = request.embedding_provider.clone() {
                workspace = workspace.with_embedding_provider(&provider);
            } else {
                let detected = Workspace::detect_provider_from_model(&model);
                workspace = workspace.with_embedding_provider(detected);
            }
            if let Some(dim) = request.embedding_dimension {
                workspace = workspace.with_embedding_dimension(dim);
            } else {
                let detected = Workspace::detect_dimension_from_model(&model);
                workspace = workspace.with_embedding_dimension(detected);
            }
        }

        // 4. Claim the slug-uniqueness key FIRST with a conditional put
        //    (attribute_not_exists) so a duplicate slug fails cleanly before any
        //    other item is written (Review edit D — no silent overwrite).
        let (slug_pk, slug_sk) = workspace_by_slug_key(tenant_id, &workspace.slug);
        let slug_result = self
            .client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(slug_pk))
            .item("SK", AttributeValue::S(slug_sk))
            .item(
                "workspace_id",
                AttributeValue::S(workspace.workspace_id.to_string()),
            )
            .condition_expression("attribute_not_exists(PK)")
            .send()
            .await;

        match slug_result {
            Ok(_) => {}
            Err(ref e) if is_condition_check_failed(e) => {
                return Err(Error::validation(format!(
                    "Workspace with slug '{}' already exists in this tenant",
                    workspace.slug
                )));
            }
            Err(e) => return Err(dynamo_err(e)),
        }

        // 5. Write the WS#{id}/META record (the get_workspace(id) one-GetItem item).
        let ws_json = workspace_to_data(&workspace)?;
        let (ws_pk, ws_sk) = workspace_meta_key(workspace.workspace_id);
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(ws_pk))
            .item("SK", AttributeValue::S(ws_sk))
            .item("data", AttributeValue::S(ws_json))
            .send()
            .await
            .map_err(dynamo_err)?;

        // 6. Write the per-tenant WSLIST entry (list_workspaces partition).
        let (list_pk, list_sk) = workspace_list_key(tenant_id, workspace.workspace_id);
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(list_pk))
            .item("SK", AttributeValue::S(list_sk))
            .item(
                "workspace_id",
                AttributeValue::S(workspace.workspace_id.to_string()),
            )
            .send()
            .await
            .map_err(dynamo_err)?;

        info!(
            workspace_id = %workspace.workspace_id,
            tenant_id = %tenant_id,
            "Created workspace (DynamoDB)"
        );
        Ok(workspace)
    }

    async fn insert_workspace(&self, workspace: Workspace) -> Result<Workspace> {
        // Tenant must exist.
        let _tenant = self
            .read_tenant(workspace.tenant_id)
            .await?
            .ok_or_else(|| {
                Error::not_found(format!("Tenant {} not found", workspace.tenant_id))
            })?;

        // Claim the slug index conditionally (clean duplicate-slug failure).
        let (slug_pk, slug_sk) = workspace_by_slug_key(workspace.tenant_id, &workspace.slug);
        let slug_result = self
            .client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(slug_pk))
            .item("SK", AttributeValue::S(slug_sk))
            .item(
                "workspace_id",
                AttributeValue::S(workspace.workspace_id.to_string()),
            )
            .condition_expression("attribute_not_exists(PK)")
            .send()
            .await;

        match slug_result {
            Ok(_) => {}
            Err(ref e) if is_condition_check_failed(e) => {
                // Slug already claimed. If it points at THIS workspace, treat the
                // insert as idempotent; if it points at a DIFFERENT workspace it's a
                // genuine conflict; if it resolves to NOTHING the index is dangling
                // (its target WS#META is gone) — proceeding would write a workspace
                // unreachable by slug, so fail loud instead of silently continuing.
                match self
                    .get_workspace_by_slug(workspace.tenant_id, &workspace.slug)
                    .await?
                {
                    Some(existing) if existing.workspace_id == workspace.workspace_id => {
                        // Idempotent re-insert of the same workspace — fall through.
                    }
                    Some(_) => {
                        return Err(Error::validation(format!(
                            "Workspace with slug '{}' already exists in this tenant",
                            workspace.slug
                        )));
                    }
                    None => {
                        return Err(Error::internal(format!(
                            "Workspace slug '{}' index exists but resolves to no workspace \
                             (inconsistent state); refusing to insert",
                            workspace.slug
                        )));
                    }
                }
            }
            Err(e) => return Err(dynamo_err(e)),
        }

        // Write WS#{id}/META.
        let ws_json = workspace_to_data(&workspace)?;
        let (ws_pk, ws_sk) = workspace_meta_key(workspace.workspace_id);
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(ws_pk))
            .item("SK", AttributeValue::S(ws_sk))
            .item("data", AttributeValue::S(ws_json))
            .send()
            .await
            .map_err(dynamo_err)?;

        // Write the WSLIST entry.
        let (list_pk, list_sk) = workspace_list_key(workspace.tenant_id, workspace.workspace_id);
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(list_pk))
            .item("SK", AttributeValue::S(list_sk))
            .item(
                "workspace_id",
                AttributeValue::S(workspace.workspace_id.to_string()),
            )
            .send()
            .await
            .map_err(dynamo_err)?;

        info!(
            workspace_id = %workspace.workspace_id,
            tenant_id = %workspace.tenant_id,
            "Inserted workspace with specific ID (DynamoDB)"
        );
        Ok(workspace)
    }

    async fn get_workspace(&self, workspace_id: Uuid) -> Result<Option<Workspace>> {
        // Pitfall 5: NO tenant scope — one GetItem on WS#{id}/META.
        self.read_workspace(workspace_id).await
    }

    async fn get_workspace_by_slug(
        &self,
        tenant_id: Uuid,
        slug: &str,
    ) -> Result<Option<Workspace>> {
        // Hop 1: read the slug index entry (strongly consistent).
        let (slug_pk, slug_sk) = workspace_by_slug_key(tenant_id, slug);
        let result = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key("PK", AttributeValue::S(slug_pk))
            .key("SK", AttributeValue::S(slug_sk))
            .consistent_read(true)
            .send()
            .await
            .map_err(dynamo_err)?;

        let workspace_id = match result.item {
            Some(item) => {
                let id_str = item
                    .get("workspace_id")
                    .and_then(|v| v.as_s().ok())
                    .ok_or_else(|| {
                        Error::internal("Missing 'workspace_id' on WSBYSLUG index entry")
                    })?;
                Uuid::parse_str(id_str)
                    .map_err(|e| Error::internal(format!("Invalid workspace_id in index: {}", e)))?
            }
            None => return Ok(None),
        };

        // Hop 2: read the workspace META (strongly consistent).
        self.read_workspace(workspace_id).await
    }

    async fn list_workspaces(&self, tenant_id: Uuid) -> Result<Vec<Workspace>> {
        // Query the WSLIST#{tenant} partition for workspace ids (strongly consistent).
        let mut ids: Vec<Uuid> = Vec::new();
        let mut last_evaluated_key = None;

        loop {
            let mut query = self
                .client
                .query()
                .table_name(&self.config.table_name)
                .key_condition_expression("PK = :pk")
                .expression_attribute_values(
                    ":pk",
                    AttributeValue::S(workspace_list_pk(tenant_id)),
                )
                .consistent_read(true);

            if let Some(key) = last_evaluated_key {
                query = query.set_exclusive_start_key(Some(key));
            }

            let result = query.send().await.map_err(dynamo_err)?;

            if let Some(items) = result.items {
                for item in items {
                    let id_str = item
                        .get("SK")
                        .and_then(|v| v.as_s().ok())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    if let Ok(id) = Uuid::parse_str(&id_str) {
                        ids.push(id);
                    }
                }
            }

            last_evaluated_key = result.last_evaluated_key;
            if last_evaluated_key.is_none() {
                break;
            }
        }

        // Dereference each id to its workspace META (strongly consistent).
        let mut workspaces = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(ws) = self.read_workspace(id).await? {
                workspaces.push(ws);
            }
        }

        debug!(
            tenant_id = %tenant_id,
            count = workspaces.len(),
            "Listed workspaces (DynamoDB)"
        );
        Ok(workspaces)
    }

    // ===================== Opportunistic implementations =====================

    async fn get_tenant_by_slug(&self, slug: &str) -> Result<Option<Tenant>> {
        // No slug index for tenants in this table design; scan the small TENANTS
        // partition (single-tenant admin). Reuses the strongly-consistent
        // list_tenants reader.
        let tenants = self.list_tenants(1000, 0).await?;
        Ok(tenants.into_iter().find(|t| t.slug == slug))
    }

    async fn update_workspace(
        &self,
        workspace_id: Uuid,
        request: UpdateWorkspaceRequest,
    ) -> Result<Workspace> {
        let mut workspace = self
            .read_workspace(workspace_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("Workspace {} not found", workspace_id)))?;

        if let Some(name) = request.name {
            workspace.name = name;
        }
        if let Some(desc) = request.description {
            workspace.description = Some(desc);
        }
        if let Some(is_active) = request.is_active {
            workspace.is_active = is_active;
        }
        if let Some(max_docs) = request.max_documents {
            workspace
                .metadata
                .insert("max_documents".to_string(), serde_json::json!(max_docs));
        }
        if let Some(llm_model) = request.llm_model {
            workspace.llm_model = llm_model;
        }
        if let Some(llm_provider) = request.llm_provider {
            workspace.llm_provider = llm_provider;
        }
        if let Some(embedding_model) = request.embedding_model {
            workspace.embedding_model = embedding_model;
        }
        if let Some(embedding_provider) = request.embedding_provider {
            workspace.embedding_provider = embedding_provider;
        }
        if let Some(embedding_dimension) = request.embedding_dimension {
            workspace.embedding_dimension = embedding_dimension;
        }
        // Phase 33 — INGEST-SCHEMA-DRAFT: merge arbitrary metadata entries.
        if let Some(extra_meta) = request.metadata {
            for (k, v) in extra_meta {
                workspace.metadata.insert(k, v);
            }
        }

        workspace.updated_at = chrono::Utc::now();

        // Persist the updated workspace META (slug unchanged → index untouched).
        let ws_json = workspace_to_data(&workspace)?;
        let (ws_pk, ws_sk) = workspace_meta_key(workspace.workspace_id);
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(ws_pk))
            .item("SK", AttributeValue::S(ws_sk))
            .item("data", AttributeValue::S(ws_json))
            .send()
            .await
            .map_err(dynamo_err)?;

        Ok(workspace)
    }

    // ===================== Stubbed methods (off the live path) =====================
    // These return the SAME values InMemoryWorkspaceService returns
    // (empty/zeros/Ok) — they must compile and never panic on an admin call.

    async fn update_tenant(&self, tenant: Tenant) -> Result<Tenant> {
        // Off the live admin path. Best-effort overwrite of the META + list items
        // (no existence check — mirrors the cheap stub posture).
        let tenant_json = tenant_to_data(&tenant)?;
        let (meta_pk, meta_sk) = tenant_meta_key(tenant.tenant_id);
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(meta_pk))
            .item("SK", AttributeValue::S(meta_sk))
            .item("data", AttributeValue::S(tenant_json.clone()))
            .send()
            .await
            .map_err(dynamo_err)?;
        let (list_pk, list_sk) = tenants_list_key(tenant.tenant_id);
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .item("PK", AttributeValue::S(list_pk))
            .item("SK", AttributeValue::S(list_sk))
            .item("data", AttributeValue::S(tenant_json))
            .send()
            .await
            .map_err(dynamo_err)?;
        Ok(tenant)
    }

    async fn delete_tenant(&self, _tenant_id: Uuid) -> Result<()> {
        // Stub: deletion cascade is off the live admin path.
        Ok(())
    }

    async fn delete_workspace(&self, _workspace_id: Uuid) -> Result<()> {
        // Stub: deletion cascade is off the live admin path.
        Ok(())
    }

    async fn get_workspace_stats(&self, workspace_id: Uuid) -> Result<WorkspaceStats> {
        // Stub (zeros) — mirrors InMemoryWorkspaceService. Real metrics require
        // storage adapters not available here.
        Ok(WorkspaceStats {
            workspace_id,
            document_count: 0,
            entity_count: 0,
            relationship_count: 0,
            chunk_count: 0,
            embedding_count: 0,
            storage_bytes: 0,
        })
    }

    async fn record_metrics_snapshot(
        &self,
        workspace_id: Uuid,
        trigger_type: MetricsTriggerType,
    ) -> Result<MetricsSnapshot> {
        // Stub — mirrors InMemoryWorkspaceService.
        Ok(MetricsSnapshot {
            id: Uuid::new_v4(),
            workspace_id,
            recorded_at: chrono::Utc::now(),
            trigger_type,
            document_count: 0,
            chunk_count: 0,
            entity_count: 0,
            relationship_count: 0,
            embedding_count: 0,
            storage_bytes: 0,
        })
    }

    async fn get_metrics_history(
        &self,
        _workspace_id: Uuid,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<MetricsSnapshot>> {
        // Stub (empty) — mirrors InMemoryWorkspaceService.
        Ok(Vec::new())
    }

    async fn add_membership(&self, membership: Membership) -> Result<Membership> {
        // Stub — memberships are off the live admin path.
        Ok(membership)
    }

    async fn get_user_memberships(&self, _user_id: Uuid) -> Result<Vec<Membership>> {
        Ok(Vec::new())
    }

    async fn get_tenant_memberships(&self, _tenant_id: Uuid) -> Result<Vec<Membership>> {
        Ok(Vec::new())
    }

    async fn update_membership_role(
        &self,
        membership_id: Uuid,
        role: MembershipRole,
    ) -> Result<Membership> {
        // Stub — return a synthetic membership echoing the requested role.
        Ok(Membership {
            membership_id,
            user_id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            workspace_id: None,
            role,
            is_active: true,
            joined_at: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
        })
    }

    async fn remove_membership(&self, _membership_id: Uuid) -> Result<()> {
        Ok(())
    }

    async fn check_tenant_access(&self, _user_id: Uuid, _tenant_id: Uuid) -> Result<bool> {
        // Stub — access control for the live admin path is enforced by the
        // handler's tenant-ownership check, not this method.
        Ok(false)
    }

    async fn check_workspace_access(&self, _user_id: Uuid, _workspace_id: Uuid) -> Result<bool> {
        Ok(false)
    }

    async fn get_user_role(
        &self,
        _user_id: Uuid,
        _tenant_id: Uuid,
    ) -> Result<Option<MembershipRole>> {
        Ok(None)
    }

    async fn build_context(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
        workspace_id: Option<Uuid>,
    ) -> Result<TenantContext> {
        // Stub — build a minimal context (no membership lookup). Off the live
        // admin path; mirrors the in-memory shape without the access gate.
        let mut ctx = TenantContext::new(tenant_id);
        if let Some(ws_id) = workspace_id {
            ctx = ctx.with_workspace(ws_id);
        }
        ctx = ctx.with_user(user_id, MembershipRole::Member);
        Ok(ctx)
    }
}

// ---------------------------------------------------------------------------
// Tests (AppState-free: pure key builders + serde round-trips)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamo_workspace_key_tenants_list() {
        let id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let (pk, sk) = tenants_list_key(id);
        assert_eq!(pk, "TENANTS");
        assert_eq!(sk, "11111111-1111-1111-1111-111111111111");
    }

    #[test]
    fn dynamo_workspace_key_tenant_meta() {
        let id = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        let (pk, sk) = tenant_meta_key(id);
        assert_eq!(pk, "TENANT#22222222-2222-2222-2222-222222222222");
        assert_eq!(sk, "META");
    }

    #[test]
    fn dynamo_workspace_key_workspace_meta() {
        let id = Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap();
        let (pk, sk) = workspace_meta_key(id);
        assert_eq!(pk, "WS#33333333-3333-3333-3333-333333333333");
        assert_eq!(sk, "META");
    }

    #[test]
    fn dynamo_workspace_key_workspace_list() {
        let t = Uuid::parse_str("44444444-4444-4444-4444-444444444444").unwrap();
        let w = Uuid::parse_str("55555555-5555-5555-5555-555555555555").unwrap();
        let (pk, sk) = workspace_list_key(t, w);
        assert_eq!(pk, "WSLIST#44444444-4444-4444-4444-444444444444");
        assert_eq!(sk, "55555555-5555-5555-5555-555555555555");
        assert_eq!(workspace_list_pk(t), "WSLIST#44444444-4444-4444-4444-444444444444");
    }

    #[test]
    fn dynamo_workspace_key_workspace_by_slug() {
        let t = Uuid::parse_str("66666666-6666-6666-6666-666666666666").unwrap();
        let (pk, sk) = workspace_by_slug_key(t, "my-kb");
        assert_eq!(pk, "WSBYSLUG#66666666-6666-6666-6666-666666666666");
        assert_eq!(sk, "my-kb");
    }

    #[test]
    fn dynamo_workspace_generate_slug() {
        assert_eq!(generate_slug("My Knowledge Base"), "my-knowledge-base");
        assert_eq!(generate_slug("  Trim--Me  "), "trim--me");
    }

    #[test]
    fn dynamo_workspace_tenant_serde_roundtrip() {
        let tenant = Tenant::new("Acme Corp", "acme")
            .with_embedding_config("text-embedding-3-small", "openai", 1536)
            .with_llm_config("gpt-4o-mini", "openai");

        let data = tenant_to_data(&tenant).unwrap();
        let restored = tenant_from_data(&data).unwrap();

        assert_eq!(restored.tenant_id, tenant.tenant_id);
        assert_eq!(restored.name, tenant.name);
        assert_eq!(restored.slug, tenant.slug);
        assert_eq!(
            restored.default_embedding_dimension,
            tenant.default_embedding_dimension
        );
        assert_eq!(restored.default_embedding_model, tenant.default_embedding_model);
        assert_eq!(restored.default_llm_provider, tenant.default_llm_provider);
        assert_eq!(restored.created_at, tenant.created_at);
    }

    #[test]
    fn dynamo_workspace_workspace_serde_roundtrip() {
        let tenant_id = Uuid::parse_str("77777777-7777-7777-7777-777777777777").unwrap();
        let workspace = Workspace::new(tenant_id, "Knowledge Base", "kb-1")
            .with_description("Primary KB")
            .with_max_documents(5000)
            .with_embedding_config("text-embedding-3-small", "openai", 1536)
            .with_llm_config("gpt-4o-mini", "openai");

        let data = workspace_to_data(&workspace).unwrap();
        let restored = workspace_from_data(&data).unwrap();

        assert_eq!(restored.workspace_id, workspace.workspace_id);
        assert_eq!(restored.tenant_id, tenant_id);
        assert_eq!(restored.slug, workspace.slug);
        assert_eq!(restored.name, workspace.name);
        assert_eq!(restored.description, workspace.description);
        assert_eq!(restored.embedding_model, workspace.embedding_model);
        assert_eq!(restored.embedding_dimension, workspace.embedding_dimension);
        assert_eq!(restored.embedding_provider, workspace.embedding_provider);
        assert_eq!(restored.llm_model, workspace.llm_model);
        assert_eq!(restored.llm_provider, workspace.llm_provider);
        assert_eq!(restored.is_active, workspace.is_active);
        assert_eq!(restored.created_at, workspace.created_at);
        assert_eq!(restored.updated_at, workspace.updated_at);
        assert_eq!(restored.max_documents(), Some(5000));
    }
}
