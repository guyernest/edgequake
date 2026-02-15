## 6.3 Token Budgeting

After retrieval, the query engine holds a collection of entities,
relationships, and chunks. This context must be assembled into a prompt
for the LLM -- but LLMs have fixed context windows. GPT-4 Turbo accepts
128K tokens; Claude supports 200K. Exceeding the limit causes API errors
or silent truncation. Even staying within the limit, stuffing too much
context degrades answer quality because the LLM's attention is diluted.

Token budgeting is the process of **fitting the most relevant context
into a fixed token budget** while preserving information priority.

### The TruncationConfig

EdgeQuake's token budget is controlled by `TruncationConfig`:

```rust
pub struct TruncationConfig {
    /// Maximum tokens for entity descriptions.
    pub max_entity_tokens: usize,
    /// Maximum tokens for relationship descriptions.
    pub max_relation_tokens: usize,
    /// Maximum total tokens for all context.
    pub max_total_tokens: usize,
}
```

The default allocation follows the LightRAG paper's strategy:

```rust
impl Default for TruncationConfig {
    fn default() -> Self {
        Self {
            max_entity_tokens: 10_000,   // 33% for entities
            max_relation_tokens: 10_000, // 33% for relationships
            max_total_tokens: 30_000,    // Total budget
        }
    }
}
```

This gives equal weight to entities, relationships, and chunks (the
chunk budget is the remainder: `total - entities - relationships`).

### Why 30,000 Tokens?

The default of 30,000 tokens is chosen based on the LightRAG paper's
findings:

```text
Total LLM context: 128,000 tokens (GPT-4 Turbo)
├── System prompt:       ~500 tokens
├── Conversation history: ~2,000 tokens
├── Retrieved context:    30,000 tokens   <-- our budget
├── Query:               ~200 tokens
└── Reserved for output:  ~4,000 tokens
                         ──────────
                         ~36,700 tokens used
                         ~91,300 tokens spare
```

Using 30,000 tokens for context leaves substantial headroom. Filling the
full context window often degrades quality because the LLM must attend to
too much information (the "lost in the middle" phenomenon).

### The Budget Allocation Strategy

Context is allocated in priority order:

```text
Token Budget: 30,000 tokens
├── Entities:      10,000 tokens (33%)  -- Graph context (priority)
│   Entity: SARAH_CHEN (PERSON)
│   Lead quantum computing researcher at MIT...
│   ──────
│   Entity: MIT (ORGANIZATION)
│   Massachusetts Institute of Technology...
│
├── Relationships: 10,000 tokens (33%)  -- Graph context (priority)
│   SARAH_CHEN -> MIT (WORKS_AT)
│   Sarah Chen has been a researcher at MIT since...
│   ──────
│   DARPA -> SARAH_CHEN (FUNDS)
│   DARPA has funded Sarah Chen's research...
│
└── Chunks:        10,000 tokens (33%)  -- Primary evidence
    Chunk from doc-001:
    "Dr. Sarah Chen's groundbreaking work on quantum error
     correction has attracted significant funding..."
```

Why entities and relationships get priority over chunks (per **BR0102**):

1. **Pre-summarized**: Entity descriptions are already concise summaries,
   making them token-efficient.
2. **Deduplicated**: Graph nodes aggregate information from multiple
   documents, eliminating redundancy.
3. **Structured**: Relationships encode explicit connections that chunks
   can only imply.

### Truncation Functions

Three functions handle truncation for each context type:

```rust
/// Truncate entities to fit within token limit.
pub fn truncate_entities(
    entities: Vec<RetrievedEntity>,
    max_tokens: usize,
    tokenizer: &dyn Tokenizer,
) -> Vec<RetrievedEntity> {
    let mut result = Vec::new();
    let mut total_tokens = 0;

    for entity in entities {
        let formatted = format!(
            "Entity: {} ({})\n{}\n",
            entity.name, entity.entity_type, entity.description
        );
        let entity_tokens = tokenizer.count_tokens(&formatted);

        if total_tokens + entity_tokens <= max_tokens {
            result.push(entity);
            total_tokens += entity_tokens;
        } else {
            break; // Budget exhausted
        }
    }

    result
}
```

The same pattern applies to `truncate_relationships` and
`truncate_chunks`. Entities are assumed to be **pre-sorted by
relevance** (score from vector search, or degree from graph), so
truncation always preserves the most relevant items.

### The balance_context Function

The master function applies individual limits, then checks the total:

```rust
pub fn balance_context(
    entities: Vec<RetrievedEntity>,
    relationships: Vec<RetrievedRelationship>,
    chunks: Vec<RetrievedChunk>,
    config: &TruncationConfig,
    tokenizer: &dyn Tokenizer,
) -> (Vec<RetrievedEntity>, Vec<RetrievedRelationship>, Vec<RetrievedChunk>) {
    // Step 1: Apply individual limits
    let entities = truncate_entities(entities, config.max_entity_tokens, tokenizer);
    let relationships = truncate_relationships(
        relationships, config.max_relation_tokens, tokenizer
    );
    let chunk_budget = config.max_total_tokens
        .saturating_sub(config.max_entity_tokens)
        .saturating_sub(config.max_relation_tokens);
    let chunks = truncate_chunks(chunks, chunk_budget, tokenizer);

    // Step 2: Check total
    let total = count_tokens(&entities, &relationships, &chunks, tokenizer);

    if total <= config.max_total_tokens {
        return (entities, relationships, chunks);
    }

    // Step 3: Proportional reduction if still over budget
    let ratio = config.max_total_tokens as f32 / total as f32;
    let new_entity_count = (entities.len() as f32 * ratio).ceil() as usize;
    let new_rel_count = (relationships.len() as f32 * ratio).ceil() as usize;
    let new_chunk_count = (chunks.len() as f32 * ratio).ceil() as usize;

    // Truncate proportionally, keeping at least 1 entity
    (
        entities[..new_entity_count.max(1)].to_vec(),
        relationships[..new_rel_count].to_vec(),
        chunks[..new_chunk_count].to_vec(),
    )
}
```

The two-step approach (individual limits then proportional reduction)
handles the case where individual limits are generous but the total
still exceeds the budget.

### The Tokenizer Trait

Token counting requires a tokenizer. EdgeQuake defines a simple trait:

```rust
pub trait Tokenizer: Send + Sync {
    fn count_tokens(&self, text: &str) -> usize;
}
```

With two implementations:

```rust
/// Simple word-based tokenizer (fast, approximate).
pub struct SimpleTokenizer;

impl Tokenizer for SimpleTokenizer {
    fn count_tokens(&self, text: &str) -> usize {
        // Approximate: 1 token per 4 characters (English average)
        text.len() / 4
    }
}

/// Mock tokenizer for testing with configurable rate.
pub struct MockTokenizer {
    chars_per_token: f32,
}
```

For production use, the `SimpleTokenizer`'s 4-characters-per-token
approximation is sufficient. The exact count does not need to be
precise -- a 5-10% margin of error is acceptable because the budget
already includes headroom.

### Configuring Token Budgets

Different deployments may need different budgets:

```rust
// Default: 30K total, balanced allocation
let default = TruncationConfig::default();

// For smaller models (e.g., 8K context window)
let small_model = TruncationConfig {
    max_entity_tokens: 2_000,
    max_relation_tokens: 2_000,
    max_total_tokens: 6_000,
};

// For graph-heavy workloads (prioritize graph context)
let graph_heavy = TruncationConfig {
    max_entity_tokens: 15_000,
    max_relation_tokens: 10_000,
    max_total_tokens: 30_000,
    // Leaves only 5K for chunks
};

// For naive-heavy workloads (prioritize raw chunks)
let chunk_heavy = TruncationConfig {
    max_entity_tokens: 5_000,
    max_relation_tokens: 5_000,
    max_total_tokens: 30_000,
    // Leaves 20K for chunks
};
```

### Mode-Specific Token Allocation

Different query modes produce different context mixes:

| Mode | Entities | Relationships | Chunks |
|------|---------|---------------|--------|
| **Naive** | 0 | 0 | 100% of budget |
| **Local** | High | Medium | Low |
| **Global** | Medium | High | Low |
| **Hybrid** | High | High | Medium |
| **Mix** | Configurable | Configurable | Configurable |

In Naive mode, the entire budget goes to chunks because there is no
graph context. In Hybrid mode, the budget is split across all three
types.

### The Lost-in-the-Middle Problem

Research has shown that LLMs attend most strongly to context at the
**beginning** and **end** of the prompt, with attention dropping in the
middle. This is called the "lost-in-the-middle" effect.

EdgeQuake mitigates this by:

1. **Sorting by relevance**: Most relevant entities, relationships, and
   chunks come first.
2. **Limiting total context**: The 30K default prevents attention
   dilution.
3. **Prioritizing graph context**: Pre-summarized entity descriptions are
   more information-dense than raw chunks, so the same token budget
   carries more useful information.

### Practical Example

Here is how token budgeting works end-to-end:

```rust
use edgequake_query::truncation::{balance_context, TruncationConfig};
use edgequake_query::tokenizer::SimpleTokenizer;

let config = TruncationConfig::default();
let tokenizer = SimpleTokenizer;

// After retrieval: 15 entities, 20 relationships, 8 chunks
let (entities, relationships, chunks) = balance_context(
    retrieved_entities,      // 15 items
    retrieved_relationships, // 20 items
    retrieved_chunks,        // 8 items
    &config,
    &tokenizer,
);

// After budgeting: maybe 10 entities, 12 relationships, 5 chunks
// Total fits within 30,000 tokens
println!("Context: {} entities, {} rels, {} chunks",
    entities.len(), relationships.len(), chunks.len());
```

### Key Takeaways

- Token budgeting ensures retrieved context fits within the LLM's
  context window.
- The default budget of 30,000 tokens is split equally across entities,
  relationships, and chunks.
- Graph context (entities + relationships) is prioritized over raw
  chunks because it is more information-dense.
- Truncation preserves the most relevant items by respecting the
  pre-sorted order from retrieval.
- The `balance_context` function applies individual limits first, then
  proportional reduction if the total still exceeds the budget.
- Different query modes and model sizes call for different budget
  configurations.

---

*Next: [Chapter 7: The EdgeQuake Orchestrator](ch07-orchestrator.md)*
