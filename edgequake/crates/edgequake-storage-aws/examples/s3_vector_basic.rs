//! Basic example of using S3 vector storage.
//!
//! This example demonstrates:
//! - Creating an S3 vector storage
//! - Inserting vectors with metadata
//! - Querying similar vectors
//! - Persistence across sessions
//!
//! # Setup
//!
//! 1. Set AWS credentials:
//!    ```bash
//!    export AWS_PROFILE=your-profile
//!    # OR
//!    export AWS_ACCESS_KEY_ID=your-key
//!    export AWS_SECRET_ACCESS_KEY=your-secret
//!    ```
//!
//! 2. Set S3 bucket (or it will use "edgequake-vectors"):
//!    ```bash
//!    export S3_BUCKET=your-bucket-name
//!    ```
//!
//! 3. Run the example:
//!    ```bash
//!    cargo run --example s3_vector_basic --features s3-vectors
//!    ```

use edgequake_storage::VectorStorage;
use edgequake_storage_aws::{S3Config, S3VectorStorage};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("🚀 EdgeQuake S3 Vector Storage Example\n");

    // Get bucket from environment or use default
    let bucket = std::env::var("S3_BUCKET").unwrap_or_else(|_| "edgequake-vectors".to_string());
    println!("📦 Using S3 bucket: {}", bucket);

    // Create configuration
    let config = S3Config::new(bucket, "example-workspace", 384)
        .with_hnsw_params(16, 200, 50) // m=16, ef_construction=200, ef_search=50
        .with_batch_size(1000)
        .with_compression(true);

    println!("⚙️  Configuration:");
    println!("   - Namespace: {}", config.namespace);
    println!("   - Dimension: {}", config.dimension);
    println!("   - HNSW M: {}", config.hnsw_m);
    println!("   - Batch size: {}", config.batch_size);
    println!();

    // Create storage
    println!("🔧 Creating S3 vector storage...");
    let storage = S3VectorStorage::new(config).await?;

    // Initialize (loads index from S3 if exists)
    println!("🔄 Initializing storage...");
    storage.initialize().await?;

    // Check if storage is empty
    let count = storage.count().await?;
    println!("📊 Current vector count: {}", count);
    println!();

    // Insert some example vectors
    println!("📝 Inserting sample vectors...");

    let sample_data = vec![
        (
            "doc-001".to_string(),
            generate_sample_vector(384, 0.1),
            json!({
                "title": "Introduction to Rust",
                "category": "programming",
                "author": "rustacean"
            }),
        ),
        (
            "doc-002".to_string(),
            generate_sample_vector(384, 0.2),
            json!({
                "title": "Advanced Rust Patterns",
                "category": "programming",
                "author": "rustacean"
            }),
        ),
        (
            "doc-003".to_string(),
            generate_sample_vector(384, 0.5),
            json!({
                "title": "AWS Cloud Architecture",
                "category": "cloud",
                "author": "cloud_expert"
            }),
        ),
        (
            "doc-004".to_string(),
            generate_sample_vector(384, 0.6),
            json!({
                "title": "Serverless Computing",
                "category": "cloud",
                "author": "cloud_expert"
            }),
        ),
        (
            "doc-005".to_string(),
            generate_sample_vector(384, 0.9),
            json!({
                "title": "Machine Learning Basics",
                "category": "ml",
                "author": "ml_researcher"
            }),
        ),
    ];

    storage.upsert(&sample_data).await?;
    println!("✅ Inserted {} vectors", sample_data.len());

    let new_count = storage.count().await?;
    println!("📊 New vector count: {}", new_count);
    println!();

    // Query similar vectors
    println!("🔍 Querying similar vectors...");
    println!("   Query: Similar to 'programming' documents");

    let query_vector = generate_sample_vector(384, 0.15); // Close to doc-001 and doc-002
    let results = storage.query(&query_vector, 3, None).await?;

    println!("\n📋 Top 3 results:");
    for (i, result) in results.iter().enumerate() {
        println!("\n   {}. ID: {}", i + 1, result.id);
        println!("      Score: {:.4}", result.score);
        println!("      Metadata: {}", serde_json::to_string_pretty(&result.metadata)?);
    }
    println!();

    // Query with filter
    println!("🔍 Querying with filter (cloud documents only)...");
    let filter_ids = vec!["doc-003".to_string(), "doc-004".to_string()];
    let filtered_results = storage
        .query(&generate_sample_vector(384, 0.55), 2, Some(&filter_ids))
        .await?;

    println!("\n📋 Filtered results:");
    for (i, result) in filtered_results.iter().enumerate() {
        println!("\n   {}. ID: {}", i + 1, result.id);
        println!("      Score: {:.4}", result.score);
        let title = result.metadata["title"].as_str().unwrap_or("N/A");
        println!("      Title: {}", title);
    }
    println!();

    // Finalize (saves index to S3)
    println!("💾 Finalizing storage (saving index to S3)...");
    storage.finalize().await?;

    println!("\n✨ Example completed successfully!");
    println!("🔄 Run this example again to see data persistence across sessions.");
    println!("\n💡 Tip: To clear data, set CLEAR_DATA=1 environment variable");

    // Optional: Clear data
    if std::env::var("CLEAR_DATA").is_ok() {
        println!("\n🗑️  Clearing all data...");
        storage.clear().await?;
        println!("✅ Data cleared");
    }

    Ok(())
}

/// Generate a sample vector with a base value.
fn generate_sample_vector(dimension: usize, base: f32) -> Vec<f32> {
    (0..dimension)
        .map(|i| base + (i as f32 * 0.001))
        .collect()
}
