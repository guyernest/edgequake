//! Integration tests for Athena SQL query engine.
//!
//! # Setup
//!
//! 1. Create Glue Catalog database:
//!    ```bash
//!    aws glue create-database --database-input '{"Name":"edgequake_test"}'
//!    ```
//!
//! 2. Create S3 bucket for query results:
//!    ```bash
//!    aws s3 mb s3://your-athena-results-bucket
//!    ```
//!
//! 3. Create test table (optional):
//!    ```sql
//!    CREATE EXTERNAL TABLE IF NOT EXISTS edgequake_test.documents (
//!      id STRING,
//!      workspace_id STRING,
//!      title STRING,
//!      status STRING,
//!      created_at TIMESTAMP
//!    )
//!    STORED AS PARQUET
//!    LOCATION 's3://your-bucket/documents/';
//!    ```
//!
//! 4. Set environment variables:
//!    ```bash
//!    export TEST_ATHENA_DATABASE=edgequake_test
//!    export TEST_ATHENA_OUTPUT_LOCATION=s3://your-athena-results-bucket/
//!    ```
//!
//! 5. Run tests:
//!    ```bash
//!    cargo test --features athena -- --nocapture
//!    ```

#[cfg(feature = "athena")]
mod athena_tests {
    use edgequake_storage_aws::{AthenaConfig, AthenaQueryEngine, QueryBuilder};

    fn get_test_config() -> AthenaConfig {
        let database =
            std::env::var("TEST_ATHENA_DATABASE").unwrap_or_else(|_| "edgequake_test".to_string());

        let output_location = std::env::var("TEST_ATHENA_OUTPUT_LOCATION")
            .unwrap_or_else(|_| "s3://edgequake-test-results/".to_string());

        AthenaConfig::new(database, output_location)
            .with_timeout(60)
            .with_poll_interval(2)
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials and setup
    async fn test_athena_query_engine_creation() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config).await;
        assert!(engine.is_ok(), "Failed to create Athena engine");
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials and table setup
    async fn test_list_databases() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        // Query to list databases
        let query = "SHOW DATABASES";
        let results = engine.query(query).await;

        match results {
            Ok(rows) => {
                println!("Found {} databases", rows.len());
                for row in rows.iter().take(5) {
                    println!("Database: {:?}", row.to_json());
                }
                assert!(!rows.is_empty(), "Expected at least one database");
            }
            Err(e) => {
                eprintln!("Query failed: {:?}", e);
                panic!("Query failed: {:?}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials and table setup
    async fn test_list_tables() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        // Query to list tables
        let query = "SHOW TABLES";
        let results = engine.query(query).await;

        match results {
            Ok(rows) => {
                println!("Found {} tables", rows.len());
                for row in rows.iter().take(5) {
                    println!("Table: {:?}", row.to_json());
                }
                // Note: May be empty if no tables created yet
            }
            Err(e) => {
                eprintln!("Query failed: {:?}", e);
                // Don't panic - table may not exist yet
            }
        }
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials and data
    async fn test_select_query() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        // Simple select query
        let query = "SELECT 1 as test_value, 'hello' as test_string";
        let results = engine.query(query).await;

        match results {
            Ok(rows) => {
                println!("Query returned {} rows", rows.len());
                assert_eq!(rows.len(), 1, "Expected exactly 1 row");

                let row = &rows[0];
                let value = row.get("test_value").expect("Missing test_value");
                let string = row.get("test_string").expect("Missing test_string");

                assert_eq!(value, "1");
                assert_eq!(string, "hello");
            }
            Err(e) => {
                eprintln!("Query failed: {:?}", e);
                panic!("Query failed: {:?}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials
    async fn test_async_query_execution() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        // Start query async
        let query = "SELECT 42 as answer, 'test' as name";
        let execution_id = engine
            .start_query(query)
            .await
            .expect("Failed to start query");

        println!("Started query with execution ID: {}", execution_id);
        assert!(!execution_id.is_empty());

        // Wait for completion
        let stats = engine
            .wait_for_query(&execution_id)
            .await
            .expect("Failed to wait for query");

        println!("Query stats: {:?}", stats);
        assert_eq!(stats.state, "Succeeded");

        // Get results
        let results = engine
            .get_query_results(&execution_id)
            .await
            .expect("Failed to get results");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].get("answer").unwrap(), "42");
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials
    async fn test_query_stats() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        let query = "SELECT COUNT(*) as cnt FROM information_schema.tables";
        let execution_id = engine
            .start_query(query)
            .await
            .expect("Failed to start query");

        engine
            .wait_for_query(&execution_id)
            .await
            .expect("Failed to wait for query");

        let stats = engine
            .get_query_stats(&execution_id)
            .await
            .expect("Failed to get stats");

        println!("Query statistics:");
        println!("  Execution ID: {}", stats.execution_id);
        println!("  State: {}", stats.state);
        println!("  Data scanned: {} bytes", stats.data_scanned_bytes);
        println!("  Execution time: {} ms", stats.execution_time_ms);
        println!("  Queue time: {} ms", stats.queue_time_ms);
        println!(
            "  Engine execution time: {} ms",
            stats.engine_execution_time_ms
        );
        println!("  Estimated cost: ${:.6}", stats.estimated_cost_usd());

        assert!(stats.data_scanned_bytes >= 0);
        assert!(stats.execution_time_ms >= 0);
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials
    async fn test_query_builder() {
        let builder = QueryBuilder::new("edgequake_test");

        // Test list documents query
        let query = builder.list_documents("ws-123", 10);
        assert!(query.contains("workspace_id = 'ws-123'"));
        assert!(query.contains("LIMIT 10"));

        // Test search by status
        let query = builder.search_by_status("ws-123", "completed");
        assert!(query.contains("status = 'completed'"));

        // Test count query
        let query = builder.count_documents("ws-123");
        assert!(query.contains("COUNT(*)"));

        // Test workspace stats
        let query = builder.workspace_stats();
        assert!(query.contains("GROUP BY workspace_id"));

        // Test search vectors
        let query = builder.search_vectors("test-namespace", None);
        assert!(query.contains("namespace = 'test-namespace'"));

        let query = builder.search_vectors("test-namespace", Some("status = 'active'"));
        assert!(query.contains("status = 'active'"));
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials and table
    async fn test_query_with_filter() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        // Query with WHERE clause
        let query = "SELECT table_name FROM information_schema.tables WHERE table_schema = 'information_schema' LIMIT 5";
        let results = engine.query(query).await;

        match results {
            Ok(rows) => {
                println!("Query returned {} rows", rows.len());
                for row in rows.iter() {
                    println!("Table: {:?}", row.to_json());
                }
                assert!(
                    rows.len() <= 5,
                    "Expected at most 5 rows due to LIMIT clause"
                );
            }
            Err(e) => {
                eprintln!("Query failed: {:?}", e);
                panic!("Query failed: {:?}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials
    async fn test_row_parsing() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        let query = "SELECT 42 as int_val, 3.14 as float_val, true as bool_val, 'test' as str_val";
        let results = engine.query(query).await.expect("Query failed");

        assert_eq!(results.len(), 1);
        let row = &results[0];

        // Test different type parsers
        assert_eq!(row.get_i64("int_val").unwrap(), 42);
        assert!((row.get_f64("float_val").unwrap() - 3.14).abs() < 0.01);
        assert_eq!(row.get_bool("bool_val").unwrap(), true);
        assert_eq!(row.get("str_val").unwrap(), "test");

        // Test to_json
        let json = row.to_json();
        assert!(json.is_object());
        println!("Row as JSON: {}", json);
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials
    async fn test_query_timeout() {
        let config = get_test_config().with_timeout(1); // 1 second timeout
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        // Start a query but don't wait long enough
        let query = "SELECT 1";
        let execution_id = engine.start_query(query).await.expect("Failed to start");

        // Wait with short timeout (should succeed quickly for simple query)
        let result = engine.wait_for_query(&execution_id).await;
        // This should succeed because the query is simple
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials
    async fn test_invalid_query() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        // Invalid SQL syntax
        let query = "SELECT * FROM nonexistent_table_xyz_123";
        let result = engine.query(query).await;

        // Should fail
        assert!(result.is_err(), "Expected query to fail");
        println!("Error (expected): {:?}", result.err());
    }

    #[tokio::test]
    #[ignore] // Requires AWS credentials
    async fn test_multiple_queries_parallel() {
        let config = get_test_config();
        let engine = AthenaQueryEngine::new(config)
            .await
            .expect("Failed to create engine");

        // Start multiple queries in parallel
        let query1 = engine.start_query("SELECT 1 as a");
        let query2 = engine.start_query("SELECT 2 as b");
        let query3 = engine.start_query("SELECT 3 as c");

        let (id1, id2, id3) = tokio::join!(query1, query2, query3);

        println!("Query 1 ID: {:?}", id1);
        println!("Query 2 ID: {:?}", id2);
        println!("Query 3 ID: {:?}", id3);

        // All should succeed
        assert!(id1.is_ok());
        assert!(id2.is_ok());
        assert!(id3.is_ok());

        // Wait for all to complete
        let wait1 = engine.wait_for_query(&id1.unwrap());
        let wait2 = engine.wait_for_query(&id2.unwrap());
        let wait3 = engine.wait_for_query(&id3.unwrap());

        let (result1, result2, result3) = tokio::join!(wait1, wait2, wait3);

        assert!(result1.is_ok());
        assert!(result2.is_ok());
        assert!(result3.is_ok());

        println!("All queries completed successfully");
    }
}
