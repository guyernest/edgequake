//! Namespace-aware query execution handlers.
//!
//! These handlers extract the `{namespace}` path parameter from the URL,
//! validate it as a `NamespaceSlug`, resolve namespace-scoped storage via
//! `resolve_namespace_storage`, and construct a namespace-scoped
//! `SOTAQueryEngine` for query execution.
//!
//! The existing top-level query handlers in `query.rs` continue to use
//! global state storage for backward compatibility.

use axum::{
    extract::{Path, State},
    response::sse::{Event, Sse},
    Json,
};
use futures::StreamExt;
use std::sync::Arc;
use tracing::{debug, warn};

use edgequake_core::NamespaceSlug;
use edgequake_query::{QueryMode, QueryRequest as EngineQueryRequest, RetrievalMode, SOTAQueryConfig, SOTAQueryEngine};

use crate::error::{ApiError, ApiResult};
use crate::middleware::TenantContext;
use crate::state::AppState;
use crate::validation::validate_query;
use super::query_types::{QueryRequest, QueryResponse, QueryStats, SourceReference, StreamQueryRequest};

/// Helper: resolve namespace-scoped storage and build a SOTAQueryEngine.
async fn resolve_ns_engine(
    state: &AppState,
    namespace: &str,
) -> ApiResult<Arc<SOTAQueryEngine>> {
    NamespaceSlug::parse(namespace)
        .map_err(|e| ApiError::BadRequest(format!("Invalid namespace: {}", e)))?;

    let ns_storage = state.resolve_namespace_storage(namespace).await?;

    // Construct a namespace-scoped SOTAQueryEngine sharing the global LLM
    // and embedding providers. The engine is lightweight (config + Arc refs);
    // the expensive AWS clients are already shared through storage instances.
    let mut config = SOTAQueryConfig::default();
    // Enable chunk content hydration so query responses include snippet text
    // and chunk metadata (document_id, chunk_index) from KV storage.
    config.enable_chunk_content = true;

    let mut engine = SOTAQueryEngine::new(
        config,
        ns_storage.vector_storage.clone(),
        ns_storage.graph_storage.clone(),
        state.embedding_provider.clone(),
        state.llm_provider.clone(),
    )
    .with_kv_storage(ns_storage.kv_storage.clone());

    // Inject BM25 storage if a factory is configured (enables hybrid/BM25 retrieval modes).
    // When the factory is None, queries fall back to vector-only (backward compatible).
    if let Some(ref bm25_factory) = state.bm25_storage_factory {
        match bm25_factory.create_bm25_storage(namespace).await {
            Ok(bm25_storage) => {
                engine = engine.with_bm25_storage(bm25_storage);
            }
            Err(e) => {
                warn!(
                    namespace = %namespace,
                    error = %e,
                    "Failed to create BM25 storage, falling back to vector-only retrieval"
                );
            }
        }
    }

    Ok(Arc::new(engine))
}

/// Execute a namespace-scoped RAG query with multi-mode retrieval.
///
/// Extracts `{namespace}` from path, validates as NamespaceSlug, resolves
/// namespace-scoped storage, constructs a namespace-scoped query engine,
/// and executes the query. All 6 query modes (naive, local, global, hybrid,
/// mix, bypass) use the namespace-scoped NeptuneGraphStorage and S3VectorsStorage.
pub async fn ns_execute_query(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    tenant_ctx: TenantContext,
    Json(request): Json<QueryRequest>,
) -> ApiResult<Json<QueryResponse>> {
    debug!(
        namespace = %namespace,
        tenant_id = ?tenant_ctx.tenant_id,
        workspace_id = ?tenant_ctx.workspace_id,
        query = %request.query,
        "Executing namespace-scoped query"
    );

    validate_query(&request.query, state.config.max_query_length)?;

    let engine = resolve_ns_engine(&state, &namespace).await?;

    // Parse query mode
    let mode = request
        .mode
        .as_ref()
        .and_then(|m| QueryMode::parse(m))
        .unwrap_or(QueryMode::Hybrid);

    // Build engine query request
    let mut engine_request = EngineQueryRequest::new(&request.query).with_mode(mode);

    if let Some(ref tenant_id) = tenant_ctx.tenant_id {
        engine_request = engine_request.with_tenant_id(tenant_id.clone());
    }
    if let Some(ref workspace_id) = tenant_ctx.workspace_id {
        engine_request = engine_request.with_workspace_id(workspace_id.clone());
    }

    if request.context_only {
        engine_request = engine_request.context_only();
    }
    if request.prompt_only {
        engine_request = engine_request.prompt_only();
    }

    // Add rerank settings
    engine_request = engine_request.with_rerank(request.enable_rerank);
    if let Some(top_k) = request.rerank_top_k {
        engine_request = engine_request.with_rerank_top_k(top_k);
    }

    // Add retrieval mode (vector/bm25/hybrid) — RET-03
    if let Some(ref rm) = request.retrieval_mode {
        match rm.parse::<RetrievalMode>() {
            Ok(mode) => {
                engine_request = engine_request.with_retrieval_mode(mode);
            }
            Err(e) => {
                return Err(ApiError::BadRequest(format!("Invalid retrieval_mode: {}", e)));
            }
        }
    }

    // Add conversation history if provided
    if let Some(history) = &request.conversation_history {
        let engine_history: Vec<edgequake_query::ConversationMessage> = history
            .iter()
            .map(|m| edgequake_query::ConversationMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();
        engine_request = engine_request.with_conversation_history(engine_history);
    }

    // Execute query with namespace-scoped engine
    let result = engine
        .query(engine_request)
        .await
        .map_err(|e| ApiError::Internal(format!("Query failed: {}", e)))?;

    // Convert sources from context (same logic as the top-level handler)
    let mut sources = Vec::new();
    let reranked = request.enable_rerank;
    let rerank_time_ms = if reranked { Some(5u64) } else { None };
    let rerank_top_k = request.rerank_top_k.unwrap_or(usize::MAX);

    let mut ref_counter = 1usize;
    let mut chunk_sources: Vec<SourceReference> = result
        .context
        .chunks
        .iter()
        .map(|chunk| {
            let rerank_score = if reranked {
                Some((chunk.score.min(1.0) * 0.95 + 0.05).min(1.0))
            } else {
                None
            };

            let ref_id = ref_counter;
            ref_counter += 1;

            SourceReference {
                source_type: "chunk".to_string(),
                id: chunk.id.clone(),
                score: chunk.score,
                rerank_score,
                snippet: Some(chunk.content.chars().take(200).collect()),
                content: if chunk.content.is_empty() { None } else { Some(chunk.content.clone()) },
                reference_id: Some(ref_id),
                document_id: chunk.document_id.clone(),
                file_path: None,
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                chunk_index: chunk.chunk_index,
            }
        })
        .collect();

    if reranked {
        chunk_sources.sort_by(|a, b| {
            b.rerank_score
                .unwrap_or(0.0)
                .partial_cmp(&a.rerank_score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        chunk_sources.truncate(rerank_top_k);
    }

    sources.extend(chunk_sources);

    for entity in &result.context.entities {
        let ref_id = ref_counter;
        ref_counter += 1;
        sources.push(SourceReference {
            source_type: "entity".to_string(),
            id: entity.name.clone(),
            score: entity.score,
            rerank_score: None,
            snippet: Some(entity.description.chars().take(200).collect()),
            content: if entity.description.is_empty() { None } else { Some(entity.description.clone()) },
            reference_id: Some(ref_id),
            document_id: entity.source_document_id.clone(),
            file_path: entity.source_file_path.clone(),
            start_line: None,
            end_line: None,
            chunk_index: None,
        });
    }

    for rel in &result.context.relationships {
        let ref_id = ref_counter;
        ref_counter += 1;
        let rel_text = format!("{} {} {}", rel.source, rel.relation_type, rel.target);
        sources.push(SourceReference {
            source_type: "relationship".to_string(),
            id: format!("{}->{}", rel.source, rel.target),
            score: rel.score,
            rerank_score: None,
            snippet: Some(rel_text.clone()),
            content: Some(rel_text),
            reference_id: Some(ref_id),
            document_id: rel.source_document_id.clone(),
            file_path: rel.source_file_path.clone(),
            start_line: None,
            end_line: None,
            chunk_index: None,
        });
    }

    let conversation_id = if request.conversation_history.is_some() {
        Some(uuid::Uuid::new_v4().to_string())
    } else {
        None
    };

    let tokens_used = if result.stats.generated_tokens > 0 {
        Some(result.stats.generated_tokens)
    } else {
        None
    };

    let tokens_per_second =
        if result.stats.generation_time_ms > 0 && result.stats.generated_tokens > 0 {
            Some(
                (result.stats.generated_tokens as f32) / (result.stats.generation_time_ms as f32)
                    * 1000.0,
            )
        } else {
            None
        };

    let response = QueryResponse {
        answer: result.answer,
        mode: result.mode.to_string(),
        sources,
        stats: QueryStats {
            embedding_time_ms: result.stats.embedding_time_ms,
            retrieval_time_ms: result.stats.retrieval_time_ms,
            generation_time_ms: result.stats.generation_time_ms,
            total_time_ms: result.stats.total_time_ms,
            sources_retrieved: result.context.chunks.len()
                + result.context.entities.len()
                + result.context.relationships.len(),
            rerank_time_ms,
            tokens_used,
            tokens_per_second,
            llm_provider: None,
            llm_model: None,
        },
        conversation_id,
        reranked,
    };

    Ok(Json(response))
}

/// Execute a namespace-scoped streaming query.
pub async fn ns_stream_query(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    tenant_ctx: TenantContext,
    Json(request): Json<StreamQueryRequest>,
) -> ApiResult<Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>>> {
    debug!(
        namespace = %namespace,
        tenant_id = ?tenant_ctx.tenant_id,
        workspace_id = ?tenant_ctx.workspace_id,
        query = %request.query,
        "Executing namespace-scoped streaming query"
    );

    validate_query(&request.query, state.config.max_query_length)?;

    let engine = resolve_ns_engine(&state, &namespace).await?;

    // Parse query mode
    let mode = request
        .mode
        .as_ref()
        .and_then(|m| QueryMode::parse(m))
        .unwrap_or(QueryMode::Hybrid);

    // Build engine query request with tenant context
    let mut engine_request = EngineQueryRequest::new(&request.query).with_mode(mode);

    if let Some(ref tenant_id) = tenant_ctx.tenant_id {
        engine_request = engine_request.with_tenant_id(tenant_id.clone());
    }
    if let Some(ref workspace_id) = tenant_ctx.workspace_id {
        engine_request = engine_request.with_workspace_id(workspace_id.clone());
    }

    // Execute streaming query using namespace-scoped engine
    let stream = engine
        .query_stream(engine_request)
        .await
        .map_err(|e| ApiError::Internal(format!("Streaming query failed: {}", e)))?;

    let sse_stream = stream.map(|res| match res {
        Ok(text) => Ok(Event::default().data(text)),
        Err(e) => Ok(Event::default().data(format!("Error: {}", e))),
    });

    Ok(Sse::new(sse_stream))
}
