//! Namespace-scoped entity handlers.
//!
//! These handlers extract the `{namespace}` path parameter from the URL,
//! resolve namespace-scoped graph storage via `resolve_ns_graph`, and
//! provide namespace-isolated entity list and detail endpoints.
//!
//! Replaces the global `list_entities` and `get_entity` handlers for
//! namespace-scoped routes (`/ns/{namespace}/graph/entities`).
//!
//! Mutation routes (create, update, delete, merge) remain on global
//! handlers since the entity browser is read-only.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use edgequake_core::namespace::NamespaceSlug;
use tracing::{debug, info, warn};

use crate::error::{ApiError, ApiResult};
use crate::middleware::TenantContext;
use crate::state::AppState;

use super::entities::node_to_entity_response;
use super::entities_types::{EntityResponse, ListEntitiesQuery, ListEntitiesResponse};
use super::namespace_graph::resolve_ns_graph;

// ============================================================================
// Namespace-Scoped Entity Handlers
// ============================================================================

/// List entities within a namespace with pagination, search, and type filter.
///
/// Uses namespace-scoped graph storage via `resolve_ns_graph` for data isolation.
/// Does NOT require TenantContext filtering -- namespace storage isolation
/// (Neptune label-prefix) replaces tenant-based filtering.
pub async fn ns_list_entities(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    _tenant_ctx: TenantContext,
    Query(query): Query<ListEntitiesQuery>,
) -> ApiResult<Json<ListEntitiesResponse>> {
    let graph = resolve_ns_graph(&state, &namespace).await?;

    // Clamp page_size to range [1, 100], default 25 for entity browser
    let page_size = query.page_size.clamp(1, 100);
    let page = query.page.max(1);
    let offset = ((page - 1) * page_size) as usize;

    debug!(
        namespace = %namespace,
        search = ?query.search,
        entity_type = ?query.entity_type,
        page = page,
        page_size = page_size,
        "Listing namespace-scoped entities"
    );

    // Build entity list using the most efficient storage method available
    let mut entities: Vec<(EntityResponse, String)> = if let Some(ref search) = query.search {
        // Semantic search: embed query → S3 Vectors → Neptune entity lookup
        NamespaceSlug::parse(&namespace)
            .map_err(|e| ApiError::BadRequest(format!("Invalid namespace: {}", e)))?;
        let ns_storage = state.resolve_namespace_storage(&namespace).await?;

        // 1. Embed the search query
        let query_embedding = state
            .embedding_provider
            .embed_one(search)
            .await
            .map_err(|e| ApiError::Internal(format!("Embedding failed: {}", e)))?;

        info!(
            namespace = %namespace,
            query = %search,
            embedding_dim = query_embedding.len(),
            "Semantic entity search"
        );

        // 2. Search S3 Vectors (request extra to allow for chunk/rel filtering)
        let top_k = 100;
        let vector_results = ns_storage
            .vector_storage
            .query(&query_embedding, top_k, None)
            .await?;

        // 3. Filter to entity vectors only (type=entity in metadata)
        let entity_ids: Vec<String> = vector_results
            .into_iter()
            .filter(|r| {
                r.metadata
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| t == "entity")
                    .unwrap_or(false)
            })
            .filter_map(|r| {
                // Vector IDs are stored as "entity:{NAME}" — strip prefix
                r.id.strip_prefix("entity:").map(|s| s.to_string())
            })
            .collect();

        debug!(
            namespace = %namespace,
            matched_entities = entity_ids.len(),
            "Vector search returned entity matches"
        );

        if entity_ids.is_empty() {
            // Fall back to Gremlin text search if no vector matches
            warn!(namespace = %namespace, "No vector entity matches, falling back to text search");
            let results = graph
                .search_nodes(search, 100, query.entity_type.as_deref(), None, None)
                .await?;
            results
                .into_iter()
                .map(|(node, degree)| {
                    let sort_key = node.id.clone();
                    (node_to_entity_response(node, degree), sort_key)
                })
                .collect()
        } else {
            // 4. Fetch matched entities from Neptune
            let nodes = graph.get_nodes_by_ids(&entity_ids).await?;
            // Get degrees in one batch query
            let degree_pairs = graph.node_degrees_batch(&entity_ids).await?;
            let degree_map: std::collections::HashMap<String, usize> =
                degree_pairs.into_iter().collect();

            let mut results: Vec<(EntityResponse, String)> = nodes
                .into_iter()
                .filter(|n| {
                    // Apply entity_type filter if specified
                    if let Some(ref et) = query.entity_type {
                        n.properties
                            .get("entity_type")
                            .and_then(|v| v.as_str())
                            .map(|t| t == et.as_str())
                            .unwrap_or(false)
                    } else {
                        true
                    }
                })
                .map(|node| {
                    let degree = degree_map.get(&node.id).copied().unwrap_or(0);
                    let sort_key = node.id.clone();
                    (node_to_entity_response(node, degree), sort_key)
                })
                .collect();

            // Preserve vector search relevance ordering (entity_ids is already ranked)
            let id_rank: std::collections::HashMap<String, usize> = entity_ids
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), i))
                .collect();
            results.sort_by_key(|(resp, _)| {
                id_rank.get(&resp.id).copied().unwrap_or(usize::MAX)
            });
            results
        }
    } else if let Some(ref entity_type) = query.entity_type {
        // Type filter only -- use get_popular_nodes_with_degree for efficient
        // filtering with degrees included
        let limit = 1000;
        let results = graph
            .get_popular_nodes_with_degree(limit, None, Some(entity_type.as_str()), None, None)
            .await?;

        results
            .into_iter()
            .map(|(node, degree)| {
                let sort_key = node.id.clone();
                (node_to_entity_response(node, degree), sort_key)
            })
            .collect()
    } else {
        // No filters -- use get_popular_nodes_with_degree for single server-side
        // query that returns nodes with degrees (avoids N+1 degree queries)
        let limit = 1000;
        let results = graph
            .get_popular_nodes_with_degree(limit, None, None, None, None)
            .await?;

        results
            .into_iter()
            .map(|(node, degree)| {
                let sort_key = node.id.clone();
                (node_to_entity_response(node, degree), sort_key)
            })
            .collect()
    };

    // Sort by entity name (alphabetical) for consistent ordering
    entities.sort_by(|(_, a_key): &(EntityResponse, String), (_, b_key): &(EntityResponse, String)| {
        a_key.cmp(b_key)
    });

    let total = entities.len();
    let total_pages = if total == 0 {
        0
    } else {
        ((total as f64) / (page_size as f64)).ceil() as u32
    };

    // Apply offset-based pagination
    let items: Vec<EntityResponse> = entities
        .into_iter()
        .map(|(entity, _)| entity)
        .skip(offset)
        .take(page_size as usize)
        .collect();

    Ok(Json(ListEntitiesResponse {
        items,
        total,
        page,
        page_size,
        total_pages,
    }))
}

/// Get a single entity within a namespace by name.
///
/// Uses namespace-scoped graph storage via `resolve_ns_graph` for data isolation.
pub async fn ns_get_entity(
    State(state): State<AppState>,
    Path((namespace, entity_name)): Path<(String, String)>,
    _tenant_ctx: TenantContext,
) -> ApiResult<Json<EntityResponse>> {
    let graph = resolve_ns_graph(&state, &namespace).await?;

    debug!(
        namespace = %namespace,
        entity_name = %entity_name,
        "Getting namespace-scoped entity"
    );

    let node = graph
        .get_node(&entity_name)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Entity '{}' not found", entity_name)))?;

    let degree = graph.node_degree(&entity_name).await?;

    Ok(Json(node_to_entity_response(node, degree)))
}
