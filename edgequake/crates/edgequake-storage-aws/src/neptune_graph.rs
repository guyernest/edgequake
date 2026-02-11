//! Amazon Neptune graph storage using Gremlin.
//!
//! This module provides graph storage using Amazon Neptune, a fully managed
//! graph database service that supports both Gremlin and SPARQL queries.
//!
//! ## Architecture
//!
//! ```text
//! Application → Gremlin Client → Neptune Cluster
//!                                    ├── Primary Instance
//!                                    └── Read Replicas (optional)
//! ```
//!
//! ## Features
//!
//! - **Serverless**: Auto-scales from 2.5 to 128 NCUs (Neptune Capacity Units)
//! - **Cost-effective**: Pay per query ($0.16/1M reads, $0.20/1M writes)
//! - **No idle costs**: Scales to zero when not in use
//! - **ACID**: Full transactional support
//! - **Highly available**: Multi-AZ deployment
//! - **Backup**: Continuous backup to S3
//!
//! ## Gremlin Query Language
//!
//! Neptune uses Apache TinkerPop Gremlin for graph traversal:
//!
//! ```gremlin
//! // Add a vertex
//! g.addV('Entity').property('id', 'doc1').property('type', 'document')
//!
//! // Add an edge
//! g.V().has('id', 'doc1').addE('relates_to').to(g.V().has('id', 'doc2'))
//!
//! // Traverse graph
//! g.V().has('id', 'doc1').out().out().limit(10)
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use edgequake_storage_aws::{NeptuneConfig, NeptuneGraphStorage};
//! use edgequake_storage::GraphStorage;
//!
//! let config = NeptuneConfig::new("my-cluster.cluster-xxx.us-east-1.neptune.amazonaws.com:8182");
//! let storage = NeptuneGraphStorage::new(config).await?;
//!
//! storage.initialize().await?;
//!
//! // Add node
//! let mut properties = HashMap::new();
//! properties.insert("type".to_string(), json!("document"));
//! storage.upsert_node("doc1", properties).await?;
//!
//! // Query graph
//! let graph = storage.get_knowledge_graph("doc1", 2, 100).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use gremlin_client::{GremlinClient, GremlinError, GraphSON, Vertex, Edge as GremlinEdge};
use serde_json::Value as JsonValue;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use edgequake_storage::{
    GraphEdge, GraphNode, GraphStorage, KnowledgeGraph, StorageError,
};

use crate::error::{AwsStorageError, Result};

/// Configuration for Neptune graph storage.
#[derive(Debug, Clone)]
pub struct NeptuneConfig {
    /// Neptune cluster endpoint (host:port)
    pub endpoint: String,
    /// Storage namespace (workspace ID)
    pub namespace: String,
    /// Use IAM authentication (recommended for production)
    pub use_iam_auth: bool,
    /// Connection pool size
    pub pool_size: usize,
    /// Query timeout in seconds
    pub timeout_secs: u64,
    /// Enable SSL/TLS
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

/// Amazon Neptune graph storage using Gremlin.
///
/// Provides a fully managed, serverless graph database for storing
/// entities and relationships in the knowledge graph.
pub struct NeptuneGraphStorage {
    config: NeptuneConfig,
    client: Arc<RwLock<Option<GremlinClient>>>,
}

impl NeptuneGraphStorage {
    /// Create a new Neptune graph storage.
    ///
    /// # Arguments
    ///
    /// * `config` - Neptune configuration
    pub async fn new(config: NeptuneConfig) -> Result<Self> {
        Ok(Self {
            config,
            client: Arc::new(RwLock::new(None)),
        })
    }

    /// Get or create the Gremlin client.
    async fn get_client(&self) -> Result<GremlinClient> {
        let mut client_guard = self.client.write().await;

        if let Some(client) = client_guard.as_ref() {
            return Ok(client.clone());
        }

        // Create new client
        info!("Connecting to Neptune at {}", self.config.endpoint);

        let connection_string = if self.config.use_ssl {
            format!("wss://{}/gremlin", self.config.endpoint)
        } else {
            format!("ws://{}/gremlin", self.config.endpoint)
        };

        let client = GremlinClient::connect(connection_string)
            .await
            .map_err(|e| AwsStorageError::NeptuneError(format!("Failed to connect: {}", e)))?;

        *client_guard = Some(client.clone());
        Ok(client)
    }

    /// Execute a Gremlin query.
    async fn execute(&self, query: &str) -> Result<Vec<JsonValue>> {
        let client = self.get_client().await?;

        debug!("Executing Gremlin query: {}", query);

        let results = client
            .execute(query, &[])
            .await
            .map_err(|e| AwsStorageError::NeptuneError(format!("Query failed: {}", e)))?;

        let values: Vec<JsonValue> = results
            .map(|r| r.take::<JsonValue>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AwsStorageError::NeptuneError(format!("Failed to parse results: {}", e)))?;

        Ok(values)
    }

    /// Convert properties HashMap to Gremlin property chain.
    fn properties_to_gremlin(&self, properties: &HashMap<String, JsonValue>) -> String {
        let mut parts = vec![
            format!(".property('namespace', '{}')", self.config.namespace)
        ];

        for (key, value) in properties {
            let value_str = match value {
                JsonValue::String(s) => format!("'{}'", s.replace('\'', "\\'")),
                JsonValue::Number(n) => n.to_string(),
                JsonValue::Bool(b) => b.to_string(),
                JsonValue::Null => "null".to_string(),
                _ => format!("'{}'", serde_json::to_string(value).unwrap_or_default().replace('\'', "\\'")),
            };
            parts.push(format!(".property('{}', {})", key, value_str));
        }

        parts.join("")
    }

    /// Parse vertex to GraphNode.
    fn vertex_to_node(&self, vertex: &JsonValue) -> Result<GraphNode> {
        let id = vertex["id"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing vertex id".into()))?
            .to_string();

        let mut properties = HashMap::new();

        if let Some(props) = vertex["properties"].as_object() {
            for (key, value_array) in props {
                // Neptune returns properties as arrays
                if let Some(arr) = value_array.as_array() {
                    if let Some(first) = arr.first() {
                        if let Some(val) = first.get("value") {
                            properties.insert(key.clone(), val.clone());
                        }
                    }
                }
            }
        }

        Ok(GraphNode {
            id,
            properties,
        })
    }

    /// Parse edge to GraphEdge.
    fn edge_to_graph_edge(&self, edge: &JsonValue) -> Result<GraphEdge> {
        let source = edge["outV"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing edge source".into()))?
            .to_string();

        let target = edge["inV"]
            .as_str()
            .ok_or_else(|| AwsStorageError::NeptuneError("Missing edge target".into()))?
            .to_string();

        let mut properties = HashMap::new();

        if let Some(props) = edge["properties"].as_object() {
            for (key, value) in props {
                properties.insert(key.clone(), value.clone());
            }
        }

        Ok(GraphEdge {
            source,
            target,
            properties,
        })
    }
}

#[async_trait]
impl GraphStorage for NeptuneGraphStorage {
    fn namespace(&self) -> &str {
        &self.config.namespace
    }

    async fn initialize(&self) -> Result<(), StorageError> {
        info!("Initializing Neptune graph storage: endpoint={}, namespace={}",
              self.config.endpoint, self.config.namespace);

        // Test connection
        let client = self.get_client().await?;

        // Verify connection with a simple query
        let query = "g.V().limit(1).count()";
        client
            .execute(query, &[])
            .await
            .map_err(|e| AwsStorageError::NeptuneError(format!("Connection test failed: {}", e)))?;

        info!("Neptune graph storage initialized successfully");
        Ok(())
    }

    async fn finalize(&self) -> Result<(), StorageError> {
        info!("Finalizing Neptune graph storage");
        // Neptune doesn't require explicit finalization
        Ok(())
    }

    // ========== Node Operations ==========

    async fn has_node(&self, node_id: &str) -> Result<bool, StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').hasNext()",
            node_id, self.config.namespace
        );

        let results = self.execute(&query).await?;
        Ok(results.first().and_then(|v| v.as_bool()).unwrap_or(false))
    }

    async fn get_node(&self, node_id: &str) -> Result<Option<GraphNode>, StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').elementMap()",
            node_id, self.config.namespace
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
    ) -> Result<(), StorageError> {
        let props = self.properties_to_gremlin(&properties);

        // Upsert: try to update existing, or create new
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').fold().coalesce(\
                unfold().sideEffect(__.properties().drop()){},\
                addV('Entity').property('id', '{}'){})",
            node_id, self.config.namespace, props, node_id, props
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn upsert_nodes_batch(
        &self,
        nodes: &[(String, HashMap<String, JsonValue>)],
    ) -> Result<(), StorageError> {
        if nodes.is_empty() {
            return Ok(());
        }

        info!("Batch upserting {} nodes", nodes.len());

        // Neptune doesn't have a native batch upsert, so we batch queries
        for chunk in nodes.chunks(100) {
            let mut queries = Vec::new();

            for (node_id, properties) in chunk {
                let props = self.properties_to_gremlin(properties);
                let query = format!(
                    "g.V().has('id', '{}').has('namespace', '{}').fold().coalesce(\
                        unfold().sideEffect(__.properties().drop()){},\
                        addV('Entity').property('id', '{}'){})",
                    node_id, self.config.namespace, props, node_id, props
                );
                queries.push(query);
            }

            // Execute queries sequentially (Neptune doesn't support true batching)
            for query in queries {
                self.execute(&query).await?;
            }
        }

        Ok(())
    }

    async fn delete_node(&self, node_id: &str) -> Result<(), StorageError> {
        // Delete node and all connected edges
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').drop()",
            node_id, self.config.namespace
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn node_degree(&self, node_id: &str) -> Result<usize, StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').bothE().count()",
            node_id, self.config.namespace
        );

        let results = self.execute(&query).await?;
        let count = results
            .first()
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        Ok(count)
    }

    async fn node_degrees_batch(&self, node_ids: &[String]) -> Result<Vec<(String, usize)>, StorageError> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        // Process in batches
        for chunk in node_ids.chunks(50) {
            for node_id in chunk {
                let degree = self.node_degree(node_id).await?;
                results.push((node_id.clone(), degree));
            }
        }

        Ok(results)
    }

    async fn get_all_nodes(&self) -> Result<Vec<GraphNode>, StorageError> {
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

    async fn get_nodes_by_ids(&self, node_ids: &[String]) -> Result<Vec<GraphNode>, StorageError> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ids_list = node_ids
            .iter()
            .map(|id| format!("'{}'", id))
            .collect::<Vec<_>>()
            .join(",");

        let query = format!(
            "g.V().has('namespace', '{}').has('id', within({}))).elementMap()",
            self.config.namespace, ids_list
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

    async fn get_nodes_batch(&self, node_ids: &[String]) -> Result<HashMap<String, GraphNode>, StorageError> {
        let nodes = self.get_nodes_by_ids(node_ids).await?;
        Ok(nodes.into_iter().map(|n| (n.id.clone(), n)).collect())
    }

    // ========== Edge Operations ==========

    async fn has_edge(&self, source: &str, target: &str) -> Result<bool, StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').outE().where(inV().has('id', '{}')).hasNext()",
            source, self.config.namespace, target
        );

        let results = self.execute(&query).await?;
        Ok(results.first().and_then(|v| v.as_bool()).unwrap_or(false))
    }

    async fn get_edge(&self, source: &str, target: &str) -> Result<Option<GraphEdge>, StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').outE().where(inV().has('id', '{}')).elementMap()",
            source, self.config.namespace, target
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
    ) -> Result<(), StorageError> {
        let props = self.properties_to_gremlin(&properties);

        // First ensure both vertices exist
        self.execute(&format!(
            "g.V().has('id', '{}').has('namespace', '{}').fold().coalesce(unfold(), addV('Entity').property('id', '{}').property('namespace', '{}'))",
            source, self.config.namespace, source, self.config.namespace
        )).await?;

        self.execute(&format!(
            "g.V().has('id', '{}').has('namespace', '{}').fold().coalesce(unfold(), addV('Entity').property('id', '{}').property('namespace', '{}'))",
            target, self.config.namespace, target, self.config.namespace
        )).await?;

        // Delete existing edge if present
        self.execute(&format!(
            "g.V().has('id', '{}').has('namespace', '{}').outE('relates_to').where(inV().has('id', '{}')).drop()",
            source, self.config.namespace, target
        )).await?;

        // Add new edge
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').addE('relates_to').to(g.V().has('id', '{}').has('namespace', '{}')){}",
            source, self.config.namespace, target, self.config.namespace, props
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn upsert_edges_batch(
        &self,
        edges: &[(String, String, HashMap<String, JsonValue>)],
    ) -> Result<(), StorageError> {
        if edges.is_empty() {
            return Ok(());
        }

        info!("Batch upserting {} edges", edges.len());

        for (source, target, properties) in edges {
            self.upsert_edge(source, target, properties.clone()).await?;
        }

        Ok(())
    }

    async fn delete_edge(&self, source: &str, target: &str) -> Result<(), StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').outE().where(inV().has('id', '{}')).drop()",
            source, self.config.namespace, target
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn get_node_edges(&self, node_id: &str) -> Result<Vec<GraphEdge>, StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').bothE().elementMap()",
            node_id, self.config.namespace
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

    async fn get_all_edges(&self) -> Result<Vec<GraphEdge>, StorageError> {
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
    ) -> Result<KnowledgeGraph, StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').repeat(both().simplePath()).times({}).limit({}).path().by(elementMap())",
            start_node, self.config.namespace, max_depth, max_nodes
        );

        let results = self.execute(&query).await?;

        let mut graph = KnowledgeGraph::new();
        let mut seen_nodes = std::collections::HashSet::new();
        let mut seen_edges = std::collections::HashSet::new();

        for path in results {
            if let Some(objects) = path["objects"].as_array() {
                for obj in objects {
                    if obj["type"].as_str() == Some("vertex") {
                        if let Ok(node) = self.vertex_to_node(obj) {
                            if seen_nodes.insert(node.id.clone()) {
                                graph.add_node(node);
                            }
                        }
                    } else if obj["type"].as_str() == Some("edge") {
                        if let Ok(edge) = self.edge_to_graph_edge(obj) {
                            let edge_key = format!("{}:{}", edge.source, edge.target);
                            if seen_edges.insert(edge_key) {
                                graph.add_edge(edge);
                            }
                        }
                    }
                }
            }
        }

        graph.is_truncated = graph.node_count() >= max_nodes;

        Ok(graph)
    }

    async fn get_popular_labels(&self, limit: usize) -> Result<Vec<String>, StorageError> {
        let query = format!(
            "g.V().has('namespace', '{}').group().by('id').by(bothE().count()).order(local).by(values, desc).limit({})",
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

    async fn search_labels(&self, query_str: &str, limit: usize) -> Result<Vec<String>, StorageError> {
        // Gremlin doesn't have a built-in "starts with" for properties
        // We'll get all and filter in Rust
        let query = format!(
            "g.V().has('namespace', '{}').values('id').limit({})",
            self.config.namespace, limit * 10
        );

        let results = self.execute(&query).await?;

        let mut labels = Vec::new();
        for result in results {
            if let Some(label) = result.as_str() {
                if label.starts_with(query_str) {
                    labels.push(label.to_string());
                    if labels.len() >= limit {
                        break;
                    }
                }
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
    ) -> Result<Vec<(GraphNode, usize)>, StorageError> {
        let mut query = format!(
            "g.V().has('namespace', '{}')",
            self.config.namespace
        );

        if let Some(et) = entity_type {
            query.push_str(&format!(".has('entity_type', '{}')", et));
        }

        query.push_str(&format!(".limit({}).project('node', 'degree').by(elementMap()).by(bothE().count())", limit));

        let results = self.execute(&query).await?;

        let mut nodes = Vec::new();
        for result in results {
            if let (Some(node_data), Some(degree)) = (result.get("node"), result.get("degree")) {
                if let Ok(node) = self.vertex_to_node(node_data) {
                    // Filter by query string
                    let matches = node.id.contains(query_str)
                        || node
                            .properties
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.contains(query_str))
                            .unwrap_or(false);

                    if matches {
                        let deg = degree.as_u64().unwrap_or(0) as usize;
                        nodes.push((node, deg));
                    }
                }
            }
        }

        Ok(nodes)
    }

    async fn get_neighbors(&self, node_id: &str, depth: usize) -> Result<Vec<GraphNode>, StorageError> {
        let query = format!(
            "g.V().has('id', '{}').has('namespace', '{}').repeat(both().simplePath()).times({}).dedup().elementMap()",
            node_id, self.config.namespace, depth
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
    ) -> Result<Vec<GraphEdge>, StorageError> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ids_list = node_ids
            .iter()
            .map(|id| format!("'{}'", id))
            .collect::<Vec<_>>()
            .join(",");

        let query = format!(
            "g.V().has('namespace', '{}').has('id', within({})).outE().where(inV().has('id', within({}))).elementMap()",
            self.config.namespace, ids_list, ids_list
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

    // ========== Utility Operations ==========

    async fn node_count(&self) -> Result<usize, StorageError> {
        let query = format!(
            "g.V().has('namespace', '{}').count()",
            self.config.namespace
        );

        let results = self.execute(&query).await?;
        let count = results
            .first()
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        Ok(count)
    }

    async fn edge_count(&self) -> Result<usize, StorageError> {
        let query = format!(
            "g.E().where(outV().has('namespace', '{}')).count()",
            self.config.namespace
        );

        let results = self.execute(&query).await?;
        let count = results
            .first()
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        Ok(count)
    }

    async fn clear(&self) -> Result<(), StorageError> {
        warn!("Clearing all nodes and edges for namespace: {}", self.config.namespace);

        let query = format!(
            "g.V().has('namespace', '{}').drop()",
            self.config.namespace
        );

        self.execute(&query).await?;
        Ok(())
    }

    async fn clear_workspace(&self, workspace_id: &uuid::Uuid) -> Result<(usize, usize), StorageError> {
        let node_count = self.node_count().await?;
        let edge_count = self.edge_count().await?;

        self.clear().await?;

        Ok((node_count, edge_count))
    }
}
