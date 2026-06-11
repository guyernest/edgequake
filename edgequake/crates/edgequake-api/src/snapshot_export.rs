//! Workspace snapshot export for the API server (Phase 24 D-05).
//!
//! Mirrors the batch pipeline's snapshot writer
//! (`edgequake-batch/src/stages/snapshot.rs`) but sources its data from the
//! live server storages instead of batch intermediates:
//!
//! | File | Source |
//! |------|--------|
//! | `documents.jsonl` | KV storage `{doc_id}-metadata` + `{doc_id}-content` |
//! | `chunks.jsonl` | KV storage chunk records (`{content, document_id, index}`) |
//! | `entities.jsonl` | Graph storage nodes (workspace-scoped) |
//! | `relationships.jsonl` | Graph storage edges (workspace-scoped) |
//! | `vectors.jsonl` | Workspace vector storage (chunk + entity embeddings) |
//! | `bm25.jsonl` | Tokenized chunks via shared `edgequake_core::bm25_text` |
//! | `manifest.json` | [`SnapshotManifest`] (counts, sha256, embedding config) |
//!
//! # Entity vectors
//!
//! The downstream snapshot contract REQUIRES entity vectors
//! (`metadata.type == "entity"`, count > 0). The server pipeline stores
//! entity embeddings under `entity:{name}` ids when the extraction produced
//! them; for any entity missing a stored embedding this module generates one
//! AT EXPORT TIME by embedding `"{name}: {description}"` with the injected
//! embedding provider (resolved from the workspace embedding config by the
//! caller, exactly like the document task processor resolves its embedder).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context};
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use edgequake_core::bm25_text::{create_bm25_analyzer, tokenize_text};
use edgequake_core::snapshot::{
    SnapshotBm25Record, SnapshotChunkRecord, SnapshotCompression, SnapshotCounts,
    SnapshotDocumentRecord, SnapshotEmbedding, SnapshotEntityRecord, SnapshotFile,
    SnapshotIndexes, SnapshotManifest, SnapshotRelationshipRecord, SnapshotSchemaMetadata,
    SnapshotVectorRecord, BM25_FILE, CHUNKS_FILE, DOCUMENTS_FILE, ENTITIES_FILE, MANIFEST_FILE,
    RELATIONSHIPS_FILE, SNAPSHOT_SCHEMA_VERSION, VECTORS_FILE,
};
use edgequake_core::types::Workspace;
use edgequake_llm::traits::EmbeddingProvider;
use edgequake_storage::traits::{GraphStorage, KVStorage, VectorStorage};

/// Batch size for export-time entity embedding generation.
const ENTITY_EMBED_BATCH_SIZE: usize = 64;

/// Storage handles the export reads from.
///
/// The vector storage must be the WORKSPACE-scoped instance (from the
/// `WorkspaceVectorRegistry`), and the embedding provider must be resolved
/// from the workspace embedding config (provider/model/dimension) — both are
/// the caller's responsibility so tests can inject memory storages and the
/// mock provider.
pub struct SnapshotExportSources {
    /// KV storage holding document metadata/content and chunk records.
    pub kv_storage: Arc<dyn KVStorage>,
    /// Graph storage holding entities and relationships.
    pub graph_storage: Arc<dyn GraphStorage>,
    /// Workspace-scoped vector storage holding chunk/entity embeddings.
    pub vector_storage: Arc<dyn VectorStorage>,
    /// Embedding provider for export-time entity embedding backfill.
    pub embedding_provider: Arc<dyn EmbeddingProvider>,
}

/// A workspace document read from KV storage.
#[derive(Debug, Clone)]
pub struct WorkspaceDocument {
    /// Document id (KV key prefix).
    pub id: String,
    /// Display filename (title or file_name metadata, falling back to id).
    pub filename: String,
    /// Full document content.
    pub content: String,
}

/// A workspace chunk read from KV storage.
#[derive(Debug, Clone)]
struct WorkspaceChunk {
    /// Chunk id (KV key; also the vector storage id).
    id: String,
    /// Owning document id.
    document_id: String,
    /// Display filename of the owning document.
    filename: String,
    /// Chunk index within the document.
    index: u64,
    /// Chunk content.
    content: String,
}

/// Returns true when `doc_workspace` belongs to `workspace_id`.
///
/// Mirrors the tolerance of the rebuild-knowledge-graph enumeration loop
/// (workspaces.rs): a record matches when its `workspace_id` equals the
/// target workspace or is the legacy `"default"` marker / absent.
fn workspace_matches(doc_workspace: Option<&str>, workspace_id: &str) -> bool {
    match doc_workspace {
        Some(ws) => ws == workspace_id || ws == "default",
        None => true,
    }
}

/// Collect all documents (metadata + content) belonging to a workspace.
///
/// Documents are stored as `{doc_id}-metadata` / `{doc_id}-content` KV pairs
/// with a `workspace_id` field on the metadata record. Results are sorted by
/// document id for deterministic output.
pub async fn collect_workspace_documents(
    kv_storage: &Arc<dyn KVStorage>,
    workspace_id: &str,
) -> anyhow::Result<Vec<WorkspaceDocument>> {
    let all_keys = kv_storage
        .keys()
        .await
        .context("Failed to list KV keys for document enumeration")?;

    let mut documents = Vec::new();
    for key in all_keys.iter().filter(|k| k.ends_with("-metadata")) {
        let Ok(Some(value)) = kv_storage.get_by_id(key).await else {
            continue;
        };
        let Some(obj) = value.as_object() else {
            continue;
        };

        let doc_workspace = obj.get("workspace_id").and_then(|v| v.as_str());
        if !workspace_matches(doc_workspace, workspace_id) {
            continue;
        }

        let Some(doc_id) = obj.get("id").and_then(|v| v.as_str()) else {
            continue;
        };

        let content_key = format!("{}-content", doc_id);
        let content = match kv_storage.get_by_id(&content_key).await {
            Ok(Some(content_value)) => content_value
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            _ => None,
        };
        let Some(content) = content else {
            continue;
        };

        let filename = obj
            .get("title")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("file_name").and_then(|v| v.as_str()))
            .unwrap_or(doc_id)
            .to_string();

        documents.push(WorkspaceDocument {
            id: doc_id.to_string(),
            filename,
            content,
        });
    }

    documents.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(documents)
}

/// Collect all chunk records belonging to the given documents.
///
/// Chunks are stored under their pipeline chunk id (`{doc_id}-chunk-{index}`)
/// with a `{content, document_id, index}` JSON value (processor.rs Stage 3).
/// Matching is done on the stored `document_id` field, not the key shape.
async fn collect_workspace_chunks(
    kv_storage: &Arc<dyn KVStorage>,
    documents: &[WorkspaceDocument],
) -> anyhow::Result<Vec<WorkspaceChunk>> {
    let doc_names: HashMap<&str, &str> = documents
        .iter()
        .map(|d| (d.id.as_str(), d.filename.as_str()))
        .collect();

    let all_keys = kv_storage
        .keys()
        .await
        .context("Failed to list KV keys for chunk enumeration")?;

    let mut chunks = Vec::new();
    for key in all_keys
        .iter()
        .filter(|k| k.contains("-chunk-") && !k.ends_with("-metadata") && !k.ends_with("-content"))
    {
        let Ok(Some(value)) = kv_storage.get_by_id(key).await else {
            continue;
        };
        let Some(obj) = value.as_object() else {
            continue;
        };
        let Some(document_id) = obj.get("document_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(filename) = doc_names.get(document_id) else {
            continue;
        };
        let Some(content) = obj.get("content").and_then(|v| v.as_str()) else {
            continue;
        };
        let index = obj.get("index").and_then(|v| v.as_u64()).unwrap_or(0);

        chunks.push(WorkspaceChunk {
            id: key.clone(),
            document_id: document_id.to_string(),
            filename: filename.to_string(),
            index,
            content: content.to_string(),
        });
    }

    chunks.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(chunks)
}

/// Returns true when ALL documents in a track have reached a terminal status.
///
/// Returns false when the track has no documents (nothing to export) or when
/// any document is still in a non-terminal stage. Used by the rebuild-track
/// auto-export trigger.
pub async fn rebuild_track_is_terminal(
    kv_storage: &Arc<dyn KVStorage>,
    track_id: &str,
) -> anyhow::Result<bool> {
    const TERMINAL_STATUSES: [&str; 4] = ["completed", "partial_failure", "failed", "cancelled"];

    let all_keys = kv_storage
        .keys()
        .await
        .context("Failed to list KV keys for track status check")?;

    let mut found_any = false;
    for key in all_keys.iter().filter(|k| k.ends_with("-metadata")) {
        let Ok(Some(value)) = kv_storage.get_by_id(key).await else {
            continue;
        };
        let Some(obj) = value.as_object() else {
            continue;
        };
        if obj.get("track_id").and_then(|v| v.as_str()) != Some(track_id) {
            continue;
        }
        found_any = true;
        let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if !TERMINAL_STATUSES.contains(&status) {
            return Ok(false);
        }
    }

    Ok(found_any)
}

/// Export a workspace snapshot to `snapshot_uri` (absolute local directory).
///
/// Produces the same 7-file layout as the batch pipeline's snapshot stage:
/// 6 JSONL data files plus `manifest.json`. The target directory is replaced
/// atomically-ish (staged in a tempdir, then the target dir is removed and
/// re-populated), mirroring batch publish semantics for local paths.
///
/// `s3://` URIs are NOT supported by the server export (use the batch
/// pipeline for S3 publication).
pub async fn export_workspace_snapshot(
    sources: &SnapshotExportSources,
    workspace: &Workspace,
    namespace: &str,
    snapshot_uri: &str,
    snapshot_mode: &str,
) -> anyhow::Result<SnapshotManifest> {
    if snapshot_uri.starts_with("s3://") {
        bail!(
            "s3:// snapshot_uri is not supported by the API server export \
             (got '{}'); use a local directory path",
            snapshot_uri
        );
    }
    if snapshot_uri.contains("..") {
        bail!("snapshot_uri must not contain '..' path-traversal sequences");
    }

    let workspace_id = workspace.workspace_id.to_string();
    info!(
        workspace_id = %workspace_id,
        namespace = %namespace,
        snapshot_uri = %snapshot_uri,
        snapshot_mode = %snapshot_mode,
        "Starting workspace snapshot export"
    );

    // --- Collect source data ---
    let documents = collect_workspace_documents(&sources.kv_storage, &workspace_id).await?;
    let chunks = collect_workspace_chunks(&sources.kv_storage, &documents).await?;
    let (entities, relationships) = collect_workspace_graph(sources, &workspace_id).await?;

    // --- Stage files locally, then publish to the target dir ---
    let staging = tempfile::tempdir().context("Failed to create snapshot staging dir")?;
    let local_dir = staging.path();

    let documents_count = write_documents(&local_dir.join(DOCUMENTS_FILE), &documents)?;
    let chunks_count = write_chunks(&local_dir.join(CHUNKS_FILE), &chunks)?;
    let (entities_count, relationships_count, entity_types, relationship_types) =
        write_graph_files(local_dir, &entities, &relationships)?;
    let vectors_count =
        write_vectors(sources, &local_dir.join(VECTORS_FILE), &chunks, &entities).await?;
    let bm25_count = write_bm25(&local_dir.join(BM25_FILE), &chunks)?;

    let mut files = BTreeMap::new();
    insert_file(&mut files, "documents", local_dir, DOCUMENTS_FILE, documents_count)?;
    insert_file(&mut files, "chunks", local_dir, CHUNKS_FILE, chunks_count)?;
    insert_file(&mut files, "entities", local_dir, ENTITIES_FILE, entities_count)?;
    insert_file(
        &mut files,
        "relationships",
        local_dir,
        RELATIONSHIPS_FILE,
        relationships_count,
    )?;
    insert_file(&mut files, "vectors", local_dir, VECTORS_FILE, vectors_count)?;
    insert_file(&mut files, "bm25", local_dir, BM25_FILE, bm25_count)?;

    let manifest = SnapshotManifest {
        schema_version: SNAPSHOT_SCHEMA_VERSION.to_string(),
        dataset_id: namespace.to_string(),
        namespace: namespace.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        compression: SnapshotCompression::None,
        embedding: SnapshotEmbedding {
            model: workspace.embedding_model.clone(),
            dimension: workspace.embedding_dimension,
        },
        indexes: SnapshotIndexes {
            documents: documents_count > 0,
            chunks: chunks_count > 0,
            entities: entities_count > 0,
            relationships: relationships_count > 0,
            vectors: vectors_count > 0,
            bm25: bm25_count > 0,
        },
        schema: SnapshotSchemaMetadata {
            entity_types: entity_types.into_iter().collect(),
            relationship_types: relationship_types.into_iter().collect(),
            metadata: json!({
                "extraction_model": workspace.llm_model,
                "snapshot_mode": snapshot_mode,
                "exported_by": "edgequake-api",
            }),
        },
        counts: SnapshotCounts {
            documents: documents_count,
            chunks: chunks_count,
            entities: entities_count,
            relationships: relationships_count,
            vectors: vectors_count,
            bm25: bm25_count,
        },
        files,
    };

    serde_json::to_writer_pretty(
        File::create(local_dir.join(MANIFEST_FILE)).context("Failed to create manifest.json")?,
        &manifest,
    )
    .context("Failed to write manifest.json")?;

    publish_snapshot(snapshot_uri, local_dir)?;

    info!(
        workspace_id = %workspace_id,
        namespace = %namespace,
        documents = documents_count,
        chunks = chunks_count,
        entities = entities_count,
        relationships = relationships_count,
        vectors = vectors_count,
        bm25 = bm25_count,
        snapshot_uri = %snapshot_uri,
        "Workspace snapshot export complete"
    );

    Ok(manifest)
}

/// Collect workspace-scoped entities (nodes) and relationships (edges).
async fn collect_workspace_graph(
    sources: &SnapshotExportSources,
    workspace_id: &str,
) -> anyhow::Result<(
    Vec<edgequake_storage::traits::GraphNode>,
    Vec<edgequake_storage::traits::GraphEdge>,
)> {
    let nodes = sources
        .graph_storage
        .get_all_nodes()
        .await
        .context("Failed to enumerate graph nodes")?;
    let edges = sources
        .graph_storage
        .get_all_edges()
        .await
        .context("Failed to enumerate graph edges")?;

    let nodes: Vec<_> = nodes
        .into_iter()
        .filter(|n| {
            workspace_matches(
                n.properties.get("workspace_id").and_then(|v| v.as_str()),
                workspace_id,
            )
        })
        .collect();
    let edges: Vec<_> = edges
        .into_iter()
        .filter(|e| {
            workspace_matches(
                e.properties.get("workspace_id").and_then(|v| v.as_str()),
                workspace_id,
            )
        })
        .collect();

    Ok((nodes, edges))
}

fn write_documents(path: &Path, documents: &[WorkspaceDocument]) -> anyhow::Result<usize> {
    let mut writer = jsonl_writer(path)?;
    for doc in documents {
        let record = SnapshotDocumentRecord {
            id: doc.id.clone(),
            content: None,
            metadata: json!({
                "filename": doc.filename,
                "content_preview": doc.content.chars().take(1000).collect::<String>(),
                "content_length": doc.content.len(),
            }),
        };
        write_jsonl_record(&mut writer, &record)?;
    }
    writer.flush()?;
    Ok(documents.len())
}

fn write_chunks(path: &Path, chunks: &[WorkspaceChunk]) -> anyhow::Result<usize> {
    let mut writer = jsonl_writer(path)?;
    for chunk in chunks {
        let record = SnapshotChunkRecord {
            id: chunk.id.clone(),
            document_id: Some(chunk.document_id.clone()),
            content: chunk.content.clone(),
            metadata: json!({
                "filename": chunk.filename,
                "index": chunk.index,
                "token_count": chunk.content.split_whitespace().count(),
            }),
        };
        write_jsonl_record(&mut writer, &record)?;
    }
    writer.flush()?;
    Ok(chunks.len())
}

/// Write entities.jsonl and relationships.jsonl; returns counts and type sets.
fn write_graph_files(
    root: &Path,
    nodes: &[edgequake_storage::traits::GraphNode],
    edges: &[edgequake_storage::traits::GraphEdge],
) -> anyhow::Result<(usize, usize, BTreeSet<String>, BTreeSet<String>)> {
    let mut entity_types = BTreeSet::new();
    let mut relationship_types = BTreeSet::new();

    let mut entity_writer = jsonl_writer(&root.join(ENTITIES_FILE))?;
    for node in nodes {
        let mut properties: HashMap<String, serde_json::Value> = node
            .properties
            .iter()
            // Scrub tenancy scoping — snapshots are single-namespace artifacts.
            .filter(|(k, _)| k.as_str() != "tenant_id" && k.as_str() != "workspace_id")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let entity_type = properties
            .get("entity_type")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();
        properties
            .entry("entity_type".to_string())
            .or_insert_with(|| json!(entity_type));
        properties
            .entry("description".to_string())
            .or_insert_with(|| json!(""));
        entity_types.insert(entity_type);

        let record = SnapshotEntityRecord {
            id: node.id.clone(),
            properties,
        };
        write_jsonl_record(&mut entity_writer, &record)?;
    }
    entity_writer.flush()?;

    let mut relationship_writer = jsonl_writer(&root.join(RELATIONSHIPS_FILE))?;
    for edge in edges {
        let mut properties: HashMap<String, serde_json::Value> = edge
            .properties
            .iter()
            .filter(|(k, _)| k.as_str() != "tenant_id" && k.as_str() != "workspace_id")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let relation_type = properties
            .get("relation_type")
            .and_then(|v| v.as_str())
            .unwrap_or("RELATED_TO")
            .to_string();
        properties
            .entry("relation_type".to_string())
            .or_insert_with(|| json!(relation_type));
        properties
            .entry("description".to_string())
            .or_insert_with(|| json!(""));
        relationship_types.insert(relation_type);

        let record = SnapshotRelationshipRecord {
            source: edge.source.clone(),
            target: edge.target.clone(),
            properties,
        };
        write_jsonl_record(&mut relationship_writer, &record)?;
    }
    relationship_writer.flush()?;

    Ok((
        nodes.len(),
        edges.len(),
        entity_types,
        relationship_types,
    ))
}

/// Write vectors.jsonl: stored chunk embeddings + entity embeddings
/// (stored when available, generated at export time otherwise).
async fn write_vectors(
    sources: &SnapshotExportSources,
    path: &Path,
    chunks: &[WorkspaceChunk],
    entities: &[edgequake_storage::traits::GraphNode],
) -> anyhow::Result<usize> {
    let mut writer = jsonl_writer(path)?;
    let mut count = 0;

    // --- Chunk vectors (from stored embeddings) ---
    let chunk_ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
    let chunk_vectors = sources
        .vector_storage
        .get_by_ids(&chunk_ids)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read chunk embeddings: {}", e))?;
    if chunk_vectors.len() < chunks.len() {
        warn!(
            expected = chunks.len(),
            found = chunk_vectors.len(),
            "Some chunks have no stored embedding; they will be missing from vectors.jsonl"
        );
    }
    for (id, embedding) in &chunk_vectors {
        write_jsonl_record(
            &mut writer,
            &SnapshotVectorRecord {
                id: id.clone(),
                embedding: embedding.clone(),
                metadata: json!({ "type": "chunk", "chunk_id": id }),
            },
        )?;
        count += 1;
    }

    // --- Entity vectors (stored when present, generated otherwise) ---
    let entity_ids: Vec<String> = entities
        .iter()
        .map(|n| format!("entity:{}", n.id))
        .collect();
    let stored_entity_vectors = sources
        .vector_storage
        .get_by_ids(&entity_ids)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read entity embeddings: {}", e))?;
    let stored_ids: HashSet<&str> = stored_entity_vectors
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();

    for (id, embedding) in &stored_entity_vectors {
        let name = id.strip_prefix("entity:").unwrap_or(id);
        write_jsonl_record(
            &mut writer,
            &SnapshotVectorRecord {
                id: id.clone(),
                embedding: embedding.clone(),
                metadata: json!({ "type": "entity", "entity_name": name }),
            },
        )?;
        count += 1;
    }

    // Export-time backfill for entities without a stored embedding.
    let missing: Vec<&edgequake_storage::traits::GraphNode> = entities
        .iter()
        .filter(|n| !stored_ids.contains(format!("entity:{}", n.id).as_str()))
        .collect();
    if !missing.is_empty() {
        info!(
            missing = missing.len(),
            provider = sources.embedding_provider.name(),
            model = sources.embedding_provider.model(),
            "Generating entity embeddings at export time"
        );
    }
    for batch in missing.chunks(ENTITY_EMBED_BATCH_SIZE) {
        let texts: Vec<String> = batch
            .iter()
            .map(|n| {
                let description = n
                    .properties
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                format!("{}: {}", n.id, description)
            })
            .collect();
        let embeddings = sources
            .embedding_provider
            .embed(&texts)
            .await
            .map_err(|e| anyhow::anyhow!("Export-time entity embedding failed: {}", e))?;
        if embeddings.len() != batch.len() {
            bail!(
                "Embedding provider returned {} embeddings for {} entities",
                embeddings.len(),
                batch.len()
            );
        }
        for (node, embedding) in batch.iter().zip(embeddings) {
            write_jsonl_record(
                &mut writer,
                &SnapshotVectorRecord {
                    id: format!("entity:{}", node.id),
                    embedding,
                    metadata: json!({ "type": "entity", "entity_name": node.id }),
                },
            )?;
            count += 1;
        }
    }

    writer.flush()?;
    Ok(count)
}

/// Write bm25.jsonl using the shared tantivy tokenizer pipeline
/// (mirrors `edgequake-batch/src/stages/bm25_index.rs::build_bm25_documents`).
fn write_bm25(path: &Path, chunks: &[WorkspaceChunk]) -> anyhow::Result<usize> {
    let mut analyzer = create_bm25_analyzer();
    let mut writer = jsonl_writer(path)?;
    for chunk in chunks {
        let terms = tokenize_text(&mut analyzer, &chunk.content);
        let doc_length = terms.len();
        write_jsonl_record(
            &mut writer,
            &SnapshotBm25Record {
                doc_id: chunk.id.clone(),
                terms,
                doc_length,
                source_id: chunk.document_id.clone(),
            },
        )?;
    }
    writer.flush()?;
    Ok(chunks.len())
}

fn jsonl_writer(path: &Path) -> anyhow::Result<BufWriter<File>> {
    Ok(BufWriter::new(File::create(path).with_context(|| {
        format!("Failed to create snapshot file {}", path.display())
    })?))
}

fn write_jsonl_record<T: serde::Serialize>(
    writer: &mut BufWriter<File>,
    record: &T,
) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn insert_file(
    files: &mut BTreeMap<String, SnapshotFile>,
    logical_name: &str,
    root: &Path,
    file_name: &str,
    count: usize,
) -> anyhow::Result<()> {
    files.insert(
        logical_name.to_string(),
        SnapshotFile {
            path: file_name.to_string(),
            record_count: count,
            sha256: sha256_file(&root.join(file_name))?,
            required: true,
        },
    );
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let bytes = fs::read(path)
        .with_context(|| format!("Failed to read snapshot file {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Publish the staged snapshot to the target directory (create/replace).
fn publish_snapshot(uri: &str, local_dir: &Path) -> anyhow::Result<()> {
    let target = std::path::PathBuf::from(uri);
    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("Failed to clear snapshot target {}", target.display()))?;
    }
    fs::create_dir_all(&target)
        .with_context(|| format!("Failed to create snapshot target {}", target.display()))?;
    for file in [
        MANIFEST_FILE,
        DOCUMENTS_FILE,
        CHUNKS_FILE,
        ENTITIES_FILE,
        RELATIONSHIPS_FILE,
        VECTORS_FILE,
        BM25_FILE,
    ] {
        fs::copy(local_dir.join(file), target.join(file))
            .with_context(|| format!("Failed to publish snapshot file {}", file))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use edgequake_core::types::{CreateWorkspaceRequest, Tenant};
    use std::io::BufRead;

    /// Build a test AppState with a workspace + documents/chunks/graph/vectors.
    async fn fixture_state() -> (AppState, Workspace) {
        let state = AppState::test_state();

        // Tenant + workspace
        let tenant = state
            .workspace_service
            .create_tenant(Tenant::new("Test Tenant", "test-tenant"))
            .await
            .expect("create tenant");
        let workspace = state
            .workspace_service
            .create_workspace(
                tenant.tenant_id,
                CreateWorkspaceRequest {
                    name: "Snapshot WS".to_string(),
                    slug: Some("snapshot-ws".to_string()),
                    description: None,
                    max_documents: None,
                    llm_model: None,
                    llm_provider: None,
                    embedding_model: Some("text-embedding-3-small".to_string()),
                    embedding_provider: Some("openai".to_string()),
                    embedding_dimension: Some(1536),
                },
            )
            .await
            .expect("create workspace");
        let ws_id = workspace.workspace_id.to_string();

        // Document metadata + content + chunks in KV
        state
            .kv_storage
            .upsert(&[
                (
                    "doc1-metadata".to_string(),
                    json!({
                        "id": "doc1",
                        "title": "course.md",
                        "workspace_id": ws_id,
                        "status": "completed",
                        "track_id": "rebuild_kg_test_1",
                    }),
                ),
                (
                    "doc1-content".to_string(),
                    json!({ "content": "CS101 teaches programming. CS401 requires CS101." }),
                ),
                (
                    "doc1-chunk-0".to_string(),
                    json!({
                        "content": "CS101 teaches programming.",
                        "document_id": "doc1",
                        "index": 0,
                    }),
                ),
                (
                    "doc1-chunk-1".to_string(),
                    json!({
                        "content": "CS401 requires CS101.",
                        "document_id": "doc1",
                        "index": 1,
                    }),
                ),
            ])
            .await
            .expect("kv upsert");

        // Graph: 2 entities + 1 relationship (workspace-scoped)
        let mut props_a = std::collections::HashMap::new();
        props_a.insert("entity_type".to_string(), json!("COURSE"));
        props_a.insert("description".to_string(), json!("Intro course"));
        props_a.insert("workspace_id".to_string(), json!(ws_id.clone()));
        let mut props_b = std::collections::HashMap::new();
        props_b.insert("entity_type".to_string(), json!("COURSE"));
        props_b.insert("description".to_string(), json!("Advanced course"));
        props_b.insert("workspace_id".to_string(), json!(ws_id.clone()));
        state
            .graph_storage
            .upsert_nodes_batch(&[
                ("CS101".to_string(), props_a),
                ("CS401".to_string(), props_b),
            ])
            .await
            .expect("nodes upsert");

        let mut edge_props = std::collections::HashMap::new();
        edge_props.insert("relation_type".to_string(), json!("REQUIRES"));
        edge_props.insert("description".to_string(), json!("prerequisite"));
        edge_props.insert("workspace_id".to_string(), json!(ws_id.clone()));
        state
            .graph_storage
            .upsert_edges_batch(&[("CS401".to_string(), "CS101".to_string(), edge_props)])
            .await
            .expect("edges upsert");

        // Stored chunk embeddings only (NO entity vectors → export-time backfill)
        state
            .vector_storage
            .upsert(&[
                (
                    "doc1-chunk-0".to_string(),
                    vec![0.1; 1536],
                    json!({ "type": "chunk", "document_id": "doc1" }),
                ),
                (
                    "doc1-chunk-1".to_string(),
                    vec![0.2; 1536],
                    json!({ "type": "chunk", "document_id": "doc1" }),
                ),
            ])
            .await
            .expect("vector upsert");

        (state, workspace)
    }

    fn sources_from_state(state: &AppState) -> SnapshotExportSources {
        SnapshotExportSources {
            kv_storage: Arc::clone(&state.kv_storage),
            graph_storage: Arc::clone(&state.graph_storage),
            vector_storage: Arc::clone(&state.vector_storage),
            embedding_provider: Arc::clone(&state.embedding_provider),
        }
    }

    fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
        let file = File::open(path).expect("open jsonl");
        std::io::BufReader::new(file)
            .lines()
            .map(|l| serde_json::from_str(&l.expect("read line")).expect("parse jsonl"))
            .collect()
    }

    #[tokio::test]
    async fn test_export_manifest_shape_and_counts() {
        // Behavior: export produces a manifest mirroring the batch writer —
        // all 6 counts, 6 file entries with sha256, embedding from workspace.
        let (state, workspace) = fixture_state().await;
        let sources = sources_from_state(&state);
        let target = tempfile::tempdir().expect("target dir");
        let snapshot_uri = target.path().join("snapshot").display().to_string();

        let manifest = export_workspace_snapshot(
            &sources,
            &workspace,
            "snapshot-ws",
            &snapshot_uri,
            "write-and-store",
        )
        .await
        .expect("export");

        assert_eq!(manifest.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(manifest.namespace, "snapshot-ws");
        assert_eq!(manifest.dataset_id, "snapshot-ws");
        assert_eq!(manifest.embedding.model, "text-embedding-3-small");
        assert_eq!(manifest.embedding.dimension, 1536);

        assert_eq!(manifest.counts.documents, 1);
        assert_eq!(manifest.counts.chunks, 2);
        assert_eq!(manifest.counts.entities, 2);
        assert_eq!(manifest.counts.relationships, 1);
        // 2 stored chunk vectors + 2 export-time entity vectors
        assert_eq!(manifest.counts.vectors, 4);
        assert_eq!(manifest.counts.bm25, 2);

        assert_eq!(manifest.files.len(), 6);
        for (name, file) in &manifest.files {
            assert!(!file.sha256.is_empty(), "sha256 missing for {}", name);
            assert!(file.required);
        }

        // Schema metadata includes snapshot_mode and discovered types
        assert_eq!(
            manifest.schema.metadata.get("snapshot_mode").and_then(|v| v.as_str()),
            Some("write-and-store")
        );
        assert_eq!(manifest.schema.entity_types, vec!["COURSE".to_string()]);
        assert_eq!(
            manifest.schema.relationship_types,
            vec!["REQUIRES".to_string()]
        );

        // All 7 files exist at the published target
        let target_dir = std::path::PathBuf::from(&snapshot_uri);
        for file in [
            MANIFEST_FILE,
            DOCUMENTS_FILE,
            CHUNKS_FILE,
            ENTITIES_FILE,
            RELATIONSHIPS_FILE,
            VECTORS_FILE,
            BM25_FILE,
        ] {
            assert!(target_dir.join(file).exists(), "{} missing", file);
        }

        // manifest.json on disk parses back to the same counts
        let on_disk: SnapshotManifest = serde_json::from_reader(
            File::open(target_dir.join(MANIFEST_FILE)).expect("open manifest"),
        )
        .expect("parse manifest");
        assert_eq!(on_disk.counts.vectors, 4);
    }

    #[tokio::test]
    async fn test_vectors_jsonl_includes_entity_records() {
        // Behavior: the downstream contract requires entity vectors with
        // metadata.type == "entity"; when the pipeline stored none, the
        // export generates them via the workspace embedding provider.
        let (state, workspace) = fixture_state().await;
        let sources = sources_from_state(&state);
        let target = tempfile::tempdir().expect("target dir");
        let snapshot_uri = target.path().join("snap").display().to_string();

        export_workspace_snapshot(
            &sources,
            &workspace,
            "snapshot-ws",
            &snapshot_uri,
            "snapshot-only",
        )
        .await
        .expect("export");

        let records = read_jsonl(&std::path::PathBuf::from(&snapshot_uri).join(VECTORS_FILE));
        let entity_records: Vec<_> = records
            .iter()
            .filter(|r| r["metadata"]["type"].as_str() == Some("entity"))
            .collect();
        let chunk_records: Vec<_> = records
            .iter()
            .filter(|r| r["metadata"]["type"].as_str() == Some("chunk"))
            .collect();

        assert_eq!(chunk_records.len(), 2);
        assert_eq!(entity_records.len(), 2, "entity vector count must be > 0");
        for record in &entity_records {
            assert!(record["metadata"]["entity_name"].as_str().is_some());
            assert!(record["id"].as_str().unwrap().starts_with("entity:"));
            assert_eq!(record["embedding"].as_array().unwrap().len(), 1536);
        }
    }

    #[tokio::test]
    async fn test_export_prefers_stored_entity_vectors() {
        // Behavior: entities with stored embeddings are exported as-is
        // (no regeneration), missing ones are backfilled.
        let (state, workspace) = fixture_state().await;
        state
            .vector_storage
            .upsert(&[(
                "entity:CS101".to_string(),
                vec![0.5; 1536],
                json!({ "type": "entity", "entity_name": "CS101" }),
            )])
            .await
            .expect("entity vector upsert");

        let sources = sources_from_state(&state);
        let target = tempfile::tempdir().expect("target dir");
        let snapshot_uri = target.path().join("snap").display().to_string();
        export_workspace_snapshot(
            &sources,
            &workspace,
            "snapshot-ws",
            &snapshot_uri,
            "write-and-store",
        )
        .await
        .expect("export");

        let records = read_jsonl(&std::path::PathBuf::from(&snapshot_uri).join(VECTORS_FILE));
        let cs101: Vec<_> = records
            .iter()
            .filter(|r| r["id"].as_str() == Some("entity:CS101"))
            .collect();
        assert_eq!(cs101.len(), 1, "stored entity vector must not be duplicated");
        // Stored vector value preserved (0.5, not the mock's 0.1 backfill)
        assert!((cs101[0]["embedding"][0].as_f64().unwrap() - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_export_rejects_s3_uri() {
        let (state, workspace) = fixture_state().await;
        let sources = sources_from_state(&state);
        let err = export_workspace_snapshot(
            &sources,
            &workspace,
            "snapshot-ws",
            "s3://bucket/prefix",
            "write-and-store",
        )
        .await
        .expect_err("s3 must be rejected");
        assert!(err.to_string().contains("s3://"));
    }

    #[tokio::test]
    async fn test_rebuild_track_is_terminal() {
        let state = AppState::test_state();

        // No documents on the track → not exportable
        assert!(!rebuild_track_is_terminal(&state.kv_storage, "rebuild_kg_x")
            .await
            .unwrap());

        state
            .kv_storage
            .upsert(&[
                (
                    "a-metadata".to_string(),
                    json!({ "id": "a", "track_id": "rebuild_kg_x", "status": "completed" }),
                ),
                (
                    "b-metadata".to_string(),
                    json!({ "id": "b", "track_id": "rebuild_kg_x", "status": "extracting" }),
                ),
            ])
            .await
            .unwrap();
        // One doc still processing → not terminal
        assert!(!rebuild_track_is_terminal(&state.kv_storage, "rebuild_kg_x")
            .await
            .unwrap());

        state
            .kv_storage
            .upsert(&[(
                "b-metadata".to_string(),
                json!({ "id": "b", "track_id": "rebuild_kg_x", "status": "failed" }),
            )])
            .await
            .unwrap();
        // All terminal (completed/failed both count) → exportable
        assert!(rebuild_track_is_terminal(&state.kv_storage, "rebuild_kg_x")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn test_collect_workspace_documents_scopes_by_workspace() {
        let (state, workspace) = fixture_state().await;
        // Add a foreign-workspace document that must be excluded
        state
            .kv_storage
            .upsert(&[
                (
                    "other-metadata".to_string(),
                    json!({ "id": "other", "title": "other.md", "workspace_id": uuid::Uuid::new_v4().to_string() }),
                ),
                (
                    "other-content".to_string(),
                    json!({ "content": "foreign content" }),
                ),
            ])
            .await
            .unwrap();

        let docs = collect_workspace_documents(
            &state.kv_storage,
            &workspace.workspace_id.to_string(),
        )
        .await
        .unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "doc1");
        assert_eq!(docs[0].filename, "course.md");
        assert!(docs[0].content.contains("CS101"));
    }
}
