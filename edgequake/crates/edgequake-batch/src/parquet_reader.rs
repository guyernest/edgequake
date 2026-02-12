//! Parquet reader for reconstructing documents from line-by-line rows.
//!
//! The Epstein dataset stores documents as individual rows with `text` column.
//! Row 0 is a CSV header (`filename,text`), and subsequent rows contain
//! `filename,text` pairs. Documents are reconstructed by grouping consecutive
//! rows with the same filename prefix.

use arrow::array::{Array, StringArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use tracing::{debug, info, warn};

/// A reconstructed document from the parquet file.
#[derive(Debug, Clone)]
pub struct ReconstructedDocument {
    /// Original filename from the parquet data.
    pub filename: String,

    /// Full text content (all lines joined).
    pub content: String,

    /// SHA-256 hash of the content.
    pub content_hash: String,

    /// Number of source rows that made up this document.
    pub source_row_count: usize,
}

/// Read and reconstruct documents from a line-by-line parquet file.
///
/// The parquet file has a single `text` column where:
/// - Row 0 is a CSV header: `filename,text`
/// - Subsequent rows are `filename,text` pairs
///
/// Documents are reconstructed by grouping rows with the same filename.
pub fn read_parquet_documents(path: &Path) -> anyhow::Result<Vec<ReconstructedDocument>> {
    info!(path = %path.display(), "Reading parquet file");

    let file = std::fs::File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    // Collect all rows: filename -> lines
    //
    // The parquet has a single `text` column containing flattened CSV:
    //   Row 0: "filename,text"        (header)
    //   Row 1: "somefile.txt,first line of doc"
    //   Row 2: "continuation line"    (no comma with .txt prefix)
    //   Row 3: ""                     (blank line in document)
    //   Row N: "otherfile.txt,first line of next doc"
    //
    // Only ~1% of rows start a new document (filename prefix before comma).
    // The rest are continuation lines that belong to the current document.
    let mut documents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current_filename: Option<String> = None;
    let mut total_rows = 0usize;
    let mut skipped_rows = 0usize;

    for batch_result in reader {
        let batch = batch_result?;

        // Get the "text" column
        let text_col = batch
            .column_by_name("text")
            .ok_or_else(|| anyhow::anyhow!("No 'text' column found in parquet file"))?;

        let text_array = text_col
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("'text' column is not a string array"))?;

        for i in 0..text_array.len() {
            total_rows += 1;

            if text_array.is_null(i) {
                skipped_rows += 1;
                continue;
            }

            let row_text = text_array.value(i);

            // Skip the CSV header row
            if total_rows == 1 && row_text.starts_with("filename,") {
                debug!("Skipping CSV header row");
                continue;
            }

            // Check if this row starts a new document (has "filename.txt," prefix)
            if let Some(comma_pos) = row_text.find(',') {
                let candidate = &row_text[..comma_pos];
                if candidate.ends_with(".txt") || candidate.ends_with(".pdf") || candidate.ends_with(".doc") {
                    let filename = candidate.trim().to_string();
                    let text = row_text[comma_pos + 1..].to_string();

                    if filename.is_empty() {
                        skipped_rows += 1;
                        continue;
                    }

                    current_filename = Some(filename.clone());
                    documents.entry(filename).or_default().push(text);
                    continue;
                }
            }

            // Continuation line — append to current document
            if let Some(ref filename) = current_filename {
                documents.entry(filename.clone()).or_default().push(row_text.to_string());
            } else {
                // No document started yet, skip orphan lines
                skipped_rows += 1;
            }
        }
    }

    info!(
        total_rows = total_rows,
        unique_documents = documents.len(),
        skipped_rows = skipped_rows,
        "Parquet file parsed"
    );

    // Reconstruct documents
    let mut result: Vec<ReconstructedDocument> = documents
        .into_iter()
        .map(|(filename, lines)| {
            let content = lines.join("\n");
            let content_hash = compute_sha256(&content);
            let source_row_count = lines.len();

            ReconstructedDocument {
                filename,
                content,
                content_hash,
                source_row_count,
            }
        })
        .collect();

    // Sort by filename for deterministic ordering
    result.sort_by(|a, b| a.filename.cmp(&b.filename));

    info!(
        documents = result.len(),
        "Documents reconstructed from parquet"
    );

    Ok(result)
}

/// Compute SHA-256 hash of content, returning first 16 hex chars.
pub fn compute_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize();
    format!("{:x}", hash)[..16].to_string()
}

/// Compute 8-char hash prefix for custom_id generation.
pub fn compute_hash8(content: &str) -> String {
    compute_sha256(content)[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256() {
        let hash = compute_sha256("test content");
        assert_eq!(hash.len(), 16);
        // Should be deterministic
        assert_eq!(hash, compute_sha256("test content"));
        // Different content should produce different hash
        assert_ne!(hash, compute_sha256("other content"));
    }

    #[test]
    fn test_compute_hash8() {
        let hash = compute_hash8("test content");
        assert_eq!(hash.len(), 8);
    }
}
