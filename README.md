# EdgeQuake (AWS Cloud Fork)

> **Cloud-Native Graph-RAG Framework in Rust**
> Production-scale knowledge graphs on AWS managed services with MCP integration and batch ingestion

[![Rust](https://img.shields.io/badge/rust-1.78+-orange.svg?style=flat&logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat)](LICENSE)
[![AWS](https://img.shields.io/badge/AWS-Neptune%20%7C%20S3%20Vectors%20%7C%20DynamoDB-FF9900?style=flat&logo=amazonaws)](https://aws.amazon.com)
[![MCP](https://img.shields.io/badge/MCP-Model%20Context%20Protocol-blueviolet?style=flat)](MCP_INTEGRATION.md)

---

This is a **fork** of [raphaelmansuy/edgequake](https://github.com/raphaelmansuy/edgequake) — the original high-performance Rust implementation of the [LightRAG algorithm](https://arxiv.org/abs/2410.05779). This fork replaces the PostgreSQL storage layer with **AWS managed services**, adds a **batch ingestion pipeline** for 100K+ documents, introduces **MCP server integration** for AI agent access, and adds **answer quality signals** and **Code Mode** for programmatic graph analysis.

## What This Fork Adds

| Capability | Upstream (PostgreSQL) | This Fork (AWS Cloud) |
|---|---|---|
| **Graph Storage** | PostgreSQL + Apache AGE | Amazon Neptune (Gremlin, IAM SigV4) |
| **Vector Search** | pgvector (HNSW) | Amazon S3 Vectors (serverless, auto-scaling) |
| **Key-Value** | PostgreSQL JSONB | Amazon DynamoDB (on-demand, single-digit ms) |
| **BM25 Search** | -- | Amazon Athena + Iceberg tables (serverless BM25 scoring over S3) |
| **Batch Ingestion** | Single-doc API | 4-phase pipeline: OpenAI Batch API, 100K+ docs |
| **AI Agent Access** | REST API only | MCP server with 4 tools + Code Mode sandbox |
| **Answer Quality** | -- | Fact density scoring, source diversity, chunk confidence |
| **Retrieval Modes** | Vector similarity | Vector + BM25 keyword + hybrid fusion (RRF) |
| **Entity Resolution** | Sequential search | Batch resolution (20 terms in 1 round-trip) |
| **Infrastructure** | Docker Compose | AWS CDK stacks (parameterized, multi-tenant) |
| **Idle Cost** | ~$297/month (RDS always-on) | ~$7/month (serverless scales to zero) |

---

## Architecture

```
                         MCP Clients (Claude, IDEs, agents)
                                      |
                                      v
                     +----------------------------------+
                     |  MCP Server (4 tools + Code Mode)|
                     |  ask | explore_entity | search   |
                     |  validate_code | execute_code    |
                     +----------------------------------+
                                      |
+-------------------------------------+--------------------------------------+
|                              EdgeQuake System                              |
|                                                                            |
|  Frontend (React 19 + Next.js)          REST API (Axum)                    |
|  - Sigma.js graph visualization         - OpenAPI 3.0 + Swagger UI         |
|  - SSE streaming responses              - SSE streaming                    |
|  - Document upload (drag-and-drop)      - /api/v1/* (100+ endpoints)       |
|  - Dark mode, i18n                      - Health: /health, /ready, /live   |
|                                                                            |
|  Backend (Rust - 14 Crates)                                                |
|  +----------------------------------------------------------------------+  |
|  | edgequake-core          | Orchestration, pipeline coordination       |  |
|  | edgequake-query         | 6 query modes, BM25, hybrid retrieval      |  |
|  | edgequake-pipeline      | Document ingestion pipeline                |  |
|  | edgequake-llm           | OpenAI, Ollama, LM Studio providers        |  |
|  | edgequake-storage       | Trait abstraction (GraphStorage, etc.)     |  |
|  | edgequake-storage-aws   | Neptune, S3 Vectors, DynamoDB, Athena      |  |
|  | edgequake-batch         | Batch ingestion CLI (100K+ docs)           |  |
|  | edgequake-schema        | LLM-assisted domain schema generation      |  |
|  | edgequake-api           | REST API + entity resolution + Code Mode   |  |
|  | edgequake-pdf           | PDF extraction (text/vision/hybrid)        |  |
|  | edgequake-auth          | JWT + API key authentication               |  |
|  | edgequake-audit         | Compliance and audit logging               |  |
|  | edgequake-tasks         | Background job processing                  |  |
|  | edgequake-rate-limiter  | Per-tenant rate limiting                   |  |
|  +----------------------------------------------------------------------+  |
|                                                                            |
|  Storage Backends (swap via traits, no code changes)                       |
|  +----------------------------+  +-------------------------------------+   |
|  | Local (Docker)             |  | AWS Managed (Production)            |   |
|  | - PostgreSQL + AGE (graph) |  | - Neptune (graph, Gremlin)          |   |
|  | - pgvector (vectors)       |  | - S3 Vectors (vector search)        |   |
|  | - PostgreSQL (KV)          |  | - DynamoDB (key-value, state)       |   |
|  | - In-Memory (dev/test)     |  | - Athena + Iceberg (BM25 search)    |   |
|  +----------------------------+  +-------------------------------------+   |
+----------------------------------------------------------------------------+
```

---

## AWS Storage Backends

EdgeQuake implements a trait-based storage abstraction (`GraphStorage`, `VectorStorage`, `KVStorage`, `Bm25Storage`). The AWS backends are designed for production workloads where serverless scaling and pay-per-use pricing matter.

| Layer | AWS Service | Why | Key Specs |
|---|---|---|---|
| **Graph** | Neptune Serverless | Managed graph DB, Gremlin queries, IAM auth, auto-replication | 1-100ms latency, scales 2.5-128 NCUs, scales to zero |
| **Vector** | S3 Vectors | Native cosine search, no capacity planning, auto-indexing | Pay-per-request, scales with data volume |
| **Key-Value** | DynamoDB On-Demand | Single-digit ms reads, batch operations, PITR | $1.25/M writes, $0.25/M reads, $0.25/GB-month |
| **BM25 Search** | Athena + Iceberg | Serverless Okapi BM25 scoring via SQL over Parquet on S3 | $5/TB scanned, no infrastructure to manage |

### Cost Comparison (100GB vectors, 1M graph nodes, 100K documents)

| Scenario | PostgreSQL (RDS) | AWS Serverless | Savings |
|---|---|---|---|
| Active workload | $297/month | $95/month | **68%** |
| Idle workload | $297/month | $7/month | **98%** |
| Annual (50% idle) | $3,570/year | $610/year | **$2,960/year** |

### Infrastructure as Code

Deploy the full AWS stack with CDK:

```bash
make infra-deploy           # Deploy all stacks
make infra-deploy-core      # DynamoDB, S3, S3 Vectors, IAM
make infra-deploy-neptune   # VPC, Neptune cluster, security groups
make infra-env              # Generate .env from deployed stack outputs
```

Parameterized by tenant ID and environment (dev/staging/prod). Outputs stored in SSM Parameter Store for service discovery.

---

## MCP Server Integration

EdgeQuake exposes its knowledge graph through the [Model Context Protocol](https://modelcontextprotocol.io/), enabling AI agents (Claude, IDE copilots, custom agents) to query, explore, and analyze the graph programmatically.

### 4 Tools + Code Mode

| Tool | Purpose | When to Use |
|---|---|---|
| `ask` | Answer questions with full retrieval pipeline | Natural language questions about your documents |
| `explore_entity` | Find entities and traverse their connections | "Who is connected to X?" / relationship mapping |
| `search` | Raw retrieval (semantic, keyword, hybrid) | When you need chunks/entities without LLM generation |
| `validate_code` + `execute_code` | Run JavaScript in a sandboxed environment | Complex analysis, cross-entity comparison, bulk operations |

The `ask` tool implements a multi-step retrieval strategy internally: extract keywords, batch-resolve entities, select query mode based on entity degree, query with fallback chain. MCP clients get optimized answers in a single tool call instead of orchestrating 3-5 calls manually.

**Code Mode** enables programmatic graph analysis via a JavaScript sandbox with an `api` object for internal REST calls. Useful for cross-referencing entities, building comparison tables, or running custom filters that individual tools can't express.

See [MCP_INTEGRATION.md](MCP_INTEGRATION.md) and [MCP_QUICKSTART.md](MCP_QUICKSTART.md) for setup.

---

## Batch Ingestion Pipeline

Process large document datasets (100K+ documents) with the `edgequake-batch` CLI. Uses the OpenAI Batch API for **50% cost savings** on entity extraction.

### 4-Phase Pipeline

```
Phase 1: Prepare     Phase 2: Extract        Phase 3: Embed      Phase 4: Store
Parse parquet/text   OpenAI Batch API        Generate vectors     Write to Neptune,
Chunk documents  --> Submit JSONL batches --> for chunks and   --> S3 Vectors,
Build JSONL          Poll for results        entities             DynamoDB
```

- **Domain-Configurable**: Entity types, aliases, extraction prompts, and few-shot examples defined in `domain.toml` -- switch domains without code changes
- **Resumable**: DynamoDB-backed state management with per-phase checkpointing; resume from any phase after interruption
- **Cost-Optimized**: Batch API (50% off) + prompt caching (50% off cached tokens) = ~75% savings on extraction costs

```bash
make batch-run            # Full pipeline
make batch-prepare        # Phase 1: Parse, chunk, build JSONL
make batch-extract        # Phase 2: Submit to OpenAI Batch API
make batch-embed          # Phase 3: Generate embeddings
make batch-store          # Phase 4: Write to Neptune/S3/DynamoDB
make batch-status         # Show job progress
make batch-resume         # Resume from checkpoint
```

---

## Query Engine

EdgeQuake implements the [LightRAG algorithm](https://arxiv.org/abs/2410.05779) with extensions for hybrid retrieval and answer quality scoring.

### 6 Query Modes

| Mode | Latency | Best For | How It Works |
|---|---|---|---|
| **Naive** | ~100-300ms | Fact lookups (who/what/when) | Vector similarity on chunks only |
| **Local** | ~200-500ms | Entity-centric questions | Vector search + local graph neighborhood traversal |
| **Global** | ~300-800ms | Thematic/high-level questions | Louvain community detection + cluster summaries |
| **Hybrid** _(default)_ | ~400-1000ms | General questions | Combines local + global context |
| **Mix** | configurable | Tuning precision/recall | Weighted blend of naive + graph results |
| **Bypass** | LLM only | General knowledge | Skip retrieval, direct LLM query |

### 3 Retrieval Modes

Each query mode can use one of three retrieval backends (orthogonal to query mode -- you can combine any query mode with any retrieval mode):

- **Vector** (default) -- Cosine similarity via embeddings (S3 Vectors)
- **BM25** -- Keyword search via Athena SQL (best for exact names, rare terms, numbers)
- **Hybrid** -- Reciprocal Rank Fusion of vector + BM25 results (recommended for most queries)

#### BM25 via Athena (this fork's addition)

The upstream project has no keyword search. This fork adds a full **Okapi BM25** implementation backed by Amazon Athena and Apache Iceberg tables on S3:

```
Batch ingestion                    Query time

Documents --> Tokenize + stem      Query --> Tokenize + stem
           |                                |
           v                                v
    Write Parquet to S3 staging     Athena SQL: BM25 scoring
           |                        (IDF * TF-norm per term)
           v                                |
    INSERT INTO Iceberg tables              v
    (postings, term_stats,          Ranked doc_ids + scores
     docs, corpus_stats)
```

Four Iceberg tables per namespace store the inverted index:
- `{ns}_postings`: term -> doc_id -> term frequency
- `{ns}_term_stats`: term -> document frequency
- `{ns}_docs`: doc_id -> document length, source chunk ID
- `{ns}_corpus_stats`: total documents (N), average document length (avgdl)

Data is ingested via **Parquet staging** (Arrow RecordBatch -> Parquet -> S3 -> `INSERT INTO...SELECT`) to avoid Athena's small-file anti-pattern. At query time, BM25 scoring runs as a single Athena SQL query using the standard Okapi formula with k1=1.5, b=0.75.

In **hybrid** retrieval mode, vector and BM25 results are fused using Reciprocal Rank Fusion (RRF), combining semantic understanding with exact keyword matching.

### Answer Quality Signals

Every query response includes quality metrics:

```json
{
  "answer": "...",
  "quality": {
    "mean_chunk_score": 0.82,
    "source_diversity": 5,
    "fact_density": 3
  },
  "extracted_keywords": {
    "high_level": ["financial relationship", "banking connections"],
    "low_level": ["Jeffrey Epstein", "Deutsche Bank"]
  }
}
```

- **mean_chunk_score**: Average relevance of retrieved chunks (0.0-1.0)
- **source_diversity**: Number of distinct source documents
- **fact_density**: Count of proper nouns + dates + numbers in the answer

---

## Knowledge Graph

- **Entity Extraction**: LLM-powered detection of people, organizations, locations, concepts, events, technologies, and products (7 configurable types)
- **Relationship Mapping**: Keyword-tagged relationships between entities
- **Gleaning**: Multi-pass extraction catches 15-25% more entities than single-pass
- **Normalization**: Case normalization + description merging reduces duplicates by ~36-40%
- **Community Detection**: Louvain modularity optimization for thematic clustering
- **Batch Entity Resolution**: Resolve 20 terms against the graph in a single round-trip

### Indexing Pipeline (per document)

1. **Chunk** -- Split into ~1200-token segments with 100-token overlap
2. **Extract** -- LLM parses each chunk into entities and relationships
3. **Glean** -- Optional second pass for missed entities (~18% recall improvement)
4. **Normalize** -- Deduplicate via case normalization and description merging
5. **Embed** -- Generate vector embeddings for chunks and entities
6. **Store** -- Write to storage backends (swap PostgreSQL/AWS via config)

---

## Quick Start

### Prerequisites

- **Rust**: 1.78+ ([Install Rust](https://rustup.rs))
- **Node.js**: 18+ or Bun 1.0+ ([Install Node](https://nodejs.org))
- **Docker**: For local PostgreSQL dev mode ([Install Docker](https://www.docker.com/get-started))
- **AWS Account**: For production deployment (Neptune, S3 Vectors, DynamoDB)

### Local Development (PostgreSQL)

```bash
# Clone the fork
git clone https://github.com/guyernest/edgequake.git
cd edgequake

# Install dependencies and start full stack
make install
make dev
```

- **Backend**: http://localhost:8080
- **Frontend**: http://localhost:3000
- **Swagger UI**: http://localhost:8080/swagger-ui

### AWS Deployment

```bash
# Deploy infrastructure
make infra-deploy

# Generate .env from deployed stack outputs
make infra-env

# Start with AWS backends
make backend-dev
```

### Upload a Document

```bash
curl -X POST http://localhost:8080/api/v1/documents/upload \
  -F "file=@your-document.md"
```

### Query the Knowledge Graph

```bash
curl -X POST http://localhost:8080/api/v1/query \
  -H "Content-Type: application/json" \
  -d '{"query": "What are the main concepts?", "mode": "hybrid", "retrieval_mode": "hybrid"}'
```

---

## REST API

OpenAPI 3.0 specification with 100+ endpoints. Full Swagger UI at `/swagger-ui`.

### Key Endpoints

| Category | Endpoints | Description |
|---|---|---|
| **Query** | `POST /api/v1/query`, `GET /api/v1/query/stream` | Execute queries with 6 modes, SSE streaming |
| **Documents** | `POST /api/v1/documents/upload`, `GET /api/v1/documents` | Upload, list, delete, retry |
| **Entities** | `GET /api/v1/entities/search`, `POST /api/v1/entities/resolve` | Search, batch resolve, get neighborhood |
| **Graph** | `GET /api/v1/graph`, `GET /api/v1/entities/{name}/neighborhood` | Visualize, traverse, community detection |
| **Search** | `POST /api/v1/search/hybrid`, `GET /api/v1/vectors/search`, `GET /api/v1/bm25/search` | Semantic, keyword, and hybrid search |
| **Text** | `POST /api/v1/text/score`, `POST /api/v1/text/extract-terms` | Fact density scoring, keyword extraction |
| **Code Mode** | `POST /api/v1/code/validate`, `POST /api/v1/code/execute` | Sandboxed JavaScript execution |
| **Admin** | `POST /api/v1/entities/merge`, `PUT /api/v1/entities/{name}` | Entity merge, update, delete |
| **Health** | `/health`, `/ready`, `/live` | Kubernetes-ready probes |

---

## LLM Providers

| Provider | Models | Use Case |
|---|---|---|
| **OpenAI** | GPT-4o, GPT-4.1-nano, text-embedding-3-small/large | Production (extraction + embeddings) |
| **Ollama** | Gemma3, Mistral, Llama, nomic-embed-text | Local development (free) |
| **LM Studio** | Any GGUF model | Local development |
| **Mock** | -- | Testing (no API calls) |

Auto-detected via environment variables. Set `OPENAI_API_KEY` for OpenAI, or configure `EDGEQUAKE_LLM_PROVIDER` explicitly.

---

## Frontend

React 19 + Next.js with interactive graph visualization.

- **Graph Visualization**: Sigma.js 3.0 with force-directed, circular, and ForceAtlas2 layouts
- **SSE Streaming**: Token-by-token answer generation display
- **Document Upload**: Drag-and-drop with progress tracking
- **Dark Mode**: Automatic theme switching
- **i18n**: Multi-language support via i18next
- **Virtualized Lists**: TanStack Virtual for large entity/document lists
- **Markdown Rendering**: Syntax highlighting (Shiki), KaTeX math, GFM tables

---

## Development

### Build Commands

```bash
# Build backend (offline mode, no running PostgreSQL needed)
cd edgequake && SQLX_OFFLINE=true cargo build --release

# Run tests
cargo test

# Build frontend
cd edgequake_webui && bun run build
```

### Make Targets

```bash
# Full development stack
make dev              # PostgreSQL + Backend + Frontend
make dev-bg           # Background mode (for agents/automation)
make dev-memory       # In-memory storage (testing only)
make stop             # Stop all services

# Backend
make backend-dev      # Run with PostgreSQL
make backend-memory   # Run with in-memory storage
make backend-test     # Run backend tests

# Frontend
make frontend-dev     # Start dev server with hot reload
make frontend-build   # Production build

# Batch ingestion
make batch-run        # Full 4-phase pipeline
make batch-status     # Show progress
make batch-resume     # Resume from checkpoint

# Infrastructure (AWS CDK)
make infra-deploy     # Deploy all stacks
make infra-diff       # Preview changes
make infra-destroy    # Tear down

# Quality
make test             # All tests
make lint             # Clippy + ESLint
make format           # rustfmt + Prettier
```

---

## Crate Reference

| Crate | Lines | Description |
|---|---|---|
| `edgequake-core` | -- | Pipeline orchestration, configuration |
| `edgequake-query` | -- | 6 query modes, BM25 scorer, fact density, hybrid retrieval |
| `edgequake-pipeline` | -- | Document ingestion (chunk, extract, embed, store) |
| `edgequake-llm` | -- | OpenAI, Ollama, LM Studio, Mock providers |
| `edgequake-storage` | -- | Trait definitions (`GraphStorage`, `VectorStorage`, `KVStorage`, `Bm25Storage`) |
| `edgequake-storage-aws` | ~2,500 | Neptune (graph), S3 Vectors (vector), DynamoDB (KV), Athena BM25 (keyword search) |
| `edgequake-batch` | -- | Batch ingestion CLI with OpenAI Batch API |
| `edgequake-schema` | -- | LLM-assisted domain schema generation from sample docs |
| `edgequake-api` | -- | REST API server, entity resolution, Code Mode sandbox |
| `edgequake-pdf` | -- | PDF extraction: text mode, vision mode (GPT-4o OCR), hybrid |
| `edgequake-auth` | -- | JWT tokens, API keys, multi-tenant isolation |
| `edgequake-audit` | -- | Structured event logging, compliance |
| `edgequake-tasks` | -- | Background job processing, status tracking |
| `edgequake-rate-limiter` | -- | Per-tenant rate limiting middleware |

---

## Documentation

| Section | Link | Description |
|---|---|---|
| **Getting Started** | [docs/getting-started/](docs/getting-started/) | Installation, quick start, first ingestion |
| **Architecture** | [docs/architecture/](docs/architecture/) | System design, data flow, crate reference |
| **Deep Dives** | [docs/deep-dives/](docs/deep-dives/) | LightRAG algorithm, query modes, entity normalization |
| **API Reference** | [docs/api-reference/](docs/api-reference/) | REST endpoints, extended API |
| **Tutorials** | [docs/tutorials/](docs/tutorials/) | First RAG app, PDF ingestion, multi-tenant setup |
| **Operations** | [docs/operations/](docs/operations/) | Deployment, configuration, monitoring, tuning |
| **MCP Integration** | [MCP_INTEGRATION.md](MCP_INTEGRATION.md) | MCP server setup and tool reference |
| **MCP Quick Start** | [MCP_QUICKSTART.md](MCP_QUICKSTART.md) | 5-minute MCP setup guide |
| **AWS Storage** | [AWS_STORAGE_COMPLETE_SUMMARY.md](AWS_STORAGE_COMPLETE_SUMMARY.md) | Cost analysis, architecture, implementation details |

### SDKs

Auto-generated from OpenAPI 3.0 schema:

- [Python](sdks/python/README.md) | [TypeScript](sdks/typescript/README.md) | [Rust](sdks/rust/README.md) | [Go, Java, Kotlin, PHP, Ruby, Swift, C#](sdks/)

---

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).

**Original work Copyright 2024-2026 Raphael MANSUY**
**Fork modifications Copyright 2025-2026 Guy Ernest**

---

## Acknowledgments

This fork builds on the excellent work of:

- **[Raphael MANSUY](https://github.com/raphaelmansuy)** -- Creator of EdgeQuake and the original Rust implementation of LightRAG. The core architecture (13-crate workspace, trait-based storage, 6 query modes, entity extraction pipeline, React frontend) is his work. This fork extends it with AWS backends, batch ingestion, and MCP integration.
- **LightRAG** ([arxiv.org/abs/2410.05779](https://arxiv.org/abs/2410.05779)) -- The foundational algorithm for knowledge graph extraction and hybrid retrieval. Authors: Zirui Guo, Lianghao Xia, Yanhua Yu, Tu Ao, Chao Huang.
- **GraphRAG** ([arxiv.org/abs/2404.16130](https://arxiv.org/abs/2404.16130)) -- Microsoft's "From Local to Global" approach to query-focused summarization.
- **Rust Community** -- Tokio, Axum, SQLx, and the async ecosystem that makes this performance possible.
