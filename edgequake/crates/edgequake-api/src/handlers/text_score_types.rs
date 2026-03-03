//! Text scoring DTO types.
//!
//! Request and response types for the `/text/score` endpoint.
//! Used by the MCP team's code mode sandbox for `api.hasSpecificFacts(text)`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request body for text scoring.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TextScoreRequest {
    /// The text to analyze for fact density.
    pub text: String,
}

/// Response from text scoring.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TextScoreResponse {
    /// Total count of facts detected (proper_nouns + dates + numbers).
    pub fact_density: u32,
    /// Proper noun phrases detected.
    pub proper_nouns: Vec<String>,
    /// Date expressions detected.
    pub dates: Vec<String>,
    /// Numeric expressions detected.
    pub numbers: Vec<String>,
}
