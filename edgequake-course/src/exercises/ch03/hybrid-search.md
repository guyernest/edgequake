::: exercise
id: hybrid-search
difficulty: intermediate
time: 25 minutes
:::

# Hybrid BM25 + Dense Search with RRF

Dense vector search excels at semantic similarity but misses exact keyword
matches. BM25 excels at keyword relevance but misses synonyms and paraphrases.
In this exercise you will implement BM25 scoring from scratch, then combine it
with pre-computed dense search results using Reciprocal Rank Fusion (RRF) to
get the best of both worlds.

::: objectives
thinking:
  - Understand why hybrid search outperforms either lexical or semantic search alone
  - Grasp how RRF avoids the score normalization problem
doing:
  - Implement BM25 term frequency with saturation (the TF component)
  - Implement inverse document frequency (IDF)
  - Combine BM25 and dense ranked lists using Reciprocal Rank Fusion
:::

::: discussion
- What kinds of queries does BM25 handle well that dense search struggles with?
- Why is it hard to combine raw BM25 scores with cosine similarity scores directly?
- How does the RRF constant k affect the fusion behavior?
:::

::: starter file="src/main.rs"
```rust
use std::collections::HashMap;

/// A document in the corpus.
#[derive(Debug, Clone)]
struct Document {
    id: String,
    text: String,
}

/// A ranked result with a score.
#[derive(Debug, Clone)]
struct RankedResult {
    id: String,
    score: f64,
}

/// BM25 parameters.
struct BM25Config {
    /// Term frequency saturation parameter. Typical value: 1.2
    k1: f64,
    /// Length normalization parameter. Typical value: 0.75
    b: f64,
}

impl Default for BM25Config {
    fn default() -> Self {
        BM25Config { k1: 1.2, b: 0.75 }
    }
}

/// A BM25 scorer that computes relevance scores for documents.
struct BM25Scorer {
    config: BM25Config,
    /// Each document stored as (id, token_list).
    documents: Vec<(String, Vec<String>)>,
    /// Average document length across the corpus.
    avg_doc_len: f64,
    /// Number of documents containing each term.
    doc_freq: HashMap<String, usize>,
}

impl BM25Scorer {
    /// Build a BM25 scorer from a collection of documents.
    fn new(documents: &[Document], config: BM25Config) -> Self {
        let mut tokenized = Vec::new();
        let mut doc_freq: HashMap<String, usize> = HashMap::new();
        let mut total_len = 0usize;

        for doc in documents {
            let tokens: Vec<String> = doc
                .text
                .to_lowercase()
                .split_whitespace()
                .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
                .filter(|s| !s.is_empty())
                .collect();
            total_len += tokens.len();

            // Count document frequency: how many docs contain each unique term
            let mut seen = std::collections::HashSet::new();
            for token in &tokens {
                if seen.insert(token.clone()) {
                    *doc_freq.entry(token.clone()).or_insert(0) += 1;
                }
            }

            tokenized.push((doc.id.clone(), tokens));
        }

        let avg_doc_len = if tokenized.is_empty() {
            0.0
        } else {
            total_len as f64 / tokenized.len() as f64
        };

        BM25Scorer {
            config,
            documents: tokenized,
            avg_doc_len,
            doc_freq,
        }
    }

    /// Compute the BM25 term frequency component for a term in a document.
    ///
    /// Formula: (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * doc_len / avg_doc_len))
    ///
    /// where tf is the raw count of the term in the document.
    fn term_frequency(&self, raw_tf: usize, doc_len: usize) -> f64 {
        // TODO: Implement the BM25 TF formula
        // The formula applies saturation so that additional occurrences
        // of a term have diminishing returns.
        todo!("Implement term_frequency")
    }

    /// Compute the inverse document frequency for a term.
    ///
    /// Formula: ln((N - df + 0.5) / (df + 0.5) + 1)
    ///
    /// where N is the total number of documents and df is the number
    /// of documents containing the term.
    fn inverse_document_frequency(&self, term: &str) -> f64 {
        // TODO: Implement the BM25 IDF formula
        // Use the doc_freq map to look up how many documents contain the term.
        // If the term is not in the corpus, df is 0.
        todo!("Implement inverse_document_frequency")
    }

    /// Score all documents for a query and return results sorted by score descending.
    fn score_query(&self, query: &str) -> Vec<RankedResult> {
        let query_terms: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let mut results: Vec<RankedResult> = self
            .documents
            .iter()
            .map(|(id, tokens)| {
                let mut score = 0.0;
                for qt in &query_terms {
                    let raw_tf = tokens.iter().filter(|t| t == qt).count();
                    if raw_tf > 0 {
                        let tf = self.term_frequency(raw_tf, tokens.len());
                        let idf = self.inverse_document_frequency(qt);
                        score += tf * idf;
                    }
                }
                RankedResult {
                    id: id.clone(),
                    score,
                }
            })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        results
    }
}

/// Combine two ranked result lists using Reciprocal Rank Fusion (RRF).
///
/// RRF score for a document = sum over all lists of: 1 / (k + rank)
///
/// where rank is the 1-based position in each list, and k is a constant
/// (typically 60).
///
/// Documents that appear in only one list still get a score from that list.
fn rrf_combine(
    bm25_results: &[RankedResult],
    dense_results: &[RankedResult],
    k: f64,
) -> Vec<RankedResult> {
    // TODO: Implement RRF combination
    // 1. Build a HashMap<String, f64> for combined scores.
    // 2. For each result in bm25_results at position i (0-indexed),
    //    add 1.0 / (k + (i + 1) as f64) to the combined score.
    // 3. Do the same for dense_results.
    // 4. Collect into a Vec<RankedResult> and sort by score descending.
    todo!("Implement rrf_combine")
}

fn main() {
    let documents = vec![
        Document { id: "d1".into(), text: "Rust programming language systems safety".into() },
        Document { id: "d2".into(), text: "Python machine learning data science".into() },
        Document { id: "d3".into(), text: "Rust memory safety borrow checker".into() },
        Document { id: "d4".into(), text: "Graph database knowledge representation".into() },
    ];

    let scorer = BM25Scorer::new(&documents, BM25Config::default());
    let bm25_results = scorer.score_query("rust safety");

    // Simulated dense search results (pre-computed)
    let dense_results = vec![
        RankedResult { id: "d3".into(), score: 0.92 },
        RankedResult { id: "d1".into(), score: 0.85 },
        RankedResult { id: "d4".into(), score: 0.45 },
        RankedResult { id: "d2".into(), score: 0.30 },
    ];

    let hybrid = rrf_combine(&bm25_results, &dense_results, 60.0);

    println!("BM25 ranking:");
    for r in &bm25_results {
        println!("  {} -> {:.4}", r.id, r.score);
    }
    println!("\nDense ranking:");
    for r in &dense_results {
        println!("  {} -> {:.4}", r.id, r.score);
    }
    println!("\nHybrid (RRF) ranking:");
    for r in &hybrid {
        println!("  {} -> {:.6}", r.id, r.score);
    }
}
```
:::

::: hint level=1 title="BM25 TF formula"
The term frequency with saturation prevents any single term from dominating:

```rust
fn term_frequency(&self, raw_tf: usize, doc_len: usize) -> f64 {
    let tf = raw_tf as f64;
    let k1 = self.config.k1;
    let b = self.config.b;
    let dl = doc_len as f64;
    (tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * dl / self.avg_doc_len))
}
```
:::

::: hint level=2 title="RRF combination"
Use a HashMap to accumulate scores. Remember that ranks are 1-based:

```rust
fn rrf_combine(bm25: &[RankedResult], dense: &[RankedResult], k: f64) -> Vec<RankedResult> {
    let mut scores: HashMap<String, f64> = HashMap::new();

    for (i, r) in bm25.iter().enumerate() {
        let rank = (i + 1) as f64;
        *scores.entry(r.id.clone()).or_insert(0.0) += 1.0 / (k + rank);
    }
    for (i, r) in dense.iter().enumerate() {
        let rank = (i + 1) as f64;
        *scores.entry(r.id.clone()).or_insert(0.0) += 1.0 / (k + rank);
    }
    // ... sort and collect
}
```
:::

::: solution
```rust
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct Document {
    id: String,
    text: String,
}

#[derive(Debug, Clone)]
struct RankedResult {
    id: String,
    score: f64,
}

struct BM25Config {
    k1: f64,
    b: f64,
}

impl Default for BM25Config {
    fn default() -> Self {
        BM25Config { k1: 1.2, b: 0.75 }
    }
}

struct BM25Scorer {
    config: BM25Config,
    documents: Vec<(String, Vec<String>)>,
    avg_doc_len: f64,
    doc_freq: HashMap<String, usize>,
}

impl BM25Scorer {
    fn new(documents: &[Document], config: BM25Config) -> Self {
        let mut tokenized = Vec::new();
        let mut doc_freq: HashMap<String, usize> = HashMap::new();
        let mut total_len = 0usize;

        for doc in documents {
            let tokens: Vec<String> = doc
                .text
                .to_lowercase()
                .split_whitespace()
                .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
                .filter(|s| !s.is_empty())
                .collect();
            total_len += tokens.len();

            let mut seen = std::collections::HashSet::new();
            for token in &tokens {
                if seen.insert(token.clone()) {
                    *doc_freq.entry(token.clone()).or_insert(0) += 1;
                }
            }

            tokenized.push((doc.id.clone(), tokens));
        }

        let avg_doc_len = if tokenized.is_empty() {
            0.0
        } else {
            total_len as f64 / tokenized.len() as f64
        };

        BM25Scorer {
            config,
            documents: tokenized,
            avg_doc_len,
            doc_freq,
        }
    }

    fn term_frequency(&self, raw_tf: usize, doc_len: usize) -> f64 {
        let tf = raw_tf as f64;
        let k1 = self.config.k1;
        let b = self.config.b;
        let dl = doc_len as f64;

        (tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * dl / self.avg_doc_len))
    }

    fn inverse_document_frequency(&self, term: &str) -> f64 {
        let n = self.documents.len() as f64;
        let df = *self.doc_freq.get(term).unwrap_or(&0) as f64;
        ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
    }

    fn score_query(&self, query: &str) -> Vec<RankedResult> {
        let query_terms: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let mut results: Vec<RankedResult> = self
            .documents
            .iter()
            .map(|(id, tokens)| {
                let mut score = 0.0;
                for qt in &query_terms {
                    let raw_tf = tokens.iter().filter(|t| t == qt).count();
                    if raw_tf > 0 {
                        let tf = self.term_frequency(raw_tf, tokens.len());
                        let idf = self.inverse_document_frequency(qt);
                        score += tf * idf;
                    }
                }
                RankedResult {
                    id: id.clone(),
                    score,
                }
            })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        results
    }
}

fn rrf_combine(
    bm25_results: &[RankedResult],
    dense_results: &[RankedResult],
    k: f64,
) -> Vec<RankedResult> {
    let mut scores: HashMap<String, f64> = HashMap::new();

    for (i, r) in bm25_results.iter().enumerate() {
        let rank = (i + 1) as f64;
        *scores.entry(r.id.clone()).or_insert(0.0) += 1.0 / (k + rank);
    }

    for (i, r) in dense_results.iter().enumerate() {
        let rank = (i + 1) as f64;
        *scores.entry(r.id.clone()).or_insert(0.0) += 1.0 / (k + rank);
    }

    let mut results: Vec<RankedResult> = scores
        .into_iter()
        .map(|(id, score)| RankedResult { id, score })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    results
}

fn main() {
    let documents = vec![
        Document { id: "d1".into(), text: "Rust programming language systems safety".into() },
        Document { id: "d2".into(), text: "Python machine learning data science".into() },
        Document { id: "d3".into(), text: "Rust memory safety borrow checker".into() },
        Document { id: "d4".into(), text: "Graph database knowledge representation".into() },
    ];

    let scorer = BM25Scorer::new(&documents, BM25Config::default());
    let bm25_results = scorer.score_query("rust safety");

    let dense_results = vec![
        RankedResult { id: "d3".into(), score: 0.92 },
        RankedResult { id: "d1".into(), score: 0.85 },
        RankedResult { id: "d4".into(), score: 0.45 },
        RankedResult { id: "d2".into(), score: 0.30 },
    ];

    let hybrid = rrf_combine(&bm25_results, &dense_results, 60.0);

    println!("BM25 ranking:");
    for r in &bm25_results {
        println!("  {} -> {:.4}", r.id, r.score);
    }
    println!("\nDense ranking:");
    for r in &dense_results {
        println!("  {} -> {:.4}", r.id, r.score);
    }
    println!("\nHybrid (RRF) ranking:");
    for r in &hybrid {
        println!("  {} -> {:.6}", r.id, r.score);
    }
}
```

### Explanation

**BM25 TF**: The formula `(tf * (k1 + 1)) / (tf + k1 * (1 - b + b * dl/avgdl))`
saturates the contribution of term frequency so that repeating a word 10 times
is only marginally more relevant than repeating it 5 times. The `b` parameter
controls length normalization -- longer documents are penalized to avoid
unfairly boosting them for having more term occurrences by chance.

**BM25 IDF**: The formula `ln((N - df + 0.5) / (df + 0.5) + 1)` assigns higher
weight to rare terms. A term appearing in every document gets near-zero IDF,
while a term appearing in only one document gets high IDF.

**RRF**: Instead of trying to normalize incompatible score scales (BM25 scores
can be any positive number, cosine similarity is [-1, 1]), RRF uses only rank
positions. The formula `1 / (k + rank)` gives higher weight to top-ranked items
and the constant k (typically 60) controls how quickly the weight drops off.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn test_corpus() -> Vec<Document> {
        vec![
            Document { id: "d1".into(), text: "the cat sat on the mat".into() },
            Document { id: "d2".into(), text: "the dog chased the cat".into() },
            Document { id: "d3".into(), text: "a bird flew over the tree".into() },
        ]
    }

    #[test]
    fn test_term_frequency_increases_with_count() {
        let scorer = BM25Scorer::new(&test_corpus(), BM25Config::default());
        let tf1 = scorer.term_frequency(1, 5);
        let tf3 = scorer.term_frequency(3, 5);
        assert!(tf3 > tf1, "Higher raw TF should produce higher BM25 TF");
    }

    #[test]
    fn test_term_frequency_saturates() {
        let scorer = BM25Scorer::new(&test_corpus(), BM25Config::default());
        let tf10 = scorer.term_frequency(10, 5);
        let tf100 = scorer.term_frequency(100, 5);
        // The difference should be small due to saturation
        assert!(
            (tf100 - tf10) < (tf10 - scorer.term_frequency(1, 5)),
            "TF should saturate: difference between 10 and 100 should be less than between 1 and 10"
        );
    }

    #[test]
    fn test_idf_rare_term_higher() {
        let scorer = BM25Scorer::new(&test_corpus(), BM25Config::default());
        let idf_the = scorer.inverse_document_frequency("the"); // in all 3 docs
        let idf_bird = scorer.inverse_document_frequency("bird"); // in 1 doc
        assert!(
            idf_bird > idf_the,
            "Rare term 'bird' should have higher IDF than common term 'the'"
        );
    }

    #[test]
    fn test_idf_unknown_term() {
        let scorer = BM25Scorer::new(&test_corpus(), BM25Config::default());
        let idf = scorer.inverse_document_frequency("xyzzy");
        assert!(idf > 0.0, "Unknown term should still have positive IDF");
    }

    #[test]
    fn test_bm25_relevant_doc_scores_highest() {
        let scorer = BM25Scorer::new(&test_corpus(), BM25Config::default());
        let results = scorer.score_query("cat");
        // d1 and d2 both contain "cat", d3 does not
        assert!(results[0].id == "d1" || results[0].id == "d2");
        let d3_score = results.iter().find(|r| r.id == "d3").unwrap().score;
        assert_eq!(d3_score, 0.0, "Document without query term should score 0");
    }

    #[test]
    fn test_rrf_combine_merges_lists() {
        let list_a = vec![
            RankedResult { id: "x".into(), score: 10.0 },
            RankedResult { id: "y".into(), score: 5.0 },
        ];
        let list_b = vec![
            RankedResult { id: "y".into(), score: 0.9 },
            RankedResult { id: "z".into(), score: 0.5 },
        ];
        let combined = rrf_combine(&list_a, &list_b, 60.0);

        assert_eq!(combined.len(), 3, "Should contain all unique documents");
        // "y" appears in both lists, so it should get the highest RRF score
        assert_eq!(combined[0].id, "y", "'y' should rank first (appears in both lists)");
    }

    #[test]
    fn test_rrf_score_calculation() {
        let list_a = vec![RankedResult { id: "a".into(), score: 1.0 }];
        let list_b = vec![RankedResult { id: "a".into(), score: 1.0 }];
        let combined = rrf_combine(&list_a, &list_b, 60.0);

        let expected = 1.0 / 61.0 + 1.0 / 61.0; // rank 1 in both lists
        assert!(
            (combined[0].score - expected).abs() < 1e-10,
            "RRF score should be 2/(k+1) for rank-1 in both lists"
        );
    }
}
```
:::

::: reflection
- How would you weight BM25 and dense results differently in RRF (e.g., giving dense results 2x the influence)?
- In what scenarios would BM25-only search outperform the hybrid approach?
- How does the RRF constant k=60 compare to k=1? What changes in the ranking behavior?
:::
