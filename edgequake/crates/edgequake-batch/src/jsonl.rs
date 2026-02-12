//! JSONL file builder for OpenAI Batch API requests.
//!
//! Builds JSONL files from chunked documents, splitting at the 50K request
//! limit per batch job.

use crate::config::BatchConfig;
use crate::parquet_reader::compute_hash8;
use edgequake_llm::providers::openai_batch::build_jsonl_request;
use edgequake_pipeline::chunker::TextChunk;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::info;

/// A chunk prepared for batch extraction with its custom_id.
#[derive(Debug, Clone)]
pub struct PreparedChunk {
    /// Custom ID for OpenAI batch correlation: chunk-{hash8}-{idx}
    pub custom_id: String,

    /// The text chunk.
    pub chunk: TextChunk,

    /// Document content hash this chunk belongs to.
    pub doc_hash: String,

    /// Original document filename.
    pub doc_filename: String,
}

/// Result of building JSONL files.
#[derive(Debug)]
pub struct JsonlBuildResult {
    /// Paths to generated JSONL files.
    pub file_paths: Vec<PathBuf>,

    /// Total number of requests across all files.
    pub total_requests: usize,

    /// Number of batch jobs needed (one per file).
    pub batch_count: usize,
}

/// Build JSONL files from prepared chunks.
///
/// Splits into multiple files if the request count exceeds the 50K limit.
///
/// # Arguments
/// * `chunks` - Prepared chunks with custom_ids
/// * `system_prompt` - Static system prompt (for prompt caching)
/// * `user_prompt_fn` - Function to generate user prompt from chunk text
/// * `config` - Batch configuration
pub fn build_jsonl_files(
    chunks: &[PreparedChunk],
    system_prompt: &str,
    user_prompt_fn: &dyn Fn(&str) -> String,
    config: &BatchConfig,
) -> anyhow::Result<JsonlBuildResult> {
    std::fs::create_dir_all(config.jsonl_dir())?;

    let mut file_paths = Vec::new();
    let mut current_file_idx = 0;
    let mut current_count = 0;
    let mut writer: Option<std::io::BufWriter<std::fs::File>> = None;

    let mut current_size: usize = 0;

    for chunk in chunks {
        // Start a new file if needed (split on request count OR file size)
        if writer.is_none()
            || current_count >= BatchConfig::MAX_BATCH_REQUESTS
            || current_size >= BatchConfig::MAX_JSONL_SIZE
        {
            // Flush previous file
            if let Some(ref mut w) = writer {
                w.flush()?;
            }

            let file_path = config
                .jsonl_dir()
                .join(format!("batch_{:04}.jsonl", current_file_idx));
            let file = std::fs::File::create(&file_path)?;
            writer = Some(std::io::BufWriter::new(file));
            file_paths.push(file_path);
            current_file_idx += 1;
            current_count = 0;
            current_size = 0;
        }

        let user_prompt = user_prompt_fn(&chunk.chunk.content);
        let request = build_jsonl_request(
            &chunk.custom_id,
            &config.extraction_model,
            system_prompt,
            &user_prompt,
            config.max_tokens,
            0.0, // Deterministic extraction
        );

        let line = serde_json::to_string(&request)?;
        if let Some(ref mut w) = writer {
            writeln!(w, "{}", line)?;
        }
        current_size += line.len() + 1; // +1 for newline
        current_count += 1;
    }

    // Flush last file
    if let Some(ref mut w) = writer {
        w.flush()?;
    }

    let total_requests = chunks.len();
    let batch_count = file_paths.len();

    info!(
        total_requests = total_requests,
        batch_files = batch_count,
        "JSONL files built"
    );

    Ok(JsonlBuildResult {
        file_paths,
        total_requests,
        batch_count,
    })
}

/// Create a custom_id for a chunk within a document.
///
/// Format: `chunk-{doc_hash8}-{chunk_index}`
pub fn make_custom_id(doc_content_hash: &str, chunk_index: usize) -> String {
    let hash8 = &doc_content_hash[..8.min(doc_content_hash.len())];
    format!("chunk-{}-{}", hash8, chunk_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_custom_id() {
        let id = make_custom_id("abc12345def67890", 0);
        assert_eq!(id, "chunk-abc12345-0");

        let id = make_custom_id("abc12345def67890", 42);
        assert_eq!(id, "chunk-abc12345-42");
    }

    #[test]
    fn test_make_custom_id_short_hash() {
        let id = make_custom_id("abc", 0);
        assert_eq!(id, "chunk-abc-0");
    }
}
