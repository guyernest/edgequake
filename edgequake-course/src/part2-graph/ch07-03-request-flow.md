## 7.3 End-to-End Request Flow

This section traces a single query from the moment it arrives as an HTTP
request through every system component until the response is sent back
to the client. Understanding this flow ties together everything from
Part II.

### The Query

A user sends:

```http
POST /api/v1/query HTTP/1.1
Content-Type: application/json
Authorization: Bearer eyJhbGciOiJIUzI1NiIs...

{
  "query": "What financial connections exist between Acme Corp and John Smith?",
  "mode": "hybrid",
  "top_k": 10
}
```

### Sequence Diagram

```mermaid
sequenceDiagram
    participant C as Client
    participant A as Axum API
    participant MW as Middleware
    participant O as Orchestrator
    participant KE as Keyword Extractor
    participant VS as Vector Storage
    participant GS as Graph Storage
    participant TB as Token Budget
    participant LLM as LLM Provider

    C->>A: POST /api/v1/query
    A->>MW: Authenticate + Extract Tenant
    MW-->>A: TenantContext (tenant_id, workspace_id)

    A->>O: query(request, tenant_context)

    Note over O: Step 1: Keyword Extraction
    O->>KE: extract_extended(query)
    KE->>LLM: "Extract keywords from query..."
    LLM-->>KE: {high: ["financial connections"], low: ["Acme Corp", "John Smith"]}
    KE-->>O: ExtractedKeywords

    Note over O: Step 2a: Local Retrieval (low-level keywords)
    O->>VS: search("Acme Corp" embedding, top_k=10)
    VS-->>O: Entity matches: [ACME_CORP, ACME_HOLDINGS, ...]
    O->>GS: get_neighbors("ACME_CORP", depth=2)
    GS-->>O: [JOHN_SMITH, BANK_X, PROJECT_Y, ...]
    O->>GS: get_node_edges("ACME_CORP")
    GS-->>O: [ACME→JOHN_SMITH, ACME→BANK_X, ...]

    O->>VS: search("John Smith" embedding, top_k=10)
    VS-->>O: Entity matches: [JOHN_SMITH, SMITH_ENTERPRISES, ...]
    O->>GS: get_neighbors("JOHN_SMITH", depth=2)
    GS-->>O: [ACME_CORP, BANK_X, OFFSHORE_LLC, ...]

    Note over O: Step 2b: Global Retrieval (high-level keywords)
    O->>VS: search("financial connections" embedding, top_k=10)
    VS-->>O: Relationship matches: [ACME→BANK_X, JOHN→OFFSHORE, ...]

    Note over O: Step 3: Context Building
    O->>O: Merge Local + Global results
    O->>O: Deduplicate entities and relationships

    Note over O: Step 4: Token Budgeting
    O->>TB: balance_context(entities, relationships, chunks)
    TB-->>O: Truncated context (fits within 30K tokens)

    Note over O: Step 5: LLM Generation
    O->>LLM: System prompt + context + query
    LLM-->>O: "Based on the knowledge graph, the financial connections..."

    O-->>A: QueryResponse
    A-->>C: 200 OK (JSON response)
```

### Step-by-Step Walkthrough

#### Step 0: Middleware Processing

The request passes through the middleware stack:

1. **CORS check** -- validates the origin header.
2. **Request logging** -- logs method, path, and request ID.
3. **Authentication** -- validates the JWT token, extracts user claims.
4. **Tenant extraction** -- reads `tenant_id` and `workspace_id` from
   JWT claims (or headers).

```rust
// After middleware, the handler receives:
pub async fn execute_query(
    State(state): State<AppState>,
    tenant: TenantContext,     // workspace_id = "ws-001"
    Json(request): Json<QueryRequest>,
) -> ApiResult<Json<QueryResponse>> { ... }
```

#### Step 1: Keyword Extraction

The query engine calls the keyword extractor (Section 6.1):

```text
Input:  "What financial connections exist between Acme Corp and John Smith?"

Output:
  high_level: ["financial connections", "business relationships"]
  low_level:  ["Acme Corp", "John Smith"]
  intent:     Analytical
```

If keywords are cached (same query within 24h), this step returns
immediately. Otherwise, an LLM call is made.

#### Step 2a: Local Retrieval

For each **low-level keyword**, the engine:

1. **Embeds the keyword** and searches the entity vector store.
2. **Fetches graph neighbors** for each matched entity.
3. **Collects entity descriptions** and relationship descriptions.

For "Acme Corp":
```text
Vector search → ACME_CORP (score: 0.95)
Graph neighbors (depth 2):
  JOHN_SMITH (CEO_OF, weight: 0.9)
  BANK_X (ACCOUNT_AT, weight: 0.7)
  PROJECT_Y (FUNDS, weight: 0.6)
  OFFSHORE_LLC (SUBSIDIARY_OF, weight: 0.5)
```

For "John Smith":
```text
Vector search → JOHN_SMITH (score: 0.97)
Graph neighbors (depth 2):
  ACME_CORP (CEO_OF, weight: 0.9)
  BANK_X (AUTHORIZED_SIGNATORY, weight: 0.8)
  OFFSHORE_LLC (BENEFICIAL_OWNER, weight: 0.7)
```

Notice that graph traversal discovers the `OFFSHORE_LLC` connection
that pure vector search would likely miss.

#### Step 2b: Global Retrieval

For each **high-level keyword**, the engine searches the relationship
vector store:

```text
Vector search for "financial connections":
  ACME_CORP → BANK_X (ACCOUNT_AT): "Acme Corp holds accounts at Bank X"
  JOHN_SMITH → OFFSHORE_LLC (BENEFICIAL_OWNER): "John Smith is beneficial owner"
  ACME_CORP → PROJECT_Y (FUNDED): "Acme Corp funded Project Y through..."
```

These relationships provide the thematic context that Local mode may
not emphasize.

#### Step 3: Context Building

The engine merges Local and Global results:

**Entities (deduplicated)**:
- ACME_CORP: "A multinational technology company headquartered in..."
- JOHN_SMITH: "CEO of Acme Corp, authorized signatory at Bank X..."
- BANK_X: "International bank providing corporate accounts..."
- OFFSHORE_LLC: "Shell company incorporated in..."
- PROJECT_Y: "R&D initiative funded by Acme Corp..."

**Relationships**:
- JOHN_SMITH --CEO_OF--> ACME_CORP
- ACME_CORP --ACCOUNT_AT--> BANK_X
- JOHN_SMITH --AUTHORIZED_SIGNATORY--> BANK_X
- JOHN_SMITH --BENEFICIAL_OWNER--> OFFSHORE_LLC
- ACME_CORP --SUBSIDIARY_OF--> OFFSHORE_LLC
- ACME_CORP --FUNDED--> PROJECT_Y

**Chunks** (from naive vector search on the original query):
- Chunk from doc-023: "The corporate structure of Acme Corp..."
- Chunk from doc-089: "Financial disclosures reveal that John Smith..."

#### Step 4: Token Budgeting

The `balance_context` function (Section 6.3) fits everything within
30,000 tokens:

```text
Before budgeting:
  5 entities (est. 2,500 tokens)
  6 relationships (est. 1,800 tokens)
  2 chunks (est. 3,000 tokens)
  Total: ~7,300 tokens

After budgeting:
  All items fit within budget -- no truncation needed.
```

In this case, the context is well within budget. For larger result sets,
the least-relevant items would be truncated.

#### Step 5: LLM Generation

The engine assembles the final prompt:

```text
[System Prompt]
You are a helpful assistant. Answer the query based on the provided
knowledge graph context and document chunks. Cite specific entities
and relationships in your answer.

[Knowledge Graph Context]
## Entities
- ACME_CORP (ORGANIZATION): A multinational technology company...
- JOHN_SMITH (PERSON): CEO of Acme Corp, authorized signatory...
- BANK_X (ORGANIZATION): International bank...
- OFFSHORE_LLC (ORGANIZATION): Shell company...

## Relationships
- JOHN_SMITH --CEO_OF--> ACME_CORP: John Smith has been CEO since...
- ACME_CORP --ACCOUNT_AT--> BANK_X: Corporate accounts maintained...
- JOHN_SMITH --AUTHORIZED_SIGNATORY--> BANK_X: Smith has signing...
- JOHN_SMITH --BENEFICIAL_OWNER--> OFFSHORE_LLC: Beneficial ownership...
- ACME_CORP --SUBSIDIARY_OF--> OFFSHORE_LLC: Offshore LLC is a...

## Document Excerpts
[Chunk content from doc-023]
[Chunk content from doc-089]

[Query]
What financial connections exist between Acme Corp and John Smith?
```

The LLM generates a comprehensive answer that weaves together entity
descriptions, relationship context, and chunk evidence:

```text
Based on the knowledge graph, there are several financial connections
between Acme Corp and John Smith:

1. **Direct employment**: John Smith serves as CEO of Acme Corp.
2. **Shared banking**: Both Acme Corp and John Smith are connected to
   Bank X -- Acme Corp holds corporate accounts there, and Smith is an
   authorized signatory on those accounts.
3. **Offshore structure**: John Smith is the beneficial owner of
   Offshore LLC, which is a subsidiary of Acme Corp, suggesting a
   layered corporate ownership structure.
4. **Project funding**: Acme Corp funded Project Y, though John Smith's
   direct involvement in this project is not explicitly documented.
```

This answer would be impossible with pure vector search -- the
connection through Bank X and the Offshore LLC relationship require
graph traversal to discover.

#### Step 6: Response Serialization

The handler serializes the response:

```json
{
  "response": "Based on the knowledge graph, there are several...",
  "mode": "hybrid",
  "sources": [
    {"chunk_id": "doc-023-c4", "score": 0.92},
    {"chunk_id": "doc-089-c1", "score": 0.87}
  ],
  "stats": {
    "entities_used": 5,
    "relationships_used": 6,
    "chunks_used": 2,
    "total_tokens": 7300
  }
}
```

### Timing Breakdown

For a typical query on a moderately sized knowledge base:

| Step | Duration | Notes |
|------|----------|-------|
| Middleware | 1-5ms | JWT validation, tenant extraction |
| Keyword extraction | 200-500ms | LLM call (or <1ms if cached) |
| Vector search | 10-50ms | Per keyword, depends on backend |
| Graph traversal | 5-20ms | Per entity, depends on depth |
| Context building | 1-5ms | CPU-only: merge and deduplicate |
| Token budgeting | 1-2ms | CPU-only: count and truncate |
| LLM generation | 500-3000ms | Depends on model and output length |
| **Total** | **~1-4 seconds** | Dominated by LLM calls |

The two LLM calls (keyword extraction + generation) dominate latency.
Keyword caching eliminates one of them for repeat queries. Streaming
responses (Section 7.2) provide first-token latency of ~500ms even
when total generation takes several seconds.

### Key Takeaways

- A query passes through middleware (auth, tenant), keyword extraction,
  mode-specific retrieval, context building, token budgeting, and LLM
  generation.
- Hybrid mode runs Local and Global retrieval in parallel for the most
  comprehensive context.
- Graph traversal discovers connections that pure vector search cannot
  find (multi-hop paths through intermediate entities).
- Token budgeting ensures the assembled context fits within the LLM's
  window while preserving the most relevant items.
- Total query latency is 1-4 seconds, dominated by LLM calls.
  Keyword caching and streaming responses reduce perceived latency.

---

*Next: [Chapter 8: Pluggable Storage Backends](../part3-production/ch08-storage-backends.md)*
