# Namespace Data Access Guide

> How to correctly query namespace-scoped data across all AWS cloud storage backends.
>
> **Audience:** MCP server developers, API integrators, SDK authors.

---

## Overview

Each EdgeQuake namespace is a fully isolated data silo spread across four AWS services. All data access is scoped by namespace — there is no cross-namespace query path.

```
┌─────────────────────────────────────────────────────────────────┐
│                    Namespace: "acquired"                         │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────────────┐  │
│  │  S3 Vectors   │  │   Neptune    │  │      DynamoDB         │  │
│  │              │  │   Graph      │  │                       │  │
│  │  Embeddings:  │  │  Entities    │  │  Chunk content        │  │
│  │  - chunks    │  │  Relationships│  │  Document metadata    │  │
│  │  - entities  │  │  (namespace  │  │  Pipeline state       │  │
│  │  - relations │  │   property)  │  │                       │  │
│  └──────────────┘  └──────────────┘  └───────────────────────┘  │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  Athena / Iceberg  (BM25 keyword search)                  │  │
│  │  Tables: acquired_postings, acquired_term_stats,          │  │
│  │          acquired_docs, acquired_corpus_stats              │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 1. S3 Vectors — Embedding Search

### Index Layout

Each namespace has a **single shared index** containing chunks, entities, and relationships:

| Resource | Format |
|---|---|
| **Bucket** | `edgequake-vectors-default-dev` (shared across namespaces) |
| **Index** | `{namespace}-embeddings` (e.g. `acquired-embeddings`) |
| **Dimension** | 3072 (`text-embedding-3-large`) |
| **Distance** | Cosine (lower distance = more similar) |

### Vector Types in the Shared Index

| Type | Key format | Metadata `type` | Example key |
|---|---|---|---|
| Chunk | `chunk-{doc_hash}-{index}` | `"chunk"` | `chunk-accd6a96-42` |
| Entity | `entity:{ENTITY_NAME}` | `"entity"` | `entity:APPLE` |
| Relationship | `rel:{SOURCE}:{TARGET}` | `"relationship"` | `rel:APPLE:TECHNOLOGY_COMPANY` |

### Critical: Always Use Metadata Filtering

The shared index mixes all vector types. Entity and relationship descriptions are semantically denser than chunk text, so they **dominate cosine similarity rankings**. A query without a type filter returns mostly entities/relationships and zero chunks, even with `top_k=100`.

**Always filter by type:**

```rust
// Rust SDK — using the VectorStorage trait
let chunks = vector_storage
    .query_by_type(&embedding, top_k, "chunk", None)
    .await?;

let entities = vector_storage
    .query_by_type(&embedding, top_k, "entity", None)
    .await?;
```

**S3 Vectors API — native metadata filter:**

```json
{
  "queryVector": { "float32": [0.1, 0.2, ...] },
  "topK": 40,
  "filter": { "type": { "$eq": "chunk" } },
  "returnDistance": true,
  "returnMetadata": true
}
```

**Score conversion:** S3 Vectors returns cosine **distance**. Convert to similarity score: `score = 1.0 - distance`.

### Vector Metadata Shapes

**Chunk vector:**
```json
{
  "chunk_id": "chunk-accd6a96-42",
  "document_id": "accd6a9653a0e910",
  "filename": "acquired_transcripts_all.txt",
  "type": "chunk"
}
```

**Entity vector:**
```json
{
  "entity_name": "APPLE",
  "entity_type": "ORGANIZATION",
  "type": "entity",
  "source_chunk_ids": ""
}
```

**Relationship vector:**
```json
{
  "source": "APPLE",
  "target": "TECHNOLOGY_COMPANY",
  "relation_type": "IS_A",
  "type": "relationship"
}
```

### Embedding Model

The index uses OpenAI `text-embedding-3-large` (3072 dimensions). Query embeddings **must** use the same model or the search will fail with a dimension mismatch.

Set via env var: `OPENAI_EMBEDDING_MODEL=text-embedding-3-large`.

### Roadmap

Splitting into separate indices (`{namespace}-chunk-embeddings`, `{namespace}-graph-embeddings`) is planned for a future milestone. Until then, always use the metadata filter approach.

---

## 2. Neptune Graph — Entities and Relationships

### Namespace Isolation: Property-Based

Neptune uses **bare labels** (`Entity`, `Relationship`) with a `namespace` **property** on every vertex and edge. This avoids label proliferation and works within Neptune's label cardinality constraints.

### Querying Entities

**List entities (paginated):**
```gremlin
g.V().hasLabel('Entity')
  .has('namespace', '{namespace}')
  .order().by('name', asc)
  .range(offset, offset + limit)
  .project('id', 'name', 'entity_type', 'description', 'degree')
    .by(T.id)
    .by('name')
    .by('entity_type')
    .by('description')
    .by(__.both().count())
```

**Search entities by name prefix:**
```gremlin
g.V().hasLabel('Entity')
  .has('namespace', '{namespace}')
  .has('name', TextP.containing('{SEARCH_TERM}'))
  .limit(limit)
```

**Entity vertex properties:**

| Property | Type | Description |
|---|---|---|
| `T.id` | String | `{ENTITY_NAME}` (uppercase, underscored) |
| `name` | String | Same as ID |
| `entity_type` | String | e.g. `ORGANIZATION`, `PERSON`, `CONCEPT` |
| `description` | String | LLM-generated entity description |
| `namespace` | String | Namespace slug |
| `source_document_id` | String | (optional) Document hash |
| `source_file_path` | String | (optional) Original filename |

### Querying Relationships

**Edges for a set of entities:**
```gremlin
g.V(entity_ids).bothE()
  .has('namespace', '{namespace}')
  .project('source', 'target', 'relation_type', 'weight', 'description')
    .by(__.outV().id())
    .by(__.inV().id())
    .by('relation_type')
    .by('weight')
    .by('description')
```

**Edge properties:**

| Property | Type | Description |
|---|---|---|
| `relation_type` | String | e.g. `FOUNDED`, `ACQUIRED`, `WORKS_AT` |
| `weight` | Double | Relationship strength (0.0–1.0) |
| `description` | String | LLM-generated relationship description |
| `namespace` | String | Namespace slug |

### Entity Name Format

Entity names are **UPPER_SNAKE_CASE** in Neptune (`APPLE_COMPUTER`). The API's display mapping converts to title case (`Apple Computer`) by replacing underscores with spaces and applying title casing. When querying Neptune directly, always use the uppercase form.

### Performance Notes

- **Degree queries are expensive.** `both().count()` per vertex triggers N+1 traversals. For listing, use `get_popular_nodes_with_degree` which batches degree computation.
- **`TextP.containing` is case-sensitive.** Uppercase search terms before querying.
- **Always include `.has('namespace', ...)`.** Without it, queries return data from all namespaces.

---

## 3. DynamoDB — Chunk Content and Metadata

### Table and Key Structure

| Setting | Value |
|---|---|
| **Table name** | `PIPELINE_STATE_TABLE` env var |
| **Partition key** | `namespace` (String) — the namespace slug |
| **Sort key** | `id` (String) — prefixed identifier |

### Key Formats

| Record type | Sort key format | Example |
|---|---|---|
| Chunk content | `chunk:{chunk_id}` | `chunk:chunk-accd6a96-42` |
| Document metadata | `__batch_docs__/{doc_hash}` | `__batch_docs__/accd6a9653a0e910` |
| Pipeline state | `__batch_job__{job_id}` | `__batch_job__abc123` |

Note the **`chunk:` prefix** on the sort key — this differs from the S3 Vectors key which has no prefix.

### Chunk Content Record

The `data` attribute is a JSON string:

```json
{
  "id": "chunk-accd6a96-42",
  "document_id": "accd6a9653a0e910",
  "filename": "acquired_transcripts_all.txt",
  "content": "Full chunk text (~4800 chars / ~1200 tokens)...",
  "index": 42,
  "token_count": 1157
}
```

### Hydration Flow

S3 Vectors stores **only metadata** for chunks (no text content). To get full text:

1. Query S3 Vectors with `filter: {"type": "chunk"}` → get `chunk_id` from metadata
2. Query DynamoDB: `namespace={slug}`, `id=chunk:{chunk_id}`
3. Parse the `data` JSON string → extract `content`, `document_id`, `index`

### Chunk Sizes

Chunks are configured at **1200 tokens** (~4800 characters) with 100-token overlap. The chunker respects sentence and paragraph boundaries using a priority-ordered separator list (`\n\n` > `\n` > `. ` > `! ` > `? ` > `;` > `,` > ` `).

---

## 4. Athena / Iceberg — BM25 Keyword Search

### Table Naming

All Iceberg tables are namespace-prefixed:

| Table | Schema |
|---|---|
| `{ns}_postings` | `chunk_id`, `term`, `tf` (term frequency) |
| `{ns}_term_stats` | `term`, `df` (document frequency) |
| `{ns}_docs` | `chunk_id`, `doc_len` (token count) |
| `{ns}_corpus_stats` | `total_docs`, `avg_doc_len` |

### BM25 Scoring

Okapi BM25 formula with `k1=1.5`, `b=0.75`. Athena runs the full scoring SQL query, returning chunk IDs ranked by BM25 score. Athena has ~6 second cold-start latency.

### S3 Data Location

BM25 Iceberg data is stored in: `s3://{bm25_bucket}/{namespace}/{table_name}/`

---

## 5. API Query Endpoint

### `POST /api/v1/ns/{namespace}/query`

```json
{
  "query": "What is Apple?",
  "mode": "mix",
  "retrieval_mode": "vector",
  "context_only": true,
  "enable_rerank": true
}
```

### Query Modes (orthogonal to retrieval mode)

| Mode | What it returns | Use when |
|---|---|---|
| `naive` | Chunks only (vector similarity) | Simple document search |
| `local` | Entities + relationships (graph neighborhood) | Entity-focused questions |
| `global` | Entities + relationships (high-level communities) | Broad topic questions |
| `hybrid` | Local + global merged (entities/relationships only) | Graph context without chunks |
| `mix` | Hybrid + direct chunk search | **Most complete** — graph context + document chunks |

### Retrieval Modes

| Mode | Backend | Latency | Best for |
|---|---|---|---|
| `vector` | S3 Vectors (cosine similarity) | ~2s | Semantic search (default) |
| `bm25` | Athena/Iceberg (keyword matching) | ~6s | Exact term matching |
| `hybrid` | RRF fusion of vector + BM25 | ~6s | Best recall |

### Response Shape

```json
{
  "answer": "...",
  "mode": "mix",
  "sources": [
    {
      "source_type": "chunk",
      "id": "chunk-accd6a96-3389",
      "score": 0.408,
      "snippet": "First 200 chars...",
      "content": "Full ~4800 char chunk text...",
      "document_id": "accd6a9653a0e910",
      "chunk_index": 3389,
      "reference_id": 1
    },
    {
      "source_type": "entity",
      "id": "APPLE",
      "score": 0.95,
      "snippet": "First 200 chars of description...",
      "content": "Full entity description...",
      "reference_id": 2,
      "document_id": "accd6a9653a0e910"
    }
  ],
  "stats": {
    "embedding_time_ms": 1108,
    "retrieval_time_ms": 6453,
    "total_time_ms": 8079,
    "sources_retrieved": 192
  },
  "reranked": true
}
```

- `snippet` — truncated to 200 characters (for listings)
- `content` — full untruncated text (for detail views)

---

## 6. Environment Variables Reference

| Variable | Required | Description | Example |
|---|---|---|---|
| `NEPTUNE_ENDPOINT` | Yes | Neptune cluster endpoint | `edgequake.cluster-xxx.neptune.amazonaws.com:8182` |
| `VECTOR_BUCKET` | Yes | S3 Vectors bucket name | `edgequake-vectors-default-dev` |
| `PIPELINE_STATE_TABLE` | Yes | DynamoDB table for chunks/state | `amplify-edgequake-...-PipelineStateTable-...` |
| `NAMESPACE_REGISTRY_TABLE` | Yes | DynamoDB table for namespace registry | `amplify-edgequake-...-NamespaceTable-...` |
| `OPENAI_API_KEY` | Yes | OpenAI API key for embeddings | `sk-...` |
| `OPENAI_EMBEDDING_MODEL` | Yes | Must match indexed dimension | `text-embedding-3-large` |
| `ATHENA_BM25_DATABASE` | Optional | Athena database for BM25 | `edgequake_bm25` |
| `BM25_S3_BUCKET` | Optional | S3 bucket for BM25 Iceberg data | `edgequake-bm25-...` |
| `ATHENA_WORKGROUP` | Optional | Athena workgroup | `primary` |

---

## 7. Common Pitfalls

1. **Never query S3 Vectors without a type filter.** Entity/relationship vectors crowd out chunks in cosine similarity. Always use `query_by_type` or the native `filter` parameter.

2. **Chunk vector keys have no prefix; DynamoDB keys do.** S3 Vectors key: `chunk-accd6a96-42`. DynamoDB sort key: `chunk:chunk-accd6a96-42`.

3. **Entity names are UPPER_SNAKE_CASE.** Neptune stores `APPLE_COMPUTER`, not `Apple Computer`. Convert for display, but query with uppercase.

4. **Neptune queries must include `namespace` property filter.** Without `.has('namespace', '{ns}')`, queries return cross-namespace data.

5. **Embedding dimension must match.** The index uses 3072d (`text-embedding-3-large`). A 1536d query vector (`text-embedding-3-small`) returns `ValidationException`.

6. **DynamoDB chunk data is a JSON string.** The `data` attribute is a serialized JSON string, not a native DynamoDB map. Parse it after retrieval.

7. **BM25 has cold-start latency.** First Athena query per session takes ~6 seconds. Subsequent queries are faster.

8. **S3 Vectors `top_k` caps at 100.** Plan pagination or narrower queries if you need more results.

9. **`PIPELINE_STATE_TABLE` vs `DYNAMODB_TABLE`.** The batch pipeline stores chunks in the table referenced by `PIPELINE_STATE_TABLE`. The API's `DYNAMODB_TABLE` env var may point to a different table. Always use `PIPELINE_STATE_TABLE` for chunk lookups.
