//! BM25 inverted index storage trait for keyword search.
//!
//! # Implements
//!
//! - **RET-01**: BM25 inverted index populated at ingestion time
//!
//! # Enforces
//!
//! - **BR0201**: Namespace-based tenant isolation (separate tables per namespace)
//!
//! # WHY: Separate BM25 Storage
//!
//! BM25 keyword search complements vector similarity search:
//! - Exact term matching (proper nouns, legal citations, technical terms)
//! - Transparent scoring formula (IDF * TF-norm)
//! - No embedding model dependency
//!
//! Abstracting as a trait allows:
//! - AthenaBm25Storage (Iceberg tables queried via Athena SQL)
//! - In-memory implementation (testing)
//! - Alternative backends (DynamoDB, OpenSearch) if needed

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A document prepared for BM25 indexing.
///
/// Terms are pre-tokenized, lowercased, and stemmed by the caller
/// using the shared tokenizer from `edgequake_core::bm25_text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Document {
    /// Document identifier (typically chunk ID, same as vector storage ID)
    pub doc_id: String,
    /// Pre-processed terms (tokenized, lowercased, stemmed)
    pub terms: Vec<String>,
    /// Number of terms in the document
    pub doc_length: usize,
    /// Source document identifier for provenance tracking
    pub source_id: String,
}

/// A single BM25 search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25SearchResult {
    /// Document identifier
    pub doc_id: String,
    /// BM25 relevance score (higher is more relevant)
    pub score: f64,
    /// Query terms that matched in this document
    pub term_matches: Vec<String>,
}

/// BM25 inverted index storage interface.
///
/// Provides storage and retrieval of BM25 inverted index data
/// with support for keyword-based search scoring.
///
/// # Table Structure
///
/// Each namespace has 4 tables:
/// - `{ns}_postings`: term -> document -> frequency
/// - `{ns}_term_stats`: per-term document frequency
/// - `{ns}_docs`: document metadata (length, source)
/// - `{ns}_corpus_stats`: global statistics (N, avgdl)
///
/// # Implementations
///
/// - `AthenaBm25Storage` - Athena Iceberg tables on S3
#[async_trait]
pub trait Bm25Storage: Send + Sync {
    /// Get the storage namespace.
    fn namespace(&self) -> &str;

    /// Ensure BM25 index tables exist for this namespace.
    ///
    /// Creates tables lazily on first use via `CREATE TABLE IF NOT EXISTS`.
    /// Idempotent -- safe to call multiple times.
    async fn ensure_tables(&self) -> Result<()>;

    /// Index a batch of documents into the BM25 inverted index.
    ///
    /// Terms in each `Bm25Document` must be pre-tokenized and stemmed
    /// using the same analyzer as query-time preprocessing.
    ///
    /// # Arguments
    ///
    /// * `docs` - Documents with pre-processed terms to index
    async fn index_batch(&self, docs: &[Bm25Document]) -> Result<()>;

    /// Recalculate corpus statistics after a batch ingestion.
    ///
    /// Updates the corpus-wide document count (N) and average document
    /// length (avgdl) used in BM25 scoring. Must be called after
    /// `index_batch` to ensure accurate scoring.
    async fn update_corpus_stats(&self) -> Result<()>;

    /// Query BM25 scores for a set of pre-processed query terms.
    ///
    /// Terms must be pre-processed with the same analyzer used at
    /// index time to ensure term matching consistency.
    ///
    /// # Arguments
    ///
    /// * `terms` - Pre-processed query terms (tokenized, lowercased, stemmed)
    /// * `top_k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results ordered by BM25 score (highest first).
    async fn query(&self, terms: &[String], top_k: usize) -> Result<Vec<Bm25SearchResult>>;
}
