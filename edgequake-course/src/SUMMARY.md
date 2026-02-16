# Summary

[Introduction](./introduction.md)
[Prerequisites](./prerequisites.md)

---

# Part I: RAG Foundations

- [The RAG Landscape](./part1-foundations/ch01-rag-landscape.md)
  - [Why RAG?](./part1-foundations/ch01-01-why-rag.md)
  - [Anatomy of a RAG Pipeline](./part1-foundations/ch01-02-rag-pipeline.md)
  - [Naive RAG in Rust](./part1-foundations/ch01-03-naive-rag-rust.md)
  - [Exercise: Naive Vector Search](./exercises/ch01/naive-vector-search.md)

- [Embeddings and Chunking](./part1-foundations/ch02-embeddings-chunking.md)
  - [Tokenization Fundamentals](./part1-foundations/ch02-01-tokenization.md)
  - [Embedding Models](./part1-foundations/ch02-02-embedding-models.md)
  - [Chunking Strategies](./part1-foundations/ch02-03-chunking-strategies.md)
  - [EdgeQuake's Chunker](./part1-foundations/ch02-04-edgequake-chunker.md)
  - [Exercise: Custom Markdown Chunking](./exercises/ch02/markdown-chunking.md)

- [Advanced Retrieval Techniques](./part1-foundations/ch03-advanced-retrieval.md)
  - [Contextual Retrieval](./part1-foundations/ch03-01-contextual-retrieval.md)
  - [Hybrid Search](./part1-foundations/ch03-02-hybrid-search.md)
  - [Reranking](./part1-foundations/ch03-03-reranking.md)
  - [Why RAG Alone Is Not Enough](./part1-foundations/ch03-04-limitations.md)
  - [Exercise: Hybrid BM25 + Dense Scoring](./exercises/ch03/hybrid-search.md)

---

# Part II: Graph-Enhanced Retrieval

- [Knowledge Graphs for RAG](./part2-graph/ch04-knowledge-graphs.md)
  - [The Property Graph Model](./part2-graph/ch04-01-property-graph.md)
  - [The LightRAG Insight](./part2-graph/ch04-02-lightrag-insight.md)
  - [The GraphStorage Trait](./part2-graph/ch04-03-graph-storage-trait.md)
  - [Exercise: In-Memory GraphStorage](./exercises/ch04/in-memory-graph.md)

- [The Ingestion Pipeline](./part2-graph/ch05-ingestion-pipeline.md)
  - [Pipeline Architecture](./part2-graph/ch05-01-pipeline-architecture.md)
  - [Entity Extraction](./part2-graph/ch05-02-entity-extraction.md)
  - [Entity Normalization](./part2-graph/ch05-03-entity-normalization.md)
  - [Gleaning for Quality](./part2-graph/ch05-04-gleaning.md)
  - [Exercise: Entity Extraction Prompt](./exercises/ch05/entity-extraction.md)

- [Multi-Mode Query Engine](./part2-graph/ch06-query-engine.md)
  - [Keyword Extraction](./part2-graph/ch06-01-keyword-extraction.md)
  - [Local, Global, and Hybrid Modes](./part2-graph/ch06-02-query-modes.md)
  - [Token Budgeting](./part2-graph/ch06-03-token-budgeting.md)
  - [Exercise: Query Mode Selector](./exercises/ch06/query-mode-selector.md)

- [The EdgeQuake Orchestrator](./part2-graph/ch07-orchestrator.md)
  - [Core Architecture](./part2-graph/ch07-01-core-architecture.md)
  - [The Axum API Layer](./part2-graph/ch07-02-axum-api.md)
  - [End-to-End Request Flow](./part2-graph/ch07-03-request-flow.md)
  - [Exercise: Wire Up EdgeQuake](./exercises/ch07/full-edgequake.md)

---

# Part III: Production Storage & Infrastructure

- [Pluggable Storage Backends](./part3-production/ch08-storage-backends.md)
  - [The Trait Abstraction](./part3-production/ch08-01-trait-abstraction.md)
  - [Memory and PostgreSQL Adapters](./part3-production/ch08-02-memory-postgres.md)
  - [AWS Storage Adapters](./part3-production/ch08-03-aws-adapters.md)
  - [Exercise: File-Based KVStorage](./exercises/ch08/file-kv-storage.md)

- [Local Development Environment](./part3-production/ch09-local-dev.md)
  - [Docker Compose Stack](./part3-production/ch09-01-docker-compose.md)
  - [PostgreSQL with pgvector and AGE](./part3-production/ch09-02-pgvector-age.md)
  - [LLM Provider Configuration](./part3-production/ch09-03-llm-providers.md)
  - [Exercise: Local Dev Setup](./exercises/ch09/local-dev-setup.md)

- [Cloud Deployment with AWS](./part3-production/ch10-aws-deployment.md)
  - [S3 Vectors for Embeddings](./part3-production/ch10-01-s3-vectors.md)
  - [Neptune and DynamoDB](./part3-production/ch10-02-neptune-dynamodb.md)
  - [CDK Infrastructure](./part3-production/ch10-03-cdk-infrastructure.md)
  - [Exercise: CDK Stack Design](./exercises/ch10/cdk-stack.md)

---

# Part IV: Enterprise Hardening

- [Security and Multi-Tenancy](./part4-enterprise/ch11-security.md)
  - [JWT Authentication](./part4-enterprise/ch11-01-jwt-auth.md)
  - [Role-Based Access Control](./part4-enterprise/ch11-02-rbac.md)
  - [Namespace-Based Tenant Isolation](./part4-enterprise/ch11-03-tenant-isolation.md)
  - [Exercise: JWT Validation Middleware](./exercises/ch11/jwt-validation.md)

- [Batch Ingestion at Scale](./part4-enterprise/ch12-batch-ingestion.md)
  - [The Four-Phase Pipeline](./part4-enterprise/ch12-01-four-phases.md)
  - [OpenAI Batch API Integration](./part4-enterprise/ch12-02-openai-batch.md)
  - [DynamoDB State Management](./part4-enterprise/ch12-03-dynamodb-state.md)
  - [Exercise: Batch State Machine](./exercises/ch12/batch-state-machine.md)

- [Observability and Operations](./part4-enterprise/ch13-observability.md)
  - [Structured Logging](./part4-enterprise/ch13-01-structured-logging.md)
  - [Middleware Stack](./part4-enterprise/ch13-02-middleware-stack.md)
  - [Performance Optimization](./part4-enterprise/ch13-03-performance.md)
  - [Exercise: Logging Middleware](./exercises/ch13/logging-middleware.md)
