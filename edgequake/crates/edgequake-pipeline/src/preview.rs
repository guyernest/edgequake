//! Extraction preview module for lightweight pipeline trials.
//!
//! Provides functions to estimate costs and run a real extraction preview
//! through the Chunker + SOTAExtractor pipeline without any graph/vector writes.
//! This is the backend foundation for the extraction preview API and UI.
//!
//! ## Key Functions
//!
//! - [`select_preview_documents`] — Select diverse documents via a caller-provided
//!   document loader (e.g. stratified sampling from `edgequake_schema`)
//! - [`estimate_preview_cost`] — Compute cost estimate from chunks + model pricing
//! - [`preview_extraction`] — Run real extraction and return per-type counts and coverage
//!
//! ## Dependency Note
//!
//! `edgequake-pipeline` cannot depend on `edgequake-core` or `edgequake-schema`
//! due to `edgequake-core`'s optional dependency on `edgequake-pipeline` (Cargo
//! cycle). As a result, this module accepts entity types and language as primitive
//! parameters rather than importing `DomainConfig`. Callers in higher-level crates
//! (e.g. `edgequake-batch`, `edgequake-api`) bridge the gap by converting
//! `DomainConfig` fields into these primitives.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::chunker::{Chunker, ChunkerConfig, TextChunk};
use crate::error::{PipelineError, Result};
use crate::extractor::{EntityExtractor, SOTAExtractor};
use crate::progress::{default_model_pricing, ModelPricing};

/// Hard cap on total chunks across all preview documents.
///
/// Keeps preview fast and cheap while providing enough signal for schema validation.
const MAX_PREVIEW_CHUNKS: usize = 20;

/// Default number of documents to sample for preview.
const DEFAULT_PREVIEW_BUDGET: usize = 3;

/// Estimated system prompt overhead per chunk (tokens).
const ESTIMATED_SYSTEM_PROMPT_TOKENS: usize = 1500;

/// Estimated output tokens per chunk (typical extraction response).
const ESTIMATED_OUTPUT_TOKENS_PER_CHUNK: usize = 500;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of a preview extraction run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewResult {
    /// Per-document extraction results.
    pub documents: Vec<PreviewDocumentResult>,
    /// Aggregate entity type counts across all documents.
    pub entity_type_counts: HashMap<String, usize>,
    /// Aggregate relationship type counts across all documents.
    pub relation_type_counts: HashMap<String, usize>,
    /// Coverage matrix: entity_type -> per-document counts.
    pub coverage_matrix: Vec<CoverageRow>,
    /// Total chunks processed.
    pub total_chunks: usize,
    /// Total entities extracted.
    pub total_entities: usize,
    /// Total relationships extracted.
    pub total_relationships: usize,
    /// Actual cost of the preview extraction.
    pub cost: PreviewCost,
    /// Total processing time in milliseconds.
    pub processing_time_ms: u64,
}

/// Extraction result for a single preview document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewDocumentResult {
    /// Document identifier.
    pub document_id: String,
    /// Document name/filename.
    pub document_name: String,
    /// Number of chunks processed for this document.
    pub chunk_count: usize,
    /// Number of entities extracted from this document.
    pub entity_count: usize,
    /// Number of relationships extracted from this document.
    pub relationship_count: usize,
}

/// A single row in the coverage matrix (entity_type -> per-document counts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageRow {
    /// Entity type name.
    pub entity_type: String,
    /// Count of this entity type in each document (same order as PreviewResult.documents).
    pub counts: Vec<usize>,
}

/// Actual cost incurred during preview extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewCost {
    /// Total input tokens consumed.
    pub input_tokens: usize,
    /// Total output tokens consumed.
    pub output_tokens: usize,
    /// Total cost in USD.
    pub total_cost_usd: f64,
    /// Model used for extraction.
    pub model: String,
}

/// Cost estimate before running extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewCostEstimate {
    /// Number of documents selected for preview.
    pub document_count: usize,
    /// Total chunks across all documents (after cap).
    pub chunk_count: usize,
    /// Estimated input tokens.
    pub estimated_input_tokens: usize,
    /// Estimated output tokens.
    pub estimated_output_tokens: usize,
    /// Estimated total cost in USD.
    pub estimated_cost_usd: f64,
    /// Model for estimation.
    pub model: String,
    /// Per-document chunk breakdown.
    pub documents: Vec<PreviewDocumentInfo>,
}

/// Per-document info in a cost estimate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewDocumentInfo {
    /// Document identifier.
    pub id: String,
    /// Document name/filename.
    pub name: String,
    /// Number of chunks for this document (after cap).
    pub chunk_count: usize,
}

// ---------------------------------------------------------------------------
// Document selection
// ---------------------------------------------------------------------------

/// Type alias for the async document loader function.
///
/// The loader receives the data location and sample budget, and returns
/// a list of `(id, name, content)` tuples.
pub type DocumentLoader = Box<
    dyn Fn(
            String,
            usize,
        ) -> Pin<
            Box<dyn Future<Output = std::result::Result<Vec<(String, String, String)>, String>> + Send>,
        > + Send
        + Sync,
>;

/// Select documents for extraction preview using a caller-provided document loader.
///
/// This function delegates to the provided `document_loader` for the actual sampling
/// strategy. Callers typically pass a closure that wraps
/// `edgequake_schema::sampler::sample_documents_stratified()`.
///
/// # Arguments
///
/// * `data_location` - Local filesystem path or `s3://bucket/prefix` URI
/// * `sample_budget` - Maximum number of documents to select (default: 3)
/// * `document_loader` - Async function that performs the actual document sampling
///
/// # Example (from a crate that can import edgequake-schema)
///
/// ```rust,ignore
/// use edgequake_pipeline::preview::select_preview_documents;
///
/// let docs = select_preview_documents(
///     "data/my-corpus",
///     3,
///     Box::new(|location, budget| {
///         Box::pin(async move {
///             let input = SuggestSchemaInput { sample_budget: Some(budget), ..Default::default() };
///             let (dataset, _) = sample_documents_stratified(&location, &input).await
///                 .map_err(|e| e.to_string())?;
///             Ok(dataset.documents.into_iter()
///                 .take(budget)
///                 .map(|d| (d.id, d.source, d.content))
///                 .collect())
///         })
///     }),
/// ).await?;
/// ```
pub async fn select_preview_documents(
    data_location: &str,
    sample_budget: usize,
    document_loader: DocumentLoader,
) -> Result<Vec<(String, String, String)>> {
    let budget = if sample_budget == 0 {
        DEFAULT_PREVIEW_BUDGET
    } else {
        sample_budget
    };

    let docs = document_loader(data_location.to_string(), budget)
        .await
        .map_err(|e| PipelineError::DocumentError(format!("Failed to sample documents: {}", e)))?;

    info!(
        selected = docs.len(),
        budget = budget,
        "Selected preview documents"
    );

    Ok(docs)
}

// ---------------------------------------------------------------------------
// Cost estimation
// ---------------------------------------------------------------------------

/// Estimate the cost of running a preview extraction without making LLM calls.
///
/// Chunks each document, enforces the 20-chunk hard cap, and computes an
/// estimated cost based on model pricing and typical token usage.
///
/// Uses standard pricing rates (NOT batch rates with 50% discount).
///
/// # Arguments
///
/// * `documents` - Pre-selected documents as `(id, name, content)` tuples
/// * `model` - Model name for pricing lookup (e.g. "gpt-4.1-nano")
/// * `chunk_config` - Chunker configuration
pub fn estimate_preview_cost(
    documents: &[(String, String, String)],
    model: &str,
    chunk_config: &ChunkerConfig,
) -> PreviewCostEstimate {
    let chunker = Chunker::new(chunk_config.clone());

    // Chunk each document
    let mut doc_chunks: Vec<(String, String, usize)> = Vec::new(); // (id, name, chunk_count)
    for (id, name, content) in documents {
        let chunk_count = chunker
            .chunk(content, id)
            .map(|chunks| chunks.len())
            .unwrap_or(1);
        doc_chunks.push((id.clone(), name.clone(), chunk_count));
    }

    // Apply 20-chunk hard cap with proportional distribution
    let total_raw: usize = doc_chunks.iter().map(|(_, _, c)| *c).sum();
    let capped_chunks = apply_chunk_cap(&doc_chunks, total_raw);

    let total_chunks: usize = capped_chunks.iter().map(|(_, _, c)| *c).sum();

    // Estimate tokens
    let estimated_input_tokens = total_chunks * ESTIMATED_SYSTEM_PROMPT_TOKENS;
    let estimated_output_tokens = total_chunks * ESTIMATED_OUTPUT_TOKENS_PER_CHUNK;

    // Look up pricing (standard rates, NOT batch)
    let pricing = lookup_pricing(model);
    let estimated_cost_usd =
        pricing.calculate_cost(estimated_input_tokens, estimated_output_tokens);

    let doc_infos: Vec<PreviewDocumentInfo> = capped_chunks
        .iter()
        .map(|(id, name, count)| PreviewDocumentInfo {
            id: id.clone(),
            name: name.clone(),
            chunk_count: *count,
        })
        .collect();

    info!(
        documents = documents.len(),
        total_chunks = total_chunks,
        estimated_input_tokens = estimated_input_tokens,
        estimated_output_tokens = estimated_output_tokens,
        estimated_cost_usd = estimated_cost_usd,
        model = model,
        "Preview cost estimated"
    );

    PreviewCostEstimate {
        document_count: documents.len(),
        chunk_count: total_chunks,
        estimated_input_tokens,
        estimated_output_tokens,
        estimated_cost_usd,
        model: model.to_string(),
        documents: doc_infos,
    }
}

// ---------------------------------------------------------------------------
// Preview extraction
// ---------------------------------------------------------------------------

/// Run a preview extraction through the real Chunker + SOTAExtractor pipeline.
///
/// Documents are processed SEQUENTIALLY (not in parallel) so that partial
/// results are available if the user cancels. No graph or vector writes occur.
///
/// # Arguments
///
/// * `documents` - Pre-selected documents as `(id, name, content)` tuples
/// * `entity_types` - Entity type names from the domain schema
/// * `language` - Output language (e.g. "English")
/// * `llm_provider` - LLM provider for extraction (standard API, not batch)
/// * `chunk_config` - Chunker configuration
/// * `on_document_complete` - Optional callback invoked after each document `(completed, total)`
pub async fn preview_extraction<L>(
    documents: Vec<(String, String, String)>,
    entity_types: &[String],
    language: &str,
    llm_provider: std::sync::Arc<L>,
    chunk_config: &ChunkerConfig,
    on_document_complete: Option<Box<dyn Fn(usize, usize) + Send>>,
) -> Result<PreviewResult>
where
    L: edgequake_llm::LLMProvider + Send + Sync + ?Sized,
{
    let start = std::time::Instant::now();
    let doc_count = documents.len();

    // Set up extractor with domain-specific entity types
    let extractor = SOTAExtractor::new(llm_provider)
        .with_entity_types(entity_types.to_vec())
        .with_language(language);

    let chunker = Chunker::new(chunk_config.clone());

    // Chunk all documents
    let mut doc_chunk_sets: Vec<(String, String, Vec<TextChunk>)> = Vec::new();
    for (id, name, content) in &documents {
        let chunks = chunker.chunk(content, id)?;
        doc_chunk_sets.push((id.clone(), name.clone(), chunks));
    }

    // Apply 20-chunk hard cap
    let raw_counts: Vec<(String, String, usize)> = doc_chunk_sets
        .iter()
        .map(|(id, name, chunks)| (id.clone(), name.clone(), chunks.len()))
        .collect();
    let total_raw: usize = raw_counts.iter().map(|(_, _, c)| *c).sum();
    let capped = apply_chunk_cap(&raw_counts, total_raw);

    // Trim chunks per document to respect cap
    for (i, (_, _, max_chunks)) in capped.iter().enumerate() {
        doc_chunk_sets[i].2.truncate(*max_chunks);
    }

    // Initialize aggregation structures
    let mut doc_results: Vec<PreviewDocumentResult> = Vec::new();
    let mut entity_type_counts: HashMap<String, usize> = HashMap::new();
    let mut relation_type_counts: HashMap<String, usize> = HashMap::new();
    // coverage_map[entity_type] = vec of per-doc counts
    let mut coverage_map: HashMap<String, Vec<usize>> = HashMap::new();

    // Pre-populate all schema entity types at 0
    for et in entity_types {
        entity_type_counts.insert(et.clone(), 0);
        coverage_map.insert(et.clone(), Vec::new());
    }

    let mut total_chunks = 0usize;
    let mut total_entities = 0usize;
    let mut total_relationships = 0usize;
    let mut total_input_tokens = 0usize;
    let mut total_output_tokens = 0usize;

    // Process documents SEQUENTIALLY for cancellation safety
    for (doc_idx, (doc_id, doc_name, chunks)) in doc_chunk_sets.iter().enumerate() {
        let mut doc_entity_count = 0usize;
        let mut doc_relationship_count = 0usize;
        let mut doc_entity_type_counts: HashMap<String, usize> = HashMap::new();

        for chunk in chunks {
            match extractor.extract(chunk).await {
                Ok(result) => {
                    // Count entities by type
                    for entity in &result.entities {
                        doc_entity_count += 1;
                        *entity_type_counts
                            .entry(entity.entity_type.clone())
                            .or_insert(0) += 1;
                        *doc_entity_type_counts
                            .entry(entity.entity_type.clone())
                            .or_insert(0) += 1;
                    }

                    // Count relationships by type (first keyword = type)
                    for rel in &result.relationships {
                        doc_relationship_count += 1;
                        let rel_type = if !rel.keywords.is_empty() {
                            rel.keywords[0].clone()
                        } else {
                            rel.relation_type.clone()
                        };
                        *relation_type_counts.entry(rel_type).or_insert(0) += 1;
                    }

                    total_input_tokens += result.input_tokens;
                    total_output_tokens += result.output_tokens;
                }
                Err(e) => {
                    warn!(
                        doc_id = %doc_id,
                        chunk_id = %chunk.id,
                        error = %e,
                        "Preview extraction failed for chunk, continuing"
                    );
                }
            }
        }

        total_chunks += chunks.len();
        total_entities += doc_entity_count;
        total_relationships += doc_relationship_count;

        // Update coverage matrix: append this document's per-type counts
        for et in entity_types {
            coverage_map
                .entry(et.clone())
                .or_default()
                .push(*doc_entity_type_counts.get(et).unwrap_or(&0));
        }
        // Also handle entity types not in the schema (discovered during extraction)
        for (et, count) in &doc_entity_type_counts {
            if !entity_types.contains(et) {
                // Backfill zeros for previous documents
                let entry = coverage_map.entry(et.clone()).or_default();
                while entry.len() < doc_idx {
                    entry.push(0);
                }
                entry.push(*count);
            }
        }

        doc_results.push(PreviewDocumentResult {
            document_id: doc_id.clone(),
            document_name: doc_name.clone(),
            chunk_count: chunks.len(),
            entity_count: doc_entity_count,
            relationship_count: doc_relationship_count,
        });

        // Invoke progress callback
        if let Some(ref callback) = on_document_complete {
            callback(doc_idx + 1, doc_count);
        }
    }

    // Build coverage matrix rows, padding any short rows
    let coverage_matrix: Vec<CoverageRow> = coverage_map
        .into_iter()
        .map(|(entity_type, mut counts)| {
            // Pad to doc_count if any types were discovered mid-way
            while counts.len() < doc_count {
                counts.push(0);
            }
            CoverageRow {
                entity_type,
                counts,
            }
        })
        .collect();

    // Compute actual cost
    let model_name = extractor.model_name().to_string();
    let pricing = lookup_pricing(&model_name);
    let total_cost_usd = pricing.calculate_cost(total_input_tokens, total_output_tokens);

    let elapsed = start.elapsed();

    info!(
        documents = doc_count,
        total_chunks = total_chunks,
        total_entities = total_entities,
        total_relationships = total_relationships,
        cost_usd = total_cost_usd,
        time_ms = elapsed.as_millis(),
        "Preview extraction complete"
    );

    Ok(PreviewResult {
        documents: doc_results,
        entity_type_counts,
        relation_type_counts,
        coverage_matrix,
        total_chunks,
        total_entities,
        total_relationships,
        cost: PreviewCost {
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
            total_cost_usd,
            model: model_name,
        },
        processing_time_ms: elapsed.as_millis() as u64,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Apply the 20-chunk hard cap with proportional distribution.
///
/// If total chunks exceed [`MAX_PREVIEW_CHUNKS`], distributes the budget
/// proportionally across documents, guaranteeing at least 1 chunk per doc
/// (up to the cap). Round-robin fills any remaining budget.
fn apply_chunk_cap(
    doc_chunks: &[(String, String, usize)],
    total_raw: usize,
) -> Vec<(String, String, usize)> {
    if total_raw <= MAX_PREVIEW_CHUNKS {
        return doc_chunks.to_vec();
    }

    let n = doc_chunks.len();
    // Phase 1: Give each document at least 1 chunk (up to their original count)
    let guaranteed = n.min(MAX_PREVIEW_CHUNKS);
    let mut capped: Vec<(String, String, usize)> = doc_chunks
        .iter()
        .map(|(id, name, count)| (id.clone(), name.clone(), (*count).min(1)))
        .collect();

    // Phase 2: Distribute remaining budget proportionally
    let remaining_budget = MAX_PREVIEW_CHUNKS.saturating_sub(guaranteed);
    if remaining_budget > 0 {
        let remaining_raw: usize = doc_chunks.iter().map(|(_, _, c)| c.saturating_sub(1)).sum();
        if remaining_raw > 0 {
            let ratio = remaining_budget as f64 / remaining_raw as f64;
            for (i, (_, _, original)) in doc_chunks.iter().enumerate() {
                let extra_available = original.saturating_sub(1);
                let extra = ((extra_available as f64 * ratio).floor() as usize).min(extra_available);
                capped[i].2 += extra;
            }
        }
    }

    // Phase 3: Round-robin any remaining budget
    let allocated: usize = capped.iter().map(|(_, _, c)| *c).sum();
    let mut leftover = MAX_PREVIEW_CHUNKS.saturating_sub(allocated);
    let mut idx = 0;
    let mut loops = 0;
    while leftover > 0 && loops < n * 2 {
        let original = doc_chunks[idx].2;
        if capped[idx].2 < original {
            capped[idx].2 += 1;
            leftover -= 1;
        }
        idx = (idx + 1) % n;
        loops += 1;
    }

    capped
}

/// Look up model pricing from the default pricing table.
///
/// Uses standard rates (NOT batch rates with 50% discount).
/// Falls back to gpt-4.1-nano pricing if the model is not found.
fn lookup_pricing(model: &str) -> ModelPricing {
    let pricing_table = default_model_pricing();
    pricing_table
        .get(model)
        .cloned()
        .unwrap_or_else(|| {
            // Default to gpt-4.1-nano standard pricing
            ModelPricing::new("gpt-4.1-nano", 0.00015, 0.0006)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_chunk_cap_under_limit() {
        let docs = vec![
            ("d1".to_string(), "doc1".to_string(), 5),
            ("d2".to_string(), "doc2".to_string(), 10),
        ];
        let result = apply_chunk_cap(&docs, 15);
        assert_eq!(result[0].2, 5);
        assert_eq!(result[1].2, 10);
    }

    #[test]
    fn test_apply_chunk_cap_over_limit() {
        let docs = vec![
            ("d1".to_string(), "doc1".to_string(), 30),
            ("d2".to_string(), "doc2".to_string(), 30),
            ("d3".to_string(), "doc3".to_string(), 40),
        ];
        let result = apply_chunk_cap(&docs, 100);
        let total: usize = result.iter().map(|(_, _, c)| *c).sum();
        assert_eq!(total, MAX_PREVIEW_CHUNKS);
    }

    #[test]
    fn test_apply_chunk_cap_single_doc() {
        let docs = vec![("d1".to_string(), "doc1".to_string(), 50)];
        let result = apply_chunk_cap(&docs, 50);
        assert_eq!(result[0].2, MAX_PREVIEW_CHUNKS);
    }

    #[test]
    fn test_apply_chunk_cap_preserves_small_docs() {
        // Even with cap, each doc gets at least 1 chunk
        let docs = vec![
            ("d1".to_string(), "doc1".to_string(), 1),
            ("d2".to_string(), "doc2".to_string(), 1),
            ("d3".to_string(), "doc3".to_string(), 100),
        ];
        let result = apply_chunk_cap(&docs, 102);
        assert!(result[0].2 >= 1);
        assert!(result[1].2 >= 1);
        let total: usize = result.iter().map(|(_, _, c)| *c).sum();
        assert_eq!(total, MAX_PREVIEW_CHUNKS);
    }

    #[test]
    fn test_estimate_preview_cost_basic() {
        let docs = vec![
            (
                "d1".to_string(),
                "doc1.txt".to_string(),
                "Hello world. This is a test document with some content for chunking.".to_string(),
            ),
        ];
        let config = ChunkerConfig::default();
        let estimate = estimate_preview_cost(&docs, "gpt-4.1-nano", &config);

        assert_eq!(estimate.document_count, 1);
        assert!(estimate.chunk_count >= 1);
        assert!(estimate.estimated_input_tokens > 0);
        assert!(estimate.estimated_output_tokens > 0);
        assert!(estimate.estimated_cost_usd > 0.0);
        assert_eq!(estimate.model, "gpt-4.1-nano");
    }

    #[test]
    fn test_estimate_preview_cost_respects_cap() {
        // Create a very long document that will generate many chunks
        let long_content = "This is a sentence. ".repeat(10000);
        let docs = vec![
            ("d1".to_string(), "long_doc.txt".to_string(), long_content),
        ];
        let config = ChunkerConfig::default();
        let estimate = estimate_preview_cost(&docs, "gpt-4.1-nano", &config);

        assert!(estimate.chunk_count <= MAX_PREVIEW_CHUNKS);
    }

    #[test]
    fn test_lookup_pricing_known_model() {
        let pricing = lookup_pricing("gpt-4o");
        assert_eq!(pricing.model, "gpt-4o");
        assert!(pricing.input_cost_per_1k > 0.0);
    }

    #[test]
    fn test_lookup_pricing_unknown_model_defaults() {
        let pricing = lookup_pricing("unknown-model-xyz");
        assert_eq!(pricing.model, "gpt-4.1-nano");
    }

    #[test]
    fn test_preview_cost_estimate_multiple_docs() {
        let docs = vec![
            ("d1".to_string(), "doc1.txt".to_string(), "Short doc.".to_string()),
            ("d2".to_string(), "doc2.txt".to_string(), "Another short doc.".to_string()),
            ("d3".to_string(), "doc3.txt".to_string(), "Third document content.".to_string()),
        ];
        let config = ChunkerConfig::default();
        let estimate = estimate_preview_cost(&docs, "gpt-4.1-nano", &config);

        assert_eq!(estimate.document_count, 3);
        assert_eq!(estimate.documents.len(), 3);
    }

    #[test]
    fn test_max_preview_chunks_constant() {
        assert_eq!(MAX_PREVIEW_CHUNKS, 20);
    }

    #[test]
    fn test_default_preview_budget() {
        assert_eq!(DEFAULT_PREVIEW_BUDGET, 3);
    }
}
