//! Dry-run mode: preview document count, chunk estimate, schema summary, and
//! estimated OpenAI API cost without making any API calls.
//!
//! Uses document sampling to extrapolate chunk counts and costs for the full
//! dataset. Requires namespace config (from DynamoDB or --config-file) to show
//! the approved schema, but does NOT require an OpenAI API key.

use crate::config::BatchConfig;
use crate::directory_scanner::DirectoryScanner;
use crate::document_reader;
use crate::namespace_resolver::ResolvedConfig;
use crate::parquet_reader::{read_parquet_documents, ReconstructedDocument};
use edgequake_pipeline::chunker::{Chunker, ChunkerConfig, HeadingBoundaryChunking, TextChunk};
use std::path::Path;
use std::sync::Arc;
use tracing::info;

/// Cost estimate breakdown for the dry-run report.
struct CostEstimate {
    /// Cost of extraction input tokens.
    extraction_input_cost: f64,
    /// Cost of extraction output tokens.
    extraction_output_cost: f64,
    /// Cost of embedding tokens.
    embedding_cost: f64,
    /// Estimated embedding items (chunks + entities + relationships).
    embedding_items: usize,
    /// Total estimated input tokens for extraction.
    total_input_tokens: f64,
    /// Total estimated output tokens for extraction.
    total_output_tokens: f64,
    /// Grand total cost.
    total: f64,
}

/// Run the dry-run preview.
///
/// Scans all documents, samples a subset for chunking, extrapolates chunk count
/// and cost, and prints a structured report. Does NOT call any external APIs.
pub async fn run_dry_run(config: &BatchConfig, resolved: &ResolvedConfig) -> anyhow::Result<()> {
    info!("Starting dry-run preview (no API calls will be made)");

    // Step 1: Scan/read all documents (local I/O only)
    let (documents, md_count, txt_count) = read_documents(config)?;
    let total_docs = documents.len();

    if total_docs == 0 {
        println!("\n=== Dry Run Preview ===\n");
        println!("Documents:        0 files");
        println!("\nNo documents found to process.");
        return Ok(());
    }

    // Step 2: Select sample using uniform distribution (every Nth doc)
    let sample_size = compute_sample_size(total_docs);
    let sample = select_uniform_sample(&documents, sample_size);

    // Step 3: Chunk the sample documents
    let chunker_config = ChunkerConfig {
        chunk_size: config.chunk_size,
        chunk_overlap: config.chunk_overlap,
        ..ChunkerConfig::default()
    };
    let text_chunker = Chunker::new(chunker_config.clone());
    let md_chunker =
        Chunker::with_strategy(chunker_config, Arc::new(HeadingBoundaryChunking::new()));

    let mut total_sample_chunks: usize = 0;
    let mut total_sample_tokens: usize = 0;

    for doc in &sample {
        let chunks = chunk_document(doc, &text_chunker, &md_chunker).await;
        total_sample_tokens += chunks.iter().map(|c| c.token_count).sum::<usize>();
        total_sample_chunks += chunks.len();
    }

    let actual_sample_size = sample.len();
    let avg_chunks_per_doc = if actual_sample_size > 0 {
        total_sample_chunks as f64 / actual_sample_size as f64
    } else {
        0.0
    };
    let estimated_chunks = (avg_chunks_per_doc * total_docs as f64).ceil() as usize;

    // Step 4: Estimate token counts and cost
    let avg_tokens_per_chunk = if total_sample_chunks > 0 {
        total_sample_tokens as f64 / total_sample_chunks as f64
    } else {
        0.0
    };
    let estimated_input_tokens = avg_tokens_per_chunk * estimated_chunks as f64;

    let use_anthropic =
        edgequake_llm::providers::anthropic_batch::is_anthropic_model(&resolved.extraction_model);

    let cost = if use_anthropic {
        estimate_anthropic_cost(
            &resolved.extraction_model,
            &resolved.embedding_model,
            estimated_input_tokens,
            estimated_chunks,
        )
    } else {
        estimate_openai_cost(
            &resolved.extraction_model,
            &resolved.embedding_model,
            estimated_input_tokens,
            estimated_chunks,
        )
    };

    // Step 5: Print the report
    print_dry_run_report(
        total_docs,
        (md_count, txt_count),
        actual_sample_size,
        avg_chunks_per_doc,
        estimated_chunks,
        resolved,
        &cost,
        &config.namespace,
        use_anthropic,
    );

    Ok(())
}

/// Read all documents from the configured data path.
///
/// Returns (documents, markdown_count, text_count).
fn read_documents(
    config: &BatchConfig,
) -> anyhow::Result<(Vec<ReconstructedDocument>, usize, usize)> {
    let data_path = &config.data_path;

    if data_path.is_dir() {
        let scanner = DirectoryScanner::new(data_path)
            .recursive(!config.no_recurse)
            .include_patterns(&config.include_patterns)
            .exclude_patterns(&config.exclude_patterns);

        let scan_result = scanner.scan()?;
        let md_count = scan_result.md_files.len();
        let txt_count = scan_result.txt_files.len();

        let supported = scan_result.supported_files();
        if supported.is_empty() {
            return Ok((Vec::new(), 0, 0));
        }

        let documents = document_reader::read_all_documents(
            &supported,
            data_path,
            config.delimiter.as_deref(),
            config.no_split,
        )?;

        Ok((documents, md_count, txt_count))
    } else if data_path.is_file() {
        match data_path.extension().and_then(|e| e.to_str()) {
            Some("parquet") => {
                let documents = read_parquet_documents(data_path)?;
                Ok((documents, 0, 0))
            }
            Some("md") | Some("markdown") => {
                let base_dir = data_path.parent().unwrap_or(Path::new("."));
                let documents = document_reader::read_single_file(
                    data_path,
                    base_dir,
                    config.delimiter.as_deref(),
                    config.no_split,
                )?;
                Ok((documents, 1, 0))
            }
            Some("txt") => {
                let base_dir = data_path.parent().unwrap_or(Path::new("."));
                let documents = document_reader::read_single_file(
                    data_path,
                    base_dir,
                    config.delimiter.as_deref(),
                    config.no_split,
                )?;
                Ok((documents, 0, 1))
            }
            Some(ext) => anyhow::bail!(
                "Unsupported file type: .{} (supported: .parquet, .txt, .md)",
                ext
            ),
            None => anyhow::bail!(
                "Cannot determine file type (no extension): {}",
                data_path.display()
            ),
        }
    } else {
        anyhow::bail!("Data path does not exist: {}", data_path.display())
    }
}

/// Compute sample size: min(50, max(3, ceil(total * 0.10)))
fn compute_sample_size(total: usize) -> usize {
    let ten_pct = ((total as f64) * 0.10).ceil() as usize;
    let at_least_3 = ten_pct.max(3.min(total));
    at_least_3.min(50)
}

/// Select a uniform sample of documents (every Nth, not first N).
fn select_uniform_sample(
    documents: &[ReconstructedDocument],
    sample_size: usize,
) -> Vec<&ReconstructedDocument> {
    if sample_size >= documents.len() {
        return documents.iter().collect();
    }

    let step = documents.len() as f64 / sample_size as f64;
    (0..sample_size)
        .map(|i| {
            let idx = (i as f64 * step).floor() as usize;
            &documents[idx.min(documents.len() - 1)]
        })
        .collect()
}

/// Chunk a single document, selecting the appropriate chunker based on file extension.
async fn chunk_document(
    doc: &ReconstructedDocument,
    text_chunker: &Chunker,
    md_chunker: &Chunker,
) -> Vec<TextChunk> {
    let is_md = doc.filename.ends_with(".md") || doc.filename.ends_with(".markdown");

    if is_md {
        match md_chunker
            .chunk_async(&doc.content, &doc.content_hash)
            .await
        {
            Ok(chunks) => chunks,
            Err(_) => {
                // Fall back to text chunker on markdown chunking error
                text_chunker
                    .chunk(&doc.content, &doc.content_hash)
                    .unwrap_or_default()
            }
        }
    } else {
        text_chunker
            .chunk(&doc.content, &doc.content_hash)
            .unwrap_or_default()
    }
}

/// Estimate Anthropic API cost based on model pricing and estimated token counts.
///
/// Uses Anthropic Batch API rates (50% discount on standard pricing).
fn estimate_anthropic_cost(
    extraction_model: &str,
    embedding_model: &str,
    estimated_input_tokens: f64,
    estimated_chunks: usize,
) -> CostEstimate {
    // Anthropic Batch API rates (50% of standard) per 1M tokens
    // Source: https://platform.claude.com/docs/en/about-claude/pricing#batch-processing
    let (input_rate, output_rate) = match extraction_model {
        // Haiku 3: $0.125 input, $0.625 output per MTok
        m if m.contains("haiku-3") && !m.contains("haiku-3.5") => {
            (0.125 / 1_000_000.0, 0.625 / 1_000_000.0)
        }
        // Haiku 3.5: $0.40 input, $2.00 output per MTok
        m if m.contains("haiku-3.5") || m.contains("haiku-3-5") => {
            (0.40 / 1_000_000.0, 2.00 / 1_000_000.0)
        }
        // Haiku 4.5: $0.50 input, $2.50 output per MTok
        m if m.contains("haiku") => (0.50 / 1_000_000.0, 2.50 / 1_000_000.0),
        // Opus 4 / 4.1: $7.50 input, $37.50 output per MTok
        m if m.contains("opus-4-1")
            || m.contains("opus-4-0")
            || (m.contains("opus-4")
                && !m.contains("opus-4.")
                && !m.contains("opus-4-5")
                && !m.contains("opus-4-6")) =>
        {
            (7.50 / 1_000_000.0, 37.50 / 1_000_000.0)
        }
        // Opus 4.5 / 4.6: $2.50 input, $12.50 output per MTok
        m if m.contains("opus") => (2.50 / 1_000_000.0, 12.50 / 1_000_000.0),
        // Sonnet (all versions 3.7, 4, 4.5, 4.6): $1.50 input, $7.50 output per MTok
        _ => (1.50 / 1_000_000.0, 7.50 / 1_000_000.0),
    };

    // Embedding rates (standard, not batch) — embeddings still use OpenAI
    let embed_rate = match embedding_model {
        m if m.contains("text-embedding-3-large") => 0.13 / 1_000_000.0,
        m if m.contains("text-embedding-3-small") => 0.02 / 1_000_000.0,
        _ => 0.02 / 1_000_000.0,
    };

    // System prompt is ~1500 tokens per request
    let system_prompt_tokens = 1500.0;
    let total_input_tokens =
        estimated_input_tokens + (system_prompt_tokens * estimated_chunks as f64);

    // Estimate ~500 output tokens per chunk on average
    let total_output_tokens = 500.0 * estimated_chunks as f64;

    let extraction_input_cost = total_input_tokens * input_rate;
    let extraction_output_cost = total_output_tokens * output_rate;

    // Embedding items: chunks + ~3 entities + ~2 relationships per chunk = 6x
    let embedding_items = estimated_chunks * 6;
    let avg_embed_tokens = 50.0;
    let embedding_cost = embedding_items as f64 * avg_embed_tokens * embed_rate;

    CostEstimate {
        extraction_input_cost,
        extraction_output_cost,
        embedding_cost,
        embedding_items,
        total_input_tokens,
        total_output_tokens,
        total: extraction_input_cost + extraction_output_cost + embedding_cost,
    }
}

/// Estimate OpenAI API cost based on model pricing and estimated token counts.
///
/// Uses OpenAI Batch API rates (50% discount on standard pricing).
fn estimate_openai_cost(
    extraction_model: &str,
    embedding_model: &str,
    estimated_input_tokens: f64,
    estimated_chunks: usize,
) -> CostEstimate {
    // Batch API rates (50% of standard) per 1M tokens
    let (input_rate, output_rate) = match extraction_model {
        m if m.contains("gpt-4o-mini") => (0.075 / 1_000_000.0, 0.30 / 1_000_000.0),
        m if m.contains("gpt-4o") => (1.25 / 1_000_000.0, 5.00 / 1_000_000.0),
        m if m.contains("gpt-4.1-mini") => (0.20 / 1_000_000.0, 0.80 / 1_000_000.0),
        m if m.contains("gpt-4.1") => (1.00 / 1_000_000.0, 4.00 / 1_000_000.0),
        _ => (0.20 / 1_000_000.0, 0.80 / 1_000_000.0), // Default: gpt-4.1-mini batch rates
    };

    // Embedding rates (standard, not batch)
    let embed_rate = match embedding_model {
        m if m.contains("text-embedding-3-large") => 0.13 / 1_000_000.0,
        m if m.contains("text-embedding-3-small") => 0.02 / 1_000_000.0,
        _ => 0.02 / 1_000_000.0,
    };

    // System prompt is ~1500 tokens per request
    let system_prompt_tokens = 1500.0;
    let total_input_tokens =
        estimated_input_tokens + (system_prompt_tokens * estimated_chunks as f64);

    // Estimate ~500 output tokens per chunk on average
    let total_output_tokens = 500.0 * estimated_chunks as f64;

    let extraction_input_cost = total_input_tokens * input_rate;
    let extraction_output_cost = total_output_tokens * output_rate;

    // Embedding items: chunks + ~3 entities + ~2 relationships per chunk = 6x
    let embedding_items = estimated_chunks * 6;
    let avg_embed_tokens = 50.0;
    let embedding_cost = embedding_items as f64 * avg_embed_tokens * embed_rate;

    CostEstimate {
        extraction_input_cost,
        extraction_output_cost,
        embedding_cost,
        embedding_items,
        total_input_tokens,
        total_output_tokens,
        total: extraction_input_cost + extraction_output_cost + embedding_cost,
    }
}

/// Format a token count as a human-readable string (e.g., "201K", "1.2M").
fn format_tokens(tokens: f64) -> String {
    if tokens >= 1_000_000.0 {
        format!("{:.1}M", tokens / 1_000_000.0)
    } else if tokens >= 10_000.0 {
        format!("{:.0}K", tokens / 1_000.0)
    } else {
        format!("{:.0}", tokens)
    }
}

/// Print the dry-run report to stdout.
#[allow(clippy::too_many_arguments)]
fn print_dry_run_report(
    total_docs: usize,
    file_breakdown: (usize, usize),
    sample_size: usize,
    avg_chunks_per_doc: f64,
    estimated_chunks: usize,
    resolved: &ResolvedConfig,
    cost: &CostEstimate,
    namespace: &str,
    use_anthropic: bool,
) {
    let (md_count, txt_count) = file_breakdown;

    println!("\n=== Dry Run Preview ===\n");

    // Documents line with file type breakdown
    if md_count > 0 || txt_count > 0 {
        let mut parts = Vec::new();
        if md_count > 0 {
            parts.push(format!("{} .md", md_count));
        }
        if txt_count > 0 {
            parts.push(format!("{} .txt", txt_count));
        }
        println!(
            "Documents:        {} files ({})",
            total_docs,
            parts.join(", ")
        );
    } else {
        println!("Documents:        {} files", total_docs);
    }

    // Chunk estimate
    println!(
        "Estimated Chunks: ~{} (sampled {} docs, avg {:.1} chunks/doc)",
        estimated_chunks, sample_size, avg_chunks_per_doc
    );

    // Schema summary from resolved config
    let entity_types: Vec<&str> = resolved
        .domain_config
        .entity_types
        .keys()
        .map(|s| s.as_str())
        .collect();
    let relation_types: Vec<String> = resolved
        .domain_config
        .relationship_keywords
        .values()
        .flat_map(|kws| kws.iter().map(|kw| kw.keyword.clone()))
        .collect();

    println!(
        "Schema:           {} entity types, {} relation types",
        entity_types.len(),
        relation_types.len()
    );
    if !entity_types.is_empty() {
        println!("  Entity Types:   {}", entity_types.join(", "));
    }
    if !relation_types.is_empty() {
        let display_types: Vec<&str> = relation_types.iter().map(|s| s.as_str()).collect();
        println!("  Relation Types: {}", display_types.join(", "));
    }

    // Cost estimate
    let api_name = if use_anthropic {
        "Anthropic Batch API"
    } else {
        "OpenAI Batch API"
    };
    println!("\nEstimated Cost ({}):", api_name);
    println!(
        "  Extraction:     ~${:.2} (input: ~{} tokens, output: ~{} tokens)",
        cost.extraction_input_cost + cost.extraction_output_cost,
        format_tokens(cost.total_input_tokens),
        format_tokens(cost.total_output_tokens),
    );
    println!(
        "  Embeddings:     ~${:.2} ({} items)",
        cost.embedding_cost, cost.embedding_items
    );
    println!("  Total:          ~${:.2}", cost.total);

    // Model info
    println!(
        "\nModel:            {} (extraction), {} (embedding)",
        resolved.extraction_model, resolved.embedding_model
    );
    println!("Namespace:        {}", namespace);

    // Disclaimer
    println!(
        "\nNote: Estimates based on sampling {}/{} documents. Actual cost may vary.",
        sample_size, total_docs
    );
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sample_size_small() {
        assert_eq!(compute_sample_size(1), 1);
        assert_eq!(compute_sample_size(2), 2);
        assert_eq!(compute_sample_size(3), 3);
    }

    #[test]
    fn test_compute_sample_size_medium() {
        // 10% of 42 = 4.2, ceil = 5, max(5, 3) = 5, min(5, 50) = 5
        assert_eq!(compute_sample_size(42), 5);
        // 10% of 100 = 10, max(10, 3) = 10, min(10, 50) = 10
        assert_eq!(compute_sample_size(100), 10);
    }

    #[test]
    fn test_compute_sample_size_large() {
        // 10% of 1000 = 100, max(100, 3) = 100, min(100, 50) = 50
        assert_eq!(compute_sample_size(1000), 50);
    }

    #[test]
    fn test_uniform_sampling() {
        let docs: Vec<ReconstructedDocument> = (0..10)
            .map(|i| ReconstructedDocument {
                filename: format!("doc{}.txt", i),
                content: format!("Content {}", i),
                content_hash: format!("hash{}", i),
                source_row_count: 1,
            })
            .collect();

        let sample = select_uniform_sample(&docs, 3);
        assert_eq!(sample.len(), 3);
        // Should be evenly spaced: indices ~0, ~3, ~6
        assert_eq!(sample[0].filename, "doc0.txt");
        assert_eq!(sample[1].filename, "doc3.txt");
        assert_eq!(sample[2].filename, "doc6.txt");
    }

    #[test]
    fn test_uniform_sampling_all() {
        let docs: Vec<ReconstructedDocument> = (0..3)
            .map(|i| ReconstructedDocument {
                filename: format!("doc{}.txt", i),
                content: format!("Content {}", i),
                content_hash: format!("hash{}", i),
                source_row_count: 1,
            })
            .collect();

        // When sample_size >= docs.len(), return all
        let sample = select_uniform_sample(&docs, 5);
        assert_eq!(sample.len(), 3);
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(500.0), "500");
        assert_eq!(format_tokens(1500.0), "1500");
        assert_eq!(format_tokens(9999.0), "9999");
        assert_eq!(format_tokens(15000.0), "15K");
        assert_eq!(format_tokens(150000.0), "150K");
        assert_eq!(format_tokens(1500000.0), "1.5M");
    }

    #[test]
    fn test_cost_estimate_gpt4o_mini() {
        let cost = estimate_openai_cost("gpt-4o-mini", "text-embedding-3-small", 100_000.0, 100);
        assert!(cost.total > 0.0);
        assert!(cost.extraction_input_cost > 0.0);
        assert!(cost.extraction_output_cost > 0.0);
        assert!(cost.embedding_cost > 0.0);
        assert_eq!(cost.embedding_items, 600); // 100 * 6
    }

    #[test]
    fn test_cost_estimate_gpt41() {
        let cost_mini =
            estimate_openai_cost("gpt-4.1-mini", "text-embedding-3-small", 100_000.0, 100);
        let cost_full = estimate_openai_cost("gpt-4.1", "text-embedding-3-small", 100_000.0, 100);
        // gpt-4.1 should be more expensive than gpt-4.1-mini
        assert!(cost_full.total > cost_mini.total);
    }
}
