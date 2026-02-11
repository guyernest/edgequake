# Amazon Athena SQL Query Engine Guide

Complete guide for using Amazon Athena with EdgeQuake for serverless SQL analytics over S3 data.

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Architecture](#architecture)
- [Setup](#setup)
- [Cost Analysis](#cost-analysis)
- [Usage Examples](#usage-examples)
- [Query Optimization](#query-optimization)
- [Security](#security)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)

## Overview

Amazon Athena is a serverless, interactive query service that enables you to analyze data in Amazon S3 using standard SQL. Athena is ideal for:

- **Analytical queries** over large datasets
- **Metadata searches** and filtering
- **Reporting and batch processing**
- **Ad-hoc data exploration**
- **Cost-effective analytics** (pay-per-query)

### Key Features

- ✅ **Serverless** - no infrastructure to manage
- ✅ **Standard SQL** - ANSI SQL support (Presto/Trino based)
- ✅ **Pay-per-query** - $5 per TB of data scanned
- ✅ **Fast** - parallel query execution
- ✅ **Integrated** - works with S3, Glue Catalog, CloudWatch
- ✅ **Secure** - IAM, encryption, VPC support

### When to Use Athena

| Use Case | Athena | DynamoDB | PostgreSQL |
|----------|--------|----------|------------|
| Ad-hoc analytics | ✅ Perfect | ❌ Not suitable | ✅ Works but expensive |
| Real-time queries | ❌ Too slow (100ms-1s+) | ✅ Single-digit ms | ✅ Fast (10-50ms) |
| Historical analysis | ✅ Cost-effective | ❌ Expensive at scale | ✅ Works but slow |
| Reporting/dashboards | ✅ Ideal | ❌ Limited | ✅ Works |
| Large dataset scans | ✅ Scales easily | ⚠️ Expensive | ⚠️ Slow |
| Complex joins | ✅ Full SQL support | ❌ Limited | ✅ Full SQL |

**Recommendation**: Use Athena for analytics and reporting, DynamoDB for hot path, S3 for vectors.

## Quick Start

### 1. Install

Add to your `Cargo.toml`:

```toml
[dependencies]
edgequake-storage-aws = { version = "0.1", features = ["athena"] }
```

### 2. Create Glue Catalog Database

```bash
aws glue create-database --database-input '{"Name":"edgequake"}'
```

### 3. Create S3 Bucket for Query Results

```bash
aws s3 mb s3://my-athena-results
```

### 4. Basic Usage

```rust
use edgequake_storage_aws::{AthenaConfig, AthenaQueryEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure Athena
    let config = AthenaConfig::new(
        "edgequake",                          // Database name
        "s3://my-athena-results/"             // Results location
    );

    // Create engine
    let engine = AthenaQueryEngine::new(config).await?;

    // Execute query
    let results = engine.query("SELECT * FROM documents LIMIT 10").await?;

    // Process results
    for row in results {
        let id = row.get("id")?;
        let title = row.get("title")?;
        println!("{}: {}", id, title);
    }

    Ok(())
}
```

### 5. Run Example

```bash
ATHENA_DATABASE=edgequake \
ATHENA_OUTPUT_LOCATION=s3://my-athena-results/ \
cargo run --example athena_sql_basic --features athena
```

## Architecture

### Data Flow

```
┌─────────────────────────────────────────────────────────┐
│                    S3 Storage                            │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │   Vectors    │  │  Documents   │  │   Metadata   │  │
│  │  (Parquet)   │  │  (Parquet)   │  │    (JSON)    │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
└─────────────┬───────────────────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────────────────┐
│              AWS Glue Data Catalog                       │
│  ┌──────────────────────────────────────────────────┐   │
│  │  Table Definitions (Schema, Partitions, Stats)   │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────┬───────────────────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────────────────┐
│                   Amazon Athena                          │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐              │
│  │  Query   │  │  Query   │  │  Query   │              │
│  │ Engine 1 │  │ Engine 2 │  │ Engine 3 │  (Parallel)  │
│  └──────────┘  └──────────┘  └──────────┘              │
└─────────────┬───────────────────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────────────────┐
│             S3 Query Results                             │
│  (CSV/JSON output stored in results bucket)             │
└─────────────────────────────────────────────────────────┘
```

### Integration with EdgeQuake

```rust
// EdgeQuake Storage Architecture with Athena

┌──────────────────────────────────────────────┐
│         EdgeQuake Application                │
└───────┬──────────┬──────────┬────────────────┘
        │          │          │
        ▼          ▼          ▼
  ┌─────────┐ ┌────────┐ ┌──────────┐
  │   S3    │ │Neptune │ │ DynamoDB │
  │ Vectors │ │ Graph  │ │    KV    │
  └────┬────┘ └────────┘ └──────────┘
       │
       │ (SQL Analytics)
       ▼
  ┌─────────┐
  │ Athena  │
  │  Query  │
  └─────────┘
```

## Setup

### 1. Create Glue Catalog Database

Using AWS CLI:

```bash
aws glue create-database \
    --database-input '{
        "Name": "edgequake",
        "Description": "EdgeQuake RAG data catalog"
    }'
```

Using AWS Console:
1. Go to AWS Glue → Databases
2. Click "Add database"
3. Name: `edgequake`
4. Click "Create"

### 2. Create Tables

#### Documents Table

Create a table pointing to your S3 documents:

```sql
CREATE EXTERNAL TABLE IF NOT EXISTS edgequake.documents (
  id STRING,
  workspace_id STRING,
  title STRING,
  content STRING,
  status STRING,
  created_at TIMESTAMP,
  updated_at TIMESTAMP,
  size_bytes BIGINT,
  metadata STRING
)
PARTITIONED BY (
  year STRING,
  month STRING,
  day STRING
)
STORED AS PARQUET
LOCATION 's3://your-bucket/documents/'
TBLPROPERTIES (
  'parquet.compression'='SNAPPY',
  'projection.enabled'='true',
  'projection.year.type'='integer',
  'projection.year.range'='2020,2030',
  'projection.month.type'='integer',
  'projection.month.range'='1,12',
  'projection.month.digits'='2',
  'projection.day.type'='integer',
  'projection.day.range'='1,31',
  'projection.day.digits'='2'
);
```

#### Vectors Metadata Table

```sql
CREATE EXTERNAL TABLE IF NOT EXISTS edgequake.vectors (
  id STRING,
  namespace STRING,
  document_id STRING,
  chunk_index INT,
  embedding_model STRING,
  metadata STRING,
  created_at TIMESTAMP
)
PARTITIONED BY (namespace STRING)
STORED AS PARQUET
LOCATION 's3://your-bucket/vectors/metadata/'
TBLPROPERTIES ('parquet.compression'='SNAPPY');
```

#### Workspace Statistics Table

```sql
CREATE EXTERNAL TABLE IF NOT EXISTS edgequake.workspace_stats (
  workspace_id STRING,
  date DATE,
  doc_count BIGINT,
  vector_count BIGINT,
  total_size_bytes BIGINT,
  query_count BIGINT
)
STORED AS PARQUET
LOCATION 's3://your-bucket/stats/'
TBLPROPERTIES ('parquet.compression'='SNAPPY');
```

### 3. Create Partitions

Load partitions for existing data:

```sql
-- For documents table
MSCK REPAIR TABLE edgequake.documents;

-- For vectors table
MSCK REPAIR TABLE edgequake.vectors;
```

Or add partitions manually:

```sql
ALTER TABLE edgequake.documents
ADD PARTITION (year='2024', month='01', day='15')
LOCATION 's3://your-bucket/documents/2024/01/15/';
```

### 4. Configure IAM Permissions

Create an IAM policy for Athena access:

```json
{
    "Version": "2012-10-17",
    "Statement": [
        {
            "Effect": "Allow",
            "Action": [
                "athena:StartQueryExecution",
                "athena:GetQueryExecution",
                "athena:GetQueryResults",
                "athena:StopQueryExecution",
                "athena:GetWorkGroup"
            ],
            "Resource": [
                "arn:aws:athena:*:*:workgroup/*"
            ]
        },
        {
            "Effect": "Allow",
            "Action": [
                "glue:GetDatabase",
                "glue:GetTable",
                "glue:GetPartitions"
            ],
            "Resource": [
                "arn:aws:glue:*:*:catalog",
                "arn:aws:glue:*:*:database/edgequake",
                "arn:aws:glue:*:*:table/edgequake/*"
            ]
        },
        {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:ListBucket"
            ],
            "Resource": [
                "arn:aws:s3:::your-bucket/*",
                "arn:aws:s3:::your-bucket"
            ]
        },
        {
            "Effect": "Allow",
            "Action": [
                "s3:PutObject",
                "s3:GetObject",
                "s3:ListBucket"
            ],
            "Resource": [
                "arn:aws:s3:::your-athena-results/*",
                "arn:aws:s3:::your-athena-results"
            ]
        }
    ]
}
```

### 5. Configure Workgroup (Optional)

Create a custom workgroup for cost control:

```bash
aws athena create-work-group \
    --name edgequake-workgroup \
    --configuration '{
        "ResultConfigurationUpdates": {
            "OutputLocation": "s3://your-athena-results/",
            "EncryptionConfiguration": {
                "EncryptionOption": "SSE_S3"
            }
        },
        "EnforceWorkGroupConfiguration": true,
        "PublishCloudWatchMetricsEnabled": true
    }' \
    --description "EdgeQuake analytics workgroup"
```

## Cost Analysis

### Pricing Model

Athena charges **$5 per TB of data scanned**. No infrastructure costs.

### Cost Calculation

```
Cost = (Data Scanned in TB) × $5
```

### Example Costs

| Scenario | Data Scanned | Cost |
|----------|--------------|------|
| Small query (10 GB) | 0.01 TB | $0.05 |
| Medium query (100 GB) | 0.1 TB | $0.50 |
| Large query (1 TB) | 1 TB | $5.00 |
| Very large (10 TB) | 10 TB | $50.00 |

### Monthly Cost Examples

**Scenario 1: Light Analytics**
- 100 queries/day
- Average 50 GB scanned per query
- Monthly: 100 × 30 × 50 GB = 150 TB
- **Cost: $750/month**

**Scenario 2: Optimized Analytics (Parquet + Partitions)**
- Same 100 queries/day
- Parquet reduces scans by 90%: 5 GB per query
- Monthly: 100 × 30 × 5 GB = 15 TB
- **Cost: $75/month** (90% savings!)

**Scenario 3: Ad-hoc Queries**
- 10 queries/day
- Average 100 GB scanned
- Monthly: 10 × 30 × 100 GB = 30 TB
- **Cost: $150/month**

### Cost Optimization Strategies

1. **Use Parquet format**: 90%+ reduction vs JSON/CSV
2. **Partition tables**: Only scan relevant partitions
3. **Use compression**: SNAPPY or GZIP
4. **Limit columns**: SELECT specific columns, not SELECT *
5. **Use WHERE clauses**: Filter early
6. **Enable result caching**: Reuse query results
7. **Optimize file sizes**: 128-512 MB files ideal

### Cost Comparison

| Storage | Query Cost | Infrastructure | Total (1000 queries/month, 50GB each) |
|---------|------------|----------------|----------------------------------------|
| **Athena + S3** | $250 (50TB × $5) | $0 | **$250/month** |
| **PostgreSQL RDS** | $0 (included) | $300 (db.r5.large) | **$300/month** |
| **BigQuery** | $250 (50TB × $5) | $0 | **$250/month** |
| **Redshift** | $0 (included) | $1,440 (dc2.large) | **$1,440/month** |

**Winner**: Athena for infrequent analytics, PostgreSQL for frequent queries.

## Usage Examples

### Basic Configuration

```rust
use edgequake_storage_aws::{AthenaConfig, AthenaQueryEngine};

let config = AthenaConfig::new(
    "edgequake",                      // Database
    "s3://my-athena-results/"         // Results location
)
.with_timeout(300)                    // 5 minutes
.with_poll_interval(2)                // Check every 2 seconds
.with_workgroup("edgequake-workgroup")
.with_result_cache(true);

let engine = AthenaQueryEngine::new(config).await?;
```

### Simple Queries

```rust
// List all documents
let results = engine.query(
    "SELECT * FROM documents WHERE workspace_id = 'ws-123' LIMIT 100"
).await?;

for row in results {
    let id = row.get("id")?;
    let title = row.get("title")?;
    println!("{}: {}", id, title);
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
println!("Scanned: {} bytes", stats.data_scanned_bytes);
println!("Cost: ${:.6}", stats.estimated_cost_usd());

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

### Parallel Queries

```rust
// Start multiple queries concurrently
let q1 = engine.start_query("SELECT COUNT(*) FROM documents");
let q2 = engine.start_query("SELECT COUNT(*) FROM vectors");
let q3 = engine.start_query("SELECT COUNT(*) FROM workspace_stats");

let (id1, id2, id3) = tokio::try_join!(q1, q2, q3)?;

// Wait for all
let (s1, s2, s3) = tokio::try_join!(
    engine.wait_for_query(&id1),
    engine.wait_for_query(&id2),
    engine.wait_for_query(&id3)
)?;

println!("Total cost: ${:.6}",
    s1.estimated_cost_usd() + s2.estimated_cost_usd() + s3.estimated_cost_usd()
);
```

### Parsing Results

```rust
let results = engine.query("SELECT id, count, price, active FROM products").await?;

for row in results {
    // String
    let id: String = row.get("id")?;

    // Integer
    let count: i64 = row.get_i64("count")?;

    // Float
    let price: f64 = row.get_f64("price")?;

    // Boolean
    let active: bool = row.get_bool("active")?;

    // JSON
    let json = row.to_json();
    println!("{}", serde_json::to_string_pretty(&json)?);
}
```

## Query Optimization

### 1. Use Parquet Format

**Before (JSON):**
```sql
-- Scans entire file (1 GB)
SELECT title FROM documents WHERE id = 'doc-123';
```

**After (Parquet):**
```sql
-- Scans only title column (10 MB, 99% reduction)
SELECT title FROM documents WHERE id = 'doc-123';
```

**Savings**: 90-99% cost reduction

### 2. Partition Tables

**Before (No partitions):**
```sql
-- Scans entire table (1 TB)
SELECT * FROM documents
WHERE created_at > '2024-01-01';
```

**After (Date partitions):**
```sql
-- Scans only January partition (30 GB, 97% reduction)
SELECT * FROM documents
WHERE year = '2024' AND month = '01';
```

**Savings**: 90-99% cost reduction

### 3. Select Specific Columns

**Before:**
```sql
-- Scans all columns
SELECT * FROM documents WHERE id = 'doc-123';
```

**After:**
```sql
-- Scans only needed columns
SELECT id, title, status FROM documents WHERE id = 'doc-123';
```

**Savings**: 50-80% cost reduction

### 4. Use Compression

```sql
CREATE TABLE documents (...)
STORED AS PARQUET
TBLPROPERTIES (
    'parquet.compression'='SNAPPY'  -- or GZIP, ZSTD
);
```

**Savings**: 60-90% storage and scan cost reduction

### 5. Optimize File Sizes

- **Too small** (<10 MB): High S3 API overhead
- **Too large** (>1 GB): Can't parallelize well
- **Ideal**: 128-512 MB files

### 6. Use CTAS (Create Table As Select)

Pre-compute common queries:

```sql
CREATE TABLE workspace_summary
WITH (
    format = 'PARQUET',
    parquet_compression = 'SNAPPY',
    external_location = 's3://your-bucket/summaries/'
) AS
SELECT
    workspace_id,
    COUNT(*) as doc_count,
    SUM(size_bytes) as total_size
FROM documents
GROUP BY workspace_id;
```

Now query the summary instead of raw data!

### 7. Enable Result Caching

```rust
let config = AthenaConfig::new(db, location)
    .with_result_cache(true);  // Cache results for 24 hours
```

Subsequent identical queries are free!

## Security

### 1. IAM Permissions

Principle of least privilege:

```json
{
    "Version": "2012-10-17",
    "Statement": [
        {
            "Effect": "Allow",
            "Action": [
                "athena:StartQueryExecution",
                "athena:GetQueryExecution",
                "athena:GetQueryResults"
            ],
            "Resource": "arn:aws:athena:*:*:workgroup/edgequake-workgroup",
            "Condition": {
                "StringEquals": {
                    "athena:database": "edgequake"
                }
            }
        }
    ]
}
```

### 2. Encryption

Enable encryption for query results:

```rust
// Results are encrypted in S3
// Configure in workgroup or query execution
```

In workgroup:
```bash
aws athena update-work-group \
    --work-group edgequake-workgroup \
    --configuration '{
        "ResultConfigurationUpdates": {
            "EncryptionConfiguration": {
                "EncryptionOption": "SSE_KMS",
                "KmsKey": "arn:aws:kms:us-east-1:123456789012:key/..."
            }
        }
    }'
```

### 3. VPC Endpoints

Use VPC endpoints to keep traffic private:

```bash
aws ec2 create-vpc-endpoint \
    --vpc-id vpc-xxx \
    --service-name com.amazonaws.us-east-1.athena \
    --route-table-ids rtb-xxx
```

### 4. CloudTrail Logging

Enable audit logging:

```bash
aws cloudtrail create-trail \
    --name athena-audit \
    --s3-bucket-name audit-logs \
    --include-global-service-events \
    --is-multi-region-trail
```

### 5. Row-Level Security

Use views with IAM conditions:

```sql
CREATE VIEW workspace_documents AS
SELECT *
FROM documents
WHERE workspace_id = current_user;  -- Simplified example
```

## Best Practices

### 1. Data Organization

```
s3://bucket/
├── documents/
│   └── workspace_id=ws-123/
│       └── year=2024/
│           └── month=01/
│               └── day=15/
│                   └── batch-1234.parquet
└── vectors/
    └── namespace=ws-123/
        └── batch-5678.parquet
```

### 2. Naming Conventions

- Use lowercase for column names
- Avoid special characters
- Be consistent with date formats
- Use semantic names

### 3. Query Patterns

```sql
-- ✅ GOOD: Specific columns, partitioned
SELECT id, title
FROM documents
WHERE workspace_id = 'ws-123'
  AND year = '2024'
  AND month = '01';

-- ❌ BAD: SELECT *, no partition filters
SELECT * FROM documents WHERE title LIKE '%test%';
```

### 4. Monitoring

Set up CloudWatch alarms:

```bash
aws cloudwatch put-metric-alarm \
    --alarm-name athena-high-cost \
    --metric-name DataScannedInBytes \
    --namespace AWS/Athena \
    --statistic Sum \
    --period 86400 \
    --threshold 1099511627776 \  # 1 TB
    --comparison-operator GreaterThanThreshold
```

### 5. Cost Controls

Use workgroup limits:

```bash
aws athena update-work-group \
    --work-group edgequake-workgroup \
    --configuration '{
        "BytesScannedCutoffPerQuery": 10737418240  # 10 GB limit
    }'
```

### 6. Testing

Test queries on small datasets first:

```sql
-- Test with LIMIT
SELECT * FROM documents LIMIT 10;

-- Check what will be scanned
EXPLAIN SELECT * FROM documents WHERE workspace_id = 'ws-123';
```

## Troubleshooting

### Query Timeout

**Problem**: Query times out after 300 seconds

**Solutions**:
- Increase timeout: `config.with_timeout(600)`
- Optimize query (add partitions, reduce data scanned)
- Use LIMIT for testing
- Break into smaller queries

### High Costs

**Problem**: Athena bill is too high

**Solutions**:
1. Check what's being scanned:
   ```sql
   -- Use EXPLAIN to see query plan
   EXPLAIN SELECT * FROM documents;
   ```

2. Add partitions to filter data:
   ```sql
   WHERE year = '2024' AND month = '01'
   ```

3. Convert to Parquet:
   ```sql
   CREATE TABLE documents_parquet
   WITH (format='PARQUET', parquet_compression='SNAPPY')
   AS SELECT * FROM documents_json;
   ```

4. Use specific columns:
   ```sql
   SELECT id, title  -- Not SELECT *
   ```

### Query Fails

**Problem**: Query returns error

**Common errors**:

1. **SYNTAX_ERROR**: Invalid SQL
   - Check SQL syntax
   - Use standard SQL (not PostgreSQL-specific)

2. **PERMISSION_DENIED**: Insufficient permissions
   - Check IAM policy
   - Verify S3 bucket permissions

3. **SCHEMA_MISMATCH**: Column type mismatch
   - Check table schema
   - Cast types explicitly: `CAST(column AS INTEGER)`

4. **RESOURCE_NOT_FOUND**: Table/database doesn't exist
   - Verify database and table names
   - Run `SHOW TABLES` to list tables

### Slow Queries

**Problem**: Query takes minutes to complete

**Solutions**:
- Add partition filters
- Reduce data scanned
- Use columnar format (Parquet)
- Optimize file sizes (128-512 MB)
- Use result caching
- Pre-aggregate with CTAS

### Column Not Found

**Problem**: "Column 'X' not found"

**Solutions**:
- Check column name spelling
- Athena is case-sensitive for column names
- Use lowercase consistently
- Run `DESCRIBE table_name` to see schema

## Integration with EdgeQuake

### Document Analytics

```rust
// Query document statistics
let query = format!(
    "SELECT
        status,
        COUNT(*) as count,
        SUM(size_bytes) as total_bytes,
        AVG(size_bytes) as avg_bytes
     FROM documents
     WHERE workspace_id = '{}'
     GROUP BY status",
    workspace_id
);

let results = engine.query(&query).await?;
for row in results {
    let status = row.get("status")?;
    let count = row.get_i64("count")?;
    let total_bytes = row.get_i64("total_bytes")?;
    println!("{}: {} docs, {} bytes", status, count, total_bytes);
}
```

### Vector Metadata Search

```rust
// Find vectors by metadata
let query = format!(
    "SELECT id, document_id, metadata
     FROM vectors
     WHERE namespace = '{}'
       AND metadata LIKE '%\"category\":\"science\"%'
     LIMIT 100",
    namespace
);

let results = engine.query(&query).await?;
```

### Workspace Reports

```rust
// Generate workspace report
let query = "
    SELECT
        workspace_id,
        COUNT(DISTINCT id) as doc_count,
        SUM(size_bytes) as total_size,
        MAX(created_at) as last_activity
    FROM documents
    GROUP BY workspace_id
    ORDER BY total_size DESC
";

let results = engine.query(query).await?;
```

## Conclusion

Amazon Athena provides cost-effective SQL analytics for EdgeQuake:

- ✅ **Serverless**: No infrastructure management
- ✅ **Scalable**: Query petabytes of data
- ✅ **Cost-effective**: Pay only for data scanned
- ✅ **Fast**: Parallel query execution
- ✅ **Integrated**: Works with S3, Glue, CloudWatch

**Best for**: Analytics, reporting, ad-hoc queries, historical analysis

**Not ideal for**: Real-time queries, transactional workloads, sub-second latency

## Resources

- [Amazon Athena Documentation](https://docs.aws.amazon.com/athena/)
- [Athena Pricing](https://aws.amazon.com/athena/pricing/)
- [Parquet Format](https://parquet.apache.org/)
- [Glue Catalog](https://docs.aws.amazon.com/glue/latest/dg/catalog-and-crawler.html)
- [Query Optimization](https://docs.aws.amazon.com/athena/latest/ug/performance-tuning.html)
- [EdgeQuake Documentation](https://docs.edgequake.io)

---

**Need help?** Open an issue at [github.com/edgequake/edgequake](https://github.com/edgequake/edgequake/issues)
