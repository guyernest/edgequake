## 5.2 Entity Extraction

Entity extraction is where raw text becomes structured knowledge. An LLM
reads each chunk and identifies the entities (people, organizations,
concepts) and relationships (connections between entities) present in the
text. This is the most expensive, most error-prone, and most important
stage of the pipeline.

### The EntityExtractor Trait

All extraction implementations conform to a single trait:

```rust
#[async_trait]
pub trait EntityExtractor: Send + Sync {
    /// Extract entities and relationships from a text chunk.
    async fn extract(&self, chunk: &TextChunk) -> Result<ExtractionResult>;

    /// Extract from multiple chunks in batch.
    async fn extract_batch(&self, chunks: &[TextChunk]) -> Result<Vec<ExtractionResult>> {
        let mut results = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            results.push(self.extract(chunk).await?);
        }
        Ok(results)
    }

    /// Get the name of this extractor.
    fn name(&self) -> &str;

    /// Get the model name used by this extractor.
    fn model_name(&self) -> &str { "unknown" }
}
```

EdgeQuake ships three implementations:

| Extractor | Strategy | Use Case |
|-----------|----------|----------|
| `SimpleExtractor` | Regex patterns | Testing (no LLM required) |
| `LLMExtractor` | JSON-based LLM prompts | Development |
| `SOTAExtractor` | Tuple-based LLM prompts (LightRAG) | Production |

### Extraction Result Structure

Every extraction call returns an `ExtractionResult`:

```rust
pub struct ExtractionResult {
    /// Extracted entities.
    pub entities: Vec<ExtractedEntity>,
    /// Extracted relationships.
    pub relationships: Vec<ExtractedRelationship>,
    /// Which chunk this came from.
    pub source_chunk_id: String,
    /// Processing metadata (extractor name, model, timing).
    pub metadata: HashMap<String, serde_json::Value>,
    /// Token usage for cost tracking.
    pub input_tokens: usize,
    pub output_tokens: usize,
    /// How long extraction took.
    pub extraction_time_ms: u64,
}
```

Each entity carries rich metadata:

```rust
pub struct ExtractedEntity {
    /// Entity name (will be normalized later).
    pub name: String,
    /// Entity type (e.g., "PERSON", "ORGANIZATION", "CONCEPT").
    pub entity_type: String,
    /// Description of the entity in context.
    pub description: String,
    /// Importance score (0.0 to 1.0).
    pub importance: f32,
    /// Source text spans for provenance.
    pub source_spans: Vec<String>,
    /// Source chunk IDs for citation tracking.
    pub source_chunk_ids: Vec<String>,
}
```

Relationships connect two entities:

```rust
pub struct ExtractedRelationship {
    pub source: String,
    pub target: String,
    pub relation_type: String,
    pub description: String,
    pub weight: f32,
    pub keywords: Vec<String>,
    pub source_chunk_id: Option<String>,
}
```

The `keywords` field on relationships is especially important -- these
power Global mode retrieval in the query engine (Chapter 6).

### The Extraction Prompt

The core of LLM-based extraction is the prompt. Here is a simplified
version of what `LLMExtractor` sends:

```text
Extract entities and relationships from the following text.

## Entity Types
PERSON, ORGANIZATION, LOCATION, EVENT, CONCEPT, TECHNOLOGY, PRODUCT

## Output Format
Respond with valid JSON in this exact format:
{
  "entities": [
    {"name": "Entity Name", "type": "ENTITY_TYPE", "description": "Brief description"}
  ],
  "relationships": [
    {"source": "Source Entity", "target": "Target Entity",
     "type": "RELATIONSHIP_TYPE", "description": "Brief description"}
  ]
}

## Text to Analyze
[chunk content here]

## JSON Response
```

Several design choices in this prompt matter:

**Constrained entity types**: By listing valid types, we reduce
hallucinated categories. The list is configurable per domain:

```rust
let extractor = LLMExtractor::new(llm_provider)
    .with_entity_types(vec![
        "PERSON".into(),
        "ORGANIZATION".into(),
        "FINANCIAL_INSTRUMENT".into(),
        "LEGAL_ENTITY".into(),
    ]);
```

**JSON output**: Structured output enables reliable parsing. The
alternative (free-form text) requires complex regex extraction.

**Description field**: Asking for descriptions enriches the graph. Each
entity description becomes the node's searchable content.

### The SOTA Extractor (Production)

The `SOTAExtractor` uses a more sophisticated prompt system ported from
LightRAG. Instead of JSON, it uses a **tuple-based format** that is
more robust to LLM output variations:

```rust
pub struct SOTAExtractor<L: LLMProvider + ?Sized> {
    llm_provider: Arc<L>,
    entity_types: Vec<String>,
    prompts: EntityExtractionPrompts,
    parser: HybridExtractionParser,
    language: String,
}
```

The `HybridExtractionParser` tries multiple parsing strategies:

1. **Tuple parsing** -- the primary format (most robust)
2. **JSON fallback** -- if tuples fail, try JSON
3. **Partial recovery** -- extract whatever entities can be parsed

This resilience is critical because LLMs occasionally produce malformed
output, especially on complex or long chunks.

### Handling LLM Output Failures

LLM extraction can fail in several ways. EdgeQuake handles each:

#### Empty Response

The LLM returns nothing (timeout, crash, context exhaustion):

```rust
let trimmed_response = response.content.trim();
if trimmed_response.is_empty() {
    // Log detailed diagnostics
    tracing::error!(
        chunk_id = %chunk.id,
        chunk_size_kb = chunk_size_bytes / 1024,
        "LLM returned EMPTY response"
    );
    // Retry with backoff
    continue;
}
```

#### Truncated JSON

The response is cut off mid-JSON (exceeded `max_tokens`):

```rust
// Detect truncation via finish_reason
let was_truncated = response.finish_reason
    .as_ref()
    .map(|r| r.contains("length"))
    .unwrap_or(false);

if was_truncated {
    // Double max_tokens and retry
    current_max_tokens = (current_max_tokens * 2).min(32768);
    continue;
}
```

This adaptive `max_tokens` strategy is important because entity-dense
chunks (academic papers, legal documents) can produce outputs **6x larger
than the input** -- a 1500-token chunk might generate 9000 tokens of
entity JSON.

#### Malformed JSON

The LLM produces invalid JSON (missing brackets, extra commas):

```rust
fn extract_json_from_response(response: &str) -> String {
    let response = response.trim();

    // Try to find ```json block markers
    if let Some(start) = response.find("```json") {
        if let Some(end) = response[start + 7..].find("```") {
            return response[start + 7..start + 7 + end].trim().to_string();
        }
    }

    // Try to find JSON starting with {
    if let Some(start) = response.find('{') {
        if let Some(end) = response.rfind('}') {
            return response[start..=end].to_string();
        }
    }

    response.to_string()
}
```

The parser strips markdown code fences, finds the outermost JSON object,
and sanitizes control characters before attempting deserialization.

### Chunk Size and Extraction Quality

There is a direct tradeoff between chunk size and extraction quality:

| Chunk Size | Extraction Quality | Cost | Failure Rate |
|-----------|-------------------|------|-------------|
| Small (300-600 tokens) | High per chunk, may split entities | Higher (more chunks = more LLM calls) | Low |
| Medium (800-1200 tokens) | Good balance | Moderate | Low |
| Large (1500+ tokens) | May miss entities in long text | Lower | Higher (timeouts, truncation) |

EdgeQuake's `SOTAExtractor` enforces a maximum chunk size to prevent
quality degradation:

```rust
const MAX_CHUNK_TOKENS: usize = 1500;

if estimated_tokens > MAX_CHUNK_TOKENS {
    return Err(PipelineError::Validation(format!(
        "Chunk too large for LLM processing. Size: ~{} tokens, max: {}",
        estimated_tokens, MAX_CHUNK_TOKENS
    )));
}
```

### Example: End-to-End Extraction

Here is a complete example of extracting from a single chunk:

```rust
use edgequake_pipeline::extractor::{SOTAExtractor, EntityExtractor};
use edgequake_pipeline::chunker::TextChunk;
use std::sync::Arc;

// Create the extractor with an LLM provider
let extractor = SOTAExtractor::new(Arc::clone(&llm_provider))
    .with_entity_types(vec![
        "PERSON".into(),
        "ORGANIZATION".into(),
        "CONCEPT".into(),
    ])
    .with_language("English");

// Create a chunk
let chunk = TextChunk::new(
    "chunk-001",
    "Dr. Sarah Chen leads the quantum computing research team at MIT. \
     Her work on error correction has been funded by DARPA since 2021.",
    0, 0, 30,
);

// Extract
let result = extractor.extract(&chunk).await?;

// Inspect results
for entity in &result.entities {
    println!("Entity: {} ({}): {}", entity.name, entity.entity_type, entity.description);
}
// Entity: SARAH_CHEN (PERSON): Lead researcher in quantum computing at MIT
// Entity: MIT (ORGANIZATION): Massachusetts Institute of Technology
// Entity: DARPA (ORGANIZATION): Defense Advanced Research Projects Agency
// Entity: QUANTUM_COMPUTING (CONCEPT): Field of computing using quantum mechanics

for rel in &result.relationships {
    println!("{} --{}--> {}", rel.source, rel.relation_type, rel.target);
}
// SARAH_CHEN --LEADS_RESEARCH_AT--> MIT
// DARPA --FUNDS--> SARAH_CHEN
// SARAH_CHEN --RESEARCHES--> QUANTUM_COMPUTING

println!("Tokens: {} in, {} out, took {}ms",
    result.input_tokens, result.output_tokens, result.extraction_time_ms);
```

### Key Takeaways

- Entity extraction converts raw text into structured entities and
  relationships via LLM prompts.
- The `EntityExtractor` trait has three implementations: regex (testing),
  JSON-based (development), and tuple-based SOTA (production).
- Prompt engineering constrains entity types and output format for
  reliable parsing.
- Adaptive `max_tokens` with retry handles the common case of
  entity-dense chunks producing large outputs.
- Chunk size directly affects extraction quality -- 800-1200 tokens is
  the sweet spot.

---

*Next: [5.3 Entity Normalization](ch05-03-entity-normalization.md)*
