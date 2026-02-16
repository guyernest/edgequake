# Modern Knowledge Bases

**Build production knowledge bases using graph-enhanced RAG in Rust.**

## What You'll Build

By the end of this course, you'll understand how to build a production knowledge base system that goes beyond naive vector search. You'll learn the architecture behind **EdgeQuake**, a Rust framework that combines:

- **Vector embeddings** for semantic similarity search
- **Knowledge graphs** for structured relationship traversal
- **Multi-mode query engines** that blend both approaches
- **Production infrastructure** for real-world deployment

## Why Graph-Enhanced RAG?

Standard RAG (Retrieval-Augmented Generation) retrieves text chunks by vector similarity. This works well for direct questions but fails when answers require connecting information across multiple documents.

Consider this query: *"What are the financial connections between Entity A and Entity B?"*

A naive RAG system might retrieve chunks mentioning each entity separately, but it cannot traverse the relationship chain connecting them. A graph-enhanced system maintains explicit entity-relationship structure, enabling multi-hop reasoning.

```text
Naive RAG:     Query → Embed → Top-K Chunks → LLM → Answer
Graph RAG:     Query → Keywords → Graph Traversal + Vector Search → Context → LLM → Answer
```

## Course Structure

| Part | Chapters | Focus |
|------|----------|-------|
| **I: RAG Foundations** | 1-3 | Embeddings, chunking, retrieval techniques |
| **II: Graph-Enhanced Retrieval** | 4-7 | Knowledge graphs, ingestion, multi-mode queries |
| **III: Production Infrastructure** | 8-10 | Storage backends, local dev, AWS deployment |
| **IV: Enterprise Hardening** | 11-13 | Security, batch ingestion, observability |

Each chapter includes:
- Conceptual explanation with diagrams
- Real EdgeQuake source code walkthrough
- A hands-on exercise with starter code, hints, and solutions
- A quiz to verify understanding

## The EdgeQuake Framework

EdgeQuake is a Rust workspace with 13 crates covering the full knowledge base lifecycle:

```text
edgequake-core          # Orchestrator
edgequake-pipeline      # Ingestion (chunk → extract → embed → store)
edgequake-query         # Multi-mode query engine (LightRAG-inspired)
edgequake-storage       # Storage traits (Graph, Vector, KV)
edgequake-storage-aws   # AWS adapters (S3 Vectors, Neptune, DynamoDB)
edgequake-llm           # LLM + embedding provider traits
edgequake-api           # Axum REST API
edgequake-batch         # Batch ingestion pipeline
edgequake-auth          # JWT + RBAC
```

## Who This Course Is For

This course is for **intermediate developers** who:
- Have basic Rust knowledge (ownership, traits, async)
- Understand what RAG is at a high level
- Want to build production-grade knowledge base systems
- Are curious about how knowledge graphs improve retrieval quality

If you're new to RAG entirely, start with the prerequisites chapter to get oriented.
