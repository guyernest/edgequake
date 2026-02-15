//! Integration tests for DynamoDB KV storage.
//!
//! Note: These tests require a DynamoDB table.
//! Set DYNAMODB_TABLE environment variable to run tests.
//!
//! Example:
//! ```bash
//! # Create table first:
//! aws dynamodb create-table \
//!     --table-name edgequake-test-kv \
//!     --attribute-definitions \
//!         AttributeName=namespace,AttributeType=S \
//!         AttributeName=id,AttributeType=S \
//!     --key-schema \
//!         AttributeName=namespace,KeyType=HASH \
//!         AttributeName=id,KeyType=RANGE \
//!     --billing-mode PAY_PER_REQUEST
//!
//! # Run tests:
//! export DYNAMODB_TABLE=edgequake-test-kv
//! cargo test --features dynamodb -- --test-threads=1
//! ```

use edgequake_storage::KVStorage;
use edgequake_storage_aws::{DynamoKVConfig, DynamoKVStorage};
use serde_json::json;
use std::collections::HashSet;

/// Helper to check if we should run DynamoDB tests.
fn should_run_dynamodb_tests() -> bool {
    std::env::var("DYNAMODB_TABLE").is_ok()
}

/// Get DynamoDB table name from environment.
fn get_dynamodb_table() -> String {
    std::env::var("DYNAMODB_TABLE").unwrap_or_else(|_| "edgequake-test-kv".to_string())
}

#[tokio::test]
async fn test_dynamodb_initialization() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-init");
    let storage = DynamoKVStorage::new(config).await.unwrap();

    let result = storage.initialize().await;
    assert!(result.is_ok(), "Failed to initialize: {:?}", result);
}

#[tokio::test]
async fn test_dynamodb_upsert_and_get() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-upsert");
    let storage = DynamoKVStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();

    // Clear any existing data
    storage.clear().await.unwrap();

    // Upsert data
    let data = vec![
        ("key1".to_string(), json!({"name": "Alice", "age": 30})),
        ("key2".to_string(), json!({"name": "Bob", "age": 25})),
    ];

    storage.upsert(&data).await.unwrap();

    // Get single item
    let result = storage.get_by_id("key1").await.unwrap();
    assert!(result.is_some());
    let value = result.unwrap();
    assert_eq!(value["name"], "Alice");
    assert_eq!(value["age"], 30);

    // Get multiple items
    let results = storage
        .get_by_ids(&["key1".to_string(), "key2".to_string()])
        .await
        .unwrap();
    assert_eq!(results.len(), 2);

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_dynamodb_update() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-update");
    let storage = DynamoKVStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Insert initial data
    let data = vec![("user1".to_string(), json!({"name": "Alice", "score": 10}))];
    storage.upsert(&data).await.unwrap();

    // Update data
    let updated_data = vec![("user1".to_string(), json!({"name": "Alice", "score": 20}))];
    storage.upsert(&updated_data).await.unwrap();

    // Verify update
    let result = storage.get_by_id("user1").await.unwrap().unwrap();
    assert_eq!(result["score"], 20);

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_dynamodb_delete() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-delete");
    let storage = DynamoKVStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Insert data
    let data = vec![
        ("item1".to_string(), json!({"value": 1})),
        ("item2".to_string(), json!({"value": 2})),
        ("item3".to_string(), json!({"value": 3})),
    ];
    storage.upsert(&data).await.unwrap();

    // Delete one item
    storage.delete(&["item2".to_string()]).await.unwrap();

    // Verify deletion
    let result = storage.get_by_id("item2").await.unwrap();
    assert!(result.is_none());

    // Verify other items still exist
    let result = storage.get_by_id("item1").await.unwrap();
    assert!(result.is_some());

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_dynamodb_count_and_empty() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-count");
    let storage = DynamoKVStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Should be empty initially
    assert!(storage.is_empty().await.unwrap());
    assert_eq!(storage.count().await.unwrap(), 0);

    // Add items
    let data = vec![
        ("a".to_string(), json!(1)),
        ("b".to_string(), json!(2)),
        ("c".to_string(), json!(3)),
    ];
    storage.upsert(&data).await.unwrap();

    // Should not be empty
    assert!(!storage.is_empty().await.unwrap());
    assert_eq!(storage.count().await.unwrap(), 3);

    // Clean up
    storage.clear().await.unwrap();
    assert!(storage.is_empty().await.unwrap());
}

#[tokio::test]
async fn test_dynamodb_keys() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-keys");
    let storage = DynamoKVStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Insert data
    let data = vec![
        ("key1".to_string(), json!("value1")),
        ("key2".to_string(), json!("value2")),
        ("key3".to_string(), json!("value3")),
    ];
    storage.upsert(&data).await.unwrap();

    // Get keys
    let keys = storage.keys().await.unwrap();
    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&"key1".to_string()));
    assert!(keys.contains(&"key2".to_string()));
    assert!(keys.contains(&"key3".to_string()));

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_dynamodb_filter_keys() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-filter");
    let storage = DynamoKVStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Insert some keys
    let data = vec![
        ("exists1".to_string(), json!("value1")),
        ("exists2".to_string(), json!("value2")),
    ];
    storage.upsert(&data).await.unwrap();

    // Check which keys are missing
    let check_keys: HashSet<String> = vec![
        "exists1".to_string(),
        "exists2".to_string(),
        "missing1".to_string(),
        "missing2".to_string(),
    ]
    .into_iter()
    .collect();

    let missing = storage.filter_keys(check_keys).await.unwrap();

    // Should only contain the missing keys
    assert_eq!(missing.len(), 2);
    assert!(missing.contains("missing1"));
    assert!(missing.contains("missing2"));
    assert!(!missing.contains("exists1"));
    assert!(!missing.contains("exists2"));

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_dynamodb_transition_if_status() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-transition");
    let storage = DynamoKVStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Insert document with status
    let data = vec![(
        "doc1".to_string(),
        json!({"title": "Document", "status": "pending"}),
    )];
    storage.upsert(&data).await.unwrap();

    // Transition from pending to processing (should succeed)
    let result = storage
        .transition_if_status("doc1", "pending", "processing")
        .await
        .unwrap();
    assert!(result, "Transition should succeed");

    // Try to transition from pending to completed (should fail - status is now processing)
    let result = storage
        .transition_if_status("doc1", "pending", "completed")
        .await
        .unwrap();
    assert!(!result, "Transition should fail due to status mismatch");

    // Transition from processing to completed (should succeed)
    let result = storage
        .transition_if_status("doc1", "processing", "completed")
        .await
        .unwrap();
    assert!(result, "Transition should succeed");

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_dynamodb_batch_operations() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    let config = DynamoKVConfig::new(get_dynamodb_table(), "test-batch");
    let storage = DynamoKVStorage::new(config).await.unwrap();
    storage.initialize().await.unwrap();
    storage.clear().await.unwrap();

    // Insert 30 items (tests batch chunking - DynamoDB limit is 25)
    let data: Vec<_> = (0..30)
        .map(|i| (format!("item{}", i), json!({"index": i})))
        .collect();

    storage.upsert(&data).await.unwrap();

    // Verify count
    assert_eq!(storage.count().await.unwrap(), 30);

    // Batch get
    let ids: Vec<_> = (0..30).map(|i| format!("item{}", i)).collect();
    let results = storage.get_by_ids(&ids).await.unwrap();
    assert_eq!(results.len(), 30);

    // Batch delete
    storage.delete(&ids).await.unwrap();
    assert_eq!(storage.count().await.unwrap(), 0);
}

#[tokio::test]
async fn test_dynamodb_namespace_isolation() {
    if !should_run_dynamodb_tests() {
        println!("Skipping DynamoDB test - set DYNAMODB_TABLE to run");
        return;
    }

    // Create two storages with different namespaces
    let config1 = DynamoKVConfig::new(get_dynamodb_table(), "workspace-1");
    let storage1 = DynamoKVStorage::new(config1).await.unwrap();
    storage1.initialize().await.unwrap();
    storage1.clear().await.unwrap();

    let config2 = DynamoKVConfig::new(get_dynamodb_table(), "workspace-2");
    let storage2 = DynamoKVStorage::new(config2).await.unwrap();
    storage2.initialize().await.unwrap();
    storage2.clear().await.unwrap();

    // Insert data in namespace 1
    let data1 = vec![("key1".to_string(), json!({"namespace": 1}))];
    storage1.upsert(&data1).await.unwrap();

    // Insert data in namespace 2
    let data2 = vec![("key1".to_string(), json!({"namespace": 2}))];
    storage2.upsert(&data2).await.unwrap();

    // Verify isolation
    let result1 = storage1.get_by_id("key1").await.unwrap().unwrap();
    assert_eq!(result1["namespace"], 1);

    let result2 = storage2.get_by_id("key1").await.unwrap().unwrap();
    assert_eq!(result2["namespace"], 2);

    // Count should be 1 for each
    assert_eq!(storage1.count().await.unwrap(), 1);
    assert_eq!(storage2.count().await.unwrap(), 1);

    // Clean up
    storage1.clear().await.unwrap();
    storage2.clear().await.unwrap();
}
