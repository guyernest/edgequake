## 4.3 The GraphStorage Trait

The `GraphStorage` trait is EdgeQuake's abstraction over any graph
database backend. It defines the complete interface for storing,
querying, and traversing the knowledge graph. Understanding this trait
is essential because every EdgeQuake component -- the ingestion
pipeline, the query engine, the API layer -- interacts with the graph
through this single interface.

### Trait Definition Overview

The trait lives in `edgequake-storage::traits::graph` and requires
`Send + Sync` for safe sharing across async tasks:

```rust
use async_trait::async_trait;
use std::collections::HashMap;

#[async_trait]
pub trait GraphStorage: Send + Sync {
    // Lifecycle
    fn namespace(&self) -> &str;
    async fn initialize(&self) -> Result<()>;
    async fn finalize(&self) -> Result<()>;

    // Node operations
    async fn has_node(&self, node_id: &str) -> Result<bool>;
    async fn get_node(&self, node_id: &str) -> Result<Option<GraphNode>>;
    async fn upsert_node(&self, node_id: &str, properties: HashMap<String, Value>) -> Result<()>;
    async fn upsert_nodes_batch(&self, nodes: &[(String, HashMap<String, Value>)]) -> Result<()>;
    async fn delete_node(&self, node_id: &str) -> Result<()>;
    async fn node_degree(&self, node_id: &str) -> Result<usize>;
    async fn get_all_nodes(&self) -> Result<Vec<GraphNode>>;
    async fn get_nodes_by_ids(&self, node_ids: &[String]) -> Result<Vec<GraphNode>>;

    // Edge operations
    async fn has_edge(&self, source: &str, target: &str) -> Result<bool>;
    async fn get_edge(&self, source: &str, target: &str) -> Result<Option<GraphEdge>>;
    async fn upsert_edge(&self, source: &str, target: &str, properties: HashMap<String, Value>) -> Result<()>;
    async fn upsert_edges_batch(&self, edges: &[(String, String, HashMap<String, Value>)]) -> Result<()>;
    async fn delete_edge(&self, source: &str, target: &str) -> Result<()>;
    async fn get_node_edges(&self, node_id: &str) -> Result<Vec<GraphEdge>>;

    // Graph queries
    async fn get_knowledge_graph(&self, start_node: &str, max_depth: usize, max_nodes: usize) -> Result<KnowledgeGraph>;
    async fn get_neighbors(&self, node_id: &str, depth: usize) -> Result<Vec<GraphNode>>;
    async fn search_nodes(&self, query: &str, limit: usize, entity_type: Option<&str>, tenant_id: Option<&str>, workspace_id: Option<&str>) -> Result<Vec<(GraphNode, usize)>>;

    // Analytics
    async fn node_count(&self) -> Result<usize>;
    async fn edge_count(&self) -> Result<usize>;
    async fn clear(&self) -> Result<()>;
}
```

This is a simplified view. The full trait has over 25 methods. Let us
walk through each group.

### Lifecycle Methods

```rust
/// Get the storage namespace (for tenant isolation).
fn namespace(&self) -> &str;

/// Initialize the graph storage (create tables, indexes).
async fn initialize(&self) -> Result<()>;

/// Flush pending changes (commit batch operations).
async fn finalize(&self) -> Result<()>;
```

**Namespace** is the foundation of multi-tenancy. Every `GraphStorage`
instance is scoped to a namespace (typically a tenant ID or workspace
ID). This means two tenants can have a node named `JOHN_SMITH` without
collision -- they live in separate namespaces.

```rust
// Different namespaces, completely isolated data
let tenant_a_graph = MemoryGraphStorage::new("tenant-a");
let tenant_b_graph = MemoryGraphStorage::new("tenant-b");

// These are different nodes in different namespaces
tenant_a_graph.upsert_node("JOHN_SMITH", props_a.clone()).await?;
tenant_b_graph.upsert_node("JOHN_SMITH", props_b.clone()).await?;
```

### Node Operations

The core CRUD operations for graph nodes:

```rust
/// Check if a node exists.
async fn has_node(&self, node_id: &str) -> Result<bool>;

/// Get a node by ID. Returns None if not found.
async fn get_node(&self, node_id: &str) -> Result<Option<GraphNode>>;

/// Insert or update a node. If the node exists, properties are merged.
async fn upsert_node(
    &self,
    node_id: &str,
    properties: HashMap<String, serde_json::Value>,
) -> Result<()>;

/// Delete a node AND its connected edges.
async fn delete_node(&self, node_id: &str) -> Result<()>;
```

The most important method here is `upsert_node`. It is an **upsert**
(insert or update), not a plain insert. This is critical for the
ingestion pipeline: when the same entity appears in multiple documents,
the pipeline calls `upsert_node` to merge the new properties with the
existing ones.

```rust
use serde_json::json;

// First document mentions Sarah Chen
graph.upsert_node("SARAH_CHEN", HashMap::from([
    ("entity_type".into(), json!("PERSON")),
    ("description".into(), json!("A researcher at MIT")),
    ("source_id".into(), json!("doc-001")),
])).await?;

// Second document adds more context
graph.upsert_node("SARAH_CHEN", HashMap::from([
    ("description".into(), json!("Lead quantum computing researcher at MIT, PhD 2019")),
    ("source_id".into(), json!("doc-001|doc-047")),
    ("importance".into(), json!(0.9)),
])).await?;

// Node now has the merged/updated properties
let node = graph.get_node("SARAH_CHEN").await?.unwrap();
```

#### Batch Operations

For ingestion performance, batch variants are provided with default
sequential implementations that backends can override:

```rust
/// Insert or update multiple nodes in batch.
/// Default: calls upsert_node sequentially.
/// PostgreSQL override: single INSERT ... ON CONFLICT with UNNEST.
async fn upsert_nodes_batch(
    &self,
    nodes: &[(String, HashMap<String, serde_json::Value>)],
) -> Result<()> {
    for (node_id, properties) in nodes {
        self.upsert_node(node_id, properties.clone()).await?;
    }
    Ok(())
}
```

The default implementation is correct but slow (N database round trips).
The PostgreSQL implementation uses `UNNEST` with arrays for a single
round trip -- a pattern inspired directly by LightRAG.

### Edge Operations

Edges follow the same pattern as nodes:

```rust
/// Insert or update an edge between two nodes.
async fn upsert_edge(
    &self,
    source: &str,
    target: &str,
    properties: HashMap<String, serde_json::Value>,
) -> Result<()>;

/// Get all edges connected to a node (both incoming and outgoing).
async fn get_node_edges(&self, node_id: &str) -> Result<Vec<GraphEdge>>;

/// Get the degree (number of edges) of a node.
async fn node_degree(&self, node_id: &str) -> Result<usize>;
```

Edge identity is defined by the `(source, target)` pair. Calling
`upsert_edge` with the same source and target updates the existing
edge's properties.

### Graph Traversal

These methods are what make graph storage powerful for RAG:

```rust
/// Extract a subgraph starting from a node.
///
/// * `start_node` - Starting node for traversal
/// * `max_depth` - Maximum traversal depth (hops)
/// * `max_nodes` - Maximum nodes to return
async fn get_knowledge_graph(
    &self,
    start_node: &str,
    max_depth: usize,
    max_nodes: usize,
) -> Result<KnowledgeGraph>;

/// Get neighbors of a node at a specific depth.
async fn get_neighbors(
    &self,
    node_id: &str,
    depth: usize,
) -> Result<Vec<GraphNode>>;
```

`get_knowledge_graph` is the workhorse of the query engine. Given a
starting entity, it performs breadth-first traversal up to `max_depth`
hops, collecting all discovered nodes and edges into a `KnowledgeGraph`
struct:

```rust
// Get 2-hop neighborhood of JOHN_SMITH, max 50 nodes
let subgraph = graph
    .get_knowledge_graph("JOHN_SMITH", 2, 50)
    .await?;

println!("Found {} nodes, {} edges", subgraph.node_count(), subgraph.edge_count());
if subgraph.is_truncated {
    println!("Warning: subgraph was truncated at 50 nodes");
}
```

### Optimized Batch Queries

The trait includes several batch operations designed to eliminate N+1
query patterns:

```rust
/// Get nodes as a HashMap keyed by node_id.
/// Optimized for O(1) lookups after retrieval.
async fn get_nodes_batch(
    &self,
    node_ids: &[String],
) -> Result<HashMap<String, GraphNode>>;

/// Get edges where BOTH endpoints are in the specified node set.
/// Eliminates the "fetch-all-edges-then-filter" anti-pattern.
async fn get_edges_for_nodes_batch(
    &self,
    node_ids: &[String],
) -> Result<Vec<GraphEdge>>;

/// Get nodes with their in/out degrees in a single batch query.
async fn get_nodes_with_degrees_batch(
    &self,
    node_ids: &[String],
) -> Result<Vec<(GraphNode, usize, usize)>>;
```

These methods have default implementations that call the basic methods
sequentially. Backend implementations override them for performance.
For example, the PostgreSQL adapter uses `UNNEST` with `ORDINALITY`
to fetch all requested nodes in a single SQL query.

### Search Methods

Text-based and popularity-based search methods support the API and
query engine:

```rust
/// Search for nodes with full text matching on label and description.
async fn search_nodes(
    &self,
    query: &str,
    limit: usize,
    entity_type: Option<&str>,
    tenant_id: Option<&str>,
    workspace_id: Option<&str>,
) -> Result<Vec<(GraphNode, usize)>>;

/// Get the most connected node labels.
async fn get_popular_labels(&self, limit: usize) -> Result<Vec<String>>;
```

`search_nodes` returns tuples of `(GraphNode, degree)` -- the degree
is included because highly-connected nodes are more likely to be
important entities. This powers the entity search in the web UI and
the Local query mode.

### Implementations

EdgeQuake ships with three `GraphStorage` implementations:

| Implementation | Backend | Use Case |
|---------------|---------|----------|
| `MemoryGraphStorage` | `HashMap` in memory | Testing, development |
| `PostgresAGEStorage` | PostgreSQL + Apache AGE | Production (local/cloud) |
| `NeptuneGraphStorage` | Amazon Neptune | AWS cloud deployment |

All three implement the same trait, so switching backends is a
configuration change:

```rust
// Development: in-memory graph
let graph: Arc<dyn GraphStorage> = Arc::new(
    MemoryGraphStorage::new("default")
);

// Production: PostgreSQL with Apache AGE
let graph: Arc<dyn GraphStorage> = Arc::new(
    PostgresAGEStorage::new(pg_config).await?
);
```

### Error Handling

All methods return `Result<T>` where `Result` is aliased to
`Result<T, StorageError>`:

```rust
// From edgequake-storage::error
pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Node not found: {0}")]
    NodeNotFound(String),
    #[error("Edge not found: {source} -> {target}")]
    EdgeNotFound { source: String, target: String },
    #[error("Database error: {0}")]
    DatabaseError(String),
    // ... more variants
}
```

### Key Takeaways

- `GraphStorage` is the single abstraction through which all EdgeQuake
  components interact with the knowledge graph.
- **Namespace-based isolation** enables multi-tenancy at the storage
  level.
- **Upsert semantics** support incremental graph building across
  multiple documents.
- **Batch operations** with default implementations allow backends to
  optimize for performance without breaking the trait contract.
- `get_knowledge_graph` is the core traversal method that powers
  graph-enhanced retrieval.
- Three implementations cover development through production use cases.

---

*Next: [Chapter 5: The Ingestion Pipeline](ch05-ingestion-pipeline.md)*
