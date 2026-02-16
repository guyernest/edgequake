## 8.2 Memory and PostgreSQL Adapters

EdgeQuake ships two families of storage adapters in the `edgequake-storage`
crate: in-memory implementations for testing and development, and PostgreSQL
implementations for production. This section examines both, focusing on the
design decisions that make each appropriate for its use case.

### Memory Adapters: Fast, Disposable, Deterministic

The memory adapters live in `edgequake_storage::adapters::memory`:

| Adapter | Trait | Internal Structure |
|---------|-------|--------------------|
| `MemoryGraphStorage` | `GraphStorage` | `RwLock<HashMap>` for nodes, edges, adjacency |
| `MemoryVectorStorage` | `VectorStorage` | `RwLock<Vec<(String, Vec<f32>, Value)>>` |
| `MemoryKVStorage` | `KVStorage` | `RwLock<HashMap<String, Value>>` |

All three share the same construction pattern -- a simple `new(namespace)` call
with no async setup:

```rust
use edgequake_storage::{
    MemoryGraphStorage, MemoryVectorStorage, MemoryKVStorage,
};

// No async, no config structs, no connection strings.
let graph = MemoryGraphStorage::new("test-workspace");
let vectors = MemoryVectorStorage::new("test-workspace", 1536);
let kv = MemoryKVStorage::new("test-workspace");
```

#### MemoryGraphStorage Internals

The graph adapter uses three `RwLock`-protected `HashMap`s:

```rust
pub struct MemoryGraphStorage {
    namespace: String,
    nodes: RwLock<HashMap<String, HashMap<String, serde_json::Value>>>,
    edges: RwLock<HashMap<(String, String), HashMap<String, serde_json::Value>>>,
    adjacency: RwLock<HashMap<String, HashSet<String>>>,
}
```

The `adjacency` map provides O(1) neighbor lookups, while the `edges` map
uses a normalized key `(min(source, target), max(source, target))` to ensure
undirected edge consistency:

```rust
fn edge_key(source: &str, target: &str) -> (String, String) {
    if source <= target {
        (source.to_string(), target.to_string())
    } else {
        (target.to_string(), source.to_string())
    }
}
```

This normalization means that `upsert_edge("A", "B", ...)` and
`upsert_edge("B", "A", ...)` write to the same slot -- an important invariant
for knowledge graphs where relationship direction is stored in properties, not
in the key.

#### Why RwLock and Not Mutex?

`RwLock` allows multiple concurrent readers with exclusive writers. Since graph
queries vastly outnumber mutations in a RAG pipeline, this means multiple query
handlers can read the graph simultaneously without blocking each other.

#### When to Use Memory Adapters

- **Unit tests**: Deterministic, no external dependencies, fast teardown.
- **Integration tests**: Spin up a full `EdgeQuakeCore` with memory backends
  in milliseconds.
- **Prototyping**: Quickly validate a pipeline before setting up PostgreSQL.
- **CI/CD**: No Docker or database required in the test stage.

```rust
#[tokio::test]
async fn test_entity_roundtrip() {
    let graph = MemoryGraphStorage::new("test");
    graph.initialize().await.unwrap();

    let mut props = HashMap::new();
    props.insert(
        "entity_type".to_string(),
        serde_json::json!("PERSON"),
    );

    graph.upsert_node("alice", props).await.unwrap();

    let node = graph.get_node("alice").await.unwrap().unwrap();
    assert_eq!(node.id, "alice");
    assert_eq!(
        node.properties["entity_type"],
        serde_json::json!("PERSON"),
    );
}
```

### PostgreSQL Adapters: Production-Grade Storage

For production, EdgeQuake uses PostgreSQL with two extensions:

- **pgvector** -- adds the `vector` column type and HNSW/IVFFlat indexes for
  similarity search.
- **Apache AGE** -- adds Cypher query support and graph traversal to
  PostgreSQL.

Both extensions run inside a single PostgreSQL instance, sharing the same
connection pool. This is a major operational advantage: one database to
back up, monitor, and scale.

```mermaid
graph TD
    subgraph "Single PostgreSQL Instance"
        PG[PostgreSQL 16]
        PG --> PGV[pgvector Extension<br/>HNSW index, cosine distance]
        PG --> AGE[Apache AGE Extension<br/>Cypher queries, graph traversal]
        PG --> KV[Standard Tables<br/>JSONB columns for KV storage]
    end

    App[EdgeQuake] --> Pool[sqlx Connection Pool]
    Pool --> PG
```

#### PostgresConfig

Connection configuration is managed by `PostgresConfig`:

```rust
use edgequake_storage::PostgresConfig;

let config = PostgresConfig::new(
    "localhost", 5432, "edgequake", "postgres", "secret"
)
.with_namespace("my-workspace")
.with_max_connections(20)
.with_vector_index(VectorIndexType::HNSW);

// Generates: postgres://postgres:secret@localhost:5432/edgequake
let url = config.connection_url();
```

Key configuration fields:

| Field | Default | Purpose |
|-------|---------|---------|
| `max_connections` | 10 | Connection pool ceiling |
| `min_connections` | 1 | Warm connections kept alive |
| `connect_timeout` | 30s | Timeout for acquiring a connection |
| `idle_timeout` | 600s | Close idle connections after this |
| `vector_index_type` | HNSW | pgvector index algorithm |
| `hnsw_m` | 16 | HNSW graph connectivity parameter |
| `hnsw_ef_construction` | 64 | HNSW build-time accuracy parameter |
| `ssl_mode` | Prefer | TLS negotiation mode |

#### Connection Pooling with PostgresPool

`PostgresPool` wraps `sqlx::PgPool` with lazy initialization and automatic
extension setup:

```rust
use edgequake_storage::{PostgresPool, PostgresConfig};

let config = PostgresConfig::default();
let pool = PostgresPool::new(config);

// First call creates the pool and enables extensions.
pool.initialize().await?;

// Subsequent calls return the cached pool.
let pg_pool = pool.get().await?;
```

During initialization, `PostgresPool` automatically runs:

```sql
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS age CASCADE;
SET search_path = ag_catalog, "$user", public;
```

If Apache AGE is not installed, initialization logs a warning but continues --
graph features gracefully degrade to the fallback storage.

#### PostgresAGEGraphStorage

The graph adapter uses Apache AGE for Cypher queries. Under the hood, it
translates `GraphStorage` trait calls into SQL that invokes AGE functions:

```sql
-- Upsert a node
SELECT * FROM cypher('edgequake', $$
    MERGE (n:Entity {id: 'ALICE'})
    SET n.entity_type = 'PERSON',
        n.description = 'A researcher',
        n.namespace = 'workspace-1'
    RETURN n
$$) as (v agtype);
```

The adapter handles the impedance mismatch between Rust's `HashMap<String,
Value>` properties and AGE's `agtype` JSON format.

#### PgVectorStorage

The vector adapter uses pgvector's `vector` column type and HNSW indexing:

```sql
-- Table schema (created by SQLx migrations)
CREATE TABLE eq_default_vectors (
    id TEXT PRIMARY KEY,
    embedding vector(1536),
    metadata JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- HNSW index for fast cosine similarity search
CREATE INDEX ON eq_default_vectors
    USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);

-- Similarity query
SELECT id, 1 - (embedding <=> $1::vector) AS score, metadata
FROM eq_default_vectors
ORDER BY embedding <=> $1::vector
LIMIT $2;
```

The `<=>` operator is pgvector's cosine distance operator. We convert to a
similarity score with `1 - distance`, matching the convention used across all
EdgeQuake vector backends.

#### PostgresKVStorage

The KV adapter stores data as JSONB in standard PostgreSQL tables:

```sql
CREATE TABLE eq_default_kv (
    namespace TEXT NOT NULL,
    id TEXT NOT NULL,
    data JSONB NOT NULL,
    status TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (namespace, id)
);
```

The `transition_if_status` method uses PostgreSQL's `UPDATE ... WHERE` for
atomic compare-and-swap:

```sql
UPDATE eq_default_kv
SET status = $3, updated_at = NOW()
WHERE namespace = $1 AND id = $2 AND status = $4
RETURNING id;
```

If the `WHERE` clause matches zero rows, the status has changed since we
checked -- the method returns `Ok(false)` and the caller can handle the
conflict.

### Choosing Between Adapters

```mermaid
graph TD
    A{Environment?} -->|Unit Tests| B[Memory Adapters]
    A -->|Local Dev| C[PostgreSQL Adapters]
    A -->|AWS Cloud| D[AWS Adapters<br/>Chapter 8.3]

    B --> B1[No setup required<br/>Deterministic<br/>Fast]
    C --> C1[Full feature parity<br/>SQL migrations<br/>Realistic performance]
    D --> D1[Managed services<br/>Pay-per-use<br/>Auto-scaling]
```

| Criterion | Memory | PostgreSQL |
|-----------|--------|------------|
| Setup time | Zero | Docker Compose |
| Persistence | None (process lifetime) | Durable |
| Concurrency | `RwLock` (single process) | Connection pool (multi-process) |
| Vector index | Brute-force O(n) | HNSW O(log n) |
| Graph queries | In-memory BFS | Cypher via AGE |
| Suitable for | Tests, prototyping | Local dev, staging, small production |

### Feature Flags

PostgreSQL adapters are gated behind a Cargo feature flag:

```toml
[dependencies]
edgequake-storage = { path = "../edgequake-storage", features = ["postgres"] }
```

Without the `postgres` feature, the crate compiles without `sqlx` or any
PostgreSQL dependencies, keeping binary size small for Lambda deployments
that only need memory or AWS adapters.

### Summary

Memory adapters provide zero-setup, deterministic storage for testing.
PostgreSQL adapters use pgvector and Apache AGE to deliver production-grade
vector search and graph traversal from a single database instance. Connection
pooling via `sqlx` ensures efficient resource usage, while feature flags keep
the dependency tree lean.

---

*Next: [8.3 AWS Storage Adapters](ch08-03-aws-adapters.md)*
