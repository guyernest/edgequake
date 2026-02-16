//! Basic example of using Athena SQL query engine.
//!
//! This example demonstrates:
//! - Connecting to Athena
//! - Executing SQL queries
//! - Parsing query results
//! - Getting query statistics
//! - Using the query builder
//!
//! # Setup
//!
//! 1. Create a Glue Catalog database:
//!    ```bash
//!    aws glue create-database --database-input '{"Name":"edgequake"}'
//!    ```
//!
//! 2. Create an S3 bucket for Athena query results:
//!    ```bash
//!    aws s3 mb s3://my-athena-results
//!    ```
//!
//! 3. (Optional) Create a sample table:
//!    ```sql
//!    CREATE EXTERNAL TABLE IF NOT EXISTS edgequake.documents (
//!      id STRING,
//!      workspace_id STRING,
//!      title STRING,
//!      content STRING,
//!      status STRING,
//!      created_at TIMESTAMP,
//!      size_bytes BIGINT
//!    )
//!    STORED AS PARQUET
//!    LOCATION 's3://my-bucket/documents/';
//!    ```
//!
//! 4. Set AWS credentials:
//!    ```bash
//!    export AWS_PROFILE=your-profile
//!    # OR
//!    export AWS_ACCESS_KEY_ID=your-key
//!    export AWS_SECRET_ACCESS_KEY=your-secret
//!    ```
//!
//! 5. Run the example:
//!    ```bash
//!    ATHENA_DATABASE=edgequake \
//!    ATHENA_OUTPUT_LOCATION=s3://my-athena-results/ \
//!    cargo run --example athena_sql_basic --features athena
//!    ```

use edgequake_storage_aws::{AthenaConfig, AthenaQueryEngine, QueryBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("⚡ EdgeQuake Athena SQL Query Engine Example\n");

    // Get configuration from environment
    let database = std::env::var("ATHENA_DATABASE").unwrap_or_else(|_| {
        eprintln!("❌ ATHENA_DATABASE not set!");
        eprintln!("   Set it to your Glue Catalog database name:");
        eprintln!("   export ATHENA_DATABASE=edgequake");
        std::process::exit(1);
    });

    let output_location = std::env::var("ATHENA_OUTPUT_LOCATION").unwrap_or_else(|_| {
        eprintln!("❌ ATHENA_OUTPUT_LOCATION not set!");
        eprintln!("   Set it to your S3 results bucket:");
        eprintln!("   export ATHENA_OUTPUT_LOCATION=s3://my-athena-results/");
        std::process::exit(1);
    });

    println!("📦 Using Athena configuration:");
    println!("   - Database: {}", database);
    println!("   - Output location: {}", output_location);
    println!();

    // Create configuration
    let config = AthenaConfig::new(database.clone(), output_location)
        .with_timeout(300) // 5 minutes
        .with_poll_interval(2) // 2 seconds
        .with_result_cache(true);

    println!("⚙️  Configuration:");
    println!("   - Query timeout: {}s", config.query_timeout_secs);
    println!("   - Poll interval: {}s", config.poll_interval_secs);
    println!(
        "   - Result cache: {}",
        if config.enable_result_cache {
            "Enabled"
        } else {
            "Disabled"
        }
    );
    println!("   - Workgroup: {}", config.workgroup);
    println!();

    // Create query engine
    println!("🔧 Creating Athena query engine...");
    let engine = AthenaQueryEngine::new(config).await?;
    println!("✅ Engine created successfully\n");

    // Example 1: List databases
    println!("📝 Example 1: List Databases\n");
    match engine.query("SHOW DATABASES").await {
        Ok(rows) => {
            println!("   Found {} databases:", rows.len());
            for row in rows.iter().take(10) {
                if let Ok(db) = row.get("database_name") {
                    println!("   - {}", db);
                }
            }
            println!();
        }
        Err(e) => {
            eprintln!("   ❌ Failed to list databases: {}", e);
        }
    }

    // Example 2: List tables
    println!("📋 Example 2: List Tables\n");
    match engine.query("SHOW TABLES").await {
        Ok(rows) => {
            if rows.is_empty() {
                println!("   No tables found in database '{}'", database);
                println!("   Create a table first (see setup instructions)\n");
            } else {
                println!("   Found {} tables:", rows.len());
                for row in rows {
                    if let Ok(table) = row.get("tab_name") {
                        println!("   - {}", table);
                    }
                }
                println!();
            }
        }
        Err(e) => {
            eprintln!("   ❌ Failed to list tables: {}", e);
        }
    }

    // Example 3: Simple SELECT query
    println!("🔍 Example 3: Simple SELECT Query\n");
    let simple_query = "SELECT 1 as number, 'Hello Athena' as message, true as success";
    println!("   Query: {}", simple_query);

    match engine.query(simple_query).await {
        Ok(rows) => {
            println!("   Results:");
            for row in rows {
                let number = row.get("number").unwrap_or_default();
                let message = row.get("message").unwrap_or_default();
                let success = row.get("success").unwrap_or_default();
                println!(
                    "   - number: {}, message: {}, success: {}",
                    number, message, success
                );
            }
            println!();
        }
        Err(e) => {
            eprintln!("   ❌ Query failed: {}", e);
        }
    }

    // Example 4: Async query execution with statistics
    println!("📊 Example 4: Async Query with Statistics\n");
    let stats_query = "SELECT COUNT(*) as table_count FROM information_schema.tables";
    println!("   Query: {}", stats_query);

    match engine.start_query(stats_query).await {
        Ok(execution_id) => {
            println!("   Started query: {}", execution_id);

            // Wait for completion
            match engine.wait_for_query(&execution_id).await {
                Ok(stats) => {
                    println!("   Query completed successfully!");
                    println!("   Statistics:");
                    println!("     - State: {}", stats.state);
                    println!("     - Data scanned: {} bytes", stats.data_scanned_bytes);
                    println!("     - Execution time: {} ms", stats.execution_time_ms);
                    println!("     - Queue time: {} ms", stats.queue_time_ms);
                    println!(
                        "     - Engine execution time: {} ms",
                        stats.engine_execution_time_ms
                    );
                    println!("     - Estimated cost: ${:.6}", stats.estimated_cost_usd());

                    // Get results
                    match engine.get_query_results(&execution_id).await {
                        Ok(rows) => {
                            println!("   Results:");
                            for row in rows {
                                println!("     {:?}", row.to_json());
                            }
                        }
                        Err(e) => {
                            eprintln!("   ❌ Failed to get results: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("   ❌ Query failed: {}", e);
                }
            }
            println!();
        }
        Err(e) => {
            eprintln!("   ❌ Failed to start query: {}", e);
        }
    }

    // Example 5: Query builder
    println!("🔨 Example 5: Query Builder\n");
    let builder = QueryBuilder::new(&database);

    let queries = vec![
        ("List documents", builder.list_documents("ws-123", 10)),
        (
            "Search by status",
            builder.search_by_status("ws-123", "completed"),
        ),
        ("Count documents", builder.count_documents("ws-123")),
        ("Workspace stats", builder.workspace_stats()),
        (
            "Search vectors",
            builder.search_vectors("test-namespace", None),
        ),
    ];

    println!("   Built queries:");
    for (name, query) in queries {
        println!("   - {}: {}", name, query);
    }
    println!();

    // Example 6: Multiple queries in parallel
    println!("⚡ Example 6: Parallel Queries\n");
    println!("   Starting 3 queries in parallel...");

    let query1 = engine.start_query("SELECT 1 as query_num");
    let query2 = engine.start_query("SELECT 2 as query_num");
    let query3 = engine.start_query("SELECT 3 as query_num");

    match tokio::try_join!(query1, query2, query3) {
        Ok((id1, id2, id3)) => {
            println!("   ✅ All queries started:");
            println!("     - Query 1: {}", id1);
            println!("     - Query 2: {}", id2);
            println!("     - Query 3: {}", id3);

            // Wait for all
            let wait1 = engine.wait_for_query(&id1);
            let wait2 = engine.wait_for_query(&id2);
            let wait3 = engine.wait_for_query(&id3);

            match tokio::try_join!(wait1, wait2, wait3) {
                Ok(_) => {
                    println!("   ✅ All queries completed successfully");
                }
                Err(e) => {
                    eprintln!("   ❌ Some queries failed: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("   ❌ Failed to start queries: {}", e);
        }
    }
    println!();

    // Example 7: Schema information
    println!("📖 Example 7: Schema Information\n");
    let schema_query = format!(
        "SELECT table_name, column_name, data_type \
         FROM information_schema.columns \
         WHERE table_schema = '{}' \
         LIMIT 10",
        database
    );
    println!("   Query: {}", schema_query);

    match engine.query(&schema_query).await {
        Ok(rows) => {
            if rows.is_empty() {
                println!("   No columns found (tables may not exist yet)");
            } else {
                println!("   Schema information:");
                for row in rows {
                    let table = row.get("table_name").unwrap_or_default();
                    let column = row.get("column_name").unwrap_or_default();
                    let data_type = row.get("data_type").unwrap_or_default();
                    println!("     - {}.{}: {}", table, column, data_type);
                }
            }
        }
        Err(e) => {
            eprintln!("   ❌ Query failed: {}", e);
        }
    }
    println!();

    println!("✨ Example completed successfully!");
    println!();
    println!("💡 Next steps:");
    println!("   1. Create tables in Glue Catalog pointing to S3 data");
    println!("   2. Use Parquet format for 90%+ cost reduction");
    println!("   3. Partition tables by workspace_id for better performance");
    println!("   4. Monitor costs in AWS Cost Explorer");
    println!("   5. Use query result caching to avoid re-scanning");
    println!();
    println!("📚 Resources:");
    println!("   - Athena docs: https://docs.aws.amazon.com/athena/");
    println!("   - Parquet docs: https://parquet.apache.org/");
    println!("   - Cost optimization: https://aws.amazon.com/athena/pricing/");

    Ok(())
}
