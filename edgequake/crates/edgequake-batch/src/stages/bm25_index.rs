//! BM25 preprocessing for batch ingestion.
//!
//! Converts prepared chunks into `Bm25Document` structs for storage
//! in the BM25 inverted index. Uses the shared tantivy tokenizer
//! from `edgequake_core::bm25_text` to ensure index/query consistency.
//!
//! # What gets indexed
//!
//! **Chunk text only** (not entity names). Per discretion recommendation:
//! chunks contain the original document text where exact terms appear in
//! natural context. Entity names are already well-served by vector search.
//!
//! # ID format
//!
//! The `doc_id` in `Bm25Document` uses `chunk.custom_id` directly,
//! matching the ID format used in vector storage (`VectorStorage::upsert`).
//! This is critical for RRF fusion to match documents across both BM25
//! and vector result sets.

use crate::jsonl::PreparedChunk;
use edgequake_core::bm25_text::{create_bm25_analyzer, tokenize_text};
use edgequake_storage::Bm25Document;

/// Preprocess chunks into BM25 documents for inverted index storage.
///
/// Creates a single tantivy analyzer and tokenizes each chunk's text content
/// into stemmed, lowercased terms. The resulting `Bm25Document` structs are
/// ready for `Bm25Storage::index_batch()`.
///
/// # Arguments
///
/// * `chunks` - Prepared chunks from the batch pipeline's Prepare phase
///
/// # Returns
///
/// A vector of `Bm25Document` structs with pre-processed terms, one per chunk.
/// Empty chunks (no terms after tokenization) are included with empty term vectors
/// so that document count (N) in corpus stats remains accurate.
pub fn build_bm25_documents(chunks: &[PreparedChunk]) -> Vec<Bm25Document> {
    let mut analyzer = create_bm25_analyzer();
    let mut docs = Vec::with_capacity(chunks.len());

    for chunk in chunks {
        let terms = tokenize_text(&mut analyzer, &chunk.chunk.content);
        let doc_length = terms.len();

        docs.push(Bm25Document {
            doc_id: chunk.custom_id.clone(),
            terms,
            doc_length,
            source_id: chunk.doc_hash.clone(),
        });
    }

    docs
}

#[cfg(test)]
mod tests {
    use super::*;
    use edgequake_pipeline::chunker::TextChunk;

    fn make_chunk(custom_id: &str, content: &str, doc_hash: &str) -> PreparedChunk {
        PreparedChunk {
            custom_id: custom_id.to_string(),
            chunk: TextChunk {
                id: custom_id.to_string(),
                content: content.to_string(),
                index: 0,
                start_offset: 0,
                end_offset: content.len(),
                start_line: 1,
                end_line: 1,
                token_count: content.split_whitespace().count(),
                embedding: None,
            },
            doc_hash: doc_hash.to_string(),
            doc_filename: "test.parquet".to_string(),
            metadata: None,
        }
    }

    #[test]
    fn test_build_bm25_documents_basic() {
        let chunks = vec![
            make_chunk(
                "chunk-abc-0",
                "Jeffrey Epstein financial documents",
                "hash1",
            ),
            make_chunk(
                "chunk-abc-1",
                "Investigation into offshore accounts",
                "hash1",
            ),
        ];

        let docs = build_bm25_documents(&chunks);

        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].doc_id, "chunk-abc-0");
        assert_eq!(docs[0].source_id, "hash1");
        assert!(docs[0].doc_length > 0);
        assert!(!docs[0].terms.is_empty());

        // Terms should be lowercased and stemmed
        for term in &docs[0].terms {
            assert_eq!(term, &term.to_lowercase());
        }
    }

    #[test]
    fn test_build_bm25_documents_empty_content() {
        let chunks = vec![make_chunk("chunk-empty-0", "", "hash2")];

        let docs = build_bm25_documents(&chunks);

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].doc_length, 0);
        assert!(docs[0].terms.is_empty());
    }

    #[test]
    fn test_doc_id_matches_vector_format() {
        // Vector storage uses chunk.custom_id directly (no prefix)
        // BM25 must match for RRF fusion compatibility
        let chunks = vec![make_chunk("chunk-abc12345-0", "test content", "hash3")];

        let docs = build_bm25_documents(&chunks);

        assert_eq!(docs[0].doc_id, "chunk-abc12345-0");
    }

    #[test]
    fn test_uses_shared_analyzer() {
        // Verify that the same tokenization is produced as edgequake_core::bm25_text
        let chunks = vec![make_chunk("c1", "running quickly", "h1")];
        let docs = build_bm25_documents(&chunks);

        let mut analyzer = create_bm25_analyzer();
        let expected_terms = tokenize_text(&mut analyzer, "running quickly");

        assert_eq!(docs[0].terms, expected_terms);
    }
}
