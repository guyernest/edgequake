::: exercise
id: naive-vector-search
difficulty: beginner
time: 20 minutes
:::

# Naive Vector Search

Every RAG system starts with the same fundamental operation: given a query
vector, find the most similar vectors in a corpus. In this exercise you will
implement cosine similarity from scratch and use it to build a brute-force
top-k search over an in-memory collection of embeddings.

This is the building block that Chapter 3 will extend with BM25 and hybrid
ranking, so take the time to understand the math before moving on.

::: objectives
thinking:
  - Understand why cosine similarity measures orientation rather than magnitude
  - Reason about the time complexity of brute-force search (O(n * d))
doing:
  - Implement the cosine similarity formula using Rust iterators
  - Build a top-k search that returns the most similar documents
  - Handle edge cases like zero-magnitude vectors
:::

::: discussion
- Why does cosine similarity ignore vector magnitude? When is that a feature, and when is it a limitation?
- At what corpus size does brute-force search become impractical? What alternatives exist?
- How does the choice of embedding model affect the similarity scores you compute?
:::

::: starter file="src/main.rs"
```rust
use std::collections::BinaryHeap;
use std::cmp::Ordering;

/// A document with its precomputed embedding vector.
#[derive(Debug, Clone)]
struct Document {
    id: String,
    text: String,
    embedding: Vec<f64>,
}

/// A search result with a similarity score.
#[derive(Debug, Clone)]
struct SearchResult {
    id: String,
    score: f64,
}

impl PartialEq for SearchResult {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score
    }
}

impl Eq for SearchResult {}

impl PartialOrd for SearchResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SearchResult {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap behavior (we want top-k highest)
        other.score.partial_cmp(&self.score).unwrap_or(Ordering::Equal)
    }
}

/// A naive vector search engine that stores documents in memory
/// and retrieves the most similar ones via cosine similarity.
struct VectorStore {
    documents: Vec<Document>,
}

impl VectorStore {
    fn new() -> Self {
        VectorStore {
            documents: Vec::new(),
        }
    }

    fn add_document(&mut self, doc: Document) {
        self.documents.push(doc);
    }

    /// Compute the cosine similarity between two vectors.
    ///
    /// Formula: dot(a, b) / (||a|| * ||b||)
    ///
    /// Returns 0.0 if either vector has zero magnitude.
    fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
        // TODO: Implement cosine similarity
        // 1. Compute the dot product of a and b
        // 2. Compute the magnitude (L2 norm) of a
        // 3. Compute the magnitude (L2 norm) of b
        // 4. Return dot / (mag_a * mag_b), or 0.0 if either magnitude is 0
        todo!("Implement cosine_similarity")
    }

    /// Search for the top-k most similar documents to the query vector.
    ///
    /// Returns results sorted by similarity score in descending order.
    fn search(&self, query: &[f64], top_k: usize) -> Vec<SearchResult> {
        // TODO: Implement top-k search
        // 1. Compute cosine similarity between query and each document
        // 2. Collect all (id, score) pairs
        // 3. Sort by score descending and take the top k
        //
        // Hint: You can use a simple Vec + sort, or try BinaryHeap for efficiency.
        todo!("Implement search")
    }
}

fn main() {
    let mut store = VectorStore::new();

    store.add_document(Document {
        id: "doc1".into(),
        text: "Rust is a systems programming language".into(),
        embedding: vec![0.1, 0.9, 0.2, 0.0],
    });
    store.add_document(Document {
        id: "doc2".into(),
        text: "Python is popular for machine learning".into(),
        embedding: vec![0.8, 0.1, 0.7, 0.3],
    });
    store.add_document(Document {
        id: "doc3".into(),
        text: "Rust has excellent memory safety".into(),
        embedding: vec![0.2, 0.8, 0.3, 0.1],
    });

    let query = vec![0.15, 0.85, 0.25, 0.05];
    let results = store.search(&query, 2);

    println!("Top 2 results for query:");
    for result in &results {
        println!("  {} (score: {:.4})", result.id, result.score);
    }
}
```
:::

::: hint level=1 title="Cosine similarity step by step"
The dot product is the sum of element-wise products. Use Rust's `iter().zip()`:

```rust
let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
```

The magnitude (L2 norm) is the square root of the sum of squares:

```rust
let mag: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
```
:::

::: hint level=2 title="Putting search together"
For a simple approach, compute all scores, collect into a Vec, sort, and truncate:

```rust
let mut results: Vec<SearchResult> = self.documents.iter()
    .map(|doc| SearchResult {
        id: doc.id.clone(),
        score: Self::cosine_similarity(query, &doc.embedding),
    })
    .collect();

results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
results.truncate(top_k);
```
:::

::: solution
```rust
use std::collections::BinaryHeap;
use std::cmp::Ordering;

#[derive(Debug, Clone)]
struct Document {
    id: String,
    text: String,
    embedding: Vec<f64>,
}

#[derive(Debug, Clone)]
struct SearchResult {
    id: String,
    score: f64,
}

impl PartialEq for SearchResult {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score
    }
}

impl Eq for SearchResult {}

impl PartialOrd for SearchResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SearchResult {
    fn cmp(&self, other: &Self) -> Ordering {
        other.score.partial_cmp(&self.score).unwrap_or(Ordering::Equal)
    }
}

struct VectorStore {
    documents: Vec<Document>,
}

impl VectorStore {
    fn new() -> Self {
        VectorStore {
            documents: Vec::new(),
        }
    }

    fn add_document(&mut self, doc: Document) {
        self.documents.push(doc);
    }

    fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
        let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let mag_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
        let mag_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();

        if mag_a == 0.0 || mag_b == 0.0 {
            return 0.0;
        }

        dot / (mag_a * mag_b)
    }

    fn search(&self, query: &[f64], top_k: usize) -> Vec<SearchResult> {
        let mut results: Vec<SearchResult> = self
            .documents
            .iter()
            .map(|doc| SearchResult {
                id: doc.id.clone(),
                score: Self::cosine_similarity(query, &doc.embedding),
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(Ordering::Equal)
        });
        results.truncate(top_k);
        results
    }
}

fn main() {
    let mut store = VectorStore::new();

    store.add_document(Document {
        id: "doc1".into(),
        text: "Rust is a systems programming language".into(),
        embedding: vec![0.1, 0.9, 0.2, 0.0],
    });
    store.add_document(Document {
        id: "doc2".into(),
        text: "Python is popular for machine learning".into(),
        embedding: vec![0.8, 0.1, 0.7, 0.3],
    });
    store.add_document(Document {
        id: "doc3".into(),
        text: "Rust has excellent memory safety".into(),
        embedding: vec![0.2, 0.8, 0.3, 0.1],
    });

    let query = vec![0.15, 0.85, 0.25, 0.05];
    let results = store.search(&query, 2);

    println!("Top 2 results for query:");
    for result in &results {
        println!("  {} (score: {:.4})", result.id, result.score);
    }
}
```

### Explanation

The `cosine_similarity` function computes three values: the dot product of the
two vectors (sum of element-wise products), and the L2 norms (magnitudes) of
each vector. Dividing the dot product by the product of the magnitudes yields
the cosine of the angle between the vectors, which ranges from -1 (opposite)
to 1 (identical direction). The zero-magnitude guard prevents division by zero
for degenerate inputs.

The `search` function applies cosine similarity to every document, collects
the results, sorts them in descending score order, and truncates to the
requested `top_k`. This brute-force approach is O(n * d) where n is the number
of documents and d is the embedding dimension -- simple and correct, but too
slow for large corpora where approximate nearest neighbor indexes (like HNSW)
are needed.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = VectorStore::cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-10, "Identical vectors should have similarity 1.0");
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = VectorStore::cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-10, "Orthogonal vectors should have similarity 0.0");
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = VectorStore::cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-10, "Opposite vectors should have similarity -1.0");
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let zero = vec![0.0, 0.0, 0.0];
        let sim = VectorStore::cosine_similarity(&a, &zero);
        assert_eq!(sim, 0.0, "Zero vector should return similarity 0.0");
    }

    #[test]
    fn test_cosine_similarity_magnitude_invariance() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![2.0, 4.0, 6.0]; // Same direction, 2x magnitude
        let sim = VectorStore::cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-10, "Parallel vectors should have similarity 1.0 regardless of magnitude");
    }

    #[test]
    fn test_search_returns_correct_top_k() {
        let mut store = VectorStore::new();
        store.add_document(Document {
            id: "a".into(), text: "".into(), embedding: vec![1.0, 0.0],
        });
        store.add_document(Document {
            id: "b".into(), text: "".into(), embedding: vec![0.0, 1.0],
        });
        store.add_document(Document {
            id: "c".into(), text: "".into(), embedding: vec![0.7, 0.7],
        });

        let results = store.search(&[1.0, 0.0], 2);
        assert_eq!(results.len(), 2, "Should return exactly 2 results");
        assert_eq!(results[0].id, "a", "Most similar should be 'a' (identical direction)");
    }

    #[test]
    fn test_search_descending_order() {
        let mut store = VectorStore::new();
        store.add_document(Document {
            id: "low".into(), text: "".into(), embedding: vec![0.0, 1.0],
        });
        store.add_document(Document {
            id: "high".into(), text: "".into(), embedding: vec![1.0, 0.0],
        });
        store.add_document(Document {
            id: "mid".into(), text: "".into(), embedding: vec![0.5, 0.5],
        });

        let results = store.search(&[1.0, 0.0], 3);
        assert!(results[0].score >= results[1].score, "Results should be in descending score order");
        assert!(results[1].score >= results[2].score, "Results should be in descending score order");
    }

    #[test]
    fn test_search_empty_store() {
        let store = VectorStore::new();
        let results = store.search(&[1.0, 0.0], 5);
        assert!(results.is_empty(), "Empty store should return no results");
    }
}
```
:::

::: reflection
- If you had a million documents, how would you make this search faster without sacrificing too much accuracy?
- Cosine similarity treats all dimensions equally. When might you want to weight certain dimensions more than others?
- How does the quality of the embedding model affect the usefulness of cosine similarity scores?
:::
