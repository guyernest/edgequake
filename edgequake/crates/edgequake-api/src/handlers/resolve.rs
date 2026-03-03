//! Batch entity resolution handlers.
//!
//! Provides `POST /entities/resolve` at both global and namespace-scoped routes.
//! Given a list of keyword terms, resolves them to knowledge graph entities using
//! batch embedding, parallel vector search, entity detail lookup, cross-term
//! deduplication, and description truncation.
//!
//! This endpoint is designed for the MCP team's `ask` tool, which needs to
//! resolve user-provided keywords to graph entities in a single API call.

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    Json,
};
use futures::future::join_all;
use tracing::{debug, info};

use edgequake_core::NamespaceSlug;
use edgequake_llm::EmbeddingProvider;
use edgequake_storage::traits::{GraphStorage, VectorStorage};

use crate::error::{ApiError, ApiResult};
use crate::middleware::TenantContext;
use crate::state::AppState;

use super::namespace_graph::resolve_ns_graph;
use super::resolve_types::*;

// ============================================================================
// Public Handlers
// ============================================================================

/// Resolve entities within a namespace by keyword terms.
///
/// Batch-embeds the input terms, performs parallel vector searches, fetches
/// entity details and degrees, deduplicates across terms, and truncates
/// descriptions.
pub async fn ns_resolve_entities(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    _tenant_ctx: TenantContext,
    Json(request): Json<ResolveEntitiesRequest>,
) -> ApiResult<Json<ResolveEntitiesResponse>> {
    NamespaceSlug::parse(&namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace: {}", e)))?;

    let ns_storage = state.resolve_namespace_storage(&namespace).await?;
    let graph = resolve_ns_graph(&state, &namespace).await?;

    info!(
        namespace = %namespace,
        terms = request.terms.len(),
        "Resolving entities in namespace"
    );

    let response = resolve_entities_impl(
        ns_storage.vector_storage.as_ref(),
        graph.as_ref(),
        state.embedding_provider.as_ref(),
        &request,
    )
    .await?;

    Ok(Json(response))
}

/// Resolve entities using global storage by keyword terms.
///
/// Same logic as `ns_resolve_entities` but uses the global storage backends.
pub async fn resolve_entities(
    State(state): State<AppState>,
    _tenant_ctx: TenantContext,
    Json(request): Json<ResolveEntitiesRequest>,
) -> ApiResult<Json<ResolveEntitiesResponse>> {
    info!(
        terms = request.terms.len(),
        "Resolving entities (global)"
    );

    let response = resolve_entities_impl(
        state.vector_storage.as_ref(),
        state.graph_storage.as_ref(),
        state.embedding_provider.as_ref(),
        &request,
    )
    .await?;

    Ok(Json(response))
}

// ============================================================================
// Shared Implementation
// ============================================================================

/// Core entity resolution logic shared between global and namespace-scoped handlers.
///
/// Steps:
/// 1. Validate request (non-empty terms, max 20)
/// 2. Batch embed all terms
/// 3. Fan out parallel vector searches per term
/// 4. Filter to entity vectors, strip "entity:" prefix
/// 5. Batch fetch entity details and degrees
/// 6. Deduplicate across terms (keep highest score)
/// 7. Truncate descriptions, sort by score, limit per term
async fn resolve_entities_impl(
    vector_storage: &dyn VectorStorage,
    graph_storage: &dyn GraphStorage,
    embedding_provider: &dyn EmbeddingProvider,
    request: &ResolveEntitiesRequest,
) -> ApiResult<ResolveEntitiesResponse> {
    // 1. Validate
    if request.terms.is_empty() {
        return Err(ApiError::BadRequest(
            "terms must not be empty".to_string(),
        ));
    }
    if request.terms.len() > 20 {
        return Err(ApiError::BadRequest(
            "terms must contain at most 20 items".to_string(),
        ));
    }

    let max_matches = request.max_matches_per_term;
    let max_desc_len = request.max_description_length;

    // 2. Batch embed all terms
    let terms_for_embed: Vec<String> = request.terms.iter().map(|t| t.to_string()).collect();
    let embeddings = embedding_provider
        .embed(&terms_for_embed)
        .await
        .map_err(|e| ApiError::Internal(format!("Embedding failed: {}", e)))?;

    if embeddings.len() != request.terms.len() {
        return Err(ApiError::Internal(format!(
            "Embedding count mismatch: got {} embeddings for {} terms",
            embeddings.len(),
            request.terms.len()
        )));
    }

    // 3. Fan out parallel vector searches
    // Over-fetch by 5x to allow for entity-type filtering
    let top_k = (max_matches as usize) * 5;
    let search_futures: Vec<_> = embeddings
        .iter()
        .map(|embedding| async move {
            vector_storage.query(embedding, top_k, None).await
        })
        .collect();

    let search_results = join_all(search_futures).await;

    // 4. For each term, filter to entity vectors and collect (entity_id, score) pairs
    let mut per_term_hits: Vec<Vec<(String, f32)>> = Vec::with_capacity(request.terms.len());
    let mut all_entity_ids: Vec<String> = Vec::new();

    for (term_idx, result) in search_results.into_iter().enumerate() {
        let vectors = result.map_err(|e| {
            ApiError::Internal(format!(
                "Vector search failed for term '{}': {}",
                request.terms[term_idx], e
            ))
        })?;

        let hits: Vec<(String, f32)> = vectors
            .into_iter()
            .filter(|r| {
                r.metadata
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| t == "entity")
                    .unwrap_or(false)
            })
            .filter_map(|r| {
                r.id.strip_prefix("entity:")
                    .map(|name| (name.to_string(), r.score))
            })
            .collect();

        for (id, _) in &hits {
            if !all_entity_ids.contains(id) {
                all_entity_ids.push(id.clone());
            }
        }

        per_term_hits.push(hits);
    }

    debug!(
        total_unique_entity_ids = all_entity_ids.len(),
        "Collected entity IDs from vector search"
    );

    // 5. Batch fetch entity details and degrees
    let nodes = if !all_entity_ids.is_empty() {
        graph_storage.get_nodes_by_ids(&all_entity_ids).await?
    } else {
        vec![]
    };

    let degree_pairs = if !all_entity_ids.is_empty() {
        graph_storage.node_degrees_batch(&all_entity_ids).await?
    } else {
        vec![]
    };

    // Build lookup maps
    let node_map: HashMap<String, _> = nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
    let degree_map: HashMap<String, usize> = degree_pairs.into_iter().collect();

    // 6. Cross-term deduplication
    // Track: entity_name -> (best_score, term_index)
    let mut seen: HashMap<String, (f32, usize)> = HashMap::new();

    for (term_idx, hits) in per_term_hits.iter().enumerate() {
        for (entity_id, score) in hits {
            match seen.get(entity_id) {
                Some(&(existing_score, _)) if *score <= existing_score => {
                    // Already seen with a higher or equal score -- skip
                }
                _ => {
                    seen.insert(entity_id.clone(), (*score, term_idx));
                }
            }
        }
    }

    // 7. Build per-term resolutions
    let mut resolutions: Vec<TermResolution> = Vec::with_capacity(request.terms.len());
    let mut unique_entity_set: HashMap<String, bool> = HashMap::new();

    for (term_idx, term) in request.terms.iter().enumerate() {
        let hits = &per_term_hits[term_idx];

        let mut matches: Vec<ResolvedEntity> = Vec::new();

        for (entity_id, score) in hits {
            // Only include if this term "owns" this entity (dedup)
            if let Some(&(_, owner_idx)) = seen.get(entity_id) {
                if owner_idx != term_idx {
                    continue;
                }
            }

            if let Some(node) = node_map.get(entity_id) {
                let props = &node.properties;

                let entity_type = props
                    .get("entity_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("UNKNOWN")
                    .to_string();

                let full_desc = props
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let description = truncate_description(full_desc, max_desc_len);

                let degree = degree_map.get(entity_id).copied().unwrap_or(0);

                matches.push(ResolvedEntity {
                    entity: entity_id.clone(),
                    entity_type,
                    degree,
                    description,
                    score: *score,
                });

                unique_entity_set.insert(entity_id.clone(), true);
            }
        }

        // Sort by score descending
        matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Truncate to max_matches_per_term
        matches.truncate(max_matches as usize);

        resolutions.push(TermResolution {
            term: term.clone(),
            matches,
        });
    }

    let total_unique_entities = unique_entity_set.len();

    Ok(ResolveEntitiesResponse {
        resolutions,
        total_unique_entities,
    })
}

/// Truncate a description string to `max_len` characters, appending "..." if truncated.
fn truncate_description(desc: &str, max_len: usize) -> String {
    if desc.len() <= max_len {
        desc.to_string()
    } else {
        // Find a char boundary to avoid panicking on multi-byte chars
        let mut end = max_len.saturating_sub(3);
        while end > 0 && !desc.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &desc[..end])
    }
}
