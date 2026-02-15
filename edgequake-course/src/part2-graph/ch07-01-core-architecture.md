## 7.1 Core Architecture

The `edgequake-core` crate contains the orchestrator -- the central
struct that wires together storage backends, LLM providers, the
ingestion pipeline, and the query engine into a single cohesive system.

### The Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│                       EdgeQuake                              │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │                    Orchestrator                          │ │
│  │  - config: EdgeQuakeConfig                              │ │
│  │  - storage: KV + Vector + Graph                         │ │
│  │  - providers: LLM + Embedding                           │ │
│  └────────────────────────┬────────────────────────────────┘ │
│                           │                                  │
│     ┌─────────────────────┼─────────────────────┐           │
│     │                     │                     │           │
│     v                     v                     v           │
│  ┌──────────┐       ┌──────────┐         ┌──────────┐       │
│  │ insert() │       │  query() │         │ delete() │       │
│  └────┬─────┘       └────┬─────┘         └────┬─────┘       │
│       │                  │                    │             │
│       v                  v                    v             │
│  ┌──────────┐       ┌──────────┐         ┌──────────┐       │
│  │ Pipeline │       │  Query   │         │ Cascade  │       │
│  │ (chunk+  │       │  Engine  │         │  Delete  │       │
│  │ extract) │       │ (6 modes)│         │ (source  │       │
│  └──────────┘       └──────────┘         │ tracking)│       │
│                                          └──────────┘       │
└─────────────────────────────────────────────────────────────┘

Storage Layer:
┌──────────┐    ┌──────────┐    ┌──────────┐
│ KVStorage│    │VectorStor│    │GraphStor │
│ (docs,   │    │(pgvector)│    │(AGE/mem) │
│  chunks) │    │          │    │          │
└──────────┘    └──────────┘    └──────────┘
```

### Dependencies as Trait Objects

The orchestrator holds its dependencies as trait objects behind `Arc`,
enabling runtime polymorphism across storage backends:

```rust
use std::sync::Arc;
use edgequake_storage::{GraphStorage, VectorStorage, KVStorage};
use edgequake_llm::traits::{LLMProvider, EmbeddingProvider};

pub struct EdgeQuake {
    /// Configuration for the orchestrator.
    config: EdgeQuakeConfig,

    /// Graph storage (knowledge graph nodes and edges).
    graph_storage: Arc<dyn GraphStorage>,

    /// Vector storage (embeddings for similarity search).
    vector_storage: Arc<dyn VectorStorage>,

    /// Key-value storage (chunks, documents, metadata).
    kv_storage: Arc<dyn KVStorage>,

    /// LLM provider (for extraction and generation).
    llm_provider: Arc<dyn LLMProvider>,

    /// Embedding provider (for vector generation).
    embedding_provider: Arc<dyn EmbeddingProvider>,

    /// The SOTA query engine.
    query_engine: SOTAQueryEngine,
}
```

Using `Arc<dyn Trait>` means the orchestrator does not know (or care)
whether it is talking to in-memory storage or PostgreSQL, to OpenAI or
Ollama. This is the key design principle: **the orchestrator depends on
traits, not implementations**.

### EdgeQuakeConfig

The configuration struct controls global behavior:

```rust
pub struct EdgeQuakeConfig {
    /// Pipeline configuration (chunking, extraction, embedding).
    pub pipeline: PipelineConfig,

    /// Query engine configuration (default mode, token budget).
    pub query: SOTAQueryConfig,

    /// Namespace for tenant isolation.
    pub namespace: String,

    /// Default workspace ID.
    pub workspace_id: Option<uuid::Uuid>,
}
```

### Initialization Flow

Creating an `EdgeQuake` instance requires assembling all dependencies:

```rust
use edgequake_core::EdgeQuake;
use edgequake_storage::adapters::memory::*;
use std::sync::Arc;

// 1. Create storage backends
let graph: Arc<dyn GraphStorage> = Arc::new(
    MemoryGraphStorage::new("default")
);
let vector: Arc<dyn VectorStorage> = Arc::new(
    MemoryVectorStorage::new("default")
);
let kv: Arc<dyn KVStorage> = Arc::new(
    MemoryKVStorage::new("default")
);

// 2. Create LLM and embedding providers
let llm: Arc<dyn LLMProvider> = Arc::new(
    OpenAIProvider::new(api_key, "gpt-4o")
);
let embedding: Arc<dyn EmbeddingProvider> = Arc::new(
    OpenAIEmbeddingProvider::new(api_key, "text-embedding-3-small")
);

// 3. Initialize storage (create tables/indexes)
graph.initialize().await?;
vector.initialize().await?;

// 4. Assemble the orchestrator
let eq = EdgeQuake::new(
    EdgeQuakeConfig::default(),
    graph,
    vector,
    kv,
    llm,
    embedding,
);
```

In production, the API layer handles this assembly using environment
variables and configuration files (Chapter 9).

### Core Operations

The orchestrator exposes three primary operations:

#### insert() -- Document Ingestion

```rust
impl EdgeQuake {
    /// Ingest a document into the knowledge base.
    ///
    /// Runs the full pipeline: chunk -> extract -> normalize -> embed -> store.
    pub async fn insert(
        &self,
        content: &str,
        document_id: Option<&str>,
    ) -> Result<ProcessingResult> {
        // 1. Chunk the document
        let chunks = self.chunker.chunk(content);

        // 2. Extract entities and relationships from each chunk
        let extractions = self.extractor
            .extract_batch(&chunks)
            .await?;

        // 3. Normalize entity names
        let normalized = self.normalize(extractions);

        // 4. Generate embeddings
        let embedded = self.embed(normalized).await?;

        // 5. Store in graph + vector + KV
        self.store(embedded).await?;

        Ok(ProcessingResult { /* stats */ })
    }
}
```

#### query() -- Knowledge Base Query

```rust
impl EdgeQuake {
    /// Query the knowledge base.
    ///
    /// Uses the SOTA query engine with mode-specific retrieval.
    pub async fn query(
        &self,
        query: &str,
        params: Option<QueryParams>,
    ) -> Result<QueryResponse> {
        let params = params.unwrap_or_default();

        // Delegate to the SOTA query engine
        let context = self.query_engine.execute(
            query,
            &params.mode,
            &self.graph_storage,
            &self.vector_storage,
            &self.kv_storage,
            &self.llm_provider,
            &self.embedding_provider,
        ).await?;

        Ok(context)
    }
}
```

#### delete() -- Document Removal

```rust
impl EdgeQuake {
    /// Delete a document and cascade-remove its entities.
    ///
    /// Uses source_id tracking to identify affected entities.
    pub async fn delete(&self, document_id: &str) -> Result<DeleteResult> {
        // 1. Find all chunks from this document
        let chunks = self.kv_storage.get_by_prefix(document_id).await?;

        // 2. Find all entities sourced from these chunks
        // 3. Remove entities that have no other source documents
        // 4. Update entities that have other sources (remove this source_id)
        // 5. Remove chunks from KV and vector storage

        Ok(DeleteResult { /* stats */ })
    }
}
```

Cascade delete is possible because every entity and relationship
tracks its `source_id` -- the pipe-separated list of document IDs
that contributed to it (Chapter 5).

### The Trait-Based Architecture

The orchestrator's trait-based design enables several powerful patterns:

#### Testing with In-Memory Storage

```rust
#[tokio::test]
async fn test_insert_and_query() {
    // All in-memory -- no database, no LLM API
    let eq = EdgeQuake::new(
        config,
        Arc::new(MemoryGraphStorage::new("test")),
        Arc::new(MemoryVectorStorage::new("test")),
        Arc::new(MemoryKVStorage::new("test")),
        Arc::new(MockLLMProvider::new()),
        Arc::new(MockEmbeddingProvider::new()),
    );

    eq.insert("John works at Acme Corp.", None).await.unwrap();
    let result = eq.query("Who works at Acme?", None).await.unwrap();
    assert!(!result.response.is_empty());
}
```

#### Swapping Backends at Runtime

```rust
// Development: PostgreSQL locally
let graph = create_postgres_graph(&local_config).await?;

// Production: Amazon Neptune
let graph = create_neptune_graph(&aws_config).await?;

// Same orchestrator code, different backend
let eq = EdgeQuake::new(config, graph, vector, kv, llm, embedding);
```

#### Multi-Tenant Isolation

Each tenant gets its own orchestrator instance with namespace-scoped
storage:

```rust
fn create_tenant_orchestrator(tenant_id: &str) -> EdgeQuake {
    let graph = Arc::new(MemoryGraphStorage::new(tenant_id));
    let vector = Arc::new(MemoryVectorStorage::new(tenant_id));
    let kv = Arc::new(MemoryKVStorage::new(tenant_id));

    EdgeQuake::new(
        EdgeQuakeConfig { namespace: tenant_id.into(), ..Default::default() },
        graph, vector, kv, llm, embedding,
    )
}
```

### Key Takeaways

- The `EdgeQuake` orchestrator coordinates all system components through
  trait objects (`Arc<dyn Trait>`).
- Three primary operations: `insert` (ingest), `query` (retrieve +
  generate), and `delete` (cascade remove).
- Trait-based design enables testing with mocks, swapping backends at
  runtime, and multi-tenant isolation.
- Configuration flows from `EdgeQuakeConfig` to the pipeline and query
  engine sub-configs.
- The orchestrator does not contain business logic itself -- it delegates
  to the pipeline (Chapter 5) and query engine (Chapter 6).

---

*Next: [7.2 The Axum API Layer](ch07-02-axum-api.md)*
