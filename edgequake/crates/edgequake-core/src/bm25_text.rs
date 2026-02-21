//! Shared BM25 text preprocessing using Tantivy's tokenizer pipeline.
//!
//! # Implements
//!
//! - **RET-01**: Consistent text preprocessing for BM25 indexing and querying
//!
//! # CRITICAL: Index/Query Consistency
//!
//! The same `TextAnalyzer` pipeline MUST be used at both:
//! - **Index time** (edgequake-batch Store phase): tokenizing chunk text into terms
//! - **Query time** (edgequake-query): tokenizing user query into search terms
//!
//! If different analyzers are used, BM25 will return zero results because
//! indexed terms and query terms will not match.
//!
//! # Pipeline
//!
//! ```text
//! Input text
//!   -> SimpleTokenizer (split on whitespace/punctuation)
//!   -> RemoveLongFilter (drop tokens > 40 chars)
//!   -> LowerCaser (normalize to lowercase)
//!   -> Stemmer (English Porter2 stemming)
//! ```
//!
//! # Example
//!
//! ```rust
//! use edgequake_core::bm25_text::{create_bm25_analyzer, tokenize_text};
//!
//! let mut analyzer = create_bm25_analyzer();
//! let terms = tokenize_text(&mut analyzer, "Jeffrey Epstein's financial documents");
//! assert!(terms.contains(&"jeffrey".to_string()) || terms.contains(&"jeffrei".to_string()));
//! ```

use tantivy::tokenizer::{LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer};

/// Create the BM25 text analyzer pipeline.
///
/// Uses Tantivy's composable tokenizer pipeline:
/// - `SimpleTokenizer`: splits on whitespace and punctuation
/// - `RemoveLongFilter`: drops tokens longer than 40 characters
/// - `LowerCaser`: normalizes tokens to lowercase
/// - `Stemmer`: applies English Porter2 stemming
///
/// This function must be used at both index time and query time
/// to ensure term consistency.
pub fn create_bm25_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::new(tantivy::tokenizer::Language::English))
        .build()
}

/// Tokenize text into stemmed, lowercased terms using the BM25 analyzer.
///
/// # Arguments
///
/// * `analyzer` - A `TextAnalyzer` created by `create_bm25_analyzer()`
/// * `text` - The input text to tokenize
///
/// # Returns
///
/// A vector of processed term strings (lowercased, stemmed).
pub fn tokenize_text(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
    let mut stream = analyzer.token_stream(text);
    let mut terms = Vec::new();
    while let Some(token) = stream.next() {
        terms.push(token.text.clone());
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_analyzer() {
        let mut analyzer = create_bm25_analyzer();
        let terms = tokenize_text(&mut analyzer, "Hello World");
        assert_eq!(terms.len(), 2);
        // All terms should be lowercase
        for term in &terms {
            assert_eq!(term, &term.to_lowercase());
        }
    }

    #[test]
    fn test_stemming() {
        let mut analyzer = create_bm25_analyzer();
        // "running" should be stemmed to "run"
        let terms = tokenize_text(&mut analyzer, "running");
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0], "run");
    }

    #[test]
    fn test_long_token_removal() {
        let mut analyzer = create_bm25_analyzer();
        let long_token = "a".repeat(50); // 50 chars, exceeds 40 limit
        let text = format!("short {}", long_token);
        let terms = tokenize_text(&mut analyzer, &text);
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0], "short");
    }

    #[test]
    fn test_punctuation_splitting() {
        let mut analyzer = create_bm25_analyzer();
        let terms = tokenize_text(&mut analyzer, "hello,world;test");
        assert!(terms.len() >= 3);
    }

    #[test]
    fn test_empty_input() {
        let mut analyzer = create_bm25_analyzer();
        let terms = tokenize_text(&mut analyzer, "");
        assert!(terms.is_empty());
    }

    #[test]
    fn test_consistency() {
        // Same input must produce same output (index/query consistency)
        let mut analyzer1 = create_bm25_analyzer();
        let mut analyzer2 = create_bm25_analyzer();
        let text = "Jeffrey Epstein financial documents investigation";
        let terms1 = tokenize_text(&mut analyzer1, text);
        let terms2 = tokenize_text(&mut analyzer2, text);
        assert_eq!(terms1, terms2);
    }
}
