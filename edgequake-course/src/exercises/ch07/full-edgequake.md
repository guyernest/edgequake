::: exercise
id: full-edgequake
difficulty: advanced
time: 35 minutes
:::

# Full EdgeQuake Integration

This is the capstone exercise. You will wire together all the components from
previous chapters -- vector storage, graph storage, chunking, extraction, query
mode selection, and retrieval -- into a simplified but complete Graph RAG system.
The goal is to ingest a document and successfully query it end-to-end.

::: objectives
thinking:
  - Understand how all Graph RAG components compose into a pipeline
  - Reason about the flow of data from raw text to generated answer
doing:
  - Initialize in-memory storage layers (vector + graph + key-value)
  - Wire the ingestion pipeline (chunk -> extract -> embed -> store)
  - Wire the query engine (mode select -> retrieve -> assemble context -> answer)
  - Run an end-to-end test: ingest then query
:::

::: discussion
- What is the minimum viable Graph RAG system? Which components are essential vs nice-to-have?
- How does EdgeQuake's 13-crate workspace map to the layers in this simplified version?
- What would break first if you scaled this to millions of documents?
:::

::: starter file="src/main.rs"
```rust
use std::collections::HashMap;
use std::cmp::Ordering;

// ============================================================
// Storage Layer (simplified from Ch01 + Ch04)
// ============================================================

#[derive(Debug, Clone)]
struct VectorEntry {
    id: String,
    text: String,
    embedding: Vec<f64>,
    metadata: HashMap<String, String>,
}

struct VectorStore {
    entries: Vec<VectorEntry>,
}

impl VectorStore {
    fn new() -> Self { VectorStore { entries: Vec::new() } }

    fn insert(&mut self, entry: VectorEntry) {
        self.entries.push(entry);
    }

    fn search(&self, query_embedding: &[f64], top_k: usize) -> Vec<(String, f64)> {
        let mut results: Vec<(String, f64)> = self.entries.iter()
            .map(|e| (e.id.clone(), cosine_similarity(query_embedding, &e.embedding)))
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        results.truncate(top_k);
        results
    }
}

fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 { 0.0 } else { dot / (mag_a * mag_b) }
}

#[derive(Debug, Clone)]
struct GraphNode {
    id: String,
    label: String,
    properties: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct GraphEdge {
    source: String,
    target: String,
    label: String,
}

struct GraphStore {
    nodes: HashMap<String, GraphNode>,
    edges: Vec<GraphEdge>,
    adjacency: HashMap<String, Vec<usize>>, // node_id -> edge indices
}

impl GraphStore {
    fn new() -> Self {
        GraphStore {
            nodes: HashMap::new(),
            edges: Vec::new(),
            adjacency: HashMap::new(),
        }
    }

    fn upsert_node(&mut self, node: GraphNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    fn upsert_edge(&mut self, edge: GraphEdge) {
        let idx = self.edges.len();
        self.adjacency.entry(edge.source.clone()).or_default().push(idx);
        self.edges.push(edge);
    }

    fn get_neighbors(&self, node_id: &str) -> Vec<&GraphEdge> {
        self.adjacency.get(node_id)
            .map(|indices| indices.iter().filter_map(|&i| self.edges.get(i)).collect())
            .unwrap_or_default()
    }
}

/// Simple key-value store for chunk text and metadata.
struct KVStore {
    data: HashMap<String, String>,
}

impl KVStore {
    fn new() -> Self { KVStore { data: HashMap::new() } }
    fn set(&mut self, key: String, value: String) { self.data.insert(key, value); }
    fn get(&self, key: &str) -> Option<&String> { self.data.get(key) }
}

// ============================================================
// Ingestion Pipeline
// ============================================================

#[derive(Debug, Clone)]
struct Chunk {
    id: String,
    text: String,
    header_path: String,
}

#[derive(Debug, Clone)]
struct ExtractedEntity {
    name: String,
    entity_type: String,
}

#[derive(Debug, Clone)]
struct ExtractedRelationship {
    source: String,
    target: String,
    label: String,
}

#[derive(Debug)]
struct ExtractionResult {
    entities: Vec<ExtractedEntity>,
    relationships: Vec<ExtractedRelationship>,
}

/// The main EdgeQuake system that ties everything together.
struct EdgeQuake {
    vectors: VectorStore,
    graph: GraphStore,
    kv: KVStore,
}

impl EdgeQuake {
    fn new() -> Self {
        // TODO: Initialize all three storage layers
        todo!("Implement EdgeQuake::new")
    }

    /// Ingest a document: chunk it, extract entities, embed chunks, store everything.
    fn ingest(&mut self, document: &str) {
        // TODO: Implement the ingestion pipeline
        //
        // Step 1: Chunk the document.
        // Use a simple chunking strategy (split by double newlines or
        // a fixed size -- you can simplify from Ch02).
        //
        // Step 2: For each chunk, extract entities and relationships.
        // Use mock_extract() below instead of a real LLM.
        //
        // Step 3: For each chunk, generate an embedding.
        // Use mock_embed() below instead of a real embedding model.
        //
        // Step 4: Store everything:
        //   - Chunk text in KVStore (key: chunk_id)
        //   - Chunk embedding in VectorStore
        //   - Entities as nodes in GraphStore
        //   - Relationships as edges in GraphStore
        todo!("Implement ingest")
    }

    /// Query the system: select mode, retrieve context, generate answer.
    fn query(&self, question: &str) -> String {
        // TODO: Implement the query pipeline
        //
        // Step 1: Select query mode using simple heuristics (from Ch06).
        // For this exercise, you can simplify to just two modes:
        //   - "local" if the question mentions a known entity
        //   - "naive" otherwise
        //
        // Step 2: Retrieve relevant chunks.
        //   - Generate a query embedding using mock_embed().
        //   - Search the VectorStore for top-3 similar chunks.
        //   - If mode is "local", also get graph neighbors for any
        //     mentioned entity and include their connected chunks.
        //
        // Step 3: Assemble context from retrieved chunk texts.
        //   - Look up chunk text from KVStore using the chunk IDs.
        //
        // Step 4: Generate answer using mock_generate().
        //   - Pass the question and assembled context.
        //
        // Return the generated answer.
        todo!("Implement query")
    }
}

// ============================================================
// Mock functions (simulating LLM and embedding model)
// ============================================================

/// Mock entity extraction. Extracts simple patterns from text.
fn mock_extract(text: &str) -> ExtractionResult {
    let mut entities = Vec::new();
    let mut relationships = Vec::new();

    // Simple heuristic: treat capitalized multi-word phrases as entities
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        let word = words[i].trim_matches(|c: char| !c.is_alphanumeric());
        if !word.is_empty()
            && word.chars().next().map_or(false, |c| c.is_uppercase())
            && word.len() > 1
        {
            // Check if next word is also capitalized (multi-word entity)
            let mut name = word.to_string();
            let mut j = i + 1;
            while j < words.len() {
                let next = words[j].trim_matches(|c: char| !c.is_alphanumeric());
                if !next.is_empty() && next.chars().next().map_or(false, |c| c.is_uppercase()) {
                    name.push(' ');
                    name.push_str(next);
                    j += 1;
                } else {
                    break;
                }
            }

            let entity_type = if name.contains("Corp") || name.contains("Inc") {
                "Organization"
            } else {
                "Entity"
            };

            entities.push(ExtractedEntity {
                name: name.clone(),
                entity_type: entity_type.to_string(),
            });
            i = j;
        } else {
            i += 1;
        }
    }

    // Create relationships between consecutive entities
    for pair in entities.windows(2) {
        relationships.push(ExtractedRelationship {
            source: pair[0].name.clone(),
            target: pair[1].name.clone(),
            label: "RELATED_TO".into(),
        });
    }

    ExtractionResult {
        entities,
        relationships,
    }
}

/// Mock embedding function. Creates a deterministic embedding from text.
fn mock_embed(text: &str) -> Vec<f64> {
    // Simple hash-based embedding (NOT a real embedding -- just for testing)
    let mut embedding = vec![0.0f64; 8];
    for (i, byte) in text.bytes().enumerate() {
        embedding[i % 8] += (byte as f64) / 256.0;
    }
    // Normalize
    let mag: f64 = embedding.iter().map(|x| x * x).sum::<f64>().sqrt();
    if mag > 0.0 {
        for x in &mut embedding {
            *x /= mag;
        }
    }
    embedding
}

/// Mock answer generation. Concatenates context with the question.
fn mock_generate(question: &str, context: &[String]) -> String {
    if context.is_empty() {
        return format!("I don't have enough context to answer: {}", question);
    }
    let context_text = context.join("\n---\n");
    format!(
        "Based on the following context:\n{}\n\nAnswer: The context discusses topics related to your question: {}",
        context_text,
        question
    )
}

/// Simple chunker: split on double newlines, assign IDs.
fn simple_chunk(text: &str) -> Vec<Chunk> {
    text.split("\n\n")
        .enumerate()
        .filter(|(_, s)| !s.trim().is_empty())
        .map(|(i, s)| Chunk {
            id: format!("chunk_{}", i),
            text: s.trim().to_string(),
            header_path: String::new(),
        })
        .collect()
}

fn main() {
    let document = r#"
EdgeQuake is a Graph RAG engine built in Rust. It combines knowledge graphs
with vector search to provide accurate answers to complex questions.

The system was created by Alice Chen at TechCorp Research. Alice designed
the hybrid retrieval architecture that uses both local and global search modes.

Bob Martinez contributed the ingestion pipeline, which processes documents
through four phases: chunking, extraction, embedding, and storage.

TechCorp Research published a paper on the system's performance, showing
significant improvements over traditional RAG approaches on multi-hop questions.

The storage layer supports multiple backends including Amazon Neptune for
graph storage and Amazon S3 Vectors for embedding search.
"#;

    println!("=== Initializing EdgeQuake ===");
    let mut eq = EdgeQuake::new();

    println!("\n=== Ingesting Document ===");
    eq.ingest(document);
    println!("Ingestion complete.");
    println!("  Vector entries: {}", eq.vectors.entries.len());
    println!("  Graph nodes: {}", eq.graph.nodes.len());
    println!("  Graph edges: {}", eq.graph.edges.len());

    println!("\n=== Querying ===");

    let q1 = "What is EdgeQuake?";
    println!("\nQ: {}", q1);
    println!("A: {}", eq.query(q1));

    let q2 = "Who created the system?";
    println!("\nQ: {}", q2);
    println!("A: {}", eq.query(q2));

    let q3 = "What storage backends are used?";
    println!("\nQ: {}", q3);
    println!("A: {}", eq.query(q3));
}
```
:::

::: hint level=1 title="Ingestion pipeline wiring"
The ingest method should chain: chunk -> extract -> embed -> store.

```rust
fn ingest(&mut self, document: &str) {
    let chunks = simple_chunk(document);

    for chunk in &chunks {
        // Store chunk text in KV
        self.kv.set(chunk.id.clone(), chunk.text.clone());

        // Extract entities and relationships
        let extraction = mock_extract(&chunk.text);

        // Store entities as graph nodes
        for entity in &extraction.entities {
            self.graph.upsert_node(GraphNode {
                id: entity.name.to_lowercase().replace(' ', "_"),
                label: entity.entity_type.clone(),
                properties: HashMap::from([("name".into(), entity.name.clone())]),
            });
        }
        // ... continue with edges and vector storage
    }
}
```
:::

::: hint level=2 title="Query pipeline wiring"
The query method should: embed the question, search vectors, look up text,
and generate.

```rust
fn query(&self, question: &str) -> String {
    let query_embedding = mock_embed(question);
    let results = self.vectors.search(&query_embedding, 3);

    let mut context: Vec<String> = Vec::new();
    for (chunk_id, _score) in &results {
        if let Some(text) = self.kv.get(chunk_id) {
            context.push(text.clone());
        }
    }

    mock_generate(question, &context)
}
```

For local mode enhancement, check if any known graph node names appear in
the question, get their neighbors, and include connected chunk context.
:::

::: solution
```rust
use std::collections::HashMap;
use std::cmp::Ordering;

#[derive(Debug, Clone)]
struct VectorEntry {
    id: String,
    text: String,
    embedding: Vec<f64>,
    metadata: HashMap<String, String>,
}

struct VectorStore {
    entries: Vec<VectorEntry>,
}

impl VectorStore {
    fn new() -> Self { VectorStore { entries: Vec::new() } }

    fn insert(&mut self, entry: VectorEntry) {
        self.entries.push(entry);
    }

    fn search(&self, query_embedding: &[f64], top_k: usize) -> Vec<(String, f64)> {
        let mut results: Vec<(String, f64)> = self.entries.iter()
            .map(|e| (e.id.clone(), cosine_similarity(query_embedding, &e.embedding)))
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        results.truncate(top_k);
        results
    }
}

fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 { 0.0 } else { dot / (mag_a * mag_b) }
}

#[derive(Debug, Clone)]
struct GraphNode {
    id: String,
    label: String,
    properties: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct GraphEdge {
    source: String,
    target: String,
    label: String,
}

struct GraphStore {
    nodes: HashMap<String, GraphNode>,
    edges: Vec<GraphEdge>,
    adjacency: HashMap<String, Vec<usize>>,
}

impl GraphStore {
    fn new() -> Self {
        GraphStore {
            nodes: HashMap::new(),
            edges: Vec::new(),
            adjacency: HashMap::new(),
        }
    }

    fn upsert_node(&mut self, node: GraphNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    fn upsert_edge(&mut self, edge: GraphEdge) {
        let idx = self.edges.len();
        self.adjacency.entry(edge.source.clone()).or_default().push(idx);
        self.edges.push(edge);
    }

    fn get_neighbors(&self, node_id: &str) -> Vec<&GraphEdge> {
        self.adjacency.get(node_id)
            .map(|indices| indices.iter().filter_map(|&i| self.edges.get(i)).collect())
            .unwrap_or_default()
    }
}

struct KVStore {
    data: HashMap<String, String>,
}

impl KVStore {
    fn new() -> Self { KVStore { data: HashMap::new() } }
    fn set(&mut self, key: String, value: String) { self.data.insert(key, value); }
    fn get(&self, key: &str) -> Option<&String> { self.data.get(key) }
}

#[derive(Debug, Clone)]
struct Chunk {
    id: String,
    text: String,
    header_path: String,
}

#[derive(Debug, Clone)]
struct ExtractedEntity {
    name: String,
    entity_type: String,
}

#[derive(Debug, Clone)]
struct ExtractedRelationship {
    source: String,
    target: String,
    label: String,
}

#[derive(Debug)]
struct ExtractionResult {
    entities: Vec<ExtractedEntity>,
    relationships: Vec<ExtractedRelationship>,
}

struct EdgeQuake {
    vectors: VectorStore,
    graph: GraphStore,
    kv: KVStore,
}

impl EdgeQuake {
    fn new() -> Self {
        EdgeQuake {
            vectors: VectorStore::new(),
            graph: GraphStore::new(),
            kv: KVStore::new(),
        }
    }

    fn ingest(&mut self, document: &str) {
        let chunks = simple_chunk(document);

        for chunk in &chunks {
            // Step 1: Store chunk text in KV store
            self.kv.set(chunk.id.clone(), chunk.text.clone());

            // Step 2: Extract entities and relationships
            let extraction = mock_extract(&chunk.text);

            // Step 3: Store entities as graph nodes
            for entity in &extraction.entities {
                let node_id = entity.name.to_lowercase().replace(' ', "_");
                self.graph.upsert_node(GraphNode {
                    id: node_id.clone(),
                    label: entity.entity_type.clone(),
                    properties: HashMap::from([
                        ("name".into(), entity.name.clone()),
                        ("source_chunk".into(), chunk.id.clone()),
                    ]),
                });
            }

            // Step 4: Store relationships as graph edges
            for rel in &extraction.relationships {
                let source_id = rel.source.to_lowercase().replace(' ', "_");
                let target_id = rel.target.to_lowercase().replace(' ', "_");
                self.graph.upsert_edge(GraphEdge {
                    source: source_id,
                    target: target_id,
                    label: rel.label.clone(),
                });
            }

            // Step 5: Embed and store the chunk vector
            let embedding = mock_embed(&chunk.text);
            self.vectors.insert(VectorEntry {
                id: chunk.id.clone(),
                text: chunk.text.clone(),
                embedding,
                metadata: HashMap::new(),
            });
        }
    }

    fn query(&self, question: &str) -> String {
        // Step 1: Embed the query
        let query_embedding = mock_embed(question);

        // Step 2: Vector search for relevant chunks
        let vector_results = self.vectors.search(&query_embedding, 3);

        // Step 3: Collect chunk IDs from vector search
        let mut chunk_ids: Vec<String> = vector_results.iter()
            .map(|(id, _)| id.clone())
            .collect();

        // Step 4: Check for entity mentions (simple local mode)
        let question_lower = question.to_lowercase();
        for (node_id, node) in &self.graph.nodes {
            let name = node.properties.get("name")
                .map(|n| n.to_lowercase())
                .unwrap_or_default();
            if !name.is_empty() && question_lower.contains(&name) {
                // Found a mentioned entity -- get its neighbors
                let neighbors = self.graph.get_neighbors(node_id);
                for edge in neighbors {
                    // Look up the target node's source chunk
                    if let Some(target_node) = self.graph.nodes.get(&edge.target) {
                        if let Some(source_chunk) = target_node.properties.get("source_chunk") {
                            if !chunk_ids.contains(source_chunk) {
                                chunk_ids.push(source_chunk.clone());
                            }
                        }
                    }
                }
                // Also include the entity's own source chunk
                if let Some(source_chunk) = node.properties.get("source_chunk") {
                    if !chunk_ids.contains(source_chunk) {
                        chunk_ids.push(source_chunk.clone());
                    }
                }
            }
        }

        // Step 5: Assemble context from chunk texts
        let context: Vec<String> = chunk_ids.iter()
            .filter_map(|id| self.kv.get(id).cloned())
            .collect();

        // Step 6: Generate answer
        mock_generate(question, &context)
    }
}

fn mock_extract(text: &str) -> ExtractionResult {
    let mut entities = Vec::new();
    let mut relationships = Vec::new();

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        let word = words[i].trim_matches(|c: char| !c.is_alphanumeric());
        if !word.is_empty()
            && word.chars().next().map_or(false, |c| c.is_uppercase())
            && word.len() > 1
        {
            let mut name = word.to_string();
            let mut j = i + 1;
            while j < words.len() {
                let next = words[j].trim_matches(|c: char| !c.is_alphanumeric());
                if !next.is_empty() && next.chars().next().map_or(false, |c| c.is_uppercase()) {
                    name.push(' ');
                    name.push_str(next);
                    j += 1;
                } else {
                    break;
                }
            }

            let entity_type = if name.contains("Corp") || name.contains("Inc") {
                "Organization"
            } else {
                "Entity"
            };

            entities.push(ExtractedEntity {
                name: name.clone(),
                entity_type: entity_type.to_string(),
            });
            i = j;
        } else {
            i += 1;
        }
    }

    for pair in entities.windows(2) {
        relationships.push(ExtractedRelationship {
            source: pair[0].name.clone(),
            target: pair[1].name.clone(),
            label: "RELATED_TO".into(),
        });
    }

    ExtractionResult {
        entities,
        relationships,
    }
}

fn mock_embed(text: &str) -> Vec<f64> {
    let mut embedding = vec![0.0f64; 8];
    for (i, byte) in text.bytes().enumerate() {
        embedding[i % 8] += (byte as f64) / 256.0;
    }
    let mag: f64 = embedding.iter().map(|x| x * x).sum::<f64>().sqrt();
    if mag > 0.0 {
        for x in &mut embedding {
            *x /= mag;
        }
    }
    embedding
}

fn mock_generate(question: &str, context: &[String]) -> String {
    if context.is_empty() {
        return format!("I don't have enough context to answer: {}", question);
    }
    let context_text = context.join("\n---\n");
    format!(
        "Based on the following context:\n{}\n\nAnswer: The context discusses topics related to your question: {}",
        context_text,
        question
    )
}

fn simple_chunk(text: &str) -> Vec<Chunk> {
    text.split("\n\n")
        .enumerate()
        .filter(|(_, s)| !s.trim().is_empty())
        .map(|(i, s)| Chunk {
            id: format!("chunk_{}", i),
            text: s.trim().to_string(),
            header_path: String::new(),
        })
        .collect()
}

fn main() {
    let document = r#"
EdgeQuake is a Graph RAG engine built in Rust. It combines knowledge graphs
with vector search to provide accurate answers to complex questions.

The system was created by Alice Chen at TechCorp Research. Alice designed
the hybrid retrieval architecture that uses both local and global search modes.

Bob Martinez contributed the ingestion pipeline, which processes documents
through four phases: chunking, extraction, embedding, and storage.

TechCorp Research published a paper on the system's performance, showing
significant improvements over traditional RAG approaches on multi-hop questions.

The storage layer supports multiple backends including Amazon Neptune for
graph storage and Amazon S3 Vectors for embedding search.
"#;

    println!("=== Initializing EdgeQuake ===");
    let mut eq = EdgeQuake::new();

    println!("\n=== Ingesting Document ===");
    eq.ingest(document);
    println!("Ingestion complete.");
    println!("  Vector entries: {}", eq.vectors.entries.len());
    println!("  Graph nodes: {}", eq.graph.nodes.len());
    println!("  Graph edges: {}", eq.graph.edges.len());

    println!("\n=== Querying ===");

    let q1 = "What is EdgeQuake?";
    println!("\nQ: {}", q1);
    println!("A: {}", eq.query(q1));

    let q2 = "Who created the system?";
    println!("\nQ: {}", q2);
    println!("A: {}", eq.query(q2));

    let q3 = "What storage backends are used?";
    println!("\nQ: {}", q3);
    println!("A: {}", eq.query(q3));
}
```

### Explanation

The EdgeQuake integration wires four stages:

**Initialization**: Three storage backends are created: `VectorStore` for
embedding search, `GraphStore` for entity/relationship traversal, and
`KVStore` for chunk text retrieval.

**Ingestion Pipeline**: For each chunk: (1) store the raw text in KV,
(2) extract entities and relationships via the mock extractor, (3) store
entities as graph nodes with a `source_chunk` property linking back to the
originating chunk, (4) store relationships as directed graph edges, and
(5) embed the chunk text and store the vector.

**Query Pipeline**: (1) Embed the question, (2) search vectors for the top-3
most similar chunks, (3) optionally enhance with graph context by checking if
any known entity names appear in the question and following their edges to find
additional relevant chunks, (4) look up all chunk texts from KV, and (5) pass
the assembled context to the mock generator.

The key insight is the `source_chunk` property on graph nodes -- this is the
bridge between the graph and vector stores, allowing graph traversal results
to be linked back to retrievable text chunks.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_document() -> &'static str {
        "Alice works at TechCorp in Seattle.\n\nBob collaborates with Alice on the GraphRAG project.\n\nTechCorp develops AI software for enterprise clients."
    }

    #[test]
    fn test_initialization() {
        let eq = EdgeQuake::new();
        assert_eq!(eq.vectors.entries.len(), 0);
        assert_eq!(eq.graph.nodes.len(), 0);
        assert_eq!(eq.graph.edges.len(), 0);
    }

    #[test]
    fn test_ingest_creates_vectors() {
        let mut eq = EdgeQuake::new();
        eq.ingest(sample_document());
        assert!(eq.vectors.entries.len() > 0, "Should create vector entries");
    }

    #[test]
    fn test_ingest_creates_graph_nodes() {
        let mut eq = EdgeQuake::new();
        eq.ingest(sample_document());
        assert!(eq.graph.nodes.len() > 0, "Should create graph nodes");
    }

    #[test]
    fn test_ingest_stores_chunk_text() {
        let mut eq = EdgeQuake::new();
        eq.ingest(sample_document());
        // At least chunk_0 should exist
        assert!(eq.kv.get("chunk_0").is_some(), "Should store chunk text in KV");
    }

    #[test]
    fn test_query_returns_answer() {
        let mut eq = EdgeQuake::new();
        eq.ingest(sample_document());
        let answer = eq.query("What does Alice do?");
        assert!(!answer.is_empty(), "Should return a non-empty answer");
        assert!(!answer.contains("don't have enough context"),
            "Should find relevant context");
    }

    #[test]
    fn test_query_empty_store() {
        let eq = EdgeQuake::new();
        let answer = eq.query("What is GraphRAG?");
        assert!(answer.contains("don't have enough context"),
            "Empty store should indicate insufficient context");
    }

    #[test]
    fn test_end_to_end_pipeline() {
        let mut eq = EdgeQuake::new();
        let doc = "Rust is a systems programming language.\n\nRust provides memory safety without garbage collection.";
        eq.ingest(doc);

        assert!(eq.vectors.entries.len() >= 2, "Should have at least 2 chunks");

        let answer = eq.query("What is Rust?");
        assert!(answer.contains("context"), "Answer should reference context");
    }

    #[test]
    fn test_mock_embed_deterministic() {
        let e1 = mock_embed("hello world");
        let e2 = mock_embed("hello world");
        assert_eq!(e1, e2, "Same input should produce same embedding");
    }

    #[test]
    fn test_mock_embed_different_inputs() {
        let e1 = mock_embed("hello");
        let e2 = mock_embed("goodbye");
        assert_ne!(e1, e2, "Different inputs should produce different embeddings");
    }

    #[test]
    fn test_simple_chunk_splits_on_double_newline() {
        let text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let chunks = simple_chunk(text);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].text, "First paragraph.");
        assert_eq!(chunks[1].text, "Second paragraph.");
    }
}
```
:::

::: reflection
- What is the weakest link in this pipeline? Which component would you improve first for production use?
- How would you add observability (logging, metrics, tracing) to each pipeline stage?
- If you replaced the mock functions with real LLM/embedding API calls, what error handling would you need to add?
- How does this simplified system compare to EdgeQuake's actual 13-crate architecture? What is missing?
:::
