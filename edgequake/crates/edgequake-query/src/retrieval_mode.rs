//! Retrieval mode for query-time ranking strategy.
//!
//! # Implements
//!
//! - **RET-02**: BM25 keyword retrieval mode
//! - **RET-03**: Hybrid retrieval mode (RRF fusion)
//!
//! # WHY: RetrievalMode vs QueryMode
//!
//! `QueryMode` controls the LightRAG graph retrieval strategy (Naive, Local,
//! Global, Hybrid, Mix). `RetrievalMode` controls the *ranking* strategy:
//! - Vector: cosine similarity ranking (existing behavior)
//! - Bm25: BM25 keyword ranking via Athena
//! - Hybrid: fuse BM25 + vector rankings via Reciprocal Rank Fusion
//!
//! These are orthogonal axes: you can combine QueryMode::Local with
//! RetrievalMode::Hybrid, for example.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Retrieval mode for query-time ranking strategy.
///
/// Controls whether BM25 keyword search, vector semantic search,
/// or a hybrid combination (via RRF) is used for document retrieval.
///
/// # Default
///
/// `Vector` is the default for backward compatibility. BM25 and Hybrid
/// are opt-in via the `retrieval_mode` field on `QueryRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RetrievalMode {
    /// Vector similarity search only (default, backward compatible).
    #[default]
    Vector,

    /// BM25 keyword search only via Athena.
    Bm25,

    /// Hybrid: fuse BM25 and vector results using Reciprocal Rank Fusion.
    Hybrid,
}

impl RetrievalMode {
    /// Get the mode name as a string slice.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Vector => "vector",
            Self::Bm25 => "bm25",
            Self::Hybrid => "hybrid",
        }
    }

    /// Whether this mode uses BM25 keyword search.
    ///
    /// True for `Bm25` and `Hybrid`.
    pub fn uses_bm25(&self) -> bool {
        matches!(self, Self::Bm25 | Self::Hybrid)
    }

    /// Whether this mode uses vector similarity search.
    ///
    /// True for `Vector` and `Hybrid`.
    pub fn uses_vector(&self) -> bool {
        matches!(self, Self::Vector | Self::Hybrid)
    }
}

impl FromStr for RetrievalMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "vector" => Ok(Self::Vector),
            "bm25" => Ok(Self::Bm25),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(format!("Unknown retrieval mode: {}", other)),
        }
    }
}

impl fmt::Display for RetrievalMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_vector() {
        assert_eq!(RetrievalMode::default(), RetrievalMode::Vector);
    }

    #[test]
    fn test_from_str() {
        assert_eq!("vector".parse::<RetrievalMode>().unwrap(), RetrievalMode::Vector);
        assert_eq!("bm25".parse::<RetrievalMode>().unwrap(), RetrievalMode::Bm25);
        assert_eq!("hybrid".parse::<RetrievalMode>().unwrap(), RetrievalMode::Hybrid);
        assert_eq!("HYBRID".parse::<RetrievalMode>().unwrap(), RetrievalMode::Hybrid);
        assert_eq!("BM25".parse::<RetrievalMode>().unwrap(), RetrievalMode::Bm25);
        assert!("unknown".parse::<RetrievalMode>().is_err());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", RetrievalMode::Vector), "vector");
        assert_eq!(format!("{}", RetrievalMode::Bm25), "bm25");
        assert_eq!(format!("{}", RetrievalMode::Hybrid), "hybrid");
    }

    #[test]
    fn test_as_str() {
        assert_eq!(RetrievalMode::Vector.as_str(), "vector");
        assert_eq!(RetrievalMode::Bm25.as_str(), "bm25");
        assert_eq!(RetrievalMode::Hybrid.as_str(), "hybrid");
    }

    #[test]
    fn test_uses_bm25() {
        assert!(!RetrievalMode::Vector.uses_bm25());
        assert!(RetrievalMode::Bm25.uses_bm25());
        assert!(RetrievalMode::Hybrid.uses_bm25());
    }

    #[test]
    fn test_uses_vector() {
        assert!(RetrievalMode::Vector.uses_vector());
        assert!(!RetrievalMode::Bm25.uses_vector());
        assert!(RetrievalMode::Hybrid.uses_vector());
    }

    #[test]
    fn test_serde_roundtrip() {
        let mode = RetrievalMode::Hybrid;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"hybrid\"");
        let deserialized: RetrievalMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, mode);
    }

    #[test]
    fn test_serde_all_variants() {
        for (variant, expected) in [
            (RetrievalMode::Vector, "\"vector\""),
            (RetrievalMode::Bm25, "\"bm25\""),
            (RetrievalMode::Hybrid, "\"hybrid\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
        }
    }
}
