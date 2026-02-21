//! BM25 query coordination and Reciprocal Rank Fusion (RRF).
//!
//! # Implements
//!
//! - **RET-02**: BM25 keyword search coordination
//! - **RET-03**: Hybrid retrieval via Reciprocal Rank Fusion
//!
//! # WHY: RRF Instead of Score Addition
//!
//! BM25 scores are unbounded (0-100+), cosine similarity scores are 0-1.
//! Adding raw scores produces meaningless results biased toward BM25.
//! RRF uses **ranks only** (1/(k+rank)), making it score-agnostic.
//!
//! # References
//!
//! - Cormack, Clarke, Buettcher (2009): "Reciprocal Rank Fusion outperforms
//!   Condorcet and individual Reciprocal Rank"
//! - Used by Azure AI Search, OpenSearch for hybrid retrieval
//! - Standard k=60 value from the original paper

use std::collections::HashMap;

use edgequake_storage::traits::Bm25Storage;

use crate::error::Result;

/// Default RRF constant (k=60).
///
/// Standard value from the original RRF paper (Cormack et al. 2009).
/// Used by Azure AI Search and OpenSearch.
/// Higher k values give more weight to lower-ranked documents.
pub const DEFAULT_RRF_K: u32 = 60;

/// Reciprocal Rank Fusion of two ranked document lists.
///
/// For each document, computes `1/(k + rank)` for each list where it appears,
/// then sums the contributions. Documents appearing in both lists get higher
/// fused scores than documents in only one list.
///
/// # Arguments
///
/// * `bm25_ranked` - Document IDs ordered by BM25 score descending
/// * `vector_ranked` - Document IDs ordered by vector similarity descending
/// * `k` - RRF smoothing constant (default 60)
/// * `top_k` - Maximum number of results to return
///
/// # Returns
///
/// `(doc_id, rrf_score)` pairs ordered by RRF score descending, truncated to `top_k`.
///
/// # CRITICAL
///
/// This function uses **ranks only**. It never adds raw BM25 scores to cosine
/// scores. The rank positions are 1-indexed per the RRF paper convention.
pub fn reciprocal_rank_fusion(
    bm25_ranked: &[String],
    vector_ranked: &[String],
    k: u32,
    top_k: usize,
) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    let k_f64 = k as f64;

    // BM25 contributions (1-indexed rank)
    for (rank_0, doc_id) in bm25_ranked.iter().enumerate() {
        let rank_1 = rank_0 as f64 + 1.0;
        *scores.entry(doc_id.clone()).or_default() += 1.0 / (k_f64 + rank_1);
    }

    // Vector contributions (1-indexed rank)
    for (rank_0, doc_id) in vector_ranked.iter().enumerate() {
        let rank_1 = rank_0 as f64 + 1.0;
        *scores.entry(doc_id.clone()).or_default() += 1.0 / (k_f64 + rank_1);
    }

    // Sort by score descending, truncate to top_k
    let mut results: Vec<(String, f64)> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(top_k);
    results
}

/// BM25 query coordinator.
///
/// Handles query-time text preprocessing and BM25 storage interaction.
/// Uses the same tantivy analyzer pipeline as index time to ensure
/// term consistency.
pub struct Bm25QueryCoordinator;

impl Bm25QueryCoordinator {
    /// Query BM25 storage for pre-processed terms, returning ranked doc_ids.
    ///
    /// 1. Tokenizes query text with the same analyzer used at index time
    /// 2. Queries BM25 storage with processed terms
    /// 3. Returns doc_ids in ranked order (highest BM25 score first)
    ///
    /// # Arguments
    ///
    /// * `bm25_storage` - BM25 storage backend
    /// * `query_text` - Raw query text (will be tokenized and stemmed)
    /// * `top_k` - Maximum number of results
    ///
    /// # Returns
    ///
    /// Document IDs ordered by BM25 score descending. Empty if query
    /// produces no terms after preprocessing.
    pub async fn query_bm25(
        bm25_storage: &dyn Bm25Storage,
        query_text: &str,
        top_k: usize,
    ) -> Result<Vec<String>> {
        // 1. Tokenize query text with the same analyzer as index time
        let mut analyzer = edgequake_core::bm25_text::create_bm25_analyzer();
        let terms = edgequake_core::bm25_text::tokenize_text(&mut analyzer, query_text);

        if terms.is_empty() {
            return Ok(vec![]);
        }

        // 2. Query BM25 storage
        let results = bm25_storage.query(&terms, top_k).await?;

        // 3. Return doc_ids in ranked order (already sorted by score desc from storage)
        Ok(results.into_iter().map(|r| r.doc_id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_identical_lists() {
        // Two identical lists: items should get double RRF contribution
        let bm25 = vec!["a".into(), "b".into(), "c".into()];
        let vector = vec!["a".into(), "b".into(), "c".into()];

        let results = reciprocal_rank_fusion(&bm25, &vector, DEFAULT_RRF_K, 10);

        assert_eq!(results.len(), 3);
        // "a" should have the highest score (rank 1 in both lists)
        assert_eq!(results[0].0, "a");
        assert_eq!(results[1].0, "b");
        assert_eq!(results[2].0, "c");

        // Score for "a" = 2 * 1/(60+1) = 2/61
        let expected_a = 2.0 / 61.0;
        assert!((results[0].1 - expected_a).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_disjoint_lists() {
        // Completely disjoint lists: all items present, each with single-list contribution
        let bm25 = vec!["a".into(), "b".into()];
        let vector = vec!["x".into(), "y".into()];

        let results = reciprocal_rank_fusion(&bm25, &vector, DEFAULT_RRF_K, 10);

        assert_eq!(results.len(), 4);

        // All rank-1 items should be tied: 1/(60+1)
        let rank1_score = 1.0 / 61.0;
        assert!((results[0].1 - rank1_score).abs() < 1e-10);
        assert!((results[1].1 - rank1_score).abs() < 1e-10);

        // All rank-2 items should be tied: 1/(60+2)
        let rank2_score = 1.0 / 62.0;
        assert!((results[2].1 - rank2_score).abs() < 1e-10);
        assert!((results[3].1 - rank2_score).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_partially_overlapping() {
        // Overlapping items should rank higher than non-overlapping
        let bm25 = vec!["a".into(), "b".into(), "c".into()];
        let vector = vec!["b".into(), "d".into(), "a".into()];

        let results = reciprocal_rank_fusion(&bm25, &vector, DEFAULT_RRF_K, 10);

        assert_eq!(results.len(), 4); // a, b, c, d

        // "b" appears at rank 2 in BM25 and rank 1 in vector
        // "a" appears at rank 1 in BM25 and rank 3 in vector
        // "b" score = 1/(60+2) + 1/(60+1) = 1/62 + 1/61
        // "a" score = 1/(60+1) + 1/(60+3) = 1/61 + 1/63
        // "b" > "a" because 1/62 + 1/61 > 1/61 + 1/63
        let b_score = 1.0 / 62.0 + 1.0 / 61.0;
        let a_score = 1.0 / 61.0 + 1.0 / 63.0;
        assert!(b_score > a_score);

        // Both "a" and "b" (overlapping) should rank above "c" and "d" (single-list)
        let overlapping_ids: Vec<&str> = results[0..2].iter().map(|r| r.0.as_str()).collect();
        assert!(overlapping_ids.contains(&"a"));
        assert!(overlapping_ids.contains(&"b"));
    }

    #[test]
    fn test_rrf_empty_lists() {
        let empty: Vec<String> = vec![];
        let results = reciprocal_rank_fusion(&empty, &empty, DEFAULT_RRF_K, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rrf_one_empty_list() {
        let bm25 = vec!["a".into(), "b".into()];
        let empty: Vec<String> = vec![];

        let results = reciprocal_rank_fusion(&bm25, &empty, DEFAULT_RRF_K, 10);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
        assert_eq!(results[1].0, "b");
    }

    #[test]
    fn test_rrf_top_k_truncation() {
        let bm25: Vec<String> = (0..10).map(|i| format!("doc_{}", i)).collect();
        let vector: Vec<String> = (5..15).map(|i| format!("doc_{}", i)).collect();

        let results = reciprocal_rank_fusion(&bm25, &vector, DEFAULT_RRF_K, 5);

        assert_eq!(results.len(), 5);
        // Overlapping docs (5-9) should be at the top since they appear in both lists
    }

    #[test]
    fn test_rrf_k_parameter_effect() {
        let bm25 = vec!["a".into(), "b".into()];
        let vector = vec!["a".into(), "b".into()];

        // With k=1, rank matters more
        let results_k1 = reciprocal_rank_fusion(&bm25, &vector, 1, 10);
        // With k=100, rank matters less (scores are more compressed)
        let results_k100 = reciprocal_rank_fusion(&bm25, &vector, 100, 10);

        // Score ratio between rank 1 and rank 2 should be higher with k=1
        let ratio_k1 = results_k1[0].1 / results_k1[1].1;
        let ratio_k100 = results_k100[0].1 / results_k100[1].1;

        assert!(ratio_k1 > ratio_k100, "Lower k should produce more spread between ranks");
    }

    #[test]
    fn test_rrf_default_k_is_60() {
        assert_eq!(DEFAULT_RRF_K, 60);
    }
}
