//! Versioned Graph-RAG snapshot contract and loader.
//!
//! Snapshots are the portable boundary between ingestion and embedded query
//! runtimes. Batch ingestion writes JSONL artifacts; embedded MCP servers
//! validate and hydrate those artifacts into in-memory storage adapters.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use edgequake_storage::{
    Bm25Document, Bm25Storage, GraphStorage, KVStorage, MemoryBm25Storage, MemoryGraphStorage,
    MemoryKVStorage, MemoryVectorStorage, VectorStorage,
};

pub const SNAPSHOT_SCHEMA_VERSION: &str = "1.0";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const DOCUMENTS_FILE: &str = "documents.jsonl";
pub const CHUNKS_FILE: &str = "chunks.jsonl";
pub const ENTITIES_FILE: &str = "entities.jsonl";
pub const RELATIONSHIPS_FILE: &str = "relationships.jsonl";
pub const VECTORS_FILE: &str = "vectors.jsonl";
pub const BM25_FILE: &str = "bm25.jsonl";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotCompression {
    None,
}

impl Default for SnapshotCompression {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub schema_version: String,
    pub dataset_id: String,
    pub namespace: String,
    pub created_at: String,
    #[serde(default)]
    pub compression: SnapshotCompression,
    pub embedding: SnapshotEmbedding,
    #[serde(default)]
    pub indexes: SnapshotIndexes,
    #[serde(default)]
    pub schema: SnapshotSchemaMetadata,
    #[serde(default)]
    pub counts: SnapshotCounts,
    #[serde(default)]
    pub files: BTreeMap<String, SnapshotFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEmbedding {
    pub model: String,
    pub dimension: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotIndexes {
    #[serde(default)]
    pub documents: bool,
    #[serde(default)]
    pub chunks: bool,
    #[serde(default)]
    pub entities: bool,
    #[serde(default)]
    pub relationships: bool,
    #[serde(default)]
    pub vectors: bool,
    #[serde(default)]
    pub bm25: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotSchemaMetadata {
    #[serde(default)]
    pub entity_types: Vec<String>,
    #[serde(default)]
    pub relationship_types: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotCounts {
    #[serde(default)]
    pub documents: usize,
    #[serde(default)]
    pub chunks: usize,
    #[serde(default)]
    pub entities: usize,
    #[serde(default)]
    pub relationships: usize,
    #[serde(default)]
    pub vectors: usize,
    #[serde(default)]
    pub bm25: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub path: String,
    pub record_count: usize,
    pub sha256: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDocumentRecord {
    pub id: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotChunkRecord {
    pub id: String,
    #[serde(default)]
    pub document_id: Option<String>,
    pub content: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntityRecord {
    pub id: String,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRelationshipRecord {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotVectorRecord {
    pub id: String,
    pub embedding: Vec<f32>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotBm25Record {
    pub doc_id: String,
    pub terms: Vec<String>,
    pub doc_length: usize,
    pub source_id: String,
}

#[derive(Clone)]
pub struct LoadedSnapshot {
    pub manifest: SnapshotManifest,
    pub kv: Arc<MemoryKVStorage>,
    pub graph: Arc<MemoryGraphStorage>,
    pub vectors: Arc<MemoryVectorStorage>,
    pub bm25: Arc<MemoryBm25Storage>,
}

pub struct SnapshotLoader {
    root: PathBuf,
}

impl SnapshotLoader {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub async fn load(&self) -> anyhow::Result<LoadedSnapshot> {
        let manifest: SnapshotManifest =
            serde_json::from_reader(File::open(self.root.join(MANIFEST_FILE))?)?;
        self.validate_manifest(&manifest)?;

        let kv = Arc::new(MemoryKVStorage::new(&manifest.namespace));
        let graph = Arc::new(MemoryGraphStorage::new(&manifest.namespace));
        let vectors = Arc::new(MemoryVectorStorage::new(
            &manifest.namespace,
            manifest.embedding.dimension,
        ));
        let bm25 = Arc::new(MemoryBm25Storage::new(&manifest.namespace));

        kv.initialize().await?;
        graph.initialize().await?;
        vectors.initialize().await?;
        bm25.ensure_tables().await?;

        self.load_documents(&manifest, &kv).await?;
        self.load_chunks(&manifest, &kv).await?;
        self.load_entities(&manifest, &graph).await?;
        self.load_relationships(&manifest, &graph).await?;
        self.load_vectors(&manifest, &vectors).await?;
        self.load_bm25(&manifest, &bm25).await?;

        Ok(LoadedSnapshot {
            manifest,
            kv,
            graph,
            vectors,
            bm25,
        })
    }

    fn validate_manifest(&self, manifest: &SnapshotManifest) -> anyhow::Result<()> {
        if manifest.schema_version != SNAPSHOT_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported snapshot schema_version {}; expected {}",
                manifest.schema_version,
                SNAPSHOT_SCHEMA_VERSION
            );
        }
        if manifest.compression != SnapshotCompression::None {
            anyhow::bail!("snapshot compression is not supported in v1");
        }
        if manifest.embedding.dimension == 0 {
            anyhow::bail!("snapshot embedding dimension must be greater than zero");
        }

        self.validate_enabled_index_file(manifest.indexes.documents, "documents", manifest)?;
        self.validate_enabled_index_file(manifest.indexes.chunks, "chunks", manifest)?;
        self.validate_enabled_index_file(manifest.indexes.entities, "entities", manifest)?;
        self.validate_enabled_index_file(
            manifest.indexes.relationships,
            "relationships",
            manifest,
        )?;
        self.validate_enabled_index_file(manifest.indexes.vectors, "vectors", manifest)?;
        self.validate_enabled_index_file(manifest.indexes.bm25, "bm25", manifest)?;
        self.validate_manifest_counts(manifest)?;

        for (logical_name, file) in &manifest.files {
            let path = self.root.join(&file.path);
            if !path.exists() {
                if file.required {
                    anyhow::bail!("required snapshot file missing: {}", file.path);
                }
                continue;
            }
            let actual_hash = sha256_file(&path)?;
            if actual_hash != file.sha256 {
                anyhow::bail!(
                    "snapshot checksum mismatch for {}: expected {}, got {}",
                    file.path,
                    file.sha256,
                    actual_hash
                );
            }
            let actual_count = count_jsonl_records(&path)?;
            if actual_count != file.record_count {
                anyhow::bail!(
                    "snapshot record count mismatch for {} ({}): expected {}, got {}",
                    logical_name,
                    file.path,
                    file.record_count,
                    actual_count
                );
            }
        }

        Ok(())
    }

    fn validate_enabled_index_file(
        &self,
        enabled: bool,
        logical_name: &str,
        manifest: &SnapshotManifest,
    ) -> anyhow::Result<()> {
        if enabled && !manifest.files.contains_key(logical_name) {
            anyhow::bail!(
                "enabled snapshot index missing file entry: {}",
                logical_name
            );
        }
        Ok(())
    }

    fn validate_manifest_counts(&self, manifest: &SnapshotManifest) -> anyhow::Result<()> {
        let expected = [
            ("documents", manifest.counts.documents),
            ("chunks", manifest.counts.chunks),
            ("entities", manifest.counts.entities),
            ("relationships", manifest.counts.relationships),
            ("vectors", manifest.counts.vectors),
            ("bm25", manifest.counts.bm25),
        ];
        for (logical_name, count) in expected {
            if let Some(file) = manifest.files.get(logical_name) {
                if file.record_count != count {
                    anyhow::bail!(
                        "snapshot manifest count mismatch for {}: counts={}, file.record_count={}",
                        logical_name,
                        count,
                        file.record_count
                    );
                }
            }
        }
        Ok(())
    }

    async fn load_documents(
        &self,
        manifest: &SnapshotManifest,
        kv: &MemoryKVStorage,
    ) -> anyhow::Result<()> {
        let Some(file) = manifest.files.get("documents") else {
            return Ok(());
        };
        let records: Vec<SnapshotDocumentRecord> = read_jsonl(self.root.join(&file.path))?;
        let items: Vec<_> = records
            .into_iter()
            .map(|record| {
                let id = record.id;
                let value = serde_json::json!({
                    "id": id,
                    "content": record.content,
                    "metadata": record.metadata,
                });
                (format!("doc:{}", id), value)
            })
            .collect();
        kv.upsert(&items).await?;
        Ok(())
    }

    async fn load_chunks(
        &self,
        manifest: &SnapshotManifest,
        kv: &MemoryKVStorage,
    ) -> anyhow::Result<()> {
        let Some(file) = manifest.files.get("chunks") else {
            return Ok(());
        };
        let records: Vec<SnapshotChunkRecord> = read_jsonl(self.root.join(&file.path))?;
        let items: Vec<_> = records
            .into_iter()
            .map(|record| {
                let id = record.id;
                let value = serde_json::json!({
                    "id": id,
                    "document_id": record.document_id,
                    "content": record.content,
                    "metadata": record.metadata,
                });
                (format!("chunk:{}", id), value)
            })
            .collect();
        kv.upsert(&items).await?;
        Ok(())
    }

    async fn load_entities(
        &self,
        manifest: &SnapshotManifest,
        graph: &MemoryGraphStorage,
    ) -> anyhow::Result<()> {
        let Some(file) = manifest.files.get("entities") else {
            return Ok(());
        };
        let records: Vec<SnapshotEntityRecord> = read_jsonl(self.root.join(&file.path))?;
        let nodes: Vec<_> = records
            .into_iter()
            .map(|record| (record.id, record.properties))
            .collect();
        graph.upsert_nodes_batch(&nodes).await?;
        Ok(())
    }

    async fn load_relationships(
        &self,
        manifest: &SnapshotManifest,
        graph: &MemoryGraphStorage,
    ) -> anyhow::Result<()> {
        let Some(file) = manifest.files.get("relationships") else {
            return Ok(());
        };
        let records: Vec<SnapshotRelationshipRecord> = read_jsonl(self.root.join(&file.path))?;
        for record in records {
            graph
                .upsert_edge(&record.source, &record.target, record.properties)
                .await?;
        }
        Ok(())
    }

    async fn load_vectors(
        &self,
        manifest: &SnapshotManifest,
        vectors: &MemoryVectorStorage,
    ) -> anyhow::Result<()> {
        let Some(file) = manifest.files.get("vectors") else {
            return Ok(());
        };
        let records: Vec<SnapshotVectorRecord> = read_jsonl(self.root.join(&file.path))?;
        for record in &records {
            if record.embedding.len() != manifest.embedding.dimension {
                anyhow::bail!(
                    "vector {} dimension {} does not match manifest dimension {}",
                    record.id,
                    record.embedding.len(),
                    manifest.embedding.dimension
                );
            }
        }
        let items: Vec<_> = records
            .into_iter()
            .map(|record| (record.id, record.embedding, record.metadata))
            .collect();
        vectors.upsert(&items).await?;
        Ok(())
    }

    async fn load_bm25(
        &self,
        manifest: &SnapshotManifest,
        bm25: &MemoryBm25Storage,
    ) -> anyhow::Result<()> {
        let Some(file) = manifest.files.get("bm25") else {
            return Ok(());
        };
        let records: Vec<SnapshotBm25Record> = read_jsonl(self.root.join(&file.path))?;
        let docs: Vec<Bm25Document> = records
            .into_iter()
            .map(|record| Bm25Document {
                doc_id: record.doc_id,
                terms: record.terms,
                doc_length: record.doc_length,
                source_id: record.source_id,
            })
            .collect();
        bm25.index_batch(&docs).await?;
        bm25.update_corpus_stats().await?;
        Ok(())
    }
}

pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn count_jsonl_records(path: &Path) -> anyhow::Result<usize> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut count = 0;
    for line in reader.lines() {
        if !line?.trim().is_empty() {
            count += 1;
        }
    }
    Ok(count)
}

pub fn read_jsonl<T: for<'de> Deserialize<'de>>(path: impl AsRef<Path>) -> anyhow::Result<Vec<T>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        records.push(
            serde_json::from_str(&line)
                .map_err(|err| anyhow::anyhow!("invalid JSONL line {}: {}", idx + 1, err))?,
        );
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use edgequake_storage::{Bm25Storage, GraphStorage, KVStorage, VectorStorage};
    use std::fs;
    use tempfile::TempDir;

    fn write_jsonl<T: Serialize>(dir: &Path, file: &str, records: &[T]) {
        let content = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.join(file), format!("{}\n", content)).unwrap();
    }

    fn file_entry(dir: &Path, path: &str, record_count: usize) -> SnapshotFile {
        SnapshotFile {
            path: path.to_string(),
            record_count,
            sha256: sha256_file(&dir.join(path)).unwrap(),
            required: true,
        }
    }

    fn fixture_snapshot() -> (TempDir, SnapshotManifest) {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        write_jsonl(
            dir,
            DOCUMENTS_FILE,
            &[SnapshotDocumentRecord {
                id: "doc-1".to_string(),
                content: Some("Alpha policy".to_string()),
                metadata: serde_json::json!({"title": "Alpha"}),
            }],
        );
        write_jsonl(
            dir,
            CHUNKS_FILE,
            &[SnapshotChunkRecord {
                id: "chunk-1".to_string(),
                document_id: Some("doc-1".to_string()),
                content: "Alpha policy applies".to_string(),
                metadata: serde_json::json!({"type": "chunk"}),
            }],
        );
        write_jsonl(
            dir,
            ENTITIES_FILE,
            &[
                SnapshotEntityRecord {
                    id: "A".to_string(),
                    properties: HashMap::from([(
                        "entity_type".to_string(),
                        serde_json::json!("Policy"),
                    )]),
                },
                SnapshotEntityRecord {
                    id: "B".to_string(),
                    properties: HashMap::from([(
                        "entity_type".to_string(),
                        serde_json::json!("Requirement"),
                    )]),
                },
            ],
        );
        write_jsonl(
            dir,
            RELATIONSHIPS_FILE,
            &[SnapshotRelationshipRecord {
                source: "A".to_string(),
                target: "B".to_string(),
                properties: HashMap::from([("kind".to_string(), serde_json::json!("requires"))]),
            }],
        );
        write_jsonl(
            dir,
            VECTORS_FILE,
            &[SnapshotVectorRecord {
                id: "chunk-1".to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                metadata: serde_json::json!({"type": "chunk"}),
            }],
        );
        write_jsonl(
            dir,
            BM25_FILE,
            &[SnapshotBm25Record {
                doc_id: "chunk-1".to_string(),
                terms: vec!["alpha".to_string(), "policy".to_string()],
                doc_length: 2,
                source_id: "doc-1".to_string(),
            }],
        );

        let files = BTreeMap::from([
            ("documents".to_string(), file_entry(dir, DOCUMENTS_FILE, 1)),
            ("chunks".to_string(), file_entry(dir, CHUNKS_FILE, 1)),
            ("entities".to_string(), file_entry(dir, ENTITIES_FILE, 2)),
            (
                "relationships".to_string(),
                file_entry(dir, RELATIONSHIPS_FILE, 1),
            ),
            ("vectors".to_string(), file_entry(dir, VECTORS_FILE, 1)),
            ("bm25".to_string(), file_entry(dir, BM25_FILE, 1)),
        ]);

        let manifest = SnapshotManifest {
            schema_version: SNAPSHOT_SCHEMA_VERSION.to_string(),
            dataset_id: "dataset-1".to_string(),
            namespace: "test".to_string(),
            created_at: "2026-05-22T00:00:00Z".to_string(),
            compression: SnapshotCompression::None,
            embedding: SnapshotEmbedding {
                model: "test-embedding".to_string(),
                dimension: 3,
            },
            indexes: SnapshotIndexes {
                documents: true,
                chunks: true,
                entities: true,
                relationships: true,
                vectors: true,
                bm25: true,
            },
            schema: SnapshotSchemaMetadata::default(),
            counts: SnapshotCounts {
                documents: 1,
                chunks: 1,
                entities: 2,
                relationships: 1,
                vectors: 1,
                bm25: 1,
            },
            files,
        };
        fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        (temp, manifest)
    }

    #[tokio::test]
    async fn snapshot_loader_hydrates_memory_indexes() {
        let (temp, _) = fixture_snapshot();

        let loaded = SnapshotLoader::new(temp.path()).load().await.unwrap();

        assert_eq!(loaded.kv.count_by_prefix("doc:").await.unwrap(), 1);
        assert_eq!(loaded.kv.count_by_prefix("chunk:").await.unwrap(), 1);
        assert!(loaded.graph.has_edge("A", "B").await.unwrap());
        assert!(!loaded.graph.has_edge("B", "A").await.unwrap());
        assert_eq!(loaded.vectors.count().await.unwrap(), 1);
        let bm25 = loaded.bm25.query(&["alpha".to_string()], 10).await.unwrap();
        assert_eq!(bm25[0].doc_id, "chunk-1");
    }

    #[tokio::test]
    async fn snapshot_loader_rejects_missing_required_file() {
        let (temp, _) = fixture_snapshot();
        fs::remove_file(temp.path().join(CHUNKS_FILE)).unwrap();

        let err = match SnapshotLoader::new(temp.path()).load().await {
            Ok(_) => panic!("expected missing file validation failure"),
            Err(err) => err.to_string(),
        };

        assert!(err.contains("required snapshot file missing"));
    }

    #[tokio::test]
    async fn snapshot_loader_rejects_count_mismatch() {
        let (temp, mut manifest) = fixture_snapshot();
        manifest.counts.chunks = 2;
        fs::write(
            temp.path().join(MANIFEST_FILE),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = match SnapshotLoader::new(temp.path()).load().await {
            Ok(_) => panic!("expected count mismatch validation failure"),
            Err(err) => err.to_string(),
        };

        assert!(err.contains("snapshot manifest count mismatch"));
    }

    #[tokio::test]
    async fn snapshot_loader_rejects_checksum_mismatch() {
        let (temp, _) = fixture_snapshot();
        fs::write(temp.path().join(BM25_FILE), "{\"doc_id\":\"changed\"}\n").unwrap();

        let err = match SnapshotLoader::new(temp.path()).load().await {
            Ok(_) => panic!("expected checksum mismatch validation failure"),
            Err(err) => err.to_string(),
        };

        assert!(err.contains("snapshot checksum mismatch"));
    }

    #[tokio::test]
    async fn snapshot_loader_rejects_vector_dimension_mismatch() {
        let (temp, mut manifest) = fixture_snapshot();
        write_jsonl(
            temp.path(),
            VECTORS_FILE,
            &[SnapshotVectorRecord {
                id: "chunk-1".to_string(),
                embedding: vec![1.0, 0.0],
                metadata: serde_json::json!({"type": "chunk"}),
            }],
        );
        if let Some(file) = manifest.files.get_mut("vectors") {
            *file = file_entry(temp.path(), VECTORS_FILE, 1);
        }
        fs::write(
            temp.path().join(MANIFEST_FILE),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = match SnapshotLoader::new(temp.path()).load().await {
            Ok(_) => panic!("expected vector dimension validation failure"),
            Err(err) => err.to_string(),
        };

        assert!(err.contains("does not match manifest dimension"));
    }
}
