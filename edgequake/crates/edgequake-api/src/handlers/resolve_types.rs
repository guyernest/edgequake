//! Request/Response DTOs for batch entity resolution.
//!
//! Used by the `/entities/resolve` endpoint to resolve user-provided keywords
//! to knowledge graph entities in a single API call.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request body for batch entity resolution.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ResolveEntitiesRequest {
    /// Terms to resolve against the knowledge graph.
    pub terms: Vec<String>,

    /// Max matches per term (default 3).
    #[serde(default = "default_max_matches")]
    pub max_matches_per_term: u32,

    /// Max description length in chars (default 300).
    #[serde(default = "default_max_description_length")]
    pub max_description_length: usize,
}

fn default_max_matches() -> u32 {
    3
}

fn default_max_description_length() -> usize {
    300
}

/// Response body for batch entity resolution.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResolveEntitiesResponse {
    /// Resolution results, one per input term.
    pub resolutions: Vec<TermResolution>,

    /// Total unique entities across all terms (after dedup).
    pub total_unique_entities: usize,
}

/// Resolution result for a single input term.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TermResolution {
    /// The input term.
    pub term: String,

    /// Matched entities, ordered by score descending.
    pub matches: Vec<ResolvedEntity>,
}

/// A resolved entity match from vector search.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResolvedEntity {
    /// Entity name.
    pub entity: String,

    /// Entity type (e.g., "PERSON", "ORGANIZATION").
    pub entity_type: String,

    /// Node degree (number of connections).
    pub degree: usize,

    /// Truncated description.
    pub description: String,

    /// Relevance score from vector search.
    pub score: f32,
}
