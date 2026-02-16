//! Integration tests for S3 vector storage.
//!
//! Note: These tests require AWS credentials and an S3 bucket.
//! Set AWS_PROFILE or AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY.
//! Set TEST_S3_BUCKET environment variable to run tests.

use edgequake_storage::VectorStorage;
use edgequake_storage_aws::{S3Config, S3VectorStorage};
use serde_json::json;

/// Helper to check if we should run S3 tests.
fn should_run_s3_tests() -> bool {
    std::env::var("TEST_S3_BUCKET").is_ok()
}

/// Get test bucket name from environment.
fn get_test_bucket() -> String {
    std::env::var("TEST_S3_BUCKET").unwrap_or_else(|_| "test-edgequake-vectors".to_string())
}

#[tokio::test]
async fn test_s3_vector_storage_initialization() {
    if !should_run_s3_tests() {
        println!("Skipping S3 test - set TEST_S3_BUCKET to run");
        return;
    }

    let config = S3Config::new(get_test_bucket(), "test-workspace", 128);
    let storage = S3VectorStorage::new(config).await.unwrap();

    let result = storage.initialize().await;
    assert!(result.is_ok(), "Failed to initialize: {:?}", result);
}

#[tokio::test]
async fn test_s3_vector_upsert_and_query() {
    if !should_run_s3_tests() {
        println!("Skipping S3 test - set TEST_S3_BUCKET to run");
        return;
    }

    let config = S3Config::new(get_test_bucket(), "test-workspace-upsert", 128);
    let storage = S3VectorStorage::new(config).await.unwrap();

    storage.initialize().await.unwrap();

    // Create test vectors
    let data = vec![
        (
            "vec1".to_string(),
            vec![0.1; 128],
            json!({"doc": "document1"}),
        ),
        (
            "vec2".to_string(),
            vec![0.2; 128],
            json!({"doc": "document2"}),
        ),
        (
            "vec3".to_string(),
            vec![0.3; 128],
            json!({"doc": "document3"}),
        ),
    ];

    // Upsert vectors
    let result = storage.upsert(&data).await;
    assert!(result.is_ok(), "Failed to upsert: {:?}", result);

    // Query similar vectors
    let query = vec![0.15; 128];
    let results = storage.query(&query, 2, None).await.unwrap();

    assert_eq!(results.len(), 2, "Expected 2 results");
    assert!(
        results[0].score >= results[1].score,
        "Results should be sorted by score"
    );

    // Clean up
    storage.clear().await.unwrap();
}

#[tokio::test]
async fn test_s3_vector_count_and_empty() {
    if !should_run_s3_tests() {
        println!("Skipping S3 test - set TEST_S3_BUCKET to run");
        return;
    }

    let config = S3Config::new(get_test_bucket(), "test-workspace-count", 64);
    let storage = S3VectorStorage::new(config).await.unwrap();

    storage.initialize().await.unwrap();

    // Should be empty initially
    assert!(storage.is_empty().await.unwrap());
    assert_eq!(storage.count().await.unwrap(), 0);

    // Add some vectors
    let data = vec![
        ("v1".to_string(), vec![0.1; 64], json!({})),
        ("v2".to_string(), vec![0.2; 64], json!({})),
    ];

    storage.upsert(&data).await.unwrap();

    // Should not be empty
    assert!(!storage.is_empty().await.unwrap());
    assert_eq!(storage.count().await.unwrap(), 2);

    // Clean up
    storage.clear().await.unwrap();
    assert!(storage.is_empty().await.unwrap());
}

#[tokio::test]
async fn test_s3_vector_dimension_validation() {
    if !should_run_s3_tests() {
        println!("Skipping S3 test - set TEST_S3_BUCKET to run");
        return;
    }

    let config = S3Config::new(get_test_bucket(), "test-workspace-dim", 128);
    let storage = S3VectorStorage::new(config).await.unwrap();

    storage.initialize().await.unwrap();

    // Try to insert vector with wrong dimension
    let data = vec![("bad".to_string(), vec![0.1; 64], json!({}))]; // Wrong dimension

    let result = storage.upsert(&data).await;
    assert!(result.is_err(), "Should fail with dimension mismatch");
}

#[tokio::test]
async fn test_s3_vector_persistence() {
    if !should_run_s3_tests() {
        println!("Skipping S3 test - set TEST_S3_BUCKET to run");
        return;
    }

    let namespace = format!("test-persist-{}", uuid::Uuid::new_v4());
    let config = S3Config::new(get_test_bucket(), &namespace, 128);

    // Create storage and add data
    {
        let storage = S3VectorStorage::new(config.clone()).await.unwrap();
        storage.initialize().await.unwrap();

        let data = vec![
            (
                "p1".to_string(),
                vec![0.5; 128],
                json!({"persistent": true}),
            ),
            (
                "p2".to_string(),
                vec![0.6; 128],
                json!({"persistent": true}),
            ),
        ];

        storage.upsert(&data).await.unwrap();
        storage.finalize().await.unwrap();
    }

    // Create new storage instance and verify data persists
    {
        let storage = S3VectorStorage::new(config).await.unwrap();
        storage.initialize().await.unwrap();

        assert_eq!(storage.count().await.unwrap(), 2);

        // Query should work
        let query = vec![0.55; 128];
        let results = storage.query(&query, 2, None).await.unwrap();
        assert_eq!(results.len(), 2);

        // Clean up
        storage.clear().await.unwrap();
    }
}

#[tokio::test]
async fn test_s3_vector_filter_query() {
    if !should_run_s3_tests() {
        println!("Skipping S3 test - set TEST_S3_BUCKET to run");
        return;
    }

    let config = S3Config::new(get_test_bucket(), "test-filter", 64);
    let storage = S3VectorStorage::new(config).await.unwrap();

    storage.initialize().await.unwrap();

    // Add vectors
    let data = vec![
        ("f1".to_string(), vec![0.1; 64], json!({"type": "a"})),
        ("f2".to_string(), vec![0.2; 64], json!({"type": "b"})),
        ("f3".to_string(), vec![0.3; 64], json!({"type": "a"})),
    ];

    storage.upsert(&data).await.unwrap();

    // Query with filter
    let query = vec![0.15; 64];
    let filter_ids = vec!["f1".to_string(), "f3".to_string()];
    let results = storage.query(&query, 10, Some(&filter_ids)).await.unwrap();

    // Should only return filtered results
    assert!(results.len() <= 2);
    for result in &results {
        assert!(filter_ids.contains(&result.id));
    }

    // Clean up
    storage.clear().await.unwrap();
}
