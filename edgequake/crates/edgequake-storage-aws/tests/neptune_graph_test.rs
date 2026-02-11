//! Integration tests for Neptune graph storage.
//!
//! Note: These tests require a running Neptune cluster.
//! Set NEPTUNE_ENDPOINT environment variable to run tests.
//!
//! Example:
//! ```bash
//! export NEPTUNE_ENDPOINT=my-cluster.cluster-xxx.us-east-1.neptune.amazonaws.com:8182
//! cargo test --features neptune -- --test-threads=1
//! ```

use edgequake_storage::GraphStorage;
use edgequake_storage_aws::{NeptuneConfig, NeptuneGraphStorage};
use serde_json::json;
use std::collections::HashMap;

/// Helper to check if we should run Neptune tests.
fn should_run_neptune_tests() -> bool {
    std::env::var("NEPTUNE_ENDPOINT").is_ok()
}

/// Get Neptune endpoint from environment.
fn get_neptune_endpoint() -> String {
    std::env::var("NEPTUNE_ENDPOINT")
        .unwrap_or_else(|_| "localhost:8182".to_string())
}

#[tokio::test]
async fn test_neptune_initialization() {
    if !should_run_neptune_tests() {
        println!("Skipping Neptune test - set NEPTUNE_ENDPOINT to run");
        return;
    }

    let config = NeptuneConfig::new(get_neptune_endpoint())
        .with_namespace("test-init");

    let storage = NeptuneGraphStorage::new(config).await.unwrap();
    let result = storage.initialize().await;

    assert!(result.is_ok(), "Failed to initialize: {:?}", result);
}

#[tokio::test]
async fn test_neptune_node_operations() {
    if !should_run_neptune_tests() {
        println!("Skipping Neptune test - set NEPTUNE_ENDPOINT to run");
        return;
    }

    let config = NeptuneConfig::new(get_neptune_endpoint())
        .with_namespace("test-nodes");

    let storage = NeptuneGraphStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();

    // Clear any existing data
    storage.clear().await.unwrap();

    // Check empty
    assert_eq!(storage.node_count().await.unwrap(), 0);

    // Upsert a node
    let mut props = HashMap::new();
    props.insert("name".to_string(), json!("Alice"));
    props.insert("type".to_string(), json!("person"));

    storage.upsert_node("user1", props.clone()).await.unwrap();

    // Check node exists
    assert!(storage.has_node("user1").await.unwrap());

    // Get node
    let node = storage.get_node("user1").await.unwrap();
    assert!(node.is_some());
    let node = node.unwrap();
    assert_eq!(node.id, "user1");
    assert_eq!(node.properties.get("name"), Some(&json!("Alice")));

    // Update node
    props.insert("age".to_string(), json!(30));
    storage.upsert_node("user1", props).await.unwrap();

    let node = storage.get_node("user1").await.unwrap().unwrap();
    assert_eq!(node.properties.get("age"), Some(&json!(30)));

    // Delete node
    storage.delete_node("user1").await.unwrap();
    assert!(!storage.has_node("user1").await.unwrap());

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_neptune_edge_operations() {
    if !should_run_neptune_tests() {
        println!("Skipping Neptune test - set NEPTUNE_ENDPOINT to run");
        return;
    }

    let config = NeptuneConfig::new(get_neptune_endpoint())
        .with_namespace("test-edges");

    let storage = NeptuneGraphStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Create nodes
    let props1 = HashMap::from([
        ("name".to_string(), json!("Alice")),
    ]);
    let props2 = HashMap::from([
        ("name".to_string(), json!("Bob")),
    ]);

    storage.upsert_node("user1", props1).await.unwrap();
    storage.upsert_node("user2", props2).await.unwrap();

    // Create edge
    let edge_props = HashMap::from([
        ("relationship".to_string(), json!("friend")),
        ("since".to_string(), json!(2020)),
    ]);

    storage.upsert_edge("user1", "user2", edge_props).await.unwrap();

    // Check edge exists
    assert!(storage.has_edge("user1", "user2").await.unwrap());

    // Get edge
    let edge = storage.get_edge("user1", "user2").await.unwrap();
    assert!(edge.is_some());
    let edge = edge.unwrap();
    assert_eq!(edge.source, "user1");
    assert_eq!(edge.target, "user2");
    assert_eq!(edge.properties.get("relationship"), Some(&json!("friend")));

    // Delete edge
    storage.delete_edge("user1", "user2").await.unwrap();
    assert!(!storage.has_edge("user1", "user2").await.unwrap());

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_neptune_graph_traversal() {
    if !should_run_neptune_tests() {
        println!("Skipping Neptune test - set NEPTUNE_ENDPOINT to run");
        return;
    }

    let config = NeptuneConfig::new(get_neptune_endpoint())
        .with_namespace("test-traversal");

    let storage = NeptuneGraphStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Create a small graph: A -> B -> C
    storage.upsert_node("A", HashMap::from([("label".to_string(), json!("Start"))])).await.unwrap();
    storage.upsert_node("B", HashMap::from([("label".to_string(), json!("Middle"))])).await.unwrap();
    storage.upsert_node("C", HashMap::from([("label".to_string(), json!("End"))])).await.unwrap();

    storage.upsert_edge("A", "B", HashMap::new()).await.unwrap();
    storage.upsert_edge("B", "C", HashMap::new()).await.unwrap();

    // Get neighbors of A (depth 1)
    let neighbors = storage.get_neighbors("A", 1).await.unwrap();
    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors[0].id, "B");

    // Get neighbors of A (depth 2)
    let neighbors = storage.get_neighbors("A", 2).await.unwrap();
    assert_eq!(neighbors.len(), 2);
    let ids: Vec<_> = neighbors.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"B"));
    assert!(ids.contains(&"C"));

    // Get knowledge graph
    let graph = storage.get_knowledge_graph("A", 2, 10).await.unwrap();
    assert!(graph.node_count() >= 2);
    assert!(graph.edge_count() >= 1);

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_neptune_batch_operations() {
    if !should_run_neptune_tests() {
        println!("Skipping Neptune test - set NEPTUNE_ENDPOINT to run");
        return;
    }

    let config = NeptuneConfig::new(get_neptune_endpoint())
        .with_namespace("test-batch");

    let storage = NeptuneGraphStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Batch upsert nodes
    let nodes = vec![
        ("n1".to_string(), HashMap::from([("index".to_string(), json!(1))])),
        ("n2".to_string(), HashMap::from([("index".to_string(), json!(2))])),
        ("n3".to_string(), HashMap::from([("index".to_string(), json!(3))])),
    ];

    storage.upsert_nodes_batch(&nodes).await.unwrap();

    // Verify all nodes exist
    assert_eq!(storage.node_count().await.unwrap(), 3);

    // Batch upsert edges
    let edges = vec![
        ("n1".to_string(), "n2".to_string(), HashMap::new()),
        ("n2".to_string(), "n3".to_string(), HashMap::new()),
    ];

    storage.upsert_edges_batch(&edges).await.unwrap();

    // Verify edges exist
    assert_eq!(storage.edge_count().await.unwrap(), 2);

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_neptune_node_degree() {
    if !should_run_neptune_tests() {
        println!("Skipping Neptune test - set NEPTUNE_ENDPOINT to run");
        return;
    }

    let config = NeptuneConfig::new(get_neptune_endpoint())
        .with_namespace("test-degree");

    let storage = NeptuneGraphStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Create hub node with multiple connections
    storage.upsert_node("hub", HashMap::new()).await.unwrap();
    storage.upsert_node("leaf1", HashMap::new()).await.unwrap();
    storage.upsert_node("leaf2", HashMap::new()).await.unwrap();
    storage.upsert_node("leaf3", HashMap::new()).await.unwrap();

    storage.upsert_edge("hub", "leaf1", HashMap::new()).await.unwrap();
    storage.upsert_edge("hub", "leaf2", HashMap::new()).await.unwrap();
    storage.upsert_edge("hub", "leaf3", HashMap::new()).await.unwrap();

    // Check degree
    let degree = storage.node_degree("hub").await.unwrap();
    assert_eq!(degree, 3);

    let degree = storage.node_degree("leaf1").await.unwrap();
    assert_eq!(degree, 1);

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_neptune_search() {
    if !should_run_neptune_tests() {
        println!("Skipping Neptune test - set NEPTUNE_ENDPOINT to run");
        return;
    }

    let config = NeptuneConfig::new(get_neptune_endpoint())
        .with_namespace("test-search");

    let storage = NeptuneGraphStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Create nodes with descriptions
    storage.upsert_node("doc1", HashMap::from([
        ("description".to_string(), json!("Rust programming language")),
    ])).await.unwrap();

    storage.upsert_node("doc2", HashMap::from([
        ("description".to_string(), json!("Python programming guide")),
    ])).await.unwrap();

    storage.upsert_node("doc3", HashMap::from([
        ("description".to_string(), json!("JavaScript framework")),
    ])).await.unwrap();

    // Search for "programming"
    let results = storage.search_nodes("programming", 10, None, None, None).await.unwrap();

    // Should find doc1 and doc2
    assert!(results.len() >= 2);

    // Clean up
    storage.clear().await.unwrap();
}
