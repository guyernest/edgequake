//! Phase 3: Embed - Generate embeddings via standard OpenAI API.
//!
//! Uses the standard (non-batch) embedding API because:
//! - Embeddings are fast enough without batching
//! - Batch API adds up to 24h delay for no benefit
//! - Rate limiting is handled with concurrency control

use crate::config::BatchConfig;
use crate::jsonl::PreparedChunk;
use crate::progress::BatchProgress;
use crate::state::{JobState, LatestRunStatus, Phase, StateManager};
use edgequake_llm::traits::EmbeddingProvider;
use edgequake_pipeline::extractor::ExtractionResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// Result of the embed phase.
pub struct EmbedResult {
    /// Chunk embeddings: custom_id -> embedding vector.
    pub chunk_embeddings: HashMap<String, Vec<f32>>,

    /// Entity embeddings: entity_name -> embedding vector.
    pub entity_embeddings: HashMap<String, Vec<f32>>,

    /// Relationship embeddings: (source, target) -> embedding vector.
    pub relationship_embeddings: HashMap<(String, String), Vec<f32>>,

    /// Total embeddings generated.
    pub total_embeddings: usize,
}

/// Serializable version of EmbedResult for disk caching.
/// JSON doesn't support tuple keys, so relationship embeddings use a flat list.
#[derive(Serialize, Deserialize)]
struct CachedEmbedResult {
    chunk_embeddings: HashMap<String, Vec<f32>>,
    entity_embeddings: HashMap<String, Vec<f32>>,
    relationship_embeddings: Vec<(String, String, Vec<f32>)>,
    total_embeddings: usize,
}

impl From<&EmbedResult> for CachedEmbedResult {
    fn from(r: &EmbedResult) -> Self {
        Self {
            chunk_embeddings: r.chunk_embeddings.clone(),
            entity_embeddings: r.entity_embeddings.clone(),
            relationship_embeddings: r
                .relationship_embeddings
                .iter()
                .map(|((s, t), v)| (s.clone(), t.clone(), v.clone()))
                .collect(),
            total_embeddings: r.total_embeddings,
        }
    }
}

impl From<CachedEmbedResult> for EmbedResult {
    fn from(c: CachedEmbedResult) -> Self {
        Self {
            chunk_embeddings: c.chunk_embeddings,
            entity_embeddings: c.entity_embeddings,
            relationship_embeddings: c
                .relationship_embeddings
                .into_iter()
                .map(|(s, t, v)| ((s, t), v))
                .collect(),
            total_embeddings: c.total_embeddings,
        }
    }
}

/// Run Phase 3: Embed.
///
/// Generates embeddings for:
/// 1. All text chunks (for vector search)
/// 2. Entity descriptions (for entity search)
/// 3. Relationship descriptions (for relationship search)
pub async fn run_embed(
    config: &BatchConfig,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    state_mgr: &StateManager,
    job: &mut JobState,
    chunks: &[PreparedChunk],
    extractions: &HashMap<String, ExtractionResult>,
    progress: &BatchProgress,
) -> anyhow::Result<EmbedResult> {
    info!("Phase 3: EMBED starting");
    job.phase = Phase::Embedding;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    // Record phase start time and write LATEST_RUN for UI visibility
    let now_millis = chrono::Utc::now().timestamp_millis();
    job.phase_started_at
        .insert("embedding".to_string(), now_millis);
    let run_started_at = job.run_started_at.unwrap_or(now_millis);

    let latest = LatestRunStatus {
        status: "embedding".to_string(),
        phase: Some("embedding".to_string()),
        job_id: Some(job.job_id.clone()),
        total_documents: Some(job.total_documents),
        processed_documents: Some(job.processed_documents),
        total_chunks: Some(job.total_chunks),
        started_at: Some(run_started_at),
        updated_at: Some(now_millis),
        phase_started_at: Some(now_millis),
        ..Default::default()
    };
    state_mgr.write_latest_run(&latest).await?;

    let batch_size = config.embedding_batch_size;

    // Step 1: Embed chunks
    let chunk_texts: Vec<(&str, &str)> = chunks
        .iter()
        .map(|c| (c.custom_id.as_str(), c.chunk.content.as_str()))
        .collect();

    let bar = progress.document_bar(
        (chunk_texts.len() / batch_size + 1) as u64,
        "Embedding chunks",
    );
    let mut chunk_embeddings: HashMap<String, Vec<f32>> = HashMap::new();

    for batch in chunk_texts.chunks(batch_size) {
        let texts: Vec<String> = batch.iter().map(|(_, text)| text.to_string()).collect();
        let ids: Vec<&str> = batch.iter().map(|(id, _)| *id).collect();

        match embedding_provider.embed(&texts).await {
            Ok(embeddings) => {
                for (id, embedding) in ids.iter().zip(embeddings) {
                    chunk_embeddings.insert(id.to_string(), embedding);
                }
            }
            Err(e) => {
                warn!(error = %e, batch_size = texts.len(), "Failed to embed chunk batch");
            }
        }

        bar.inc(1);
    }
    bar.finish_with_message(format!("Embedded {} chunks", chunk_embeddings.len()));

    // Step 2: Embed entity descriptions
    let mut entity_texts: Vec<(String, String)> = Vec::new();
    for result in extractions.values() {
        for entity in &result.entities {
            let text = format!("{}: {}", entity.name, entity.description);
            entity_texts.push((entity.name.clone(), text));
        }
    }
    // Deduplicate by entity name
    entity_texts.sort_by(|a, b| a.0.cmp(&b.0));
    entity_texts.dedup_by(|a, b| a.0 == b.0);

    let bar = progress.document_bar(
        (entity_texts.len() / batch_size + 1) as u64,
        "Embedding entities",
    );
    let mut entity_embeddings: HashMap<String, Vec<f32>> = HashMap::new();

    for batch in entity_texts.chunks(batch_size) {
        let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();
        let names: Vec<&str> = batch.iter().map(|(name, _)| name.as_str()).collect();

        match embedding_provider.embed(&texts).await {
            Ok(embeddings) => {
                for (name, embedding) in names.iter().zip(embeddings) {
                    entity_embeddings.insert(name.to_string(), embedding);
                }
            }
            Err(e) => {
                warn!(error = %e, batch_size = texts.len(), "Failed to embed entity batch");
            }
        }

        bar.inc(1);
    }
    bar.finish_with_message(format!("Embedded {} entities", entity_embeddings.len()));

    // Step 3: Embed relationship descriptions
    let mut rel_texts: Vec<((String, String), String)> = Vec::new();
    for result in extractions.values() {
        for rel in &result.relationships {
            let text = format!(
                "{} -> {}: {} {}",
                rel.source, rel.target, rel.relation_type, rel.description
            );
            rel_texts.push(((rel.source.clone(), rel.target.clone()), text));
        }
    }
    // Deduplicate
    rel_texts.sort_by(|a, b| a.0.cmp(&b.0));
    rel_texts.dedup_by(|a, b| a.0 == b.0);

    let bar = progress.document_bar(
        (rel_texts.len() / batch_size + 1) as u64,
        "Embedding relationships",
    );
    let mut relationship_embeddings: HashMap<(String, String), Vec<f32>> = HashMap::new();

    for batch in rel_texts.chunks(batch_size) {
        let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();
        let keys: Vec<(String, String)> = batch.iter().map(|(key, _)| key.clone()).collect();

        match embedding_provider.embed(&texts).await {
            Ok(embeddings) => {
                for (key, embedding) in keys.into_iter().zip(embeddings) {
                    relationship_embeddings.insert(key, embedding);
                }
            }
            Err(e) => {
                warn!(error = %e, batch_size = texts.len(), "Failed to embed relationship batch");
            }
        }

        bar.inc(1);
    }
    bar.finish_with_message(format!(
        "Embedded {} relationships",
        relationship_embeddings.len()
    ));

    let total_embeddings =
        chunk_embeddings.len() + entity_embeddings.len() + relationship_embeddings.len();

    // Update job state
    job.total_embeddings = total_embeddings;
    job.phase = Phase::Embedded;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    state_mgr.save_job(job).await?;

    info!(
        chunks = chunk_embeddings.len(),
        entities = entity_embeddings.len(),
        relationships = relationship_embeddings.len(),
        total = total_embeddings,
        "Phase 3: EMBED complete"
    );

    let result = EmbedResult {
        chunk_embeddings,
        entity_embeddings,
        relationship_embeddings,
        total_embeddings,
    };

    // Cache results to disk for standalone store command
    let cache_path = config.work_dir.join("embed_results.json");
    let cached: CachedEmbedResult = (&result).into();
    match serde_json::to_string(&cached) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&cache_path, json) {
                warn!(error = %e, "Failed to cache embed results to disk");
            } else {
                info!(path = %cache_path.display(), "Embed results cached to disk");
            }
        }
        Err(e) => warn!(error = %e, "Failed to serialize embed results"),
    }

    Ok(result)
}

/// Load cached embed results from disk.
///
/// Returns None if the cache file doesn't exist or can't be parsed.
pub fn load_cached_embed_results(config: &BatchConfig) -> Option<EmbedResult> {
    let cache_path = config.work_dir.join("embed_results.json");
    match std::fs::read_to_string(&cache_path) {
        Ok(json) => match serde_json::from_str::<CachedEmbedResult>(&json) {
            Ok(cached) => {
                let result: EmbedResult = cached.into();
                info!(
                    path = %cache_path.display(),
                    chunks = result.chunk_embeddings.len(),
                    entities = result.entity_embeddings.len(),
                    relationships = result.relationship_embeddings.len(),
                    "Loaded cached embed results"
                );
                Some(result)
            }
            Err(e) => {
                warn!(error = %e, "Failed to parse cached embed results");
                None
            }
        },
        Err(_) => {
            info!("No cached embed results found at {}", cache_path.display());
            None
        }
    }
}
