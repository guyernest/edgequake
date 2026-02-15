//! Amazon Neptune graph storage using the Neptune Data API.
//!
//! Uses `aws-sdk-neptunedata` to execute Gremlin queries over HTTP with
//! automatic IAM SigV4 authentication. This replaces the previous WebSocket-based
//! `gremlin-client` approach, which didn't support IAM auth.
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

/// Configuration for Neptune graph storage.
#[derive(Debug, Clone)]
pub struct NeptuneConfig {
    /// Neptune cluster endpoint (host:port)
    pub endpoint: String,
    /// Storage namespace (workspace ID)
    pub namespace: String,
    /// Use IAM authentication (always true with Neptune Data API)
    pub use_iam_auth: bool,
    /// Connection pool size (unused with HTTP API, kept for config compat)
    pub pool_size: usize,
    /// Query timeout in seconds
    pub timeout_secs: u64,
    /// Enable SSL/TLS (always true with Neptune Data API)
    pub use_ssl: bool,
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
        }
    }

    /// Set the namespace (workspace ID).
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
}

/// Amazon Neptune graph storage using the Neptune Data API.
///
/// Executes Gremlin queries over HTTP with IAM SigV4 authentication,
/// providing a fully managed, serverless graph database for storing
/// entities and relationships in the knowledge graph.
pub struct NeptuneGraphStorage {
    config: NeptuneConfig,
    client: Client,
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

        Ok(Self { config, client })
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
    fn properties_to_gremlin(&self, properties: &HashMap<String, JsonValue>) -> String {
        let mut parts = vec![format!(
            ".property('namespace', '{}')",
            self.config.namespace
        )];

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
    /// `{"id": "SEC", "label": "Entity", "entity_type": "ORGANIZATION", ...}`
    fn vertex_to_node(&self, vertex: &JsonValue) -> crate::error::Result<GraphNode> {
        let id = vertex["id"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing vertex id".into()))?
            .to_string();

        let mut properties = HashMap::new();

        // elementMap() returns a flat object — all keys except "id" and "label"
        // are vertex properties
        if let Some(obj) = vertex.as_object() {
            for (key, value) in obj {
                if key != "id" && key != "label" {
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
    ///   "id": "COMPANY_A:AGENT",
    ///   "label": "relates_to",
    ///   "OUT": {"id": "COMPANY_A", "label": "Entity"},
    ///   "IN": {"id": "AGENT", "label": "Entity"},
    ///   "description": "...",
    ///   "weight": 0.5,
    ///   "namespace": "epstein"
    /// }
    /// ```
    fn edge_to_graph_edge(&self, edge: &JsonValue) -> crate::error::Result<GraphEdge> {
        // Source vertex is under "OUT" key (outgoing direction)
        let source = edge["OUT"]["id"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing edge source (OUT.id)".into()))?
            .to_string();

        // Target vertex is under "IN" key (incoming direction)
        let target = edge["IN"]["id"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing edge target (IN.id)".into()))?
            .to_string();

        let mut properties = HashMap::new();

        // All keys except structural ones are edge properties
        if let Some(obj) = edge.as_object() {
            for (key, value) in obj {
                if key != "id" && key != "label" && key != "IN" && key != "OUT" {
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
    // NOTE: Neptune stores entity IDs as T.id (structural vertex ID), not as a
    // regular property named "id". All queries use g.V('id') for T.id lookup
    // instead of g.V().has('id', 'value').

    async fn has_node(&self, node_id: &str) -> edgequake_storage::error::Result<bool> {
        let query = format!(
            "g.V('{}').has('namespace', '{}').hasNext()",
            gremlin_escape(node_id),
            self.config.namespace
        );

        let results = self.execute(&query).await?;
        Ok(results.first().and_then(|v| v.as_bool()).unwrap_or(false))
    }

    async fn get_node(&self, node_id: &str) -> edgequake_storage::error::Result<Option<GraphNode>> {
        let query = format!(
            "g.V('{}').has('namespace', '{}').elementMap()",
            gremlin_escape(node_id),
            self.config.namespace
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

        let escaped_id = gremlin_escape(node_id);
        let query = format!(
            "g.V('{}').has('namespace', '{}').fold().coalesce(\
                unfold().sideEffect(__.properties().drop()){},\
                addV('Entity').property(id, '{}'){})",
            escaped_id, self.config.namespace, props, escaped_id, props
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

        for chunk in nodes.chunks(100) {
            for (node_id, properties) in chunk {
                let props = self.properties_to_gremlin(properties);
                let escaped_id = gremlin_escape(node_id);
                let query = format!(
                    "g.V('{}').has('namespace', '{}').fold().coalesce(\
                        unfold().sideEffect(__.properties().drop()){},\
                        addV('Entity').property(id, '{}'){})",
                    escaped_id, self.config.namespace, props, escaped_id, props
                );
                self.execute(&query).await?;
            }
        }

        Ok(())
    }

    async fn delete_node(&self, node_id: &str) -> edgequake_storage::error::Result<()> {
        let query = format!(
            "g.V('{}').has('namespace', '{}').drop()",
            gremlin_escape(node_id),
            self.config.namespace
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn node_degree(&self, node_id: &str) -> edgequake_storage::error::Result<usize> {
        let query = format!(
            "g.V('{}').has('namespace', '{}').bothE().count()",
            gremlin_escape(node_id),
            self.config.namespace
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
            "g.V().has('namespace', '{}').elementMap()",
            self.config.namespace
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
            .map(|id| format!("'{}'", gremlin_escape(id)))
            .collect::<Vec<_>>()
            .join(",");

        let query = format!(
            "g.V({}).has('namespace', '{}').elementMap()",
            ids_list, self.config.namespace
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
            "g.V('{}').has('namespace', '{}').outE().where(inV().hasId('{}')).hasNext()",
            gremlin_escape(source),
            self.config.namespace,
            gremlin_escape(target)
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
            "g.V('{}').has('namespace', '{}').outE().where(inV().hasId('{}')).elementMap()",
            gremlin_escape(source),
            self.config.namespace,
            gremlin_escape(target)
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
        let escaped_source = gremlin_escape(source);
        let escaped_target = gremlin_escape(target);

        // First ensure both vertices exist
        self.execute(&format!(
            "g.V('{}').has('namespace', '{}').fold().coalesce(unfold(), addV('Entity').property(id, '{}').property('namespace', '{}'))",
            escaped_source, self.config.namespace, escaped_source, self.config.namespace
        )).await?;

        self.execute(&format!(
            "g.V('{}').has('namespace', '{}').fold().coalesce(unfold(), addV('Entity').property(id, '{}').property('namespace', '{}'))",
            escaped_target, self.config.namespace, escaped_target, self.config.namespace
        )).await?;

        // Delete existing edge if present
        self.execute(&format!(
            "g.V('{}').has('namespace', '{}').outE('relates_to').where(inV().hasId('{}')).drop()",
            escaped_source, self.config.namespace, escaped_target
        ))
        .await?;

        // Add new edge (use __.V() anonymous traversal inside .to() — Neptune
        // requires child traversals to be spawned anonymously, not via g.V())
        let query = format!(
            "g.V('{}').has('namespace', '{}').addE('relates_to').to(__.V('{}').has('namespace', '{}')){}",
            escaped_source, self.config.namespace, escaped_target, self.config.namespace, props
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
            "g.V('{}').has('namespace', '{}').outE().where(inV().hasId('{}')).drop()",
            gremlin_escape(source),
            self.config.namespace,
            gremlin_escape(target)
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn get_node_edges(
        &self,
        node_id: &str,
    ) -> edgequake_storage::error::Result<Vec<GraphEdge>> {
        let query = format!(
            "g.V('{}').has('namespace', '{}').bothE().elementMap()",
            gremlin_escape(node_id),
            self.config.namespace
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
            "g.E().where(outV().has('namespace', '{}')).elementMap()",
            self.config.namespace
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
        // Use emit() to collect vertices at each depth, then get edges separately
        let node_query = format!(
            "g.V('{}').has('namespace', '{}').emit().repeat(both().simplePath()).times({}).dedup().limit({}).elementMap()",
            gremlin_escape(start_node), self.config.namespace, max_depth, max_nodes
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
                .map(|id| format!("'{}'", gremlin_escape(id)))
                .collect::<Vec<_>>()
                .join(",");

            let edge_query = format!(
                "g.V({}).has('namespace', '{}').outE().where(inV().hasId(within({}))).elementMap()",
                ids_list, self.config.namespace, ids_list
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
            "g.V().has('namespace', '{}').group().by(T.id).by(bothE().count()).order(local).by(values, desc).limit(local, {})",
            self.config.namespace, limit
        );

        let results = self.execute(&query).await?;

        let mut labels = Vec::new();
        if let Some(map) = results.first().and_then(|v| v.as_object()) {
            for (key, _) in map {
                labels.push(key.clone());
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
        // IDs are uppercase, so uppercase the query for matching.
        let escaped_upper = gremlin_escape(&query_str.to_uppercase());
        let query = format!(
            "g.V().has('namespace', '{}').has(T.id, TextP.containing('{}')).limit({}).id()",
            self.config.namespace, escaped_upper, limit
        );

        let results = self.execute(&query).await?;

        let mut labels = Vec::new();
        for result in results {
            if let Some(label) = result.as_str() {
                labels.push(label.to_string());
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

        let mut query = format!("g.V().has('namespace', '{}')", self.config.namespace);

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
        let query = format!(
            "g.V('{}').has('namespace', '{}').repeat(both().simplePath()).times({}).dedup().elementMap()",
            gremlin_escape(node_id), self.config.namespace, depth
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
            .map(|id| format!("'{}'", gremlin_escape(id)))
            .collect::<Vec<_>>()
            .join(",");

        let query = format!(
            "g.V({}).has('namespace', '{}').outE().where(inV().hasId(within({}))).elementMap()",
            ids_list, self.config.namespace, ids_list
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
        let mut query = format!("g.V().has('namespace', '{}')", self.config.namespace);

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
            "g.V().has('namespace', '{}').count()",
            self.config.namespace
        );

        let results = self.execute(&query).await?;
        let count = results.first().and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        Ok(count)
    }

    async fn edge_count(&self) -> edgequake_storage::error::Result<usize> {
        let query = format!(
            "g.E().where(outV().has('namespace', '{}')).count()",
            self.config.namespace
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

        let query = format!("g.V().has('namespace', '{}').drop()", self.config.namespace);

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
