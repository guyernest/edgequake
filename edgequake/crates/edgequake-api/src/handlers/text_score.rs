//! Text scoring handlers.
//!
//! Provides standalone fact density analysis for text without running a full
//! RAG query. Used by the MCP team's code mode sandbox for
//! `api.hasSpecificFacts(text)`.
//!
//! # Endpoints
//!
//! | Method | Path | Handler | Description |
//! |--------|------|---------|-------------|
//! | POST | `/api/v1/text/score` | [`score_text`] | Analyze text fact density |
//! | POST | `/api/v1/ns/{namespace}/text/score` | [`ns_score_text`] | Namespace-scoped text scoring |

use axum::{
    extract::{Path, State},
    Json,
};

use edgequake_query::FactDensityAnalyzer;

use crate::error::ApiResult;
use crate::middleware::TenantContext;
use crate::state::AppState;

pub use super::text_score_types::{TextScoreRequest, TextScoreResponse};

/// Analyze text for fact density (proper nouns, dates, numbers).
///
/// This is a stateless analysis endpoint -- no RAG retrieval is performed.
/// Returns extracted facts and a total fact_density count.
pub async fn score_text(
    State(_state): State<AppState>,
    _tenant_ctx: TenantContext,
    Json(request): Json<TextScoreRequest>,
) -> ApiResult<Json<TextScoreResponse>> {
    let result = FactDensityAnalyzer::analyze(&request.text);

    Ok(Json(TextScoreResponse {
        fact_density: result.fact_density,
        proper_nouns: result.proper_nouns,
        dates: result.dates,
        numbers: result.numbers,
    }))
}

/// Namespace-scoped text scoring (same logic -- text scoring is stateless).
///
/// The namespace parameter is accepted for API consistency but not used
/// since fact density analysis is purely text-based.
pub async fn ns_score_text(
    State(_state): State<AppState>,
    Path(_namespace): Path<String>,
    _tenant_ctx: TenantContext,
    Json(request): Json<TextScoreRequest>,
) -> ApiResult<Json<TextScoreResponse>> {
    let result = FactDensityAnalyzer::analyze(&request.text);

    Ok(Json(TextScoreResponse {
        fact_density: result.fact_density,
        proper_nouns: result.proper_nouns,
        dates: result.dates,
        numbers: result.numbers,
    }))
}
