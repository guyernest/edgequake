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
use tracing::debug;

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
        // Search by name/description -- uses graph.search_nodes which returns
        // Vec<(GraphNode, usize)> with degrees already included (avoids N+1)
        let limit = 1000; // fetch enough to paginate client-side
        let results = graph
            .search_nodes(search, limit, query.entity_type.as_deref(), None, None)
            .await?;

        results
            .into_iter()
            .map(|(node, degree)| {
                let sort_key = node.id.clone();
                (node_to_entity_response(node, degree), sort_key)
            })
            .collect()
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
        // No filters -- get all nodes + batch degree lookup
        let all_nodes = graph.get_all_nodes().await?;
        let node_ids: Vec<String> = all_nodes.iter().map(|n| n.id.clone()).collect();
        let degree_pairs = graph.node_degrees_batch(&node_ids).await?;
        let degree_map: std::collections::HashMap<String, usize> =
            degree_pairs.into_iter().collect();

        all_nodes
            .into_iter()
            .map(|node| {
                let degree = degree_map.get(&node.id).copied().unwrap_or(0);
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
