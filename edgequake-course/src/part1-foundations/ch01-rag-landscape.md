# Chapter 1: The RAG Landscape

> **"The best LLM is the one that knows when to look things up."**

Retrieval-Augmented Generation has become the dominant pattern for building
production AI systems that need to reason over private, domain-specific, or
rapidly changing data. But the landscape is broader and more nuanced than most
introductions suggest.

In this chapter we survey the full RAG landscape -- from the fundamental
problem it solves, through the anatomy of a production pipeline, to a
working naive implementation in Rust. By the end, you will have both the
conceptual framework and a running code example to build on throughout the
rest of this course.

## Learning Objectives

After completing this chapter you will be able to:

1. **Articulate why RAG exists** -- the specific LLM failure modes it
   addresses and where it outperforms fine-tuning.
2. **Diagram a complete RAG pipeline** -- ingestion, retrieval, and
   generation stages with their sub-components.
3. **Implement naive RAG in Rust** -- a minimal but functional
   vector-similarity retrieval system using only the standard library and
   basic linear algebra.
4. **Identify the limitations** of naive RAG that motivate the advanced
   techniques in Chapters 2 and 3.

## Chapter Outline

| Section | Topic |
|---------|-------|
| [1.1 Why RAG Matters](ch01-01-why-rag.md) | LLM limitations, RAG vs fine-tuning, real-world use cases |
| [1.2 Anatomy of a RAG Pipeline](ch01-02-rag-pipeline.md) | Ingestion, retrieval, generation stages |
| [1.3 Naive RAG in Rust](ch01-03-naive-rag-rust.md) | Cosine similarity, in-memory vector store, simple retrieval |

## Prerequisites

- Comfortable reading and writing Rust (ownership, traits, iterators)
- Basic understanding of what LLMs do (token prediction, context windows)
- Familiarity with the concept of vector embeddings (we will deepen this in
  Chapter 2)

## Key Terminology

| Term | Definition |
|------|-----------|
| **RAG** | Retrieval-Augmented Generation -- grounding LLM output in retrieved evidence |
| **Embedding** | A dense vector representation of text in a high-dimensional space |
| **Chunk** | A segment of a source document sized for embedding and retrieval |
| **Top-k retrieval** | Returning the *k* most similar vectors to a query embedding |
| **Context window** | The maximum number of tokens an LLM can process in a single call |

## What We Are Building

Throughout Part I you will build increasingly sophisticated retrieval
systems, culminating in a hybrid search pipeline. In Part II we layer graph
structure on top, and in Part III we integrate everything into the EdgeQuake
framework for production deployment.

```text
Part I: Foundations          Part II: Graphs          Part III: EdgeQuake
───────────────────         ──────────────          ─────────────────
Naive RAG ──────────>  Graph-Enhanced RAG ──────>  Production System
Vector search            Knowledge graphs           AWS deployment
Chunking/embedding       Entity extraction          Batch ingestion
Hybrid retrieval         Graph traversal            S3 Vectors
```

Let us begin by understanding *why* RAG exists in the first place.

---

*Next: [1.1 Why RAG Matters](ch01-01-why-rag.md)*

{{#quiz ../quizzes/ch01-rag-landscape.toml}}
