//! Storage trait definitions.
//!
//! # Implements
//!
//! This module defines the core storage abstractions:
//!
//! - [`KVStorage`] (FEAT0010): Document and metadata storage
//! - [`VectorStorage`] (FEAT0201): Embedding similarity search
//! - [`GraphStorage`] (FEAT0202-0204): Entity/relationship graph
//! - [`WorkspaceVectorRegistry`] (FEAT0350): Per-workspace vector isolation
//!
//! # Enforces
//!
//! - **BR0201**: All traits support namespace-based tenant isolation
//! - **BR0008**: GraphStorage normalizes entity names on write
//! - **BR0350**: Each workspace has isolated vector storage
//!
//! # WHY: Trait-Based Abstraction
//!
//! Using traits instead of concrete types enables:
//! - **Testing**: Mock implementations for unit tests
//! - **Flexibility**: Multiple backend support (Postgres, Memory, SurrealDB)
//! - **Modularity**: Storage can be swapped without changing business logic

mod bm25;
mod graph;
mod kv;
mod vector;
mod workspace_vector;

pub use bm25::{Bm25Document, Bm25SearchResult, Bm25Storage};
pub use graph::{GraphEdge, GraphNode, GraphStorage, KnowledgeGraph};
pub use kv::KVStorage;
pub use vector::{VectorSearchResult, VectorStorage};
pub use workspace_vector::{WorkspaceVectorConfig, WorkspaceVectorRegistry};
