## 6.1 Keyword Extraction

Before retrieval begins, the query engine must understand what the user
is asking. Keyword extraction converts a natural-language query into
two sets of structured keywords that drive the retrieval process.

### Why Two Levels of Keywords?

The LightRAG paper's key insight is that queries contain two types of
information:

- **High-level keywords** are themes, concepts, and abstract topics.
  They match against relationship descriptions and community summaries.
  Example: "financial fraud", "climate regulation", "drug interactions".

- **Low-level keywords** are specific entities and concrete terms.
  They match against entity descriptions in the graph.
  Example: "CERN", "Sarah Chen", "GPT-4", "aspirin".

By extracting both levels, the engine can route each to the most
effective retrieval path.

### The KeywordExtractor Trait

```rust
#[async_trait]
pub trait KeywordExtractor: Send + Sync {
    /// Extract simple keywords from a query.
    async fn extract(&self, query: &str) -> Result<Keywords>;

    /// Extract keywords with full metadata (intent, cache key, timestamp).
    async fn extract_extended(&self, query: &str) -> Result<ExtractedKeywords> {
        let keywords = self.extract(query).await?;
        let intent = QueryIntent::classify_heuristic(query);
        Ok(ExtractedKeywords::new(
            keywords.high_level,
            keywords.low_level,
            intent,
        ))
    }

    /// Compute a cache key for this query.
    fn cache_key(&self, query: &str) -> String {
        // SHA-256 hash of the query
        // ...
    }
}
```

The trait has three implementations:

| Implementation | Strategy | Use Case |
|---------------|----------|----------|
| `LLMKeywordExtractor` | LLM prompt | Production (accurate) |
| `MockKeywordExtractor` | Returns fixed keywords | Testing |
| `CachedKeywordExtractor` | Wraps another extractor with cache | Production (cost savings) |

### The Keywords Types

The simple form carries just the two keyword lists:

```rust
pub struct Keywords {
    /// Themes, concepts, abstract topics.
    pub high_level: Vec<String>,
    /// Entities, specific terms.
    pub low_level: Vec<String>,
}
```

The extended form adds metadata for caching and adaptive retrieval:

```rust
pub struct ExtractedKeywords {
    pub high_level: Vec<String>,
    pub low_level: Vec<String>,
    /// Classified query intent (Factual, Exploratory, Comparative, ...).
    pub query_intent: QueryIntent,
    /// Cache key (hash of query + mode).
    pub cache_key: String,
    /// When keywords were extracted.
    pub extracted_at: chrono::DateTime<chrono::Utc>,
}
```

### LLM-Based Keyword Extraction

The `LLMKeywordExtractor` sends a prompt to the LLM asking it to
classify the query and extract both keyword levels:

```text
Given the following query, identify:
1. HIGH-LEVEL KEYWORDS: Themes, concepts, and abstract topics
2. LOW-LEVEL KEYWORDS: Specific entities, names, and concrete terms

Query: "How did financial institutions contribute to the 2008 crisis?"

Respond with JSON:
{
  "high_level": ["financial regulation", "systemic risk", "economic crisis"],
  "low_level": ["2008 financial crisis", "investment banks", "subprime mortgages"]
}
```

Example extraction for various query types:

| Query | High-Level | Low-Level |
|-------|-----------|----------|
| "What does CERN do?" | ["particle physics", "scientific research"] | ["CERN"] |
| "Compare climate policies in EU and US" | ["climate policy", "environmental regulation"] | ["European Union", "United States"] |
| "How are drug interactions managed?" | ["pharmacology", "drug safety"] | ["drug interactions"] |

### Query Intent Classification

The `QueryIntent` enum helps the engine adapt its retrieval strategy:

```rust
pub enum QueryIntent {
    /// Direct factual question ("What is X?")
    Factual,
    /// Open-ended exploration ("Tell me about...")
    Exploratory,
    /// Compare two or more things
    Comparative,
    /// Analyze causes or effects
    Analytical,
    /// Summarize a topic
    Summary,
}
```

Intent classification uses both LLM analysis and heuristic fallback:

```rust
impl QueryIntent {
    pub fn classify_heuristic(query: &str) -> Self {
        let lower = query.to_lowercase();
        if lower.starts_with("what is") || lower.starts_with("who is") {
            Self::Factual
        } else if lower.contains("compare") || lower.contains("difference") {
            Self::Comparative
        } else if lower.contains("why") || lower.contains("how does") {
            Self::Analytical
        } else if lower.contains("summarize") || lower.contains("overview") {
            Self::Summary
        } else {
            Self::Exploratory
        }
    }
}
```

The query intent influences mode selection (Section 6.2) and token
budget allocation (Section 6.3).

### Keyword Caching

Keyword extraction requires an LLM call, which is expensive and adds
latency. The `CachedKeywordExtractor` wraps any extractor with an
in-memory cache:

```rust
pub struct CachedKeywordExtractor<E: KeywordExtractor> {
    inner: E,
    cache: Arc<InMemoryKeywordCache>,
}

pub struct InMemoryKeywordCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
    ttl: Duration,
}

struct CacheEntry {
    keywords: ExtractedKeywords,
    expires_at: Instant,
}
```

The cache uses SHA-256 hashes of the query as keys, with a configurable
TTL (default: 24 hours per **BR0106**):

```rust
let cache = InMemoryKeywordCache::new(Duration::from_secs(86400)); // 24h TTL

let cached_extractor = CachedKeywordExtractor::new(
    LLMKeywordExtractor::new(llm_provider),
    Arc::new(cache),
);

// First call: LLM extraction (slow)
let kw1 = cached_extractor.extract_extended("What is CERN?").await?;

// Second call with same query: cache hit (fast)
let kw2 = cached_extractor.extract_extended("What is CERN?").await?;
```

In practice, many users ask similar or identical questions. The cache
eliminates redundant LLM calls and provides sub-millisecond keyword
retrieval for repeat queries.

### How Keywords Drive Retrieval

The extracted keywords directly control which vector stores are
searched:

```rust
match query_mode {
    QueryMode::Naive => {
        // Ignore keywords, embed the raw query
        let results = vector_store.search(&query_embedding, top_k).await?;
    }
    QueryMode::Local => {
        // Use low-level keywords to find entities
        for keyword in &keywords.low_level {
            let embedding = embed(keyword).await?;
            let entities = entity_vector_store.search(&embedding, top_k).await?;
            // Expand each entity via graph traversal
        }
    }
    QueryMode::Global => {
        // Use high-level keywords to find relationships
        for keyword in &keywords.high_level {
            let embedding = embed(keyword).await?;
            let rels = relationship_vector_store.search(&embedding, top_k).await?;
        }
    }
    QueryMode::Hybrid => {
        // Both Local AND Global paths
    }
    QueryMode::Mix => {
        // Weighted combination of Naive + Local + Global
    }
}
```

This routing is why the keyword extraction step matters so much: **the
quality of extracted keywords directly determines the quality of
retrieved context.**

### Provider Override

When a user selects a specific LLM provider in the UI, all LLM
operations -- including keyword extraction -- must use that provider:

```rust
async fn extract_with_llm_override(
    &self,
    query: &str,
    llm_override: Option<Arc<dyn LLMProvider>>,
) -> Result<ExtractedKeywords> {
    // Use the override provider if provided,
    // otherwise fall back to the default
    // ...
}
```

This prevents the inconsistency of extracting keywords with one LLM
(e.g., Ollama) while generating the answer with another (e.g., GPT-4).

### Key Takeaways

- Keyword extraction splits a query into high-level (themes) and
  low-level (entities) keywords.
- High-level keywords drive Global mode; low-level keywords drive
  Local mode.
- Query intent classification adapts the retrieval strategy to the
  question type.
- Caching eliminates redundant LLM calls for repeated queries (24h TTL).
- Keywords are the bridge between natural-language queries and
  structured graph/vector retrieval.

---

*Next: [6.2 Query Modes](ch06-02-query-modes.md)*
