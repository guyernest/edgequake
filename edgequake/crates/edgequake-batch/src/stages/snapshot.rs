//! Snapshot writer for portable embedded Graph-RAG runtimes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::config::BatchConfig;
use crate::jsonl::PreparedChunk;
use crate::parquet_reader::ReconstructedDocument;
use crate::stages::embed::EmbedResult;
use edgequake_core::snapshot::*;
use edgequake_pipeline::extractor::ExtractionResult;

pub async fn write_snapshot(
    config: &BatchConfig,
    documents: &[ReconstructedDocument],
    chunks: &[PreparedChunk],
    extractions: &HashMap<String, ExtractionResult>,
    embeddings: &EmbedResult,
) -> anyhow::Result<Option<SnapshotManifest>> {
    let Some(uri) = config.snapshot_uri.as_deref() else {
        return Ok(None);
    };

    let local_dir = config.work_dir.join("snapshot");
    if local_dir.exists() {
        fs::remove_dir_all(&local_dir)?;
    }
    fs::create_dir_all(&local_dir)?;

    let documents_count = write_documents(&local_dir.join(DOCUMENTS_FILE), documents)?;
    let chunks_count = write_chunks(&local_dir.join(CHUNKS_FILE), chunks)?;
    let (entities_count, relationships_count, entity_types, relationship_types) =
        write_graph_files(&local_dir, extractions)?;
    let vectors_count = write_vectors(&local_dir.join(VECTORS_FILE), embeddings)?;
    let bm25_count = write_bm25(&local_dir.join(BM25_FILE), chunks)?;

    let mut files = BTreeMap::new();
    insert_file(
        &mut files,
        "documents",
        &local_dir,
        DOCUMENTS_FILE,
        documents_count,
    )?;
    insert_file(&mut files, "chunks", &local_dir, CHUNKS_FILE, chunks_count)?;
    insert_file(
        &mut files,
        "entities",
        &local_dir,
        ENTITIES_FILE,
        entities_count,
    )?;
    insert_file(
        &mut files,
        "relationships",
        &local_dir,
        RELATIONSHIPS_FILE,
        relationships_count,
    )?;
    insert_file(
        &mut files,
        "vectors",
        &local_dir,
        VECTORS_FILE,
        vectors_count,
    )?;
    insert_file(&mut files, "bm25", &local_dir, BM25_FILE, bm25_count)?;

    let manifest = SnapshotManifest {
        schema_version: SNAPSHOT_SCHEMA_VERSION.to_string(),
        dataset_id: config.namespace.clone(),
        namespace: config.namespace.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        compression: SnapshotCompression::None,
        embedding: SnapshotEmbedding {
            model: config.embedding_model.clone(),
            dimension: config.embedding_dimension as usize,
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
            metadata: serde_json::json!({
                "extraction_model": config.extraction_model,
                "chunk_size": config.chunk_size,
                "chunk_overlap": config.chunk_overlap,
                "snapshot_mode": config.snapshot_mode.as_str(),
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

    serde_json::to_writer_pretty(File::create(local_dir.join(MANIFEST_FILE))?, &manifest)?;
    publish_snapshot(uri, &local_dir).await?;
    Ok(Some(manifest))
}

fn write_documents(path: &Path, documents: &[ReconstructedDocument]) -> anyhow::Result<usize> {
    let mut writer = jsonl_writer(path)?;
    for doc in documents {
        let record = SnapshotDocumentRecord {
            id: doc.content_hash.clone(),
            content: None,
            metadata: serde_json::json!({
                "filename": doc.filename,
                "content_preview": doc.content.chars().take(1000).collect::<String>(),
                "content_length": doc.content.len(),
                "content_hash": doc.content_hash,
                "source_row_count": doc.source_row_count,
            }),
        };
        write_jsonl_record(&mut writer, &record)?;
    }
    Ok(documents.len())
}

fn write_chunks(path: &Path, chunks: &[PreparedChunk]) -> anyhow::Result<usize> {
    let mut writer = jsonl_writer(path)?;
    for chunk in chunks {
        let record = SnapshotChunkRecord {
            id: chunk.custom_id.clone(),
            document_id: Some(chunk.doc_hash.clone()),
            content: chunk.chunk.content.clone(),
            metadata: serde_json::json!({
                "filename": chunk.doc_filename,
                "index": chunk.chunk.index,
                "token_count": chunk.chunk.token_count,
            }),
        };
        write_jsonl_record(&mut writer, &record)?;
    }
    Ok(chunks.len())
}

fn write_graph_files(
    root: &Path,
    extractions: &HashMap<String, ExtractionResult>,
) -> anyhow::Result<(usize, usize, BTreeSet<String>, BTreeSet<String>)> {
    let mut entities: BTreeMap<String, SnapshotEntityRecord> = BTreeMap::new();
    let mut relationships: BTreeMap<(String, String), SnapshotRelationshipRecord> = BTreeMap::new();
    let mut entity_types = BTreeSet::new();
    let mut relationship_types = BTreeSet::new();

    for result in extractions.values() {
        for entity in &result.entities {
            entity_types.insert(entity.entity_type.clone());
            entities.entry(entity.name.clone()).or_insert_with(|| {
                let mut properties = HashMap::new();
                properties.insert(
                    "entity_type".to_string(),
                    serde_json::json!(entity.entity_type),
                );
                properties.insert(
                    "description".to_string(),
                    serde_json::json!(entity.description),
                );
                properties.insert(
                    "source_chunk_ids".to_string(),
                    serde_json::json!(entity.source_chunk_ids),
                );
                if let Some(doc_id) = &entity.source_document_id {
                    properties.insert("source_document_id".to_string(), serde_json::json!(doc_id));
                }
                if let Some(file_path) = &entity.source_file_path {
                    properties.insert("source_file_path".to_string(), serde_json::json!(file_path));
                }
                SnapshotEntityRecord {
                    id: entity.name.clone(),
                    properties,
                }
            });
        }

        for rel in &result.relationships {
            let relation_type = rel
                .keywords
                .first()
                .cloned()
                .unwrap_or_else(|| "RELATED_TO".to_string());
            relationship_types.insert(relation_type.clone());
            relationships
                .entry((rel.source.clone(), rel.target.clone()))
                .or_insert_with(|| {
                    let mut properties = HashMap::new();
                    properties.insert(
                        "relation_type".to_string(),
                        serde_json::json!(relation_type),
                    );
                    properties.insert(
                        "description".to_string(),
                        serde_json::json!(rel.description),
                    );
                    properties.insert("keywords".to_string(), serde_json::json!(rel.keywords));
                    properties.insert("weight".to_string(), serde_json::json!(rel.weight));
                    if let Some(chunk_id) = &rel.source_chunk_id {
                        properties
                            .insert("source_chunk_id".to_string(), serde_json::json!(chunk_id));
                    }
                    if let Some(doc_id) = &rel.source_document_id {
                        properties
                            .insert("source_document_id".to_string(), serde_json::json!(doc_id));
                    }
                    if let Some(file_path) = &rel.source_file_path {
                        properties
                            .insert("source_file_path".to_string(), serde_json::json!(file_path));
                    }
                    SnapshotRelationshipRecord {
                        source: rel.source.clone(),
                        target: rel.target.clone(),
                        properties,
                    }
                });
        }
    }

    let mut entity_writer = jsonl_writer(&root.join(ENTITIES_FILE))?;
    for record in entities.values() {
        write_jsonl_record(&mut entity_writer, record)?;
    }
    let mut relationship_writer = jsonl_writer(&root.join(RELATIONSHIPS_FILE))?;
    for record in relationships.values() {
        write_jsonl_record(&mut relationship_writer, record)?;
    }

    Ok((
        entities.len(),
        relationships.len(),
        entity_types,
        relationship_types,
    ))
}

fn write_vectors(path: &Path, embeddings: &EmbedResult) -> anyhow::Result<usize> {
    let mut writer = jsonl_writer(path)?;
    let mut count = 0;

    for (id, embedding) in &embeddings.chunk_embeddings {
        write_jsonl_record(
            &mut writer,
            &SnapshotVectorRecord {
                id: id.clone(),
                embedding: embedding.clone(),
                metadata: serde_json::json!({ "type": "chunk", "chunk_id": id }),
            },
        )?;
        count += 1;
    }
    for (name, embedding) in &embeddings.entity_embeddings {
        write_jsonl_record(
            &mut writer,
            &SnapshotVectorRecord {
                id: format!("entity:{}", name),
                embedding: embedding.clone(),
                metadata: serde_json::json!({ "type": "entity", "entity_name": name }),
            },
        )?;
        count += 1;
    }
    for ((source, target), embedding) in &embeddings.relationship_embeddings {
        write_jsonl_record(
            &mut writer,
            &SnapshotVectorRecord {
                id: format!("rel:{}:{}", source, target),
                embedding: embedding.clone(),
                metadata: serde_json::json!({
                    "type": "relationship",
                    "source": source,
                    "target": target,
                }),
            },
        )?;
        count += 1;
    }
    Ok(count)
}

fn write_bm25(path: &Path, chunks: &[PreparedChunk]) -> anyhow::Result<usize> {
    let docs = crate::stages::bm25_index::build_bm25_documents(chunks);
    let mut writer = jsonl_writer(path)?;
    for doc in &docs {
        write_jsonl_record(
            &mut writer,
            &SnapshotBm25Record {
                doc_id: doc.doc_id.clone(),
                terms: doc.terms.clone(),
                doc_length: doc.doc_length,
                source_id: doc.source_id.clone(),
            },
        )?;
    }
    Ok(docs.len())
}

fn jsonl_writer(path: &Path) -> anyhow::Result<BufWriter<File>> {
    Ok(BufWriter::new(File::create(path)?))
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

async fn publish_snapshot(uri: &str, local_dir: &Path) -> anyhow::Result<()> {
    if let Some((bucket, prefix)) = parse_s3_uri(uri) {
        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = aws_sdk_s3::Client::new(&aws_config);
        for file in [
            MANIFEST_FILE,
            DOCUMENTS_FILE,
            CHUNKS_FILE,
            ENTITIES_FILE,
            RELATIONSHIPS_FILE,
            VECTORS_FILE,
            BM25_FILE,
        ] {
            let key = if prefix.is_empty() {
                file.to_string()
            } else {
                format!("{}/{}", prefix.trim_end_matches('/'), file)
            };
            let body = aws_sdk_s3::primitives::ByteStream::from_path(local_dir.join(file)).await?;
            client
                .put_object()
                .bucket(&bucket)
                .key(key)
                .body(body)
                .send()
                .await?;
        }
        return Ok(());
    }

    let target = PathBuf::from(uri);
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::create_dir_all(&target)?;
    for file in [
        MANIFEST_FILE,
        DOCUMENTS_FILE,
        CHUNKS_FILE,
        ENTITIES_FILE,
        RELATIONSHIPS_FILE,
        VECTORS_FILE,
        BM25_FILE,
    ] {
        fs::copy(local_dir.join(file), target.join(file))?;
    }
    Ok(())
}

fn parse_s3_uri(uri: &str) -> Option<(String, String)> {
    let rest = uri.strip_prefix("s3://")?;
    let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
    Some((bucket.to_string(), prefix.trim_matches('/').to_string()))
}
