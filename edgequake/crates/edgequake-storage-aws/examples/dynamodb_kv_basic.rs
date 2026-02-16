//! Basic example of using DynamoDB KV storage.
//!
//! This example demonstrates:
//! - Connecting to DynamoDB
//! - Storing and retrieving JSON documents
//! - Batch operations
//! - Atomic status transitions
//!
//! # Setup
//!
//! 1. Create a DynamoDB table:
//!    ```bash
//!    aws dynamodb create-table \
//!        --table-name edgequake-kv \
//!        --attribute-definitions \
//!            AttributeName=namespace,AttributeType=S \
//!            AttributeName=id,AttributeType=S \
//!        --key-schema \
//!            AttributeName=namespace,KeyType=HASH \
//!            AttributeName=id,KeyType=RANGE \
//!        --billing-mode PAY_PER_REQUEST
//!    ```
//!
//! 2. Set AWS credentials:
//!    ```bash
//!    export AWS_PROFILE=your-profile
//!    # OR
//!    export AWS_ACCESS_KEY_ID=your-key
//!    export AWS_SECRET_ACCESS_KEY=your-secret
//!    ```
//!
//! 3. Run the example:
//!    ```bash
//!    DYNAMODB_TABLE=edgequake-kv cargo run --example dynamodb_kv_basic --features dynamodb
//!    ```

use edgequake_storage::KVStorage;
use edgequake_storage_aws::{DynamoKVConfig, DynamoKVStorage};
use serde_json::json;
use std::collections::HashSet;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("⚡ EdgeQuake DynamoDB KV Storage Example\n");

    // Get table name from environment
    let table_name = std::env::var("DYNAMODB_TABLE").unwrap_or_else(|_| {
        eprintln!("❌ DYNAMODB_TABLE not set!");
        eprintln!("   Set it to your DynamoDB table name:");
        eprintln!("   export DYNAMODB_TABLE=edgequake-kv");
        std::process::exit(1);
    });

    println!("📦 Using DynamoDB table: {}", table_name);

    // Create configuration
    let config = DynamoKVConfig::new(table_name, "example-workspace")
        .with_on_demand(true) // Pay-per-request pricing
        .with_pitr(true); // Point-in-time recovery

    println!("⚙️  Configuration:");
    println!("   - Namespace: {}", config.namespace);
    println!(
        "   - Billing: {}",
        if config.on_demand {
            "On-Demand"
        } else {
            "Provisioned"
        }
    );
    println!(
        "   - PITR: {}",
        if config.enable_pitr {
            "Enabled"
        } else {
            "Disabled"
        }
    );
    println!();

    // Create storage
    println!("🔧 Creating DynamoDB KV storage...");
    let storage = DynamoKVStorage::new(config).await?;

    // Initialize
    println!("🔄 Initializing storage...");
    storage.initialize().await?;

    // Check current state
    let count = storage.count().await?;
    println!("📊 Current item count: {}", count);
    println!();

    // Optional: Clear existing data
    if std::env::var("CLEAR_DATA").is_ok() {
        println!("🗑️  Clearing existing data...");
        storage.clear().await?;
    }

    // Example 1: Basic CRUD operations
    println!("📝 Example 1: Basic CRUD Operations\n");

    // Insert documents
    let documents = vec![
        (
            "doc-001".to_string(),
            json!({
                "title": "Introduction to Rust",
                "author": "rustacean",
                "category": "programming",
                "published": "2024-01-15",
                "tags": ["rust", "programming", "tutorial"]
            }),
        ),
        (
            "doc-002".to_string(),
            json!({
                "title": "Advanced DynamoDB",
                "author": "cloud_expert",
                "category": "cloud",
                "published": "2024-02-20",
                "tags": ["aws", "dynamodb", "database"]
            }),
        ),
    ];

    println!("   Inserting {} documents...", documents.len());
    storage.upsert(&documents).await?;
    println!("   ✅ Documents inserted");

    // Read a document
    println!("\n   Reading document 'doc-001':");
    let doc = storage.get_by_id("doc-001").await?;
    if let Some(doc) = doc {
        println!("   Title: {}", doc["title"]);
        println!("   Author: {}", doc["author"]);
        println!("   Category: {}", doc["category"]);
    }

    // Update a document
    println!("\n   Updating document 'doc-001':");
    let updated = vec![(
        "doc-001".to_string(),
        json!({
            "title": "Introduction to Rust (Updated)",
            "author": "rustacean",
            "category": "programming",
            "published": "2024-01-15",
            "updated": "2024-03-01",
            "tags": ["rust", "programming", "tutorial", "beginner"]
        }),
    )];
    storage.upsert(&updated).await?;
    println!("   ✅ Document updated");

    // Example 2: Batch operations
    println!("\n📦 Example 2: Batch Operations\n");

    // Batch insert
    let batch_data: Vec<_> = (0..10)
        .map(|i| {
            (
                format!("item-{:03}", i),
                json!({
                    "index": i,
                    "value": format!("Value {}", i),
                    "timestamp": chrono::Utc::now().timestamp()
                }),
            )
        })
        .collect();

    println!("   Batch inserting {} items...", batch_data.len());
    storage.upsert(&batch_data).await?;
    println!("   ✅ Batch insert complete");

    // Batch read
    let ids: Vec<_> = (0..10).map(|i| format!("item-{:03}", i)).collect();
    let results = storage.get_by_ids(&ids).await?;
    println!("   Retrieved {} items", results.len());

    // Example 3: Filter keys (deduplication)
    println!("\n🔍 Example 3: Filter Keys (Find Missing)\n");

    let check_keys: HashSet<String> = vec![
        "doc-001".to_string(),
        "doc-002".to_string(),
        "doc-003".to_string(), // doesn't exist
        "doc-004".to_string(), // doesn't exist
    ]
    .into_iter()
    .collect();

    println!("   Checking which keys are missing...");
    let missing = storage.filter_keys(check_keys).await?;
    println!("   Missing keys: {:?}", missing);

    // Example 4: Atomic status transitions
    println!("\n🔄 Example 4: Atomic Status Transitions\n");

    // Insert document with status
    let status_doc = vec![(
        "task-001".to_string(),
        json!({
            "name": "Process Document",
            "status": "pending",
            "created_at": chrono::Utc::now().to_rfc3339()
        }),
    )];
    storage.upsert(&status_doc).await?;
    println!("   Created task with status 'pending'");

    // Transition: pending -> processing
    let success = storage
        .transition_if_status("task-001", "pending", "processing")
        .await?;
    println!(
        "   Transition pending -> processing: {}",
        if success { "✅ Success" } else { "❌ Failed" }
    );

    // Try invalid transition: pending -> completed (should fail)
    let success = storage
        .transition_if_status("task-001", "pending", "completed")
        .await?;
    println!(
        "   Transition pending -> completed: {}",
        if success {
            "✅ Success"
        } else {
            "❌ Failed (expected)"
        }
    );

    // Transition: processing -> completed
    let success = storage
        .transition_if_status("task-001", "processing", "completed")
        .await?;
    println!(
        "   Transition processing -> completed: {}",
        if success { "✅ Success" } else { "❌ Failed" }
    );

    // Example 5: List all keys
    println!("\n📋 Example 5: List All Keys\n");

    let all_keys = storage.keys().await?;
    println!("   Total keys: {}", all_keys.len());
    println!("   First 5 keys:");
    for key in all_keys.iter().take(5) {
        println!("   - {}", key);
    }

    // Example 6: Count and statistics
    println!("\n📊 Example 6: Statistics\n");

    let total_count = storage.count().await?;
    let is_empty = storage.is_empty().await?;
    println!("   Total items: {}", total_count);
    println!("   Is empty: {}", is_empty);

    // Finalize
    println!("\n💾 Finalizing storage...");
    storage.finalize().await?;

    println!("\n✨ Example completed successfully!");
    println!("🔄 Run this example again to see data persistence.");
    println!("\n💡 Tips:");
    println!("   - Set CLEAR_DATA=1 to clear data before running");
    println!("   - View data in DynamoDB Console");
    println!("   - Use PartiQL for ad-hoc queries");
    println!("   - Monitor costs in AWS Cost Explorer");

    Ok(())
}
