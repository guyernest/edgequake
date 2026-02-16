# Modern Knowledge Bases: Graph-Enhanced RAG with EdgeQuake

An mdbook course teaching how to build production knowledge bases using graph-enhanced RAG with the EdgeQuake framework.

## Prerequisites

- Rust (stable toolchain)
- Docker & Docker Compose
- Basic understanding of RAG concepts

## Building

```bash
make install-deps  # First time only
make build
make serve         # Opens in browser with live reload
```

## Course Structure

- **Part I: RAG Foundations** (Ch 1-3) — Embeddings, chunking, retrieval techniques
- **Part II: Graph-Enhanced Retrieval** (Ch 4-7) — Knowledge graphs, ingestion, multi-mode queries
- **Part III: Production Infrastructure** (Ch 8-10) — Storage backends, local dev, AWS deployment
- **Part IV: Enterprise Hardening** (Ch 11-13) — Security, batch ingestion, observability
