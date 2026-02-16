//! Basic example of using Neptune graph storage.
//!
//! This example demonstrates:
//! - Connecting to Amazon Neptune
//! - Creating nodes and edges
//! - Traversing the graph
//! - Searching for nodes
//!
//! # Setup
//!
//! 1. Deploy a Neptune cluster in AWS (or use Neptune Serverless)
//! 2. Ensure your EC2 instance or Lambda has network access to Neptune
//! 3. Set the Neptune endpoint:
//!    ```bash
//!    export NEPTUNE_ENDPOINT=my-cluster.cluster-xxx.region.neptune.amazonaws.com:8182
//!    ```
//!
//! 4. Run the example:
//!    ```bash
//!    cargo run --example neptune_graph_basic --features neptune
//!    ```

use edgequake_storage::GraphStorage;
use edgequake_storage_aws::{NeptuneConfig, NeptuneGraphStorage};
use serde_json::json;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("🌊 EdgeQuake Neptune Graph Storage Example\n");

    // Get Neptune endpoint from environment
    let endpoint = std::env::var("NEPTUNE_ENDPOINT").unwrap_or_else(|_| {
        eprintln!("❌ NEPTUNE_ENDPOINT not set!");
        eprintln!("   Set it to your Neptune cluster endpoint:");
        eprintln!("   export NEPTUNE_ENDPOINT=cluster.region.neptune.amazonaws.com:8182");
        std::process::exit(1);
    });

    println!("🔌 Connecting to Neptune: {}", endpoint);

    // Create configuration
    let config = NeptuneConfig::new(endpoint)
        .with_namespace("example-workspace")
        .with_iam_auth(true)
        .with_timeout(30);

    println!("⚙️  Configuration:");
    println!("   - Namespace: {}", config.namespace);
    println!("   - IAM Auth: {}", config.use_iam_auth);
    println!("   - Timeout: {}s", config.timeout_secs);
    println!();

    // Create storage
    println!("🔧 Creating Neptune graph storage...");
    let storage = NeptuneGraphStorage::new(config).await?;

    // Initialize
    println!("🔄 Initializing storage...");
    storage.initialize().await?;

    // Check current state
    let node_count = storage.node_count().await?;
    let edge_count = storage.edge_count().await?;
    println!(
        "📊 Current state: {} nodes, {} edges",
        node_count, edge_count
    );
    println!();

    // Optional: Clear existing data
    if std::env::var("CLEAR_DATA").is_ok() {
        println!("🗑️  Clearing existing data...");
        storage.clear().await?;
    }

    // Create a knowledge graph about programming languages
    println!("📝 Creating knowledge graph...");

    // Add language nodes
    let languages = vec![
        ("rust", "Rust", "Systems programming language"),
        ("python", "Python", "High-level programming language"),
        ("javascript", "JavaScript", "Web programming language"),
    ];

    for (id, name, desc) in languages {
        let mut props = HashMap::new();
        props.insert("name".to_string(), json!(name));
        props.insert("description".to_string(), json!(desc));
        props.insert("type".to_string(), json!("language"));

        storage.upsert_node(id, props).await?;
        println!("   ✓ Added node: {}", id);
    }

    // Add concept nodes
    let concepts = vec![
        ("memory-safety", "Memory Safety", "concept"),
        ("web-dev", "Web Development", "concept"),
        ("data-science", "Data Science", "concept"),
    ];

    for (id, name, category) in concepts {
        let mut props = HashMap::new();
        props.insert("name".to_string(), json!(name));
        props.insert("category".to_string(), json!(category));

        storage.upsert_node(id, props).await?;
        println!("   ✓ Added node: {}", id);
    }

    // Add relationships
    println!("\n🔗 Creating relationships...");

    let edges = vec![
        ("rust", "memory-safety", "provides"),
        ("python", "data-science", "used_for"),
        ("javascript", "web-dev", "used_for"),
        ("rust", "web-dev", "used_for"),
    ];

    for (source, target, rel_type) in edges {
        let mut props = HashMap::new();
        props.insert("type".to_string(), json!(rel_type));

        storage.upsert_edge(source, target, props).await?;
        println!("   ✓ {} -> {}", source, target);
    }

    println!("\n📊 Final state:");
    println!("   Nodes: {}", storage.node_count().await?);
    println!("   Edges: {}", storage.edge_count().await?);
    println!();

    // Query the graph
    println!("🔍 Querying the graph...\n");

    // Get neighbors of Rust
    println!("1. What concepts is Rust related to?");
    let neighbors = storage.get_neighbors("rust", 1).await?;
    println!("   Rust is connected to:");
    for neighbor in neighbors {
        let name = neighbor
            .properties
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&neighbor.id);
        println!("   - {}", name);
    }
    println!();

    // Get node degree
    println!("2. How connected is each language?");
    for (id, name, _) in &languages {
        let degree = storage.node_degree(id).await?;
        println!("   {} has {} connections", name, degree);
    }
    println!();

    // Extract knowledge graph
    println!("3. Knowledge graph starting from 'rust' (depth 2):");
    let graph = storage.get_knowledge_graph("rust", 2, 20).await?;
    println!(
        "   Found {} nodes and {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    if !graph.nodes.is_empty() {
        println!("   Nodes:");
        for node in &graph.nodes {
            let name = node
                .properties
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&node.id);
            println!("   - {}", name);
        }
    }

    if !graph.edges.is_empty() {
        println!("   Edges:");
        for edge in &graph.edges {
            println!("   - {} -> {}", edge.source, edge.target);
        }
    }
    println!();

    // Search nodes
    println!("4. Search for 'web' related nodes:");
    let results = storage.search_nodes("web", 10, None, None, None).await?;
    println!("   Found {} results:", results.len());
    for (node, degree) in results {
        let name = node
            .properties
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&node.id);
        println!("   - {} (degree: {})", name, degree);
    }
    println!();

    // Get all nodes of a specific type
    println!("5. Get all language nodes:");
    let all_nodes = storage.get_all_nodes().await?;
    let languages: Vec<_> = all_nodes
        .iter()
        .filter(|n| n.properties.get("type").and_then(|v| v.as_str()) == Some("language"))
        .collect();

    println!("   Found {} languages:", languages.len());
    for node in languages {
        let name = node
            .properties
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&node.id);
        let desc = node
            .properties
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        println!("   - {}: {}", name, desc);
    }
    println!();

    // Finalize
    println!("💾 Finalizing storage...");
    storage.finalize().await?;

    println!("\n✨ Example completed successfully!");
    println!("🔄 Run this example again to see persistence.");
    println!("\n💡 Tips:");
    println!("   - Set CLEAR_DATA=1 to clear data before running");
    println!("   - View your data in Neptune Console or Neptune Workbench");
    println!("   - Use Gremlin queries for advanced graph operations");

    Ok(())
}
