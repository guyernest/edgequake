//! Amazon Neptune graph storage using the Neptune Data API.
//!
//! Uses `aws-sdk-neptunedata` to execute Gremlin queries over HTTP with
//! automatic IAM SigV4 authentication. This replaces the previous WebSocket-based
//! `gremlin-client` approach, which didn't support IAM auth.
//!
//! ## Namespace Isolation
//!
//! Neptune namespace isolation supports two strategies (selected via `NamespaceMode`):
//!
//! ### Label-Prefix (default)
//! - Vertex labels: `{namespace}:Entity` (e.g., `epstein-files:Entity`)
//! - Edge labels: `{namespace}:relates_to` (e.g., `epstein-files:relates_to`)
//! - Node IDs: `{namespace}:{id}` (e.g., `epstein-files:JOHN_DOE`)
//!
//! This approach is recommended by AWS Prescriptive Guidance for O(1) partition
//! pruning via Neptune's index on labels.
//!
//! ### Property-Filter
//! - Bare vertex labels: `Entity`
//! - Bare edge labels: `relates_to`
//! - Bare node IDs: `JOHN_DOE`
//! - Namespace stored as a vertex/edge property: `namespace = "{namespace}"`
//!
//! This mode is used by the edgequake batch ingestion pipeline, which writes
//! data with property-based namespace isolation.
//!
//! The namespace prefix on IDs (`T.id`) in label-prefix mode prevents
//! cross-namespace ID collisions: two namespaces can each have a node called
//! "JOHN_DOE" without conflict. Prefixes are stripped transparently when
//! returning data to callers.
//!
//! ## Architecture
//!
//! ```text
//! Application → Neptune Data API (HTTP + SigV4) → Neptune Cluster
//!                                                     ├── Primary Instance
//!                                                     └── Read Replicas (optional)
//! ```

use std::collections::HashMap;

use async_trait::async_trait;
use aws_sdk_neptunedata::Client;
use serde_json::Value as JsonValue;
use tracing::{debug, info, warn};

use edgequake_storage::{GraphEdge, GraphNode, GraphStorage, KnowledgeGraph};

use crate::error::AwsStorageError;

/// Extract a descriptive error message from an AWS SDK error.
///
/// `SdkError::to_string()` often returns just "service error" which is useless
/// for debugging. This helper extracts the actual service error message using
/// the Debug trait when Display is uninformative.
fn neptune_err<E, R>(err: aws_smithy_runtime_api::client::result::SdkError<E, R>) -> AwsStorageError
where
    E: std::fmt::Display + std::fmt::Debug,
    R: std::fmt::Debug,
{
    let msg = match &err {
        aws_smithy_runtime_api::client::result::SdkError::ServiceError(ctx) => {
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
    AwsStorageError::NeptuneError(msg)
}

/// Escape a string value for safe inclusion in a Gremlin single-quoted string literal.
///
/// Prevents injection by escaping single quotes and backslashes in entity IDs
/// and other user-provided values interpolated into Gremlin query strings.
fn gremlin_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Controls how namespace isolation is implemented in Gremlin queries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NamespaceMode {
    /// Namespace encoded in labels and IDs (e.g., `epstein-files:Entity`, `epstein-files:JOHN_DOE`).
    #[default]
    LabelPrefix,
    /// Namespace stored as a vertex/edge property (`namespace = "acquired"`), bare labels and IDs.
    PropertyFilter,
}

/// Configuration for Neptune graph storage.
#[derive(Debug, Clone)]
pub struct NeptuneConfig {
    /// Neptune cluster endpoint (host:port)
    pub endpoint: String,
    /// Storage namespace slug (e.g., "epstein-files").
    ///
    /// Used as the label prefix for vertex/edge isolation and as an ID prefix
    /// for T.id uniqueness across namespaces.
    pub namespace: String,
    /// Use IAM authentication (always true with Neptune Data API)
    pub use_iam_auth: bool,
    /// Connection pool size (unused with HTTP API, kept for config compat)
    pub pool_size: usize,
    /// Query timeout in seconds
    pub timeout_secs: u64,
    /// Enable SSL/TLS (always true with Neptune Data API)
    pub use_ssl: bool,
    /// Namespace isolation strategy.
    pub namespace_mode: NamespaceMode,
}

impl NeptuneConfig {
    /// Create a new Neptune configuration.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - Neptune cluster endpoint (e.g., "my-cluster.cluster-xxx.region.neptune.amazonaws.com:8182")
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            namespace: "default".to_string(),
            use_iam_auth: true,
            pool_size: 10,
            timeout_secs: 30,
            use_ssl: true,
            namespace_mode: NamespaceMode::default(),
        }
    }

    /// Set the namespace slug.
    ///
    /// The namespace is used as a label prefix for vertex/edge isolation
    /// (e.g., `epstein-files:Entity`) and as an ID prefix for T.id uniqueness.
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    /// Enable or disable IAM authentication.
    pub fn with_iam_auth(mut self, enable: bool) -> Self {
        self.use_iam_auth = enable;
        self
    }

    /// Set connection pool size.
    pub fn with_pool_size(mut self, size: usize) -> Self {
        self.pool_size = size;
        self
    }

    /// Set query timeout.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Enable or disable SSL/TLS.
    pub fn with_ssl(mut self, enable: bool) -> Self {
        self.use_ssl = enable;
        self
    }

    /// Use property-based namespace filtering instead of label-prefix.
    pub fn with_property_namespace(mut self) -> Self {
        self.namespace_mode = NamespaceMode::PropertyFilter;
        self
    }
}

/// Amazon Neptune graph storage using the Neptune Data API.
///
/// Executes Gremlin queries over HTTP with IAM SigV4 authentication,
/// providing a fully managed, serverless graph database for storing
/// entities and relationships in the knowledge graph.
///
/// Namespace isolation strategy is controlled by `NamespaceMode`:
/// - **LabelPrefix**: vertices use `{ns}:Entity`, edges use `{ns}:relates_to`,
///   node IDs prefixed with `{ns}:` for T.id uniqueness.
/// - **PropertyFilter**: bare labels (`Entity`, `relates_to`), bare IDs,
///   namespace stored as a property filtered via `.has('namespace', '{ns}')`.
pub struct NeptuneGraphStorage {
    config: NeptuneConfig,
    client: Client,
    // Pre-computed namespace strings (avoids repeated format!() allocations)
    cached_vertex_label: String,
    cached_edge_label: String,
    cached_ns_filter: String,
    cached_ns_write_properties: String,
    cached_ns_id_prefix: Option<String>,
}

impl NeptuneGraphStorage {
    /// Create a new Neptune graph storage.
    pub async fn new(config: NeptuneConfig) -> crate::error::Result<Self> {
        let endpoint = &config.endpoint;

        // Parse host:port, stripping any protocol prefix
        let clean = endpoint
            .strip_prefix("wss://")
            .or_else(|| endpoint.strip_prefix("https://"))
            .or_else(|| endpoint.strip_prefix("ws://"))
            .or_else(|| endpoint.strip_prefix("http://"))
            .unwrap_or(endpoint);

        // Strip any path suffix (e.g., /gremlin)
        let host_port = clean.split('/').next().unwrap_or(clean);

        let neptune_url = format!("https://{}", host_port);

        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

        let neptune_config = aws_sdk_neptunedata::config::Builder::from(&aws_config)
            .endpoint_url(&neptune_url)
            .build();

        let client = Client::from_conf(neptune_config);

        // Pre-compute namespace strings once
        let cached_vertex_label = match config.namespace_mode {
            NamespaceMode::LabelPrefix => format!("{}:Entity", config.namespace),
            NamespaceMode::PropertyFilter => "Entity".to_string(),
        };
        let cached_edge_label = match config.namespace_mode {
            NamespaceMode::LabelPrefix => format!("{}:relates_to", config.namespace),
            NamespaceMode::PropertyFilter => "relates_to".to_string(),
        };
        let cached_ns_filter = match config.namespace_mode {
            NamespaceMode::LabelPrefix => String::new(),
            NamespaceMode::PropertyFilter => {
                format!(".has('namespace', '{}')", gremlin_escape(&config.namespace))
            }
        };
        let cached_ns_write_properties = match config.namespace_mode {
            NamespaceMode::LabelPrefix => String::new(),
            NamespaceMode::PropertyFilter => {
                format!(".property('namespace', '{}')", gremlin_escape(&config.namespace))
            }
        };
        let cached_ns_id_prefix = match config.namespace_mode {
            NamespaceMode::LabelPrefix => Some(format!("{}:", config.namespace)),
            NamespaceMode::PropertyFilter => None,
        };

        Ok(Self {
            config,
            client,
            cached_vertex_label,
            cached_edge_label,
            cached_ns_filter,
            cached_ns_write_properties,
            cached_ns_id_prefix,
        })
    }

    // ========== Namespace Helpers ==========

    /// Return the vertex label for this namespace mode.
    ///
    /// - LabelPrefix: `"{ns}:Entity"` (e.g., `epstein-files:Entity`)
    /// - PropertyFilter: `"Entity"`
    fn vertex_label(&self) -> &str {
        &self.cached_vertex_label
    }

    /// Return the edge label for this namespace mode.
    fn edge_label(&self) -> &str {
        &self.cached_edge_label
    }

    /// Translate a caller-supplied node ID to a Neptune T.id.
    fn ns_id(&self, node_id: &str) -> String {
        match &self.cached_ns_id_prefix {
            Some(prefix) => format!("{}{}", prefix, node_id),
            None => node_id.to_string(),
        }
    }

    /// Strip the namespace prefix from a Neptune T.id, returning the caller-facing ID.
    fn strip_ns_id<'a>(&self, neptune_id: &'a str) -> &'a str {
        match &self.cached_ns_id_prefix {
            Some(prefix) => neptune_id.strip_prefix(prefix.as_str()).unwrap_or(neptune_id),
            None => neptune_id,
        }
    }

    /// Return a Gremlin `.has('namespace', '{ns}')` filter step for PropertyFilter mode,
    /// or an empty string for LabelPrefix mode.
    fn ns_filter(&self) -> &str {
        &self.cached_ns_filter
    }

    /// Return Gremlin property steps to write the namespace property in PropertyFilter mode,
    /// or an empty string for LabelPrefix mode.
    fn ns_write_properties(&self) -> &str {
        &self.cached_ns_write_properties
    }

    /// Translate a caller-supplied node ID to a Gremlin-escaped Neptune T.id.
    fn escaped_ns_id(&self, node_id: &str) -> String {
        gremlin_escape(&self.ns_id(node_id))
    }

    /// Execute a Gremlin query via the Neptune Data API.
    ///
    /// The Neptune Data API returns results in GraphSON format with `@type`/`@value`
    /// wrappers. This method converts to plain JSON via `unwrap_graphson()`.
    ///
    /// Response structure: `{ "result": { "data": { "@type": "g:List", "@value": [...] }, "meta": ... } }`
    async fn execute(&self, query: &str) -> crate::error::Result<Vec<JsonValue>> {
        debug!("Executing Gremlin query: {}", query);

        let result = self
            .client
            .execute_gremlin_query()
            .gremlin_query(query)
            .send()
            .await
            .map_err(neptune_err)?;

        // The result field contains the Gremlin response as a Document.
        // Structure: {"data": {"@type": "g:List", "@value": [...]}, "meta": {...}}
        let result_doc = result.result();

        let json_value = if let Some(doc) = result_doc {
            document_to_json(doc)
        } else {
            return Ok(vec![]);
        };

        // Unwrap GraphSON typed values to plain JSON
        let unwrapped = unwrap_graphson(&json_value);

        // Extract the data array from the response envelope
        match unwrapped {
            JsonValue::Object(ref obj) => {
                if let Some(data) = obj.get("data") {
                    match data {
                        JsonValue::Array(arr) => Ok(arr.clone()),
                        other => Ok(vec![other.clone()]),
                    }
                } else {
                    // No "data" key — return as-is
                    Ok(vec![unwrapped])
                }
            }
            JsonValue::Array(arr) => Ok(arr),
            other => Ok(vec![other]),
        }
    }

    /// Convert properties HashMap to Gremlin property chain.
    ///
    /// Note: Does NOT inject the `namespace` property — that is handled
    /// separately by `ns_write_properties()` for PropertyFilter mode.
    fn properties_to_gremlin(&self, properties: &HashMap<String, JsonValue>) -> String {
        let mut parts = Vec::new();

        for (key, value) in properties {
            let value_str = match value {
                JsonValue::String(s) => format!("'{}'", gremlin_escape(s)),
                JsonValue::Number(n) => n.to_string(),
                JsonValue::Bool(b) => b.to_string(),
                JsonValue::Null => "null".to_string(),
                _ => format!(
                    "'{}'",
                    gremlin_escape(&serde_json::to_string(value).unwrap_or_default())
                ),
            };
            parts.push(format!(".property('{}', {})", key, value_str));
        }

        parts.join("")
    }

    /// Parse vertex to GraphNode from unwrapped elementMap() format.
    ///
    /// After GraphSON unwrapping, elementMap() returns a flat JSON object:
    /// `{"id": "epstein-files:SEC", "label": "epstein-files:Entity", "entity_type": "ORGANIZATION", ...}`
    ///
    /// The namespace prefix is stripped from the vertex ID before returning
    /// to callers, providing transparent isolation.
    fn vertex_to_node(&self, vertex: &JsonValue) -> crate::error::Result<GraphNode> {
        let raw_id = vertex["id"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing vertex id".into()))?;

        // Strip namespace prefix from the Neptune T.id
        let id = self.strip_ns_id(raw_id).to_string();

        let mut properties = HashMap::new();

        // elementMap() returns a flat object — all keys except "id", "label",
        // and "namespace" (internal isolation property) are vertex properties
        if let Some(obj) = vertex.as_object() {
            for (key, value) in obj {
                if key != "id" && key != "label" && key != "namespace" {
                    properties.insert(key.clone(), value.clone());
                }
            }
        }

        Ok(GraphNode { id, properties })
    }

    /// Parse edge to GraphEdge from unwrapped elementMap() format.
    ///
    /// After GraphSON unwrapping, edge elementMap() returns:
    /// ```json
    /// {
    ///   "id": "epstein-files:COMPANY_A->AGENT",
    ///   "label": "epstein-files:relates_to",
    ///   "OUT": {"id": "epstein-files:COMPANY_A", "label": "epstein-files:Entity"},
    ///   "IN": {"id": "epstein-files:AGENT", "label": "epstein-files:Entity"},
    ///   "description": "...",
    ///   "weight": 0.5
    /// }
    /// ```
    ///
    /// Namespace prefixes are stripped from source/target vertex IDs before
    /// returning to callers.
    fn edge_to_graph_edge(&self, edge: &JsonValue) -> crate::error::Result<GraphEdge> {
        // Source vertex is under "OUT" key (outgoing direction)
        let raw_source = edge["OUT"]["id"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing edge source (OUT.id)".into()))?;

        // Target vertex is under "IN" key (incoming direction)
        let raw_target = edge["IN"]["id"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing edge target (IN.id)".into()))?;

        // Strip namespace prefix from source and target IDs
        let source = self.strip_ns_id(raw_source).to_string();
        let target = self.strip_ns_id(raw_target).to_string();

        let mut properties = HashMap::new();

        // All keys except structural ones and "namespace" are edge properties
        if let Some(obj) = edge.as_object() {
            for (key, value) in obj {
                if key != "id" && key != "label" && key != "IN" && key != "OUT" && key != "namespace" {
                    properties.insert(key.clone(), value.clone());
                }
            }
        }

        Ok(GraphEdge {
            source,
            target,
            properties,
        })
    }
}

/// Convert an `aws_smithy_types::Document` to `serde_json::Value`.
fn document_to_json(doc: &aws_smithy_types::Document) -> JsonValue {
    match doc {
        aws_smithy_types::Document::Null => JsonValue::Null,
        aws_smithy_types::Document::Bool(b) => JsonValue::Bool(*b),
        aws_smithy_types::Document::Number(n) => match *n {
            aws_smithy_types::Number::PosInt(i) => JsonValue::Number(serde_json::Number::from(i)),
            aws_smithy_types::Number::NegInt(i) => JsonValue::Number(serde_json::Number::from(i)),
            aws_smithy_types::Number::Float(f) => serde_json::Number::from_f64(f)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
        },
        aws_smithy_types::Document::String(s) => JsonValue::String(s.clone()),
        aws_smithy_types::Document::Array(arr) => {
            JsonValue::Array(arr.iter().map(document_to_json).collect())
        }
        aws_smithy_types::Document::Object(map) => {
            let obj: serde_json::Map<String, JsonValue> = map
                .iter()
                .map(|(k, v)| (k.clone(), document_to_json(v)))
                .collect();
            JsonValue::Object(obj)
        }
    }
}

/// Recursively unwrap GraphSON-typed values to plain JSON.
///
/// The Neptune Data API returns Gremlin results in GraphSON format where values
/// are wrapped in `{"@type": "g:...", "@value": ...}` envelopes. This function
/// strips those wrappers to produce plain JSON values.
///
/// Supported types:
/// - `g:Int32`, `g:Int64` → number
/// - `g:Double`, `g:Float` → number
/// - `g:List` → array (recursively unwrapped)
/// - `g:Map` → object (from flat `[k1, v1, k2, v2, ...]` pairs)
/// - `g:T` → string (traversal tokens: "id", "label")
/// - `g:Direction` → string ("IN", "OUT")
/// - `g:Vertex`, `g:Edge`, `g:VertexProperty`, `g:Path` → recursively unwrapped inner value
fn unwrap_graphson(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(obj) => {
            if let (Some(type_val), Some(val)) = (obj.get("@type"), obj.get("@value")) {
                let type_str = type_val.as_str().unwrap_or("");
                match type_str {
                    // Scalars — return the inner value directly
                    "g:Int32" | "g:Int64" | "g:Float" | "g:Double" => unwrap_graphson(val),
                    // Tokens and enums — return the string value
                    "g:T" | "g:Direction" => unwrap_graphson(val),
                    // List — unwrap each element
                    "g:List" => {
                        if let JsonValue::Array(arr) = val {
                            JsonValue::Array(arr.iter().map(unwrap_graphson).collect())
                        } else {
                            unwrap_graphson(val)
                        }
                    }
                    // Map — flat interleaved [k1, v1, k2, v2, ...] → JSON object
                    "g:Map" => {
                        if let JsonValue::Array(arr) = val {
                            let mut map = serde_json::Map::new();
                            let mut i = 0;
                            while i + 1 < arr.len() {
                                let key = unwrap_graphson(&arr[i]);
                                let value = unwrap_graphson(&arr[i + 1]);
                                let key_str = match &key {
                                    JsonValue::String(s) => s.clone(),
                                    other => other.to_string(),
                                };
                                map.insert(key_str, value);
                                i += 2;
                            }
                            JsonValue::Object(map)
                        } else {
                            unwrap_graphson(val)
                        }
                    }
                    // Complex types — unwrap the inner value recursively
                    "g:Vertex" | "g:Edge" | "g:VertexProperty" | "g:Path" | "g:Property"
                    | "g:Star" => unwrap_graphson(val),
                    // Unknown typed value — unwrap anyway
                    _ => {
                        debug!("Unknown GraphSON type: {}", type_str);
                        unwrap_graphson(val)
                    }
                }
            } else {
                // Plain object (no @type/@value) — recursively unwrap values
                let map: serde_json::Map<String, JsonValue> = obj
                    .iter()
                    .map(|(k, v)| (k.clone(), unwrap_graphson(v)))
                    .collect();
                JsonValue::Object(map)
            }
        }
        JsonValue::Array(arr) => JsonValue::Array(arr.iter().map(unwrap_graphson).collect()),
        // Primitives pass through
        other => other.clone(),
    }
}

#[async_trait]
impl GraphStorage for NeptuneGraphStorage {
    fn namespace(&self) -> &str {
        &self.config.namespace
    }

    async fn initialize(&self) -> edgequake_storage::error::Result<()> {
        info!(
            "Initializing Neptune graph storage: endpoint={}, namespace={}",
            self.config.endpoint, self.config.namespace
        );

        // Test connection with a simple query
        let query = "g.V().limit(1).count()";
        self.execute(query).await.map_err(|e| {
            edgequake_storage::StorageError::Database(format!(
                "Neptune connection test failed: {}",
                e
            ))
        })?;

        info!("Neptune graph storage initialized successfully");
        Ok(())
    }

    async fn finalize(&self) -> edgequake_storage::error::Result<()> {
        info!("Finalizing Neptune graph storage");
        Ok(())
    }

    // ========== Node Operations ==========
    //
    // NOTE: Neptune stores entity IDs as T.id (structural vertex ID), prefixed
    // with the namespace slug for cross-namespace uniqueness. All queries use
    // g.V('{ns}:{id}') for T.id lookup and hasLabel('{ns}:Entity') for
    // namespace filtering instead of the old has('namespace', ...) pattern.

    async fn has_node(&self, node_id: &str) -> edgequake_storage::error::Result<bool> {
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.hasNext()",
            self.escaped_ns_id(node_id),
            self.vertex_label(),
            self.ns_filter()
        );

        let results = self.execute(&query).await?;
        Ok(results.first().and_then(|v| v.as_bool()).unwrap_or(false))
    }

    async fn get_node(&self, node_id: &str) -> edgequake_storage::error::Result<Option<GraphNode>> {
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.elementMap()",
            self.escaped_ns_id(node_id),
            self.vertex_label(),
            self.ns_filter()
        );

        let results = self.execute(&query).await?;

        if let Some(vertex) = results.first() {
            let node = self.vertex_to_node(vertex)?;
            Ok(Some(node))
        } else {
            Ok(None)
        }
    }

    async fn upsert_node(
        &self,
        node_id: &str,
        properties: HashMap<String, JsonValue>,
    ) -> edgequake_storage::error::Result<()> {
        let props = self.properties_to_gremlin(&properties);
        let ns_escaped_id = self.escaped_ns_id(node_id);
        let vlabel = self.vertex_label();
        let ns_filter = self.ns_filter();
        let ns_write = self.ns_write_properties();

        let query = format!(
            "g.V('{}').hasLabel('{}'){}.fold().coalesce(\
                unfold().sideEffect(__.properties().drop()){}{},\
                addV('{}').property(id, '{}'){}{})",
            ns_escaped_id, vlabel, ns_filter, props, ns_write, vlabel, ns_escaped_id, props, ns_write
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn upsert_nodes_batch(
        &self,
        nodes: &[(String, HashMap<String, JsonValue>)],
    ) -> edgequake_storage::error::Result<()> {
        if nodes.is_empty() {
            return Ok(());
        }

        info!("Batch upserting {} nodes", nodes.len());

        let vlabel = self.vertex_label();
        let ns_filter = self.ns_filter();
        let ns_write = self.ns_write_properties();

        for chunk in nodes.chunks(100) {
            for (node_id, properties) in chunk {
                let props = self.properties_to_gremlin(properties);
                let ns_escaped_id = self.escaped_ns_id(node_id);
                let query = format!(
                    "g.V('{}').hasLabel('{}'){}.fold().coalesce(\
                        unfold().sideEffect(__.properties().drop()){}{},\
                        addV('{}').property(id, '{}'){}{})",
                    ns_escaped_id, vlabel, ns_filter, props, ns_write, vlabel, ns_escaped_id, props, ns_write
                );
                self.execute(&query).await?;
            }
        }

        Ok(())
    }

    async fn delete_node(&self, node_id: &str) -> edgequake_storage::error::Result<()> {
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.drop()",
            self.escaped_ns_id(node_id),
            self.vertex_label(),
            self.ns_filter()
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn node_degree(&self, node_id: &str) -> edgequake_storage::error::Result<usize> {
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.bothE().count()",
            self.escaped_ns_id(node_id),
            self.vertex_label(),
            self.ns_filter()
        );

        let results = self.execute(&query).await?;
        let count = results.first().and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        Ok(count)
    }

    async fn node_degrees_batch(
        &self,
        node_ids: &[String],
    ) -> edgequake_storage::error::Result<Vec<(String, usize)>> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for chunk in node_ids.chunks(50) {
            for node_id in chunk {
                let degree = self.node_degree(node_id).await?;
                results.push((node_id.clone(), degree));
            }
        }

        Ok(results)
    }

    async fn get_all_nodes(&self) -> edgequake_storage::error::Result<Vec<GraphNode>> {
        let query = format!(
            "g.V().hasLabel('{}'){}.elementMap()",
            self.vertex_label(),
            self.ns_filter()
        );

        let results = self.execute(&query).await?;
        let mut nodes = Vec::new();

        for vertex in results {
            if let Ok(node) = self.vertex_to_node(&vertex) {
                nodes.push(node);
            }
        }

        Ok(nodes)
    }

    async fn get_nodes_by_ids(
        &self,
        node_ids: &[String],
    ) -> edgequake_storage::error::Result<Vec<GraphNode>> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ids_list = node_ids
            .iter()
            .map(|id| format!("'{}'", self.escaped_ns_id(id)))
            .collect::<Vec<_>>()
            .join(",");

        let query = format!(
            "g.V({}).hasLabel('{}'){}.elementMap()",
            ids_list, self.vertex_label(), self.ns_filter()
        );

        let results = self.execute(&query).await?;
        let mut nodes = Vec::new();

        for vertex in results {
            if let Ok(node) = self.vertex_to_node(&vertex) {
                nodes.push(node);
            }
        }

        Ok(nodes)
    }

    async fn get_nodes_batch(
        &self,
        node_ids: &[String],
    ) -> edgequake_storage::error::Result<HashMap<String, GraphNode>> {
        let nodes = self.get_nodes_by_ids(node_ids).await?;
        Ok(nodes.into_iter().map(|n| (n.id.clone(), n)).collect())
    }

    // ========== Edge Operations ==========

    async fn has_edge(&self, source: &str, target: &str) -> edgequake_storage::error::Result<bool> {
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.outE('{}'){}.where(inV().hasId('{}')).hasNext()",
            self.escaped_ns_id(source),
            self.vertex_label(),
            self.ns_filter(),
            self.edge_label(),
            self.ns_filter(),
            self.escaped_ns_id(target)
        );

        let results = self.execute(&query).await?;
        Ok(results.first().and_then(|v| v.as_bool()).unwrap_or(false))
    }

    async fn get_edge(
        &self,
        source: &str,
        target: &str,
    ) -> edgequake_storage::error::Result<Option<GraphEdge>> {
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.outE('{}'){}.where(inV().hasId('{}')).elementMap()",
            self.escaped_ns_id(source),
            self.vertex_label(),
            self.ns_filter(),
            self.edge_label(),
            self.ns_filter(),
            self.escaped_ns_id(target)
        );

        let results = self.execute(&query).await?;

        if let Some(edge) = results.first() {
            let graph_edge = self.edge_to_graph_edge(edge)?;
            Ok(Some(graph_edge))
        } else {
            Ok(None)
        }
    }

    async fn upsert_edge(
        &self,
        source: &str,
        target: &str,
        properties: HashMap<String, JsonValue>,
    ) -> edgequake_storage::error::Result<()> {
        let props = self.properties_to_gremlin(&properties);
        let ns_source = self.escaped_ns_id(source);
        let ns_target = self.escaped_ns_id(target);
        let vlabel = self.vertex_label();
        let elabel = self.edge_label();
        let ns_filter = self.ns_filter();
        let ns_write = self.ns_write_properties();

        // First ensure both vertices exist
        self.execute(&format!(
            "g.V('{}').hasLabel('{}'){}.fold().coalesce(unfold(), addV('{}').property(id, '{}'){})",
            ns_source, vlabel, ns_filter, vlabel, ns_source, ns_write
        )).await?;

        self.execute(&format!(
            "g.V('{}').hasLabel('{}'){}.fold().coalesce(unfold(), addV('{}').property(id, '{}'){})",
            ns_target, vlabel, ns_filter, vlabel, ns_target, ns_write
        )).await?;

        // Delete existing edge if present
        self.execute(&format!(
            "g.V('{}').hasLabel('{}'){}.outE('{}'){}.where(inV().hasId('{}')).drop()",
            ns_source, vlabel, ns_filter, elabel, ns_filter, ns_target
        ))
        .await?;

        // Add new edge (use __.V() anonymous traversal inside .to() — Neptune
        // requires child traversals to be spawned anonymously, not via g.V())
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.addE('{}').to(__.V('{}').hasLabel('{}'){}){}{}",
            ns_source, vlabel, ns_filter, elabel, ns_target, vlabel, ns_filter, props, ns_write
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn upsert_edges_batch(
        &self,
        edges: &[(String, String, HashMap<String, JsonValue>)],
    ) -> edgequake_storage::error::Result<()> {
        if edges.is_empty() {
            return Ok(());
        }

        info!("Batch upserting {} edges", edges.len());

        for (source, target, properties) in edges {
            self.upsert_edge(source, target, properties.clone()).await?;
        }

        Ok(())
    }

    async fn delete_edge(
        &self,
        source: &str,
        target: &str,
    ) -> edgequake_storage::error::Result<()> {
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.outE('{}'){}.where(inV().hasId('{}')).drop()",
            self.escaped_ns_id(source),
            self.vertex_label(),
            self.ns_filter(),
            self.edge_label(),
            self.ns_filter(),
            self.escaped_ns_id(target)
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn get_node_edges(
        &self,
        node_id: &str,
    ) -> edgequake_storage::error::Result<Vec<GraphEdge>> {
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.bothE(){}.elementMap()",
            self.escaped_ns_id(node_id),
            self.vertex_label(),
            self.ns_filter(),
            self.ns_filter()
        );

        let results = self.execute(&query).await?;
        let mut edges = Vec::new();

        for edge in results {
            if let Ok(graph_edge) = self.edge_to_graph_edge(&edge) {
                edges.push(graph_edge);
            }
        }

        Ok(edges)
    }

    async fn get_all_edges(&self) -> edgequake_storage::error::Result<Vec<GraphEdge>> {
        let query = format!(
            "g.E().hasLabel('{}'){}.elementMap()",
            self.edge_label(),
            self.ns_filter()
        );

        let results = self.execute(&query).await?;
        let mut edges = Vec::new();

        for edge in results {
            if let Ok(graph_edge) = self.edge_to_graph_edge(&edge) {
                edges.push(graph_edge);
            }
        }

        Ok(edges)
    }

    // ========== Graph Queries ==========

    async fn get_knowledge_graph(
        &self,
        start_node: &str,
        max_depth: usize,
        max_nodes: usize,
    ) -> edgequake_storage::error::Result<KnowledgeGraph> {
        // Use emit() to collect vertices at each depth, then get edges separately.
        // hasLabel filter ensures we only traverse within the current namespace.
        let ns_filter = self.ns_filter();
        let node_query = format!(
            "g.V('{}').hasLabel('{}'){}.emit().repeat(both().hasLabel('{}'){}.simplePath()).times({}).dedup().limit({}).elementMap()",
            self.escaped_ns_id(start_node), self.vertex_label(), ns_filter, self.vertex_label(), ns_filter, max_depth, max_nodes
        );

        let node_results = self.execute(&node_query).await?;

        let mut graph = KnowledgeGraph::new();
        let mut node_ids = Vec::new();

        for vertex in &node_results {
            if let Ok(node) = self.vertex_to_node(vertex) {
                node_ids.push(node.id.clone());
                graph.add_node(node);
            }
        }

        // Get edges between the discovered nodes
        if node_ids.len() > 1 {
            let ids_list = node_ids
                .iter()
                .map(|id| format!("'{}'", self.escaped_ns_id(id)))
                .collect::<Vec<_>>()
                .join(",");

            let edge_query = format!(
                "g.V({}).hasLabel('{}'){}.outE('{}'){}.where(inV().hasId(within({}))).elementMap()",
                ids_list, self.vertex_label(), ns_filter, self.edge_label(), ns_filter, ids_list
            );

            let edge_results = self.execute(&edge_query).await?;
            for edge_data in edge_results {
                if let Ok(edge) = self.edge_to_graph_edge(&edge_data) {
                    graph.add_edge(edge);
                }
            }
        }

        graph.is_truncated = graph.node_count() >= max_nodes;

        Ok(graph)
    }

    async fn get_popular_labels(
        &self,
        limit: usize,
    ) -> edgequake_storage::error::Result<Vec<String>> {
        let query = format!(
            "g.V().hasLabel('{}'){}.group().by(T.id).by(bothE().count()).order(local).by(values, desc).limit(local, {})",
            self.vertex_label(), self.ns_filter(), limit
        );

        let results = self.execute(&query).await?;

        let mut labels = Vec::new();
        if let Some(map) = results.first().and_then(|v| v.as_object()) {
            for (key, _) in map {
                // Strip namespace prefix from T.id keys before returning
                labels.push(self.strip_ns_id(key).to_string());
                if labels.len() >= limit {
                    break;
                }
            }
        }

        Ok(labels)
    }

    async fn search_labels(
        &self,
        query_str: &str,
        limit: usize,
    ) -> edgequake_storage::error::Result<Vec<String>> {
        // Use TextP.containing() for server-side filtering.
        // IDs are uppercase after the namespace prefix, so uppercase the query for matching.
        // The T.id contains the namespace prefix, so we search for the prefixed pattern.
        let escaped_upper = gremlin_escape(&query_str.to_uppercase());
        let query = format!(
            "g.V().hasLabel('{}'){}.has(T.id, TextP.containing('{}')).limit({}).id()",
            self.vertex_label(), self.ns_filter(), escaped_upper, limit
        );

        let results = self.execute(&query).await?;

        let mut labels = Vec::new();
        for result in results {
            if let Some(label) = result.as_str() {
                // Strip namespace prefix from returned T.id values
                labels.push(self.strip_ns_id(label).to_string());
            }
        }

        Ok(labels)
    }

    async fn search_nodes(
        &self,
        query_str: &str,
        limit: usize,
        entity_type: Option<&str>,
        _tenant_id: Option<&str>,
        _workspace_id: Option<&str>,
    ) -> edgequake_storage::error::Result<Vec<(GraphNode, usize)>> {
        // Push text filter into Gremlin using TextP.containing() so Neptune
        // filters server-side instead of fetching random vertices.
        // Entity IDs are uppercase (e.g. BRADLEY_EDWARDS), so we match
        // the uppercased query against the ID and the original query against
        // the description (mixed-case text).
        let escaped = gremlin_escape(query_str);
        let escaped_upper = gremlin_escape(&query_str.to_uppercase());

        let mut query = format!("g.V().hasLabel('{}')", self.vertex_label());
        query.push_str(&self.ns_filter());

        if let Some(et) = entity_type {
            query.push_str(&format!(".has('entity_type', '{}')", gremlin_escape(et)));
        }

        // Filter: ID contains uppercased query OR description contains original query
        query.push_str(&format!(
            ".or(__.has(T.id, TextP.containing('{}')), __.has('description', TextP.containing('{}')))",
            escaped_upper, escaped
        ));

        query.push_str(&format!(
            ".limit({}).project('node', 'degree').by(elementMap()).by(bothE().count())",
            limit
        ));

        let results = self.execute(&query).await?;

        let mut nodes = Vec::new();
        for result in results {
            if let (Some(node_data), Some(degree)) = (result.get("node"), result.get("degree")) {
                if let Ok(node) = self.vertex_to_node(node_data) {
                    let deg = degree.as_u64().unwrap_or(0) as usize;
                    nodes.push((node, deg));
                }
            }
        }

        Ok(nodes)
    }

    async fn get_neighbors(
        &self,
        node_id: &str,
        depth: usize,
    ) -> edgequake_storage::error::Result<Vec<GraphNode>> {
        let ns_filter = self.ns_filter();
        let query = format!(
            "g.V('{}').hasLabel('{}'){}.repeat(both().hasLabel('{}'){}.simplePath()).times({}).dedup().elementMap()",
            self.escaped_ns_id(node_id), self.vertex_label(), ns_filter, self.vertex_label(), ns_filter, depth
        );

        let results = self.execute(&query).await?;
        let mut nodes = Vec::new();

        for vertex in results {
            if let Ok(node) = self.vertex_to_node(&vertex) {
                if node.id != node_id {
                    nodes.push(node);
                }
            }
        }

        Ok(nodes)
    }

    async fn get_edges_for_node_set(
        &self,
        node_ids: &[String],
        _tenant_id: Option<&str>,
        _workspace_id: Option<&str>,
    ) -> edgequake_storage::error::Result<Vec<GraphEdge>> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ids_list = node_ids
            .iter()
            .map(|id| format!("'{}'", self.escaped_ns_id(id)))
            .collect::<Vec<_>>()
            .join(",");

        let ns_filter = self.ns_filter();
        let query = format!(
            "g.V({}).hasLabel('{}'){}.outE('{}'){}.where(inV().hasId(within({}))).elementMap()",
            ids_list, self.vertex_label(), ns_filter, self.edge_label(), ns_filter, ids_list
        );

        let results = self.execute(&query).await?;
        let mut edges = Vec::new();

        for edge in results {
            if let Ok(graph_edge) = self.edge_to_graph_edge(&edge) {
                edges.push(graph_edge);
            }
        }

        Ok(edges)
    }

    async fn get_popular_nodes_with_degree(
        &self,
        limit: usize,
        min_degree: Option<usize>,
        entity_type: Option<&str>,
        _tenant_id: Option<&str>,
        _workspace_id: Option<&str>,
    ) -> edgequake_storage::error::Result<Vec<(GraphNode, usize)>> {
        // Server-side Gremlin query that pushes all filters into Neptune,
        // avoiding the default trait impl's N+1 pattern and in-memory filtering.
        let mut query = format!("g.V().hasLabel('{}')", self.vertex_label());
        query.push_str(&self.ns_filter());

        if let Some(et) = entity_type {
            query.push_str(&format!(".has('entity_type', '{}')", gremlin_escape(et)));
        }

        query.push_str(".project('node','degree').by(elementMap()).by(bothE().count())");
        query.push_str(".order().by(select('degree'), desc)");

        if let Some(min) = min_degree {
            query.push_str(&format!(".where(select('degree').is(gte({})))", min));
        }

        query.push_str(&format!(".limit({})", limit));

        let results = self.execute(&query).await?;

        let mut nodes = Vec::new();
        for result in results {
            if let (Some(node_data), Some(degree)) = (result.get("node"), result.get("degree")) {
                if let Ok(node) = self.vertex_to_node(node_data) {
                    let deg = degree.as_u64().unwrap_or(0) as usize;
                    nodes.push((node, deg));
                }
            }
        }

        Ok(nodes)
    }

    // ========== Utility Operations ==========

    async fn node_count(&self) -> edgequake_storage::error::Result<usize> {
        let query = format!(
            "g.V().hasLabel('{}'){}.count()",
            self.vertex_label(),
            self.ns_filter()
        );

        let results = self.execute(&query).await?;
        let count = results.first().and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        Ok(count)
    }

    async fn edge_count(&self) -> edgequake_storage::error::Result<usize> {
        let query = format!(
            "g.E().hasLabel('{}'){}.count()",
            self.edge_label(),
            self.ns_filter()
        );

        let results = self.execute(&query).await?;
        let count = results.first().and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        Ok(count)
    }

    async fn clear(&self) -> edgequake_storage::error::Result<()> {
        warn!(
            "Clearing all nodes and edges for namespace: {}",
            self.config.namespace
        );

        let query = format!("g.V().hasLabel('{}'){}.drop()", self.vertex_label(), self.ns_filter());

        self.execute(&query).await?;
        Ok(())
    }

    async fn clear_workspace(
        &self,
        _workspace_id: &uuid::Uuid,
    ) -> edgequake_storage::error::Result<(usize, usize)> {
        let node_count = self.node_count().await?;
        let edge_count = self.edge_count().await?;

        self.clear().await?;

        Ok((node_count, edge_count))
    }
}
