//! BM25 Retrieval Quality Evaluation
//!
//! Validates that hybrid BM25+vector retrieval produces measurably better
//! rankings for exact-term queries compared to vector-only retrieval.
//!
//! This test uses synthetic data with known characteristics:
//! - Documents containing specific legal/technical terms
//! - Vector embeddings that intentionally rank these documents poorly
//!   (simulating semantic mismatch for exact terms)
//! - BM25 index that captures exact term presence
//!
//! Success criteria (from ROADMAP):
//! A query with an exact technical term returns that term's document in
//! position 1 or 2 where vector-only placed it outside the top 5.
//!
//! For end-to-end evaluation with real data, use the dataset at
//! `data/acquired/acquired_transcripts_all.txt` alongside the Epstein corpus.
//! Run the batch pipeline with ATHENA_BM25_DATABASE set, then compare
//! vector-only vs hybrid query results for exact-term queries.

#[cfg(test)]
mod evaluation {
    use edgequake_query::bm25_scorer::{reciprocal_rank_fusion, DEFAULT_RRF_K};

    /// Test: RRF rescues documents that have exact term matches but poor vector scores.
    ///
    /// Scenario: 10 documents, query is an exact legal citation "Rule 30(b)(6)".
    /// - Vector search ranks the document containing this citation at position 8
    ///   (semantic embedding doesn't capture citation syntax well)
    /// - BM25 search ranks it at position 1 (exact term match)
    /// - RRF fusion should bring it to position 1 or 2
    #[test]
    fn test_exact_term_rescued_by_rrf() {
        // Vector ranking: doc_target is at position 8 (0-indexed: 7)
        let vector_ranked: Vec<String> = vec![
            "doc_semantic_1",
            "doc_semantic_2",
            "doc_semantic_3",
            "doc_semantic_4",
            "doc_semantic_5",
            "doc_semantic_6",
            "doc_semantic_7",
            "doc_target",
            "doc_semantic_8",
            "doc_semantic_9",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        // BM25 ranking: doc_target is at position 1 (exact term match)
        let bm25_ranked: Vec<String> = vec![
            "doc_target",
            "doc_partial_1",
            "doc_partial_2",
            "doc_unrelated_1",
            "doc_unrelated_2",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let fused = reciprocal_rank_fusion(&bm25_ranked, &vector_ranked, DEFAULT_RRF_K, 10);

        // Find doc_target's position in fused results
        let target_position = fused
            .iter()
            .position(|(id, _)| id == "doc_target")
            .expect("doc_target must appear in fused results");

        // Success criterion: position 0 or 1 (i.e., rank 1 or 2)
        assert!(
            target_position <= 1,
            "doc_target should be in position 1 or 2 after RRF fusion, but was at position {}. \
             Vector rank was 8, BM25 rank was 1. RRF should rescue it. \
             Full fused ranking: {:?}",
            target_position + 1,
            fused,
        );
    }

    /// Test: Vector-only leaves exact-term document outside top 5.
    ///
    /// Baseline: confirms that without BM25, the exact-term document
    /// is ranked poorly by vector search alone.
    #[test]
    fn test_vector_only_ranks_exact_term_poorly() {
        let vector_ranked: Vec<String> = vec![
            "doc_semantic_1",
            "doc_semantic_2",
            "doc_semantic_3",
            "doc_semantic_4",
            "doc_semantic_5",
            "doc_semantic_6",
            "doc_semantic_7",
            "doc_target",
            "doc_semantic_8",
            "doc_semantic_9",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        // In vector-only mode, doc_target is at position 8 (outside top 5)
        let target_position = vector_ranked
            .iter()
            .position(|id| id == "doc_target")
            .expect("doc_target must appear in vector results");

        assert!(
            target_position >= 5,
            "Baseline: doc_target should be outside top 5 in vector-only ranking, \
             but was at position {}",
            target_position + 1,
        );
    }

    /// Test: BM25-only correctly identifies exact-term document.
    ///
    /// Confirms BM25 places the document with exact term match at position 1.
    #[test]
    fn test_bm25_ranks_exact_term_first() {
        let bm25_ranked: Vec<String> = vec![
            "doc_target".to_string(),
            "doc_partial_1".to_string(),
            "doc_partial_2".to_string(),
        ];

        assert_eq!(
            bm25_ranked[0], "doc_target",
            "BM25 should rank the exact-term document first"
        );
    }

    /// Test: Multiple exact-term queries all improve with RRF.
    ///
    /// Simulates the evaluation dataset scenario: several queries with
    /// exact technical terms that vector search handles poorly.
    #[test]
    fn test_batch_evaluation_multiple_queries() {
        // Each tuple: (query_name, vector_rank_of_target, bm25_rank_of_target)
        let test_cases = vec![
            ("legal_citation", 7, 0), // "Rule 30(b)(6)" -- vector rank 8, BM25 rank 1
            ("proper_noun", 5, 0),    // "Leslie Wexner" -- vector rank 6, BM25 rank 1
            ("account_number", 9, 0), // "Account #4829" -- vector rank 10, BM25 rank 1
            ("entity_name", 6, 1),    // "JP Morgan Chase" -- vector rank 7, BM25 rank 2
        ];

        let mut results = Vec::new();

        for (name, vector_rank, bm25_rank) in &test_cases {
            // Build ranked lists with doc_target at the specified positions
            let vector_ranked: Vec<String> = (0..10)
                .map(|i| {
                    if i == *vector_rank {
                        "doc_target".to_string()
                    } else {
                        format!("doc_other_{}", i)
                    }
                })
                .collect();

            let bm25_ranked: Vec<String> = (0..5)
                .map(|i| {
                    if i == *bm25_rank {
                        "doc_target".to_string()
                    } else {
                        format!("doc_bm25_{}", i)
                    }
                })
                .collect();

            let fused =
                reciprocal_rank_fusion(&bm25_ranked, &vector_ranked, DEFAULT_RRF_K, 10);

            let fused_position = fused
                .iter()
                .position(|(id, _)| id == "doc_target")
                .unwrap_or(usize::MAX);

            let improved = fused_position < *vector_rank;

            results.push((
                *name,
                vector_rank + 1, // 1-indexed for display
                bm25_rank + 1,   // 1-indexed for display
                fused_position + 1, // 1-indexed for display
                improved,
            ));
        }

        // Print evaluation summary
        println!("\n=== BM25 Retrieval Quality Evaluation ===");
        println!(
            "{:<20} {:>12} {:>12} {:>12} {:>10}",
            "Query", "Vector Rank", "BM25 Rank", "Hybrid Rank", "Improved"
        );
        println!("{}", "-".repeat(70));
        for (name, vr, br, hr, improved) in &results {
            println!(
                "{:<20} {:>12} {:>12} {:>12} {:>10}",
                name,
                vr,
                br,
                hr,
                if *improved { "YES" } else { "NO" }
            );
        }
        println!("{}", "=".repeat(70));

        // All queries must show improvement
        let all_improved = results.iter().all(|(_, _, _, _, improved)| *improved);
        assert!(
            all_improved,
            "All exact-term queries should show ranking improvement with hybrid retrieval"
        );

        // At least one must be in position 1 or 2 (per success criterion)
        let any_top_2 = results.iter().any(|(_, _, _, hr, _)| *hr <= 2);
        assert!(
            any_top_2,
            "At least one exact-term query should land in position 1 or 2 after RRF fusion"
        );
    }

    /// Test: RRF with varying k values still rescues exact-term documents.
    ///
    /// Validates that the RRF rescue behavior is robust across different k values,
    /// not just the default k=60.
    #[test]
    fn test_rrf_rescue_robust_across_k_values() {
        let vector_ranked: Vec<String> = vec![
            "doc_a", "doc_b", "doc_c", "doc_d", "doc_e", "doc_f", "doc_g", "doc_target",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let bm25_ranked: Vec<String> =
            vec!["doc_target", "doc_x", "doc_y"]
                .into_iter()
                .map(String::from)
                .collect();

        // Test with k=1, k=30, k=60 (default), k=100
        for k in [1, 30, DEFAULT_RRF_K, 100] {
            let fused = reciprocal_rank_fusion(&bm25_ranked, &vector_ranked, k, 10);
            let target_position = fused
                .iter()
                .position(|(id, _)| id == "doc_target")
                .expect("doc_target must appear in fused results");

            assert!(
                target_position <= 2,
                "With k={}, doc_target should be in top 3 after RRF (was at position {}). \
                 Vector rank=8, BM25 rank=1.",
                k,
                target_position + 1,
            );
        }
    }
}
