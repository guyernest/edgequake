//! # EdgeQuake Schema Suggestion
//!
//! LLM-based schema inference for knowledge graph construction.
//!
//! This crate implements the pre-ingestion schema suggestion workflow:
//! 1. Sample documents from a dataset (local or S3)
//! 2. Send samples to OpenAI for entity/relation type analysis
//! 3. Normalize and deduplicate proposed types
//! 4. Return a `SchemaProposal` ready for partner review
//!
//! ## Usage
//!
//! ```rust,ignore
//! use edgequake_schema::{suggest_schema, analyzer::OpenAiConfig};
//!
//! let config = OpenAiConfig::new("sk-...");
//! let proposal = suggest_schema(
//!     "data/epstein/0000.parquet",
//!     10.0,
//!     Some("legal"),
//!     &config,
//! ).await?;
//!
//! println!("Proposed {} entity types, {} relation types",
//!     proposal.entity_types.len(),
//!     proposal.relation_types.len(),
//! );
//! ```

pub mod analyzer;
pub mod normalizer;
pub mod prompt;
pub mod sampler;
pub mod tfidf;
pub mod types;

// Re-export key types for convenience
pub use analyzer::OpenAiConfig;
pub use types::{
    BucketBreakdown, BucketInfo, EntityTypeProposal, RelationTypeProposal, SamplingMetadata,
    SchemaProposal, SchemaStatus, SuggestSchemaInput,
};

/// Orchestrate the full schema suggestion workflow.
///
/// 1. Sample documents from the dataset at `location` using stratified sampling
/// 2. Analyze samples with OpenAI to propose entity/relation types
/// 3. Normalize names, deduplicate, ensure baseline types, merge user-specified types
///
/// # Arguments
///
/// * `location` - Local filesystem path or `s3://bucket/prefix` URI
/// * `input` - Sampling and schema suggestion configuration
/// * `openai_config` - OpenAI API configuration
///
/// # Returns
///
/// A `SchemaProposal` with status `Proposed`, ready for partner review.
pub async fn suggest_schema(
    location: &str,
    input: &SuggestSchemaInput,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<SchemaProposal> {
    let domain_hint = input
        .domain_description
        .as_deref()
        .or(input.domain_hint.as_deref());

    tracing::info!(
        location = location,
        domain_hint = domain_hint,
        sample_budget = input.sample_budget,
        "Starting schema suggestion"
    );

    // Step 1: Stratified sampling
    let (dataset, sampling_metadata) =
        sampler::sample_documents_stratified(location, input).await?;

    if dataset.documents.is_empty() {
        anyhow::bail!("No documents found at location: {}", location);
    }

    tracing::info!(
        sampled = dataset.documents.len(),
        total = dataset.total_count,
        "Document sampling complete"
    );

    // Step 2: Analyze with LLM
    let raw_proposal =
        analyzer::analyze_schema(&dataset.documents, domain_hint, openai_config).await?;

    // Step 3: Normalize
    let proposal = normalizer::normalize_proposal(
        raw_proposal,
        dataset.documents.len(),
        dataset.total_count,
        domain_hint.map(|s| s.to_string()),
        Some(sampling_metadata),
        Some(input),
    );

    tracing::info!(
        entity_types = proposal.entity_types.len(),
        relation_types = proposal.relation_types.len(),
        "Schema suggestion complete"
    );

    Ok(proposal)
}
