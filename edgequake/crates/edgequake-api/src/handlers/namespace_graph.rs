//! Namespace-aware graph handlers.
//!
//! These handlers extract the `{namespace}` path parameter from the URL,
//! validate it as a `NamespaceSlug`, resolve namespace-scoped storage via
//! `resolve_namespace_storage`, and use the namespace-scoped graph storage
//! for all operations.
//!
//! The existing top-level graph handlers in `graph.rs` continue to use
//! global state storage for backward compatibility.
//!
//! NOTE: Entity and relationship namespace-scoped handlers reuse the
//! same route registrations with global handlers for now. Since the
//! Neptune label-prefix isolation is already configured per-namespace
//! at the storage layer, data isolation is enforced when the factory
//! creates namespace-scoped NeptuneGraphStorage instances. The
//! entity/relationship handlers that need Path extraction will be
//! implemented in a follow-up when document namespace scoping is added.

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::StreamExt;
use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use edgequake_core::NamespaceSlug;
use edgequake_storage::traits::GraphStorage;

use crate::error::{ApiError, ApiResult};
use crate::middleware::TenantContext;
use crate::state::AppState;

// Import graph DTOs
use super::graph_types::*;

/// Helper: validate namespace and resolve namespace-scoped graph storage.
pub async fn resolve_ns_graph(
    state: &AppState,
    namespace: &str,
) -> ApiResult<Arc<dyn GraphStorage>> {
    NamespaceSlug::parse(namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace: {}", e)))?;
    let ns_storage = state.resolve_namespace_storage(namespace).await?;
    Ok(ns_storage.graph_storage.clone())
}

// ============================================================================
// Graph Visualization Handlers
// ============================================================================

/// Get namespace-scoped knowledge graph.
pub async fn ns_get_graph(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    _tenant_ctx: TenantContext,
    Query(params): Query<GraphQueryParams>,
) -> ApiResult<Json<KnowledgeGraphResponse>> {
    let graph = resolve_ns_graph(&state, &namespace).await?;
    let params = params.validated();

    debug!(
        namespace = %namespace,
        "Getting namespace-scoped graph"
    );

    let (nodes, edges, is_truncated) = if let Some(start) = &params.start_node {
        let kg = graph
            .get_knowledge_graph(start, params.depth, params.max_nodes)
            .await?;

        let nodes: Vec<GraphNodeResponse> = kg
            .nodes
            .into_iter()
            .map(|n| GraphNodeResponse {
                id: n.id.clone(),
                label: n.id.clone(),
                node_type: n.properties.get("entity_type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
                description: n.properties.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                degree: 0,
                properties: serde_json::to_value(&n.properties).unwrap_or_default(),
            })
            .collect();

        let node_ids: HashSet<_> = nodes.iter().map(|n| &n.id).collect();
        let edges: Vec<GraphEdgeResponse> = kg
            .edges
            .into_iter()
            .filter(|e| node_ids.contains(&e.source) && node_ids.contains(&e.target))
            .map(|e| GraphEdgeResponse {
                source: e.source,
                target: e.target,
                edge_type: e.properties.get("relation_type").and_then(|v| v.as_str()).unwrap_or("RELATED_TO").to_string(),
                weight: e.properties.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                properties: serde_json::to_value(&e.properties).unwrap_or_default(),
            })
            .collect();

        (nodes, edges, kg.is_truncated)
    } else {
        // No start node -- get popular nodes
        let nodes_with_degrees = graph
            .get_popular_nodes_with_degree(params.max_nodes, None, None, None, None)
            .await
            .unwrap_or_default();

        let nodes: Vec<GraphNodeResponse> = nodes_with_degrees
            .into_iter()
            .map(|(node, degree)| GraphNodeResponse {
                id: node.id.clone(),
                label: node.id.clone(),
                node_type: node.properties.get("entity_type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
                description: node.properties.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                degree,
                properties: serde_json::to_value(&node.properties).unwrap_or_default(),
            })
            .collect();

        let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
        let filtered_edges = graph
            .get_edges_for_node_set(&node_ids, None, None)
            .await
            .unwrap_or_default();

        let edges: Vec<GraphEdgeResponse> = filtered_edges
            .into_iter()
            .map(|e| GraphEdgeResponse {
                source: e.source,
                target: e.target,
                edge_type: e.properties.get("relation_type").and_then(|v| v.as_str()).unwrap_or("RELATED_TO").to_string(),
                weight: e.properties.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                properties: serde_json::to_value(&e.properties).unwrap_or_default(),
            })
            .collect();

        (nodes, edges, false)
    };

    let total_nodes = graph.node_count().await.unwrap_or(nodes.len());
    let total_edges = graph.edge_count().await.unwrap_or(edges.len());
    let is_truncated = is_truncated || total_nodes > params.max_nodes;

    Ok(Json(KnowledgeGraphResponse {
        nodes,
        edges,
        is_truncated,
        total_nodes,
        total_edges,
    }))
}

/// Stream namespace-scoped graph data via SSE.
pub async fn ns_stream_graph(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    _tenant_ctx: TenantContext,
    Query(params): Query<GraphStreamQueryParams>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let graph = resolve_ns_graph(&state, &namespace).await?;
    let params = params.validated();

    debug!(
        namespace = %namespace,
        max_nodes = params.max_nodes,
        "Starting namespace-scoped graph stream"
    );

    let (tx, rx) = mpsc::channel::<GraphStreamEvent>(100);

    tokio::spawn(async move {
        let start_time = std::time::Instant::now();
        let total_nodes = graph.node_count().await.unwrap_or(0);
        let total_edges = graph.edge_count().await.unwrap_or(0);

        let nodes_with_degrees = graph
            .get_popular_nodes_with_degree(params.max_nodes, None, None, None, None)
            .await
            .unwrap_or_default();

        let nodes_to_stream = nodes_with_degrees.len();
        let total_batches = nodes_to_stream.div_ceil(params.batch_size);

        if tx
            .send(GraphStreamEvent::Metadata {
                total_nodes,
                total_edges,
                nodes_to_stream,
                edges_to_stream: 0,
            })
            .await
            .is_err()
        {
            return;
        }

        let all_node_ids: Vec<String> = nodes_with_degrees.iter().map(|(n, _)| n.id.clone()).collect();

        for (batch_idx, chunk) in nodes_with_degrees.chunks(params.batch_size).enumerate() {
            let batch_nodes: Vec<GraphNodeResponse> = chunk
                .iter()
                .map(|(node, degree)| GraphNodeResponse {
                    id: node.id.clone(),
                    label: node.id.clone(),
                    node_type: node.properties.get("entity_type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
                    description: node.properties.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    degree: *degree,
                    properties: serde_json::to_value(&node.properties).unwrap_or_default(),
                })
                .collect();

            if tx
                .send(GraphStreamEvent::Nodes {
                    batch: batch_idx + 1,
                    total_batches,
                    nodes: batch_nodes,
                })
                .await
                .is_err()
            {
                return;
            }
            tokio::task::yield_now().await;
        }

        let edges = graph
            .get_edges_for_node_set(&all_node_ids, None, None)
            .await
            .unwrap_or_default();
        let edge_responses: Vec<GraphEdgeResponse> = edges
            .into_iter()
            .map(|e| GraphEdgeResponse {
                source: e.source,
                target: e.target,
                edge_type: e.properties.get("relation_type").and_then(|v| v.as_str()).unwrap_or("RELATED_TO").to_string(),
                weight: e.properties.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                properties: serde_json::to_value(&e.properties).unwrap_or_default(),
            })
            .collect();

        let edges_count = edge_responses.len();
        let _ = tx.send(GraphStreamEvent::Edges { edges: edge_responses }).await;

        let duration_ms = start_time.elapsed().as_millis() as u64;
        let _ = tx
            .send(GraphStreamEvent::Done {
                nodes_count: nodes_to_stream,
                edges_count,
                duration_ms,
            })
            .await;
    });

    let sse_stream = ReceiverStream::new(rx).map(|event| {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok::<_, Infallible>(Event::default().data(json))
    });

    Ok(Sse::new(sse_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// Get a specific node within a namespace.
pub async fn ns_get_node(
    State(state): State<AppState>,
    Path((namespace, node_id)): Path<(String, String)>,
) -> ApiResult<Json<GraphNodeResponse>> {
    let graph = resolve_ns_graph(&state, &namespace).await?;

    let node = graph
        .get_node(&node_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Node '{}' not found", node_id)))?;

    let degree = graph.node_degree(&node_id).await?;

    Ok(Json(GraphNodeResponse {
        id: node.id.clone(),
        label: node.id.clone(),
        node_type: node.properties.get("entity_type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
        description: node.properties.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        degree,
        properties: serde_json::to_value(&node.properties).unwrap_or_default(),
    }))
}

/// Search nodes within a namespace.
pub async fn ns_search_nodes(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    _tenant_ctx: TenantContext,
    Query(params): Query<SearchNodesQuery>,
) -> ApiResult<Json<SearchNodesResponse>> {
    let graph = resolve_ns_graph(&state, &namespace).await?;

    let matching_nodes = graph
        .search_nodes(&params.q, params.limit, params.entity_type.as_deref(), None, None)
        .await?;

    let total_matches = matching_nodes.len();
    let is_truncated = total_matches >= params.limit;

    let mut node_ids: HashSet<String> = matching_nodes.iter().map(|(n, _)| n.id.clone()).collect();
    let mut all_nodes = matching_nodes;

    if params.include_neighbors && !all_nodes.is_empty() {
        let initial_node_ids: Vec<String> = all_nodes.iter().take(10).map(|(n, _)| n.id.clone()).collect();
        for node_id in initial_node_ids {
            if let Ok(neighbors) = graph.get_neighbors(&node_id, params.neighbor_depth).await {
                for neighbor in neighbors {
                    if !node_ids.contains(&neighbor.id) {
                        node_ids.insert(neighbor.id.clone());
                        let degree = graph.node_degree(&neighbor.id).await.unwrap_or(0);
                        all_nodes.push((neighbor, degree));
                    }
                }
            }
        }
    }

    let edges = if all_nodes.len() > 1 {
        let node_id_vec: Vec<String> = node_ids.into_iter().collect();
        graph.get_edges_for_node_set(&node_id_vec, None, None).await.unwrap_or_default()
    } else {
        vec![]
    };

    let nodes_response: Vec<GraphNodeResponse> = all_nodes
        .into_iter()
        .map(|(node, degree)| GraphNodeResponse {
            id: node.id.clone(),
            label: node.id,
            node_type: node.properties.get("entity_type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
            description: node.properties.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            degree,
            properties: serde_json::to_value(&node.properties).unwrap_or_default(),
        })
        .collect();

    let edges_response: Vec<GraphEdgeResponse> = edges
        .into_iter()
        .map(|edge| GraphEdgeResponse {
            source: edge.source,
            target: edge.target,
            edge_type: edge.properties.get("relationship_type").and_then(|v| v.as_str()).unwrap_or("RELATED_TO").to_string(),
            weight: edge.properties.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
            properties: serde_json::to_value(&edge.properties).unwrap_or_default(),
        })
        .collect();

    Ok(Json(SearchNodesResponse {
        nodes: nodes_response,
        edges: edges_response,
        total_matches,
        is_truncated,
    }))
}

/// Search labels within a namespace.
pub async fn ns_search_labels(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Query(params): Query<SearchLabelsQuery>,
) -> ApiResult<Json<SearchLabelsResponse>> {
    let graph = resolve_ns_graph(&state, &namespace).await?;
    let labels = graph.search_labels(&params.q, params.limit).await?;
    Ok(Json(SearchLabelsResponse { labels }))
}

/// Get popular labels within a namespace.
pub async fn ns_get_popular_labels(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Query(params): Query<PopularLabelsQuery>,
) -> ApiResult<Json<PopularLabelsResponse>> {
    let graph = resolve_ns_graph(&state, &namespace).await?;
    let total_entities = graph.node_count().await?;

    let popular_nodes = graph
        .get_popular_nodes_with_degree(params.limit, params.min_degree, params.entity_type.as_deref(), None, None)
        .await?;

    let labels: Vec<PopularLabel> = popular_nodes
        .into_iter()
        .map(|(node, degree)| PopularLabel {
            label: node.id,
            entity_type: node.properties.get("entity_type").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string(),
            degree,
            description: node.properties.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        })
        .collect();

    Ok(Json(PopularLabelsResponse { labels, total_entities }))
}

/// Get degrees for multiple nodes in a namespace.
pub async fn ns_get_degrees_batch(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Json(request): Json<BatchDegreeRequest>,
) -> ApiResult<Json<BatchDegreeResponse>> {
    let graph = resolve_ns_graph(&state, &namespace).await?;

    if request.node_ids.is_empty() {
        return Ok(Json(BatchDegreeResponse { degrees: Vec::new(), count: 0 }));
    }

    let degrees_result = graph.node_degrees_batch(&request.node_ids).await?;
    let degrees: Vec<NodeDegree> = degrees_result
        .into_iter()
        .map(|(node_id, degree)| NodeDegree { node_id, degree })
        .collect();

    let count = degrees.len();
    Ok(Json(BatchDegreeResponse { degrees, count }))
}
