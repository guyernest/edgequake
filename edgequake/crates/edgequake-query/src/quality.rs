//! Answer quality scoring.
//!
//! Computes quality signals from retrieved context and generated answer:
//! - `mean_chunk_score`: average relevance score across retrieved chunks
//! - `source_diversity`: count of distinct source documents
//! - `fact_density`: count of proper nouns, dates, and numbers in the answer

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::context::QueryContext;
use crate::fact_density::FactDensityAnalyzer;

/// Quality signals for a generated answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerQuality {
    /// Mean relevance score across retrieved chunks.
    pub mean_chunk_score: f32,
    /// Number of distinct source documents in the context.
    pub source_diversity: u32,
    /// Fact density of the generated answer text.
    pub fact_density: u32,
}

impl AnswerQuality {
    /// Compute quality signals from the query context and generated answer.
    pub fn compute(context: &QueryContext, answer: &str) -> Self {
        let mean_chunk_score = if context.chunks.is_empty() {
            0.0
        } else {
            context.chunks.iter().map(|c| c.score).sum::<f32>() / context.chunks.len() as f32
        };

        let source_diversity = {
            let unique_docs: HashSet<&str> = context
                .chunks
                .iter()
                .filter_map(|c| c.document_id.as_deref())
                .collect();
            unique_docs.len() as u32
        };

        let fact_density = FactDensityAnalyzer::analyze(answer).fact_density;

        Self {
            mean_chunk_score,
            source_diversity,
            fact_density,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{QueryContext, RetrievedChunk};

    #[test]
    fn test_quality_mean_chunk_score() {
        let mut ctx = QueryContext::new();
        ctx.add_chunk(RetrievedChunk::new("c1", "a", 0.8));
        ctx.add_chunk(RetrievedChunk::new("c2", "b", 0.6));
        ctx.add_chunk(RetrievedChunk::new("c3", "c", 0.4));

        let quality = AnswerQuality::compute(&ctx, "some answer");
        let expected = (0.8 + 0.6 + 0.4) / 3.0;
        assert!(
            (quality.mean_chunk_score - expected).abs() < 0.001,
            "Expected mean_chunk_score ~= {}, got {}",
            expected,
            quality.mean_chunk_score
        );
    }

    #[test]
    fn test_quality_source_diversity() {
        let mut ctx = QueryContext::new();
        ctx.add_chunk(RetrievedChunk::new("c1", "a", 0.9).with_document_id("doc-A"));
        ctx.add_chunk(RetrievedChunk::new("c2", "b", 0.8).with_document_id("doc-B"));
        ctx.add_chunk(RetrievedChunk::new("c3", "c", 0.7).with_document_id("doc-A")); // duplicate

        let quality = AnswerQuality::compute(&ctx, "text");
        assert_eq!(
            quality.source_diversity, 2,
            "Expected 2 distinct docs, got {}",
            quality.source_diversity
        );
    }

    #[test]
    fn test_quality_empty_chunks() {
        let ctx = QueryContext::new();
        let quality = AnswerQuality::compute(&ctx, "");

        assert_eq!(quality.mean_chunk_score, 0.0);
        assert_eq!(quality.source_diversity, 0);
    }

    #[test]
    fn test_quality_fact_density_from_answer() {
        let ctx = QueryContext::new();
        let quality = AnswerQuality::compute(
            &ctx,
            "Jeffrey Epstein visited New York on March 15, 2005",
        );

        assert!(
            quality.fact_density >= 2,
            "Expected fact_density >= 2 for answer with proper nouns and date, got {}",
            quality.fact_density
        );
    }
}
