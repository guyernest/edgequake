::: exercise
id: query-mode-selector
difficulty: intermediate
time: 25 minutes
:::

# Query Mode Selection

Graph RAG supports multiple query modes, each optimized for different kinds of
questions. Choosing the wrong mode wastes compute and degrades answer quality.
In this exercise you will implement heuristic-based query mode selection that
analyzes the structure of a query to route it to the most appropriate retrieval
strategy.

::: objectives
thinking:
  - Understand the four Graph RAG query modes and their strengths
  - Reason about which query characteristics signal each mode
doing:
  - Implement pattern-matching heuristics for query classification
  - Handle ambiguous queries with a sensible default
  - Support entity name detection for Local mode routing
:::

::: discussion
- Why not always use Hybrid mode if it combines the best of Local and Global?
- How would a production system decide the query mode differently than heuristics?
- What happens if the mode selector makes the wrong choice? How bad is the degradation?
:::

::: starter file="src/main.rs"
```rust
/// The query modes supported by the Graph RAG system.
#[derive(Debug, Clone, PartialEq)]
enum QueryMode {
    /// Simple vector similarity search. Best for short, keyword-like queries.
    Naive,
    /// Entity-centric subgraph traversal. Best for "who/what" questions
    /// about specific entities.
    Local,
    /// Community summary search. Best for broad "why/how" analytical
    /// questions about themes and patterns.
    Global,
    /// Combines Local and Global. Best for complex multi-part questions.
    Hybrid,
}

/// Metadata about a query that informs mode selection.
#[derive(Debug)]
struct QueryAnalysis {
    /// The original query text.
    query: String,
    /// The query converted to lowercase for matching.
    query_lower: String,
    /// Number of words in the query.
    word_count: usize,
    /// Whether the query contains a known entity name.
    has_entity_mention: bool,
    /// Whether the query contains question words (who, what, where, etc.).
    has_question_word: bool,
    /// Whether the query contains analytical words (why, how, explain, etc.).
    has_analytical_word: bool,
    /// Whether the query contains boolean connectors (and, or, both, also).
    has_connector: bool,
}

/// A set of known entity names for entity detection.
/// In a real system, this would come from the knowledge graph.
struct KnownEntities {
    names: Vec<String>,
}

impl KnownEntities {
    fn new(names: Vec<String>) -> Self {
        KnownEntities { names }
    }

    fn contains_any(&self, text: &str) -> bool {
        let text_lower = text.to_lowercase();
        self.names.iter().any(|name| text_lower.contains(&name.to_lowercase()))
    }
}

/// Analyze a query to extract features for mode selection.
fn analyze_query(query: &str, known_entities: &KnownEntities) -> QueryAnalysis {
    // TODO: Implement query analysis
    //
    // 1. Convert to lowercase.
    // 2. Count words.
    // 3. Check for known entity mentions.
    // 4. Check for question words: who, what, which, where, when
    // 5. Check for analytical words: why, how, explain, describe,
    //    compare, analyze, summarize, overview, theme, pattern
    // 6. Check for boolean connectors: and, or, both, also, as well as
    todo!("Implement analyze_query")
}

/// Select the query mode based on query analysis.
///
/// Heuristics:
/// - Naive: Very short queries (1-3 words) with no question/analytical words.
/// - Local: Contains entity mention + question word, but not analytical.
/// - Global: Contains analytical words, no specific entity mention.
/// - Hybrid: Contains both entity mention and analytical words, OR has
///           boolean connectors suggesting a multi-part question.
fn select_mode(analysis: &QueryAnalysis) -> QueryMode {
    // TODO: Implement mode selection heuristics
    //
    // Start with the most specific rules and fall back to broader ones.
    // The order of checks matters -- more specific modes should be tested first.
    todo!("Implement select_mode")
}

fn main() {
    let entities = KnownEntities::new(vec![
        "Alice Johnson".into(),
        "TechCorp".into(),
        "Bob Smith".into(),
        "CloudBase".into(),
    ]);

    let test_queries = vec![
        "TechCorp CEO",
        "Who is Alice Johnson?",
        "Why did TechCorp acquire CloudBase?",
        "Explain the main themes in the dataset",
        "Who is Bob Smith and how does he relate to TechCorp?",
        "overview of organizational patterns",
        "CloudBase",
        "Compare the roles of Alice Johnson and Bob Smith at TechCorp",
    ];

    println!("Query Mode Selection:");
    println!("{:-<70}", "");
    for query in test_queries {
        let analysis = analyze_query(query, &entities);
        let mode = select_mode(&analysis);
        println!("  {:?:<8} | {}", mode, query);
    }
}
```
:::

::: hint level=1 title="Analyzing query features"
Use simple string checks for word detection. Check for word boundaries to avoid
false matches (e.g., "how" inside "showdown"):

```rust
fn contains_word(text: &str, word: &str) -> bool {
    text.split_whitespace().any(|w| {
        w.trim_matches(|c: char| !c.is_alphanumeric()) == word
    })
}
```

Then use this in `analyze_query`:
```rust
let question_words = ["who", "what", "which", "where", "when"];
let has_question_word = question_words.iter().any(|w| contains_word(&query_lower, w));
```
:::

::: hint level=2 title="Mode selection logic"
Think of mode selection as a decision tree. Check the most specific conditions
first:

```rust
fn select_mode(analysis: &QueryAnalysis) -> QueryMode {
    // Hybrid: multi-part (has connector) OR both entity + analytical
    if (analysis.has_entity_mention && analysis.has_analytical_word)
        || (analysis.has_connector && analysis.word_count > 5) {
        return QueryMode::Hybrid;
    }

    // Global: analytical without specific entity
    if analysis.has_analytical_word && !analysis.has_entity_mention {
        return QueryMode::Global;
    }

    // Local: specific entity question
    if analysis.has_entity_mention && (analysis.has_question_word || analysis.word_count > 3) {
        return QueryMode::Local;
    }

    // Naive: short keyword queries
    QueryMode::Naive
}
```
:::

::: solution
```rust
#[derive(Debug, Clone, PartialEq)]
enum QueryMode {
    Naive,
    Local,
    Global,
    Hybrid,
}

#[derive(Debug)]
struct QueryAnalysis {
    query: String,
    query_lower: String,
    word_count: usize,
    has_entity_mention: bool,
    has_question_word: bool,
    has_analytical_word: bool,
    has_connector: bool,
}

struct KnownEntities {
    names: Vec<String>,
}

impl KnownEntities {
    fn new(names: Vec<String>) -> Self {
        KnownEntities { names }
    }

    fn contains_any(&self, text: &str) -> bool {
        let text_lower = text.to_lowercase();
        self.names
            .iter()
            .any(|name| text_lower.contains(&name.to_lowercase()))
    }
}

fn contains_word(text: &str, word: &str) -> bool {
    text.split_whitespace()
        .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == word)
}

fn analyze_query(query: &str, known_entities: &KnownEntities) -> QueryAnalysis {
    let query_lower = query.to_lowercase();
    let word_count = query.split_whitespace().count();
    let has_entity_mention = known_entities.contains_any(query);

    let question_words = ["who", "what", "which", "where", "when"];
    let has_question_word = question_words
        .iter()
        .any(|w| contains_word(&query_lower, w));

    let analytical_words = [
        "why", "how", "explain", "describe", "compare", "analyze",
        "summarize", "overview", "theme", "themes", "pattern", "patterns",
    ];
    let has_analytical_word = analytical_words
        .iter()
        .any(|w| contains_word(&query_lower, w));

    let connectors = ["and", "or", "both", "also"];
    let has_connector = connectors
        .iter()
        .any(|w| contains_word(&query_lower, w));

    QueryAnalysis {
        query: query.to_string(),
        query_lower,
        word_count,
        has_entity_mention,
        has_question_word,
        has_analytical_word,
        has_connector,
    }
}

fn select_mode(analysis: &QueryAnalysis) -> QueryMode {
    // Hybrid: multi-part question (connector + substance) OR entity + analytical
    if (analysis.has_entity_mention && analysis.has_analytical_word)
        || (analysis.has_connector && analysis.word_count > 5)
    {
        return QueryMode::Hybrid;
    }

    // Global: broad analytical questions without specific entities
    if analysis.has_analytical_word && !analysis.has_entity_mention {
        return QueryMode::Global;
    }

    // Local: entity-focused questions
    if analysis.has_entity_mention
        && (analysis.has_question_word || analysis.word_count > 3)
    {
        return QueryMode::Local;
    }

    // Naive: short keyword lookups and simple queries
    QueryMode::Naive
}

fn main() {
    let entities = KnownEntities::new(vec![
        "Alice Johnson".into(),
        "TechCorp".into(),
        "Bob Smith".into(),
        "CloudBase".into(),
    ]);

    let test_queries = vec![
        "TechCorp CEO",
        "Who is Alice Johnson?",
        "Why did TechCorp acquire CloudBase?",
        "Explain the main themes in the dataset",
        "Who is Bob Smith and how does he relate to TechCorp?",
        "overview of organizational patterns",
        "CloudBase",
        "Compare the roles of Alice Johnson and Bob Smith at TechCorp",
    ];

    println!("Query Mode Selection:");
    println!("{:-<70}", "");
    for query in test_queries {
        let analysis = analyze_query(query, &entities);
        let mode = select_mode(&analysis);
        println!("  {:?:<8} | {}", mode, query);
    }
}
```

### Explanation

The mode selector uses a two-phase approach: first **analyze** the query to
extract boolean features, then **select** the mode using a decision tree.

**Analysis** checks for: question words (who/what/where), analytical words
(why/how/explain/compare), boolean connectors (and/or/both), known entity
mentions, and query length. The `contains_word` helper avoids false positives
by checking word boundaries.

**Selection** uses a priority ordering: Hybrid is checked first (most specific:
requires both entity + analytical signals, or multi-part structure), then
Global (analytical without entity), then Local (entity-focused), and finally
Naive as the default for short keyword queries.

This heuristic approach is simple and interpretable but imperfect -- ambiguous
queries like "What happened?" will default to Naive. A production system would
likely use a lightweight classifier or ask the LLM itself to choose the mode.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn test_entities() -> KnownEntities {
        KnownEntities::new(vec![
            "Alice".into(),
            "TechCorp".into(),
            "Bob".into(),
        ])
    }

    #[test]
    fn test_naive_short_query() {
        let entities = test_entities();
        let analysis = analyze_query("search terms", &entities);
        let mode = select_mode(&analysis);
        assert_eq!(mode, QueryMode::Naive, "Short generic queries should be Naive");
    }

    #[test]
    fn test_naive_single_unknown_word() {
        let entities = test_entities();
        let analysis = analyze_query("quantum", &entities);
        let mode = select_mode(&analysis);
        assert_eq!(mode, QueryMode::Naive, "Single unknown word should be Naive");
    }

    #[test]
    fn test_local_who_question() {
        let entities = test_entities();
        let analysis = analyze_query("Who is Alice?", &entities);
        let mode = select_mode(&analysis);
        assert_eq!(mode, QueryMode::Local, "'Who is [entity]' should be Local");
    }

    #[test]
    fn test_local_what_question() {
        let entities = test_entities();
        let analysis = analyze_query("What does TechCorp do?", &entities);
        let mode = select_mode(&analysis);
        assert_eq!(mode, QueryMode::Local, "'What does [entity]' should be Local");
    }

    #[test]
    fn test_global_analytical_no_entity() {
        let entities = test_entities();
        let analysis = analyze_query("Explain the main themes", &entities);
        let mode = select_mode(&analysis);
        assert_eq!(mode, QueryMode::Global, "Analytical without entity should be Global");
    }

    #[test]
    fn test_global_overview() {
        let entities = test_entities();
        let analysis = analyze_query("overview of organizational patterns", &entities);
        let mode = select_mode(&analysis);
        assert_eq!(mode, QueryMode::Global, "'overview of patterns' should be Global");
    }

    #[test]
    fn test_hybrid_entity_plus_analytical() {
        let entities = test_entities();
        let analysis = analyze_query("Why did TechCorp fail?", &entities);
        let mode = select_mode(&analysis);
        assert_eq!(mode, QueryMode::Hybrid, "Entity + analytical should be Hybrid");
    }

    #[test]
    fn test_hybrid_multi_part() {
        let entities = test_entities();
        let analysis = analyze_query("Who is Alice and what is her role at TechCorp?", &entities);
        let mode = select_mode(&analysis);
        assert_eq!(mode, QueryMode::Hybrid, "Multi-part question with connector should be Hybrid");
    }

    #[test]
    fn test_analysis_word_count() {
        let entities = test_entities();
        let analysis = analyze_query("one two three", &entities);
        assert_eq!(analysis.word_count, 3);
    }

    #[test]
    fn test_entity_detection_case_insensitive() {
        let entities = test_entities();
        let analysis = analyze_query("tell me about techcorp", &entities);
        assert!(analysis.has_entity_mention, "Entity detection should be case-insensitive");
    }
}
```
:::

::: reflection
- What queries would your heuristics classify incorrectly? How would you handle those edge cases?
- How would you use user feedback (e.g., "this answer was not helpful") to improve mode selection over time?
- Could you use the LLM itself to classify queries? What are the latency tradeoffs?
:::
