//! In-memory BM25 inverted index storage.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

use crate::error::{Result, StorageError};
use crate::traits::{Bm25Document, Bm25SearchResult, Bm25Storage};

const K1: f64 = 1.5;
const B: f64 = 0.75;

#[derive(Default)]
struct CorpusStats {
    doc_count: usize,
    avg_doc_length: f64,
}

pub struct MemoryBm25Storage {
    namespace: String,
    docs: RwLock<HashMap<String, Bm25Document>>,
    postings: RwLock<HashMap<String, HashMap<String, usize>>>,
    stats: RwLock<CorpusStats>,
}

impl MemoryBm25Storage {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            docs: RwLock::new(HashMap::new()),
            postings: RwLock::new(HashMap::new()),
            stats: RwLock::new(CorpusStats::default()),
        }
    }
}

#[async_trait]
impl Bm25Storage for MemoryBm25Storage {
    fn namespace(&self) -> &str {
        &self.namespace
    }

    async fn ensure_tables(&self) -> Result<()> {
        Ok(())
    }

    async fn index_batch(&self, docs: &[Bm25Document]) -> Result<()> {
        let mut all_docs = self
            .docs
            .write()
            .map_err(|e| StorageError::Database(format!("Lock error: {}", e)))?;
        let mut postings = self
            .postings
            .write()
            .map_err(|e| StorageError::Database(format!("Lock error: {}", e)))?;

        for doc in docs {
            if let Some(old_doc) = all_docs.insert(doc.doc_id.clone(), doc.clone()) {
                for term in old_doc.terms {
                    if let Some(term_postings) = postings.get_mut(&term) {
                        term_postings.remove(&old_doc.doc_id);
                    }
                }
            }

            let mut term_freqs: HashMap<&str, usize> = HashMap::new();
            for term in &doc.terms {
                *term_freqs.entry(term.as_str()).or_default() += 1;
            }
            for (term, freq) in term_freqs {
                postings
                    .entry(term.to_string())
                    .or_default()
                    .insert(doc.doc_id.clone(), freq);
            }
        }

        Ok(())
    }

    async fn update_corpus_stats(&self) -> Result<()> {
        let docs = self
            .docs
            .read()
            .map_err(|e| StorageError::Database(format!("Lock error: {}", e)))?;
        let doc_count = docs.len();
        let total_len: usize = docs.values().map(|doc| doc.doc_length).sum();
        let avg_doc_length = if doc_count == 0 {
            0.0
        } else {
            total_len as f64 / doc_count as f64
        };
        let mut stats = self
            .stats
            .write()
            .map_err(|e| StorageError::Database(format!("Lock error: {}", e)))?;
        stats.doc_count = doc_count;
        stats.avg_doc_length = avg_doc_length;
        Ok(())
    }

    async fn query(&self, terms: &[String], top_k: usize) -> Result<Vec<Bm25SearchResult>> {
        let docs = self
            .docs
            .read()
            .map_err(|e| StorageError::Database(format!("Lock error: {}", e)))?;
        let postings = self
            .postings
            .read()
            .map_err(|e| StorageError::Database(format!("Lock error: {}", e)))?;
        let stats = self
            .stats
            .read()
            .map_err(|e| StorageError::Database(format!("Lock error: {}", e)))?;

        if stats.doc_count == 0 || terms.is_empty() {
            return Ok(Vec::new());
        }

        let mut scores: HashMap<String, (f64, Vec<String>)> = HashMap::new();
        for term in terms {
            let Some(term_postings) = postings.get(term) else {
                continue;
            };
            let df = term_postings.len() as f64;
            let idf = ((stats.doc_count as f64 - df + 0.5) / (df + 0.5) + 1.0).ln();

            for (doc_id, tf) in term_postings {
                let Some(doc) = docs.get(doc_id) else {
                    continue;
                };
                let doc_len = doc.doc_length.max(1) as f64;
                let avgdl = stats.avg_doc_length.max(1.0);
                let tf = *tf as f64;
                let denom = tf + K1 * (1.0 - B + B * (doc_len / avgdl));
                let contribution = idf * ((tf * (K1 + 1.0)) / denom);
                let entry = scores.entry(doc_id.clone()).or_default();
                entry.0 += contribution;
                if !entry.1.contains(term) {
                    entry.1.push(term.clone());
                }
            }
        }

        let mut results: Vec<_> = scores
            .into_iter()
            .map(|(doc_id, (score, term_matches))| Bm25SearchResult {
                doc_id,
                score,
                term_matches,
            })
            .collect();
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.doc_id.cmp(&b.doc_id))
        });
        results.truncate(top_k);
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_bm25_ranks_deterministically() {
        let storage = MemoryBm25Storage::new("test");
        storage.ensure_tables().await.unwrap();
        storage
            .index_batch(&[
                Bm25Document {
                    doc_id: "doc-a".to_string(),
                    terms: vec!["alpha".into(), "alpha".into(), "beta".into()],
                    doc_length: 3,
                    source_id: "source-a".to_string(),
                },
                Bm25Document {
                    doc_id: "doc-b".to_string(),
                    terms: vec!["alpha".into(), "gamma".into()],
                    doc_length: 2,
                    source_id: "source-b".to_string(),
                },
                Bm25Document {
                    doc_id: "doc-c".to_string(),
                    terms: vec!["delta".into()],
                    doc_length: 1,
                    source_id: "source-c".to_string(),
                },
            ])
            .await
            .unwrap();
        storage.update_corpus_stats().await.unwrap();

        let results = storage.query(&["alpha".to_string()], 10).await.unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].doc_id, "doc-a");
        assert_eq!(results[1].doc_id, "doc-b");
        assert_eq!(results[0].term_matches, vec!["alpha"]);
    }
}
