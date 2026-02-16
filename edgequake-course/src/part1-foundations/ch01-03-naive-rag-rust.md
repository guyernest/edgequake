# 1.3 Building Naive RAG in Rust

Theory is necessary but insufficient. In this section we implement a
complete (if minimal) RAG system in Rust. No external vector database, no
embedding API calls -- just pure Rust and linear algebra. This will give
you a visceral understanding of what every RAG system does under the hood.

> **Note**: We use mock embeddings here (random vectors) to focus on the
> mechanics. In Chapter 2 we will swap in real embedding models.

## Cosine Similarity

The foundation of vector retrieval is a similarity function. Cosine
similarity measures the angle between two vectors, returning a value
between -1 (opposite) and 1 (identical direction).

```rust
/// Compute cosine similarity between two vectors.
///
/// Returns a value in [-1.0, 1.0] where 1.0 means identical direction.
/// Panics if vectors have different lengths or zero magnitude.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "vectors must have equal dimensions");

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }

    dot / (mag_a * mag_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors_have_similarity_one() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors_have_similarity_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }
}
```

### Why Cosine Similarity?

Cosine similarity is preferred over Euclidean distance for text embeddings
because it is *magnitude-invariant*. Two chunks about the same topic will
point in the same direction regardless of how long they are. Euclidean
distance penalizes magnitude differences, which are often meaningless for
normalized embeddings.

## In-Memory Vector Store

Next we build a minimal vector store. In production you would use a
purpose-built database, but the interface is the same.

```rust
use std::collections::HashMap;

/// A chunk stored in our vector index
#[derive(Debug, Clone)]
struct Chunk {
    id: String,
    text: String,
    embedding: Vec<f32>,
    metadata: HashMap<String, String>,
}

/// A minimal in-memory vector store
struct VectorStore {
    chunks: Vec<Chunk>,
    dimension: usize,
}

impl VectorStore {
    fn new(dimension: usize) -> Self {
        Self {
            chunks: Vec::new(),
            dimension,
        }
    }

    /// Insert a chunk into the store
    fn insert(&mut self, chunk: Chunk) {
        assert_eq!(
            chunk.embedding.len(),
            self.dimension,
            "embedding dimension mismatch: expected {}, got {}",
            self.dimension,
            chunk.embedding.len()
        );
        self.chunks.push(chunk);
    }

    /// Retrieve the top-k most similar chunks to a query embedding
    fn search(&self, query: &[f32], k: usize) -> Vec<SearchResult> {
        assert_eq!(query.len(), self.dimension);

        let mut results: Vec<SearchResult> = self
            .chunks
            .iter()
            .map(|chunk| SearchResult {
                chunk: chunk.clone(),
                score: cosine_similarity(query, &chunk.embedding),
            })
            .collect();

        // Sort by descending similarity
        results.sort_by(|a, b| {
            b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(k);
        results
    }

    /// Return the number of stored chunks
    fn len(&self) -> usize {
        self.chunks.len()
    }
}

#[derive(Debug, Clone)]
struct SearchResult {
    chunk: Chunk,
    score: f32,
}
```

### Complexity Analysis

This brute-force search is O(n * d) where *n* is the number of chunks and
*d* is the embedding dimension. For a million chunks at 1536 dimensions,
that is ~1.5 billion floating-point operations per query -- too slow for
production, but perfectly fine for understanding the mechanics.

Production vector stores use approximate nearest neighbor (ANN) algorithms
like HNSW or IVF to achieve sub-millisecond search at the cost of a small
accuracy tradeoff.

## The Chunker

A simple word-boundary chunker with overlap:

```rust
/// Split text into overlapping chunks by word count
fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();

    if words.len() <= chunk_size {
        return vec![words.join(" ")];
    }

    let step = chunk_size.saturating_sub(overlap).max(1);
    let mut chunks = Vec::new();

    let mut start = 0;
    while start < words.len() {
        let end = (start + chunk_size).min(words.len());
        chunks.push(words[start..end].join(" "));

        start += step;
        if end == words.len() {
            break;
        }
    }

    chunks
}

#[cfg(test)]
mod chunk_tests {
    use super::*;

    #[test]
    fn chunking_with_overlap() {
        let text = "one two three four five six seven eight nine ten";
        let chunks = chunk_text(text, 4, 2);

        assert_eq!(chunks[0], "one two three four");
        assert_eq!(chunks[1], "three four five six");
        assert_eq!(chunks[2], "five six seven eight");
    }
}
```

## Mock Embeddings

For this example we use deterministic mock embeddings based on simple text
features. In a real system you would call an embedding API.

```rust
/// Generate a mock embedding from text features.
///
/// This is NOT a real embedding model -- it produces a fixed-dimension
/// vector based on simple text statistics. We use it here to demonstrate
/// the pipeline mechanics without requiring an API key.
fn mock_embed(text: &str, dimension: usize) -> Vec<f32> {
    let mut embedding = vec![0.0f32; dimension];
    let words: Vec<&str> = text.split_whitespace().collect();

    for (i, word) in words.iter().enumerate() {
        // Hash each word to a dimension index
        let hash: usize = word
            .bytes()
            .fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(b as usize));
        let idx = hash % dimension;
        embedding[idx] += 1.0;
    }

    // L2-normalize the vector
    let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for val in &mut embedding {
            *val /= magnitude;
        }
    }

    embedding
}
```

## Putting It All Together

Here is a complete working example that ingests documents, retrieves
relevant chunks, and builds a prompt:

```rust
fn main() {
    let dimension = 64;
    let mut store = VectorStore::new(dimension);

    // --- Stage 1: Ingestion ---

    let documents = vec![
        ("doc1", "Rust is a systems programming language focused on safety, \
                  speed, and concurrency. It achieves memory safety without \
                  garbage collection through its ownership system."),
        ("doc2", "RAG combines retrieval with generation. Documents are \
                  chunked, embedded, and stored in a vector database. At \
                  query time, relevant chunks are retrieved and passed to \
                  an LLM."),
        ("doc3", "Vector similarity search finds the nearest neighbors \
                  in embedding space. Cosine similarity measures the angle \
                  between vectors, making it magnitude-invariant."),
    ];

    for (id, content) in &documents {
        let chunks = chunk_text(content, 20, 5);
        for (i, chunk_text) in chunks.iter().enumerate() {
            let embedding = mock_embed(chunk_text, dimension);
            store.insert(Chunk {
                id: format!("{}-chunk-{}", id, i),
                text: chunk_text.clone(),
                embedding,
                metadata: HashMap::from([
                    ("source".into(), id.to_string()),
                ]),
            });
        }
    }

    println!("Ingested {} chunks from {} documents", store.len(), documents.len());

    // --- Stage 2: Retrieval ---

    let query = "How does vector similarity search work?";
    let query_embedding = mock_embed(query, dimension);
    let results = store.search(&query_embedding, 3);

    println!("\nQuery: {}\n", query);
    for (i, result) in results.iter().enumerate() {
        println!(
            "  [{}] score={:.4} source={} \n      {}",
            i + 1,
            result.score,
            result.chunk.metadata.get("source").unwrap_or(&"?".into()),
            result.chunk.text
        );
    }

    // --- Stage 3: Generation (prompt only -- no LLM call) ---

    let context: String = results
        .iter()
        .enumerate()
        .map(|(i, r)| format!("[Source {}] {}", i + 1, r.chunk.text))
        .collect::<Vec<_>>()
        .join("\n\n");

    let prompt = format!(
        "Answer the question using ONLY the context below.\n\n\
         ## Context\n\n{context}\n\n\
         ## Question\n\n{query}\n\n\
         ## Answer\n"
    );

    println!("\n--- Generated Prompt ---\n{}", prompt);
}
```

## What This Gets Right

Our naive implementation correctly demonstrates:

- **Semantic retrieval**: Chunks about vector similarity score higher for a
  query about vector similarity
- **Decoupled stages**: Ingestion and retrieval are independent
- **Metadata preservation**: Source attribution flows through the pipeline
- **Prompt engineering**: The generated prompt instructs the LLM to ground
  its answer

## What This Gets Wrong

This naive approach has serious limitations that motivate the rest of Part I:

| Limitation | Consequence | Covered In |
|-----------|-------------|-----------|
| Mock embeddings | No real semantic understanding | Chapter 2 |
| Word-boundary chunking | Breaks mid-sentence, loses structure | Chapter 2 |
| Brute-force search | O(n*d) does not scale | Chapter 3 |
| No hybrid search | Misses keyword matches | Chapter 3 |
| No reranking | Top-k ordering may be suboptimal | Chapter 3 |
| No graph awareness | Cannot follow relationships | Part II |

## Exercises

1. **Extend the similarity function**: Implement dot product and Euclidean
   distance. Compare results with cosine similarity on the same query.

2. **Add metadata filtering**: Modify `VectorStore::search` to accept an
   optional metadata filter (e.g., only search chunks from a specific
   source document).

3. **Measure performance**: Generate 10,000 random chunks and measure
   search latency. How does it scale compared to 1,000 chunks?

## Summary

We built a complete naive RAG pipeline in Rust: chunking, embedding
(mocked), storage, retrieval, and prompt construction. Every production
RAG system does exactly these steps -- the difference is in the quality
of each component. In Chapter 2 we replace our mock embeddings with real
models and our word-boundary chunker with sophisticated strategies.

---

*Next: [Chapter 2: Embeddings and Chunking](ch02-embeddings-chunking.md)*
