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
pub mod types;

// Re-export key types for convenience
pub use analyzer::OpenAiConfig;
pub use types::{EntityTypeProposal, RelationTypeProposal, SchemaProposal, SchemaStatus};

/// Orchestrate the full schema suggestion workflow.
///
/// 1. Sample documents from the dataset at `location`
/// 2. Analyze samples with OpenAI to propose entity/relation types
/// 3. Normalize names, deduplicate, and ensure baseline types
///
/// # Arguments
///
/// * `location` - Local filesystem path or `s3://bucket/prefix` URI
/// * `sample_percentage` - Percentage of documents to sample (e.g. 10.0 = 10%)
/// * `domain_hint` - Optional domain hint to guide the LLM (e.g. "legal", "healthcare")
/// * `openai_config` - OpenAI API configuration
///
/// # Returns
///
/// A `SchemaProposal` with status `Proposed`, ready for partner review.
pub async fn suggest_schema(
    location: &str,
    sample_percentage: f64,
    domain_hint: Option<&str>,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<SchemaProposal> {
    tracing::info!(
        location = location,
        sample_percentage = sample_percentage,
        domain_hint = domain_hint,
        "Starting schema suggestion"
    );

    // Step 1: Sample documents
    let dataset = sampler::sample_documents(location, sample_percentage).await?;

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
    );

    tracing::info!(
        entity_types = proposal.entity_types.len(),
        relation_types = proposal.relation_types.len(),
        "Schema suggestion complete"
    );

    Ok(proposal)
}
