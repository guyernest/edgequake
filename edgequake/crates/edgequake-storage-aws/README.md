# EdgeQuake AWS Storage

Cost-effective, scalable AWS storage adapters for the EdgeQuake RAG system.

## Features

- **S3 Vector Storage**: Store embeddings in S3 with HNSW indexing for fast similarity search ✅
- **Neptune Graph Storage**: Managed graph database using Gremlin ✅
- **DynamoDB KV Storage**: Low-latency key-value store ✅
- **Athena SQL Queries**: Serverless SQL over S3 data ✅

## Cost Benefits

Compared to PostgreSQL RDS:

- **95% cost reduction** for typical workloads
- **Pay-per-use pricing** - no idle costs
- **Auto-scaling** from zero to massive scale
- **S3 storage**: $0.023/GB vs $0.115/GB for RDS

### Example Cost Comparison

**100GB vectors, 1M graph nodes, 100K documents:**

| Component | PostgreSQL RDS | AWS | Savings |
|-----------|---------------|-----|---------|
| Monthly Cost | $297 | $12.50 | 95.7% |
| Annual Cost | $3,564 | $150 | $3,414 |

See [AWS_MIGRATION_DESIGN.md](../../AWS_MIGRATION_DESIGN.md) for detailed cost analysis.

## S3 Vector Storage

### Architecture

```
S3 Bucket:
├── {namespace}/
│   ├── vectors/
│   │   ├── batch-{timestamp}.json  (vector batches)
│   │   └── metadata.json           (vector metadata)
│   ├── index/
│   │   └── hnsw.bin               (HNSW index)
│   └── manifest.json              (file manifest)
```

### Features

- **In-memory HNSW index** for fast queries (sub-50ms)
- **Batch uploads** to S3 (configurable batch size)
- **Compression** support (gzip)
- **LRU caching** for frequently accessed vectors
- **Persistence** - index and data saved to S3
- **Durability** - 99.999999999% (11 nines)

### Usage

```rust
use edgequake_storage::VectorStorage;
use edgequake_storage_aws::{S3Config, S3VectorStorage};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure S3 storage
    let config = S3Config::new("my-bucket", "workspace-123", 1536)
        .with_hnsw_params(16, 200, 50)  // m, ef_construction, ef_search
        .with_batch_size(1000)
        .with_compression(true);

    // Create storage
    let storage = S3VectorStorage::new(config).await?;
    storage.initialize().await?;

    // Insert vectors
    let data = vec![
        (
            "doc1".to_string(),
            vec![0.1; 1536],
            json!({"title": "Document 1"})
        ),
        (
            "doc2".to_string(),
            vec![0.2; 1536],
            json!({"title": "Document 2"})
        ),
    ];
    storage.upsert(&data).await?;

    // Query similar vectors
    let query = vec![0.15; 1536];
    let results = storage.query(&query, 10, None).await?;

    for result in results {
        println!("ID: {}, Score: {:.4}", result.id, result.score);
    }

    // Save index to S3
    storage.finalize().await?;

    Ok(())
}
```

### Configuration

#### S3Config Options

- **bucket**: S3 bucket name (required)
- **namespace**: Storage namespace, typically workspace ID (required)
- **dimension**: Vector embedding dimension (required)
- **region**: AWS region (optional, uses default if not set)
- **hnsw_m**: HNSW connections per layer (default: 16)
- **hnsw_ef_construction**: HNSW construction parameter (default: 200)
- **hnsw_ef_search**: HNSW search parameter (default: 50)
- **batch_size**: Vectors per batch upload (default: 1000)
- **enable_compression**: Enable gzip compression (default: true)
- **cache_size**: LRU cache size (default: 10000 vectors)

#### HNSW Parameter Tuning

| Parameter | Description | Impact | Recommended |
|-----------|-------------|--------|-------------|
| **m** | Connections per layer | Higher = better recall, more memory | 16-32 |
| **ef_construction** | Construction search depth | Higher = better index quality, slower build | 200-400 |
| **ef_search** | Query search depth | Higher = better recall, slower query | 50-200 |

**Tuning Guidelines:**

- **High recall needed**: m=32, ef_construction=400, ef_search=200
- **Balanced (default)**: m=16, ef_construction=200, ef_search=50
- **Fast queries**: m=8, ef_construction=100, ef_search=20

### Performance

**Query Latency** (100K vectors, 1536 dimensions):

- Warm cache: 20-50ms
- Cold start: 100-500ms (Lambda cold start)
- Index rebuild: ~2 seconds per 10K vectors

**Throughput**:

- Upsert: 1000-5000 vectors/second (batched)
- Query: 100-500 queries/second (depending on ef_search)

### Examples

See `examples/` directory:

```bash
# Basic usage
cargo run --example s3_vector_basic --features s3-vectors

# Set custom bucket
S3_BUCKET=my-bucket cargo run --example s3_vector_basic --features s3-vectors

# Clear data after running
CLEAR_DATA=1 cargo run --example s3_vector_basic --features s3-vectors
```

### Testing

Integration tests require AWS credentials and an S3 bucket:

```bash
# Set test bucket
export TEST_S3_BUCKET=my-test-bucket

# Run tests
cargo test --features s3-vectors

# Run specific test
cargo test --features s3-vectors -- test_s3_vector_upsert_and_query
```

**Note**: Tests will create and clean up data in the test bucket.

## Athena SQL Queries

### Overview

Amazon Athena provides serverless SQL analytics over S3-stored data. Ideal for:
- Analytical queries and reporting
- Metadata searches
- Historical data analysis
- Ad-hoc data exploration

**Cost**: $5 per TB of data scanned (90%+ reduction with Parquet format)

### Usage

```rust
use edgequake_storage_aws::{AthenaConfig, AthenaQueryEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure Athena
    let config = AthenaConfig::new(
        "edgequake",                      // Glue Catalog database
        "s3://my-athena-results/"         // Query results location
    )
    .with_timeout(300)                    // 5 minutes
    .with_poll_interval(2)                // Poll every 2 seconds
    .with_result_cache(true);             // Enable result caching

    // Create engine
    let engine = AthenaQueryEngine::new(config).await?;

    // Execute query
    let results = engine.query(
        "SELECT * FROM documents WHERE workspace_id = 'ws-123' LIMIT 10"
    ).await?;

    // Process results
    for row in results {
        let id = row.get("id")?;
        let title = row.get("title")?;
        println!("{}: {}", id, title);
    }

    Ok(())
}
```

### Async Query Execution

```rust
// Start query (non-blocking)
let execution_id = engine.start_query(
    "SELECT COUNT(*) FROM documents WHERE status = 'completed'"
).await?;

// Do other work...

// Wait for completion
let stats = engine.wait_for_query(&execution_id).await?;
println!("Data scanned: {} bytes", stats.data_scanned_bytes);
println!("Estimated cost: ${:.6}", stats.estimated_cost_usd());

// Get results
let results = engine.get_query_results(&execution_id).await?;
```

### Query Builder

```rust
use edgequake_storage_aws::QueryBuilder;

let builder = QueryBuilder::new("edgequake");

// Pre-built queries
let query = builder.list_documents("ws-123", 50);
let query = builder.search_by_status("ws-123", "completed");
let query = builder.count_documents("ws-123");
let query = builder.workspace_stats();

let results = engine.query(&query).await?;
```

### Setup

1. **Create Glue Catalog database**:
   ```bash
   aws glue create-database --database-input '{"Name":"edgequake"}'
   ```

2. **Create S3 bucket for query results**:
   ```bash
   aws s3 mb s3://my-athena-results
   ```

3. **Create table** (example):
   ```sql
   CREATE EXTERNAL TABLE IF NOT EXISTS edgequake.documents (
     id STRING,
     workspace_id STRING,
     title STRING,
     content STRING,
     status STRING,
     created_at TIMESTAMP,
     size_bytes BIGINT
   )
   STORED AS PARQUET
   LOCATION 's3://my-bucket/documents/'
   TBLPROPERTIES ('parquet.compression'='SNAPPY');
   ```

### Cost Optimization

| Technique | Cost Reduction | Implementation |
|-----------|----------------|----------------|
| **Parquet format** | 90%+ | `STORED AS PARQUET` |
| **Partitioning** | 90%+ | `PARTITIONED BY (year, month)` |
| **Column selection** | 50-80% | `SELECT id, title` not `SELECT *` |
| **Compression** | 60-90% | `parquet.compression=SNAPPY` |
| **Result caching** | 100% | Reuses results for 24h |

**Example**: 100 queries/day × 50GB/query = 150TB/month = $750/month

With optimization: 100 queries/day × 5GB/query = 15TB/month = **$75/month** (90% savings!)

### Performance

| Query Type | Latency | Notes |
|------------|---------|-------|
| Simple SELECT | 1-3 seconds | Includes startup |
| Medium scan (100GB) | 5-15 seconds | Parallel execution |
| Large scan (1TB) | 30-60 seconds | Depends on complexity |
| Cached query | <1 second | Result reuse |

### Examples

See `examples/` directory:

```bash
# Basic usage
ATHENA_DATABASE=edgequake \
ATHENA_OUTPUT_LOCATION=s3://my-athena-results/ \
cargo run --example athena_sql_basic --features athena
```

### Testing

Integration tests require AWS credentials and Glue Catalog:

```bash
# Set test configuration
export TEST_ATHENA_DATABASE=edgequake_test
export TEST_ATHENA_OUTPUT_LOCATION=s3://test-athena-results/

# Run tests
cargo test --features athena -- --nocapture

# Run specific test
cargo test --features athena -- test_list_databases
```

**Note**: Tests require Glue Catalog database and S3 bucket for results.

### Complete Guide

See [ATHENA_GUIDE.md](ATHENA_GUIDE.md) for:
- Detailed setup instructions
- Cost analysis and optimization strategies
- Query patterns and best practices
- Security configuration
- Troubleshooting guide
- Integration examples with EdgeQuake

## Security

### IAM Permissions

Minimum required S3 permissions:

```json
{
    "Version": "2012-10-17",
    "Statement": [
        {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:PutObject",
                "s3:DeleteObject",
                "s3:ListBucket"
            ],
            "Resource": [
                "arn:aws:s3:::your-bucket/*",
                "arn:aws:s3:::your-bucket"
            ]
        }
    ]
}
```

### Best Practices

1. **Use separate buckets** per environment (dev/staging/prod)
2. **Enable versioning** on S3 buckets for data protection
3. **Configure lifecycle policies** to transition old data to Glacier
4. **Use encryption** at rest (SSE-S3 or SSE-KMS)
5. **Enable access logging** for audit trails
6. **Use VPC endpoints** to avoid internet routing

### Encryption

S3 encryption options:

```rust
// Enable server-side encryption (in AWS Console or via SDK)
// Option 1: SSE-S3 (AES-256, managed by AWS)
// Option 2: SSE-KMS (customer managed keys)

// Encryption is transparent to the application
let config = S3Config::new("encrypted-bucket", "workspace", 1536);
let storage = S3VectorStorage::new(config).await?;
// All data is automatically encrypted at rest
```

## Roadmap

### Implemented ✅

- [x] S3 vector storage with HNSW indexing
- [x] Compression support
- [x] LRU caching
- [x] Batch uploads
- [x] Query filtering
- [x] Integration tests
- [x] Neptune graph storage (Gremlin)
- [x] Gremlin query builder utilities
- [x] Neptune integration tests
- [x] DynamoDB key-value storage
- [x] Batch operations (get/put/delete)
- [x] Atomic status transitions
- [x] DynamoDB integration tests
- [x] Athena SQL query engine
- [x] Query builder utilities
- [x] Query statistics and cost estimation
- [x] Athena integration tests

### In Progress 🚧

- [ ] Multi-region replication

### Planned 📋

- [ ] Parquet format for vectors (more efficient than JSON)
- [ ] Multi-region replication
- [ ] Incremental HNSW updates (avoid full rebuild)
- [ ] S3 Select integration for metadata filtering
- [ ] CloudWatch metrics and alarms
- [ ] Cost monitoring dashboard

## Benchmarks

Run benchmarks:

```bash
cargo bench --features s3-vectors
```

## Migration from PostgreSQL

See the [migration guide](../../AWS_MIGRATION_DESIGN.md) for step-by-step instructions on migrating from PostgreSQL to AWS storage.

Key steps:

1. Deploy AWS infrastructure (S3 bucket, IAM roles)
2. Implement dual-write to both PostgreSQL and S3
3. Bulk export existing data to S3
4. Validate data consistency
5. Switch reads to S3
6. Decommission PostgreSQL

Estimated timeline: 8-12 weeks

## Troubleshooting

### Common Issues

**1. "Cannot access bucket" error**

- Check AWS credentials: `aws s3 ls s3://your-bucket`
- Verify IAM permissions
- Ensure bucket exists in the correct region

**2. Slow queries**

- Increase `ef_search` parameter
- Check if index is loaded: should see "Loaded HNSW index" in logs
- Enable compression to reduce S3 transfer time

**3. High costs**

- Review S3 access logs for unexpected traffic
- Configure lifecycle policies to move old data to Glacier
- Use Parquet format instead of JSON (coming soon)

**4. Memory issues**

- Reduce `cache_size` if running on small instances
- Consider using Lambda with more memory
- Use streaming for large index loads

### Debug Logging

Enable detailed logs:

```bash
RUST_LOG=edgequake_storage_aws=debug cargo run --example s3_vector_basic
```

## Contributing

Contributions welcome! Please see [CONTRIBUTING.md](../../CONTRIBUTING.md).

### Development Setup

```bash
# Clone repository
git clone https://github.com/edgequake/edgequake.git
cd edgequake/crates/edgequake-storage-aws

# Run tests (requires AWS credentials)
export TEST_S3_BUCKET=test-bucket
cargo test --features s3-vectors

# Run examples
cargo run --example s3_vector_basic --features s3-vectors

# Run benchmarks
cargo bench --features s3-vectors
```

## License

Dual-licensed under MIT OR Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).

## Related Projects

- [EdgeQuake Core](../edgequake-core) - Core RAG types and traits
- [EdgeQuake Storage](../edgequake-storage) - Storage trait definitions
- [EdgeQuake API](../edgequake-api) - REST API server

## Support

- 📖 [Documentation](https://docs.edgequake.io)
- 💬 [Discussions](https://github.com/edgequake/edgequake/discussions)
- 🐛 [Issue Tracker](https://github.com/edgequake/edgequake/issues)
- 📧 Email: support@edgequake.io
