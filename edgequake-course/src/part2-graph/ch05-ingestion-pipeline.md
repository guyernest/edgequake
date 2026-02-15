# Chapter 5: The Ingestion Pipeline

> **"Garbage in, garbage out -- but with a good pipeline, you can turn
> raw documents into structured knowledge."**

The knowledge graph does not build itself. Raw documents -- PDFs, markdown
files, web pages -- must be processed through a multi-stage pipeline that
chunks text, extracts entities and relationships, normalizes names,
generates embeddings, and stores everything in the graph and vector
stores.

This chapter walks through EdgeQuake's ingestion pipeline from the
`edgequake-pipeline` crate. You will understand each stage, the
configuration that controls it, and the practical tradeoffs involved in
extraction quality versus cost.

## Learning Objectives

After completing this chapter you will be able to:

1. **Diagram the five-stage ingestion pipeline** -- Chunk, Extract,
   Normalize, Embed, Store -- and explain the purpose of each stage.
2. **Configure `PipelineConfig`** to tune chunking, extraction, and
   embedding settings for different document types.
3. **Explain how LLM-based entity extraction works** -- the prompt
   engineering, output parsing, and error handling strategies.
4. **Understand entity normalization** -- why "John Smith" and
   "J. Smith" must resolve to the same graph node.
5. **Evaluate gleaning tradeoffs** -- when a second-pass extraction
   is worth the additional LLM cost.

## Chapter Outline

| Section | Topic |
|---------|-------|
| [5.1 Pipeline Architecture](ch05-01-pipeline-architecture.md) | Five-stage pipeline, `PipelineConfig`, mermaid flowchart |
| [5.2 Entity Extraction](ch05-02-entity-extraction.md) | LLM-based extraction, prompt engineering, output parsing |
| [5.3 Entity Normalization](ch05-03-entity-normalization.md) | Name resolution, deduplication strategies |
| [5.4 Gleaning for Quality](ch05-04-gleaning.md) | Multi-pass extraction, cost-benefit analysis |

## Prerequisites

- Chapter 4 (Knowledge Graphs) -- understanding `GraphNode` and `GraphEdge`
- Chapter 2 (Embeddings and Chunking) -- basic chunking and embedding concepts
- Familiarity with LLM API calls (prompts, completions, tokens)

## Key Terminology

| Term | Definition |
|------|-----------|
| **Chunking** | Splitting a document into sized segments for processing |
| **Entity extraction** | Identifying named entities (people, organizations, concepts) from text |
| **Relationship extraction** | Identifying connections between entities |
| **Normalization** | Converting entity names to a canonical form for deduplication |
| **Gleaning** | A second-pass extraction to catch entities missed in the first pass |
| **Lineage** | Tracking which document and chunk produced each entity |

## The Big Picture

The ingestion pipeline transforms unstructured text into a structured
knowledge graph. Here is the journey of a single document:

```text
"John Smith is the CEO of Acme Corp. Acme Corp is based in New York."
                                    |
                              [ Chunk ]
                                    |
                              [ Extract ]
                                    |
              Entities: JOHN_SMITH, ACME_CORP, NEW_YORK
              Relationships: JOHN_SMITH --CEO_OF--> ACME_CORP
                             ACME_CORP --BASED_IN--> NEW_YORK
                                    |
                              [ Normalize ]
                                    |
              (Resolve aliases, uppercase names, merge duplicates)
                                    |
                              [ Embed ]
                                    |
              (Generate vectors for each entity and relationship)
                                    |
                              [ Store ]
                                    |
              Graph: 3 nodes + 2 edges
              Vector: 5 embeddings (3 entity + 2 relationship)
              KV: chunk content + metadata
```

Each stage has its own configuration, error handling, and performance
characteristics. Let us explore them in detail.

---

*Next: [5.1 Pipeline Architecture](ch05-01-pipeline-architecture.md)*

{{#quiz ../quizzes/ch05-ingestion-pipeline.toml}}
