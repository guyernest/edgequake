//! Lightweight TF-IDF fingerprinting for topic diversity in document sampling.
//!
//! Hand-rolled implementation (~100 lines) that avoids heavy NLP dependencies.
//! Used by the stratified sampler to ensure topic coverage across sampled documents.

use std::collections::HashMap;

/// Tokenize text into lowercase words, filtering tokens with length < 3.
fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| w.len() >= 3)
        .collect()
}

/// Compute TF-IDF vectors for a collection of documents.
///
/// For each document, computes term frequency (count / total tokens) weighted
/// by inverse document frequency `ln(N / df)` where `df` is the number of
/// documents containing the term. Returns a sparse TF-IDF vector per document.
pub fn compute_tfidf(documents: &[&str]) -> Vec<HashMap<String, f64>> {
    let n = documents.len() as f64;
    if n == 0.0 {
        return vec![];
    }

    // Tokenize all documents
    let tokenized: Vec<Vec<String>> = documents.iter().map(|d| tokenize(d)).collect();

    // Compute document frequency for each term
    let mut df: HashMap<String, usize> = HashMap::new();
    for tokens in &tokenized {
        let unique: std::collections::HashSet<&str> =
            tokens.iter().map(|s| s.as_str()).collect();
        for term in unique {
            *df.entry(term.to_string()).or_insert(0) += 1;
        }
    }

    // Compute TF-IDF for each document
    tokenized
        .iter()
        .map(|tokens| {
            if tokens.is_empty() {
                return HashMap::new();
            }
            let total = tokens.len() as f64;

            // Compute term frequency
            let mut tf: HashMap<String, f64> = HashMap::new();
            for token in tokens {
                *tf.entry(token.clone()).or_insert(0.0) += 1.0;
            }

            // Apply TF-IDF weighting (smoothed IDF to handle single-doc and identical-doc cases)
            let mut tfidf_vec = HashMap::new();
            for (term, count) in &tf {
                let tf_val = *count / total;
                let idf_val = (1.0 + n / *df.get(term).unwrap_or(&1) as f64).ln();
                let score = tf_val * idf_val;
                if score > 0.0 {
                    tfidf_vec.insert(term.clone(), score);
                }
            }
            tfidf_vec
        })
        .collect()
}

/// Compute cosine similarity between two sparse TF-IDF vectors.
///
/// Returns 0.0 if either vector has zero norm.
pub fn cosine_similarity(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let dot: f64 = a
        .iter()
        .filter_map(|(key, val)| b.get(key).map(|bval| val * bval))
        .sum();

    let norm_a: f64 = a.values().map(|v| v * v).sum::<f64>().sqrt();
    let norm_b: f64 = b.values().map(|v| v * v).sum::<f64>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Return the top-N terms by TF-IDF score.
pub fn top_terms(tfidf_vec: &HashMap<String, f64>, n: usize) -> Vec<String> {
    let mut entries: Vec<_> = tfidf_vec.iter().collect();
    entries.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    entries.into_iter().take(n).map(|(k, _)| k.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("Hello World foo");
        assert_eq!(tokens, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn test_tokenize_filters_short() {
        let tokens = tokenize("I am a test");
        // "I" -> "i" (len 1), "am" (len 2), "a" (len 1) -> filtered
        assert_eq!(tokens, vec!["test"]);
    }

    #[test]
    fn test_tfidf_different_docs() {
        let docs = vec![
            "the quick brown fox jumps over the lazy dog",
            "quantum computing enables parallel processing of complex algorithms",
        ];
        let vectors = compute_tfidf(&docs);
        assert_eq!(vectors.len(), 2);
        let sim = cosine_similarity(&vectors[0], &vectors[1]);
        assert!(
            sim < 0.3,
            "Dissimilar docs should have cosine similarity < 0.3, got {}",
            sim
        );
    }

    #[test]
    fn test_tfidf_identical_docs() {
        let docs = vec![
            "the quick brown fox jumps over the lazy dog",
            "the quick brown fox jumps over the lazy dog",
        ];
        let vectors = compute_tfidf(&docs);
        let sim = cosine_similarity(&vectors[0], &vectors[1]);
        assert!(
            (sim - 1.0).abs() < 0.001,
            "Identical docs should have cosine similarity ~1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_top_terms() {
        let docs = vec!["apple banana apple cherry apple banana"];
        let vectors = compute_tfidf(&docs);
        let top = top_terms(&vectors[0], 2);
        assert_eq!(top.len(), 2);
        // With smoothed IDF, single-doc terms still have positive scores.
        // "apple" has highest TF (3/6), then "banana" (2/6), then "cherry" (1/6).
        // All have same IDF since it's 1 doc. So top 2 = apple, banana.
        assert!(top.contains(&"apple".to_string()));
        assert!(top.contains(&"banana".to_string()));
    }

    #[test]
    fn test_top_terms_multi_doc() {
        // With multiple docs, terms unique to one doc get higher IDF
        let docs = vec![
            "apple apple apple banana cherry",
            "banana banana banana dates elderberry",
        ];
        let vectors = compute_tfidf(&docs);
        let top = top_terms(&vectors[0], 2);
        assert_eq!(top.len(), 2);
        // "apple" and "cherry" are unique to doc 0, so they should have highest TF-IDF
        assert!(top.contains(&"apple".to_string()));
        assert!(top.contains(&"cherry".to_string()));
    }

    #[test]
    fn test_empty_document() {
        let docs: Vec<&str> = vec![""];
        let vectors = compute_tfidf(&docs);
        assert_eq!(vectors.len(), 1);
        assert!(vectors[0].is_empty());

        // Cosine similarity with empty vector is 0.0
        let other = HashMap::from([("test".to_string(), 1.0)]);
        assert_eq!(cosine_similarity(&vectors[0], &other), 0.0);
    }

    #[test]
    fn test_empty_corpus() {
        let docs: Vec<&str> = vec![];
        let vectors = compute_tfidf(&docs);
        assert!(vectors.is_empty());
    }
}
