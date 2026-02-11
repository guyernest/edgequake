# DynamoDB Key-Value Storage Guide

Complete guide for using Amazon DynamoDB with EdgeQuake.

## Table of Contents

- [Overview](#overview)
- [Cost Analysis](#cost-analysis)
- [Setup Guide](#setup-guide)
- [Usage](#usage)
- [Performance](#performance)
- [Security](#security)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)

## Overview

Amazon DynamoDB is a fully managed NoSQL database service that provides fast and predictable performance with seamless scalability. EdgeQuake uses DynamoDB for key-value storage of documents, metadata, and cache entries.

### Key Features

- **Low Latency**: Single-digit millisecond response times
- **Auto-scaling**: Scales automatically with demand
- **Pay-per-request**: No idle costs with on-demand mode
- **Durable**: 99.999999999% (11 nines) durability
- **Global Tables**: Multi-region replication
- **Point-in-time Recovery**: Continuous backups
- **ACID**: Transactions across multiple items

### When to Use DynamoDB

✅ **Good For:**
- Hot data requiring low latency (< 10ms)
- Document metadata and cache
- Session data
- User profiles
- Frequently accessed key-value pairs

❌ **Not Ideal For:**
- Cold/archival data (use S3 + Athena)
- Complex queries (use Neptune for graphs)
- Large objects > 400KB (use S3)
- Ad-hoc analytics (use Athena)

## Cost Analysis

### DynamoDB Pricing

**On-Demand Mode** (Recommended for EdgeQuake):

| Operation | Price |
|-----------|-------|
| **Write Requests** | $1.25 per million |
| **Read Requests** | $0.25 per million |
| **Storage** | $0.25/GB-month |
| **Backup Storage** | $0.20/GB-month |
| **Data Transfer (out)** | $0.09/GB |

**Provisioned Mode** (For predictable workloads):

| Capacity | Price |
|----------|-------|
| **Write Capacity Unit (WCU)** | $0.00065/hour/WCU |
| **Read Capacity Unit (RCU)** | $0.00013/hour/RCU |

### Cost Comparison: PostgreSQL vs DynamoDB

**Scenario: 100K documents, 1M reads/day, 100K writes/day**

| Component | PostgreSQL RDS | DynamoDB On-Demand | Savings |
|-----------|---------------|-------------------|---------|
| **Compute** | $73/month (db.t3.medium) | $0 (serverless) | 100% |
| **Storage (10GB)** | $1.15/month | $2.50/month | - |
| **Requests** | Included | $37.50/month + $7.50/month | - |
| **Backup** | $1.15/month | $2.00/month | - |
| **Total** | **$75.30/month** | **$49.50/month** | **34%** |

**Annual savings**: ~$310/year

### Cost at Scale

**10x Growth (1M documents, 10M reads/day, 1M writes/day):**

| Architecture | Monthly Cost |
|-------------|--------------|
| PostgreSQL RDS (r5.large) | $250-300 |
| DynamoDB On-Demand | $100-150 |
| **Savings** | **50-60%** |

### Cost Optimization Tips

1. **Use on-demand for variable workloads** (dev, staging, low traffic)
2. **Use provisioned for steady workloads** (production with predictable patterns)
3. **Enable TTL** to auto-delete expired items
4. **Use batch operations** (25 items per request)
5. **Optimize item size** (smaller items = lower costs)
6. **Archive old data to S3** (use lifecycle policies)

## Setup Guide

### Option 1: AWS CLI

#### Step 1: Create Table

```bash
aws dynamodb create-table \
    --table-name edgequake-kv \
    --attribute-definitions \
        AttributeName=namespace,AttributeType=S \
        AttributeName=id,AttributeType=S \
    --key-schema \
        AttributeName=namespace,KeyType=HASH \
        AttributeName=id,KeyType=RANGE \
    --billing-mode PAY_PER_REQUEST \
    --tags Key=Environment,Value=production Key=Application,Value=EdgeQuake
```

**Table Design:**
- **Partition Key**: `namespace` (String) - Isolates workspaces
- **Sort Key**: `id` (String) - Unique identifier within namespace

#### Step 2: Enable Point-in-Time Recovery

```bash
aws dynamodb update-continuous-backups \
    --table-name edgequake-kv \
    --point-in-time-recovery-specification PointInTimeRecoveryEnabled=true
```

#### Step 3: Enable Encryption (Optional but Recommended)

```bash
aws dynamodb update-table \
    --table-name edgequake-kv \
    --sse-specification Enabled=true,SSEType=KMS
```

#### Step 4: Configure Auto-scaling (Provisioned Mode Only)

```bash
# Register scalable target
aws application-autoscaling register-scalable-target \
    --service-namespace dynamodb \
    --resource-id table/edgequake-kv \
    --scalable-dimension dynamodb:table:ReadCapacityUnits \
    --min-capacity 5 \
    --max-capacity 100

# Create scaling policy
aws application-autoscaling put-scaling-policy \
    --service-namespace dynamodb \
    --resource-id table/edgequake-kv \
    --scalable-dimension dynamodb:table:ReadCapacityUnits \
    --policy-name ReadAutoScalingPolicy \
    --policy-type TargetTrackingScaling \
    --target-tracking-scaling-policy-configuration file://scaling-policy.json
```

### Option 2: AWS CDK (TypeScript)

```typescript
import * as cdk from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';

export class EdgeQuakeDynamoStack extends cdk.Stack {
  constructor(scope: cdk.App, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    // Create DynamoDB table
    const table = new dynamodb.Table(this, 'EdgeQuakeKV', {
      tableName: 'edgequake-kv',
      partitionKey: {
        name: 'namespace',
        type: dynamodb.AttributeType.STRING,
      },
      sortKey: {
        name: 'id',
        type: dynamodb.AttributeType.STRING,
      },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      pointInTimeRecovery: true,
      encryption: dynamodb.TableEncryption.AWS_MANAGED,
      removalPolicy: cdk.RemovalPolicy.RETAIN,
    });

    // Add GSI for status queries (optional)
    table.addGlobalSecondaryIndex({
      indexName: 'StatusIndex',
      partitionKey: {
        name: 'namespace',
        type: dynamodb.AttributeType.STRING,
      },
      sortKey: {
        name: 'status',
        type: dynamodb.AttributeType.STRING,
      },
      projectionType: dynamodb.ProjectionType.ALL,
    });

    // Output table name
    new cdk.CfnOutput(this, 'TableName', {
      value: table.tableName,
    });
  }
}
```

### Option 3: Terraform

```hcl
resource "aws_dynamodb_table" "edgequake_kv" {
  name           = "edgequake-kv"
  billing_mode   = "PAY_PER_REQUEST"
  hash_key       = "namespace"
  range_key      = "id"

  attribute {
    name = "namespace"
    type = "S"
  }

  attribute {
    name = "id"
    type = "S"
  }

  attribute {
    name = "status"
    type = "S"
  }

  # Global Secondary Index for status queries
  global_secondary_index {
    name            = "StatusIndex"
    hash_key        = "namespace"
    range_key       = "status"
    projection_type = "ALL"
  }

  point_in_time_recovery {
    enabled = true
  }

  server_side_encryption {
    enabled     = true
    kms_key_arn = aws_kms_key.dynamodb.arn
  }

  tags = {
    Environment = "production"
    Application = "EdgeQuake"
  }
}

resource "aws_kms_key" "dynamodb" {
  description             = "KMS key for DynamoDB encryption"
  deletion_window_in_days = 10
}

output "table_name" {
  value = aws_dynamodb_table.edgequake_kv.name
}
```

## Usage

### Basic Example

```rust
use edgequake_storage::KVStorage;
use edgequake_storage_aws::{DynamoKVConfig, DynamoKVStorage};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure
    let config = DynamoKVConfig::new("edgequake-kv", "workspace-123")
        .with_on_demand(true)
        .with_pitr(true);

    // Create storage
    let storage = DynamoKVStorage::new(config).await?;
    storage.initialize().await?;

    // Insert data
    let data = vec![
        ("doc-001".to_string(), json!({"title": "Document 1", "author": "Alice"}))
    ];
    storage.upsert(&data).await?;

    // Get data
    let result = storage.get_by_id("doc-001").await?;
    if let Some(doc) = result {
        println!("Title: {}", doc["title"]);
    }

    // Update data
    let updated = vec![
        ("doc-001".to_string(), json!({"title": "Document 1 (Updated)", "author": "Alice", "updated": true}))
    ];
    storage.upsert(&updated).await?;

    // Delete data
    storage.delete(&["doc-001".to_string()]).await?;

    Ok(())
}
```

### Configuration Options

```rust
let config = DynamoKVConfig::new("table-name", "namespace")
    .with_region("us-east-1")      // AWS region
    .with_on_demand(true)          // Pay-per-request (recommended)
    .with_capacity(10, 10)         // Provisioned: 10 RCUs, 10 WCUs
    .with_pitr(true);              // Point-in-time recovery
```

### Batch Operations

```rust
// Batch insert (max 25 items per request)
let data: Vec<_> = (0..100)
    .map(|i| (format!("item-{}", i), json!({"index": i})))
    .collect();

storage.upsert(&data).await?;  // Automatically chunked

// Batch read (max 100 items per request)
let ids: Vec<_> = (0..100).map(|i| format!("item-{}", i)).collect();
let results = storage.get_by_ids(&ids).await?;  // Automatically chunked

// Batch delete (max 25 items per request)
storage.delete(&ids).await?;  // Automatically chunked
```

### Atomic Status Transitions

```rust
// Prevent race conditions with conditional updates
let doc = vec![
    ("task-001".to_string(), json!({"name": "Process", "status": "pending"}))
];
storage.upsert(&doc).await?;

// Atomically transition status
let success = storage
    .transition_if_status("task-001", "pending", "processing")
    .await?;

if success {
    // Status was updated
    println!("Started processing");
} else {
    // Status changed (another process started it)
    println!("Already processing");
}
```

### Filter Keys (Deduplication)

```rust
use std::collections::HashSet;

// Check which documents need to be inserted
let check_keys: HashSet<_> = vec![
    "doc-001".to_string(),
    "doc-002".to_string(),
    "doc-003".to_string(),
].into_iter().collect();

let missing = storage.filter_keys(check_keys).await?;

// Only insert missing documents
for key in missing {
    storage.upsert(&[(key, json!({"new": true}))]).await?;
}
```

### Running Examples

```bash
# Set table name
export DYNAMODB_TABLE=edgequake-kv

# Run basic example
cargo run --example dynamodb_kv_basic --features dynamodb

# Clear data before running
CLEAR_DATA=1 cargo run --example dynamodb_kv_basic --features dynamodb
```

### Running Tests

```bash
# Create test table first
aws dynamodb create-table \
    --table-name edgequake-test-kv \
    --attribute-definitions \
        AttributeName=namespace,AttributeType=S \
        AttributeName=id,AttributeType=S \
    --key-schema \
        AttributeName=namespace,KeyType=HASH \
        AttributeName=id,KeyType=RANGE \
    --billing-mode PAY_PER_REQUEST

# Run tests
export DYNAMODB_TABLE=edgequake-test-kv
cargo test --features dynamodb -- --test-threads=1

# Clean up test table
aws dynamodb delete-table --table-name edgequake-test-kv
```

## Performance

### Latency

| Operation | Average Latency | Notes |
|-----------|----------------|-------|
| GetItem | 1-5ms | Single item read |
| PutItem | 5-10ms | Single item write |
| BatchGetItem (25) | 10-30ms | Batch read |
| BatchWriteItem (25) | 20-50ms | Batch write |
| Query | 5-20ms | Partition query |
| Scan | 100ms-10s | Full table scan (avoid!) |

### Throughput

**On-Demand Mode:**
- **Reads**: Up to 40,000 RCUs per table (can request increase)
- **Writes**: Up to 40,000 WCUs per table (can request increase)
- **Auto-scaling**: Instant (no warm-up)

**Provisioned Mode:**
- **Reads**: Up to configured RCUs
- **Writes**: Up to configured WCUs
- **Auto-scaling**: 2-5 minutes to scale up

### Optimization Tips

#### 1. Use Batch Operations

```rust
// Bad: N round trips
for id in ids {
    storage.get_by_id(id).await?;
}

// Good: 1 round trip per 100 items
storage.get_by_ids(&ids).await?;
```

#### 2. Optimize Item Size

```rust
// Bad: Large items (> 4KB) are expensive
let data = json!({
    "id": "doc-001",
    "content": "very long text...",  // Store in S3 instead
    "metadata": { /* lots of data */ }
});

// Good: Keep items small, reference S3 for large content
let data = json!({
    "id": "doc-001",
    "content_s3_key": "s3://bucket/doc-001.txt",  // Reference
    "metadata": { /* essential data only */ }
});
```

#### 3. Use Sparse Indexes

Only add attributes when needed (DynamoDB doesn't charge for null attributes).

#### 4. Choose Right Consistency

```rust
// Eventually consistent reads (default, cheaper)
storage.get_by_id("doc-001").await?;  // Eventually consistent

// Strongly consistent reads (more expensive)
// Not directly exposed in current API, but available in DynamoDB SDK
```

### Capacity Planning

**Calculate Required Capacity:**

1. **Read Capacity Units (RCU)**:
   - 1 RCU = 1 strongly consistent read/sec of ≤4KB
   - 1 RCU = 2 eventually consistent reads/sec of ≤4KB

2. **Write Capacity Units (WCU)**:
   - 1 WCU = 1 write/sec of ≤1KB

**Example:**
- 1000 reads/sec of 2KB items (eventually consistent)
- 100 writes/sec of 2KB items

```
RCUs = (1000 / 2) * (2KB / 4KB) = 250 RCUs
WCUs = 100 * (2KB / 1KB) = 200 WCUs

Provisioned cost = (250 * $0.00013 + 200 * $0.00065) * 730 hours/month
                 = ($0.0325 + $0.13) * 730
                 = $118.70/month

On-demand cost = (1000 * 60 * 60 * 24 * 30 / 1,000,000 * $0.25) +
                 (100 * 60 * 60 * 24 * 30 / 1,000,000 * $1.25)
                 = $64.80 + $32.40
                 = $97.20/month

→ On-demand is cheaper for this workload
```

## Security

### IAM Authentication

DynamoDB uses IAM roles (no passwords).

**Minimum Required Permissions:**

```json
{
    "Version": "2012-10-17",
    "Statement": [{
        "Effect": "Allow",
        "Action": [
            "dynamodb:GetItem",
            "dynamodb:PutItem",
            "dynamodb:UpdateItem",
            "dynamodb:DeleteItem",
            "dynamodb:Query",
            "dynamodb:BatchGetItem",
            "dynamodb:BatchWriteItem"
        ],
        "Resource": "arn:aws:dynamodb:region:account:table/edgequake-kv"
    }]
}
```

**Read-Only Access:**

```json
{
    "Version": "2012-10-17",
    "Statement": [{
        "Effect": "Allow",
        "Action": [
            "dynamodb:GetItem",
            "dynamodb:Query",
            "dynamodb:BatchGetItem",
            "dynamodb:Scan"
        ],
        "Resource": "arn:aws:dynamodb:region:account:table/edgequake-kv"
    }]
}
```

### Encryption

#### At Rest

```bash
# Enable AWS-managed encryption (default)
aws dynamodb update-table \
    --table-name edgequake-kv \
    --sse-specification Enabled=true,SSEType=KMS

# Use customer-managed KMS key
aws dynamodb update-table \
    --table-name edgequake-kv \
    --sse-specification \
        Enabled=true,SSEType=KMS,KMSMasterKeyId=arn:aws:kms:region:account:key/xxx
```

#### In Transit

All DynamoDB API calls use HTTPS (TLS 1.2+) by default.

### VPC Endpoints

For enhanced security, use VPC endpoints to avoid internet routing:

```bash
aws ec2 create-vpc-endpoint \
    --vpc-id vpc-xxx \
    --service-name com.amazonaws.region.dynamodb \
    --route-table-ids rtb-xxx
```

### Audit Logging

Enable CloudTrail logging:

```bash
aws cloudtrail create-trail \
    --name dynamodb-audit \
    --s3-bucket-name my-audit-logs

aws cloudtrail start-logging --name dynamodb-audit
```

Logs all API calls including:
- Who accessed the table
- What operations were performed
- When they occurred
- Source IP address

## Best Practices

### 1. Design for Access Patterns

DynamoDB is optimized for key-value access, not ad-hoc queries.

**Good:**
```rust
// Direct key access
storage.get_by_id("doc-001").await?;

// Partition query
// (requires GSI on namespace + status)
```

**Bad:**
```rust
// Full table scan (slow and expensive)
let all_items = storage.keys().await?;  // Avoid!
```

### 2. Use Composite Keys

Partition key + sort key enables:
- Range queries
- Multi-tenancy
- Hierarchical data

**Example:**
```
Partition Key: namespace = "workspace-123"
Sort Key: id = "doc-001"
```

Allows queries like:
- Get all items in workspace: `namespace = "workspace-123"`
- Get specific item: `namespace = "workspace-123" AND id = "doc-001"`

### 3. Enable TTL for Auto-Cleanup

```bash
aws dynamodb update-time-to-live \
    --table-name edgequake-kv \
    --time-to-live-specification \
        Enabled=true,AttributeName=expires_at
```

Then set expiration:
```rust
let expires_at = chrono::Utc::now().timestamp() + 86400;  // 24 hours
let data = vec![(
    "temp-cache".to_string(),
    json!({"value": "cached", "expires_at": expires_at})
)];
storage.upsert(&data).await?;
// Automatically deleted after 24 hours
```

### 4. Use Conditional Writes

Prevent overwrites:

```rust
// Use transition_if_status for atomic updates
storage.transition_if_status("doc-001", "pending", "processing").await?;
```

### 5. Monitor Metrics

Key CloudWatch metrics:
- `ConsumedReadCapacityUnits`
- `ConsumedWriteCapacityUnits`
- `UserErrors` (throttling)
- `SystemErrors`
- `SuccessfulRequestLatency`

### 6. Handle Throttling

DynamoDB may throttle requests if you exceed capacity.

```rust
// Implement retry with exponential backoff
use tokio::time::{sleep, Duration};

async fn upsert_with_retry(storage: &DynamoKVStorage, data: &[(String, JsonValue)]) -> Result<()> {
    let mut retries = 0;
    let max_retries = 3;

    loop {
        match storage.upsert(data).await {
            Ok(_) => return Ok(()),
            Err(e) if retries < max_retries => {
                let delay = 2u64.pow(retries) * 100;  // 100ms, 200ms, 400ms
                sleep(Duration::from_millis(delay)).await;
                retries += 1;
            }
            Err(e) => return Err(e),
        }
    }
}
```

## Troubleshooting

### Issue: Table Not Found

**Error:**
```
DynamoDbError("Table 'edgequake-kv' not found...")
```

**Solution:**

1. Check table exists:
   ```bash
   aws dynamodb describe-table --table-name edgequake-kv
   ```

2. Create table if missing (see Setup Guide)

3. Verify AWS credentials and region:
   ```bash
   aws sts get-caller-identity
   aws configure get region
   ```

### Issue: Access Denied

**Error:**
```
DynamoDbError("User: arn:aws:iam::xxx is not authorized...")
```

**Solution:**

1. Check IAM permissions:
   ```bash
   aws iam get-role-policy --role-name YourRole --policy-name DynamoDBAccess
   ```

2. Add required permissions (see Security section)

### Issue: Item Size Too Large

**Error:**
```
DynamoDbError("Item size has exceeded the maximum allowed size")
```

**Limit**: 400KB per item

**Solution:**

1. **Store large content in S3**:
   ```rust
   // Upload to S3
   let s3_key = format!("content/{}.json", id);
   s3_client.put_object()
       .bucket("edgequake-data")
       .key(&s3_key)
       .body(large_content.into())
       .send()
       .await?;

   // Store reference in DynamoDB
   let data = vec![(
       id.to_string(),
       json!({"title": "Doc", "content_s3_key": s3_key})
   )];
   storage.upsert(&data).await?;
   ```

2. **Compress data**:
   ```rust
   use flate2::write::GzEncoder;
   use flate2::Compression;

   let compressed = /* compress json */;
   let encoded = base64::encode(compressed);
   ```

### Issue: Throttling

**Error:**
```
DynamoDbError("ProvisionedThroughputExceededException")
```

**Solution:**

1. **Switch to on-demand mode**:
   ```bash
   aws dynamodb update-table \
       --table-name edgequake-kv \
       --billing-mode PAY_PER_REQUEST
   ```

2. **Increase provisioned capacity**:
   ```bash
   aws dynamodb update-table \
       --table-name edgequake-kv \
       --provisioned-throughput ReadCapacityUnits=100,WriteCapacityUnits=100
   ```

3. **Implement retry logic** (see Best Practices)

### Issue: High Costs

**Check billing:**
```bash
aws ce get-cost-and-usage \
    --time-period Start=2024-01-01,End=2024-01-31 \
    --granularity MONTHLY \
    --metrics BlendedCost \
    --filter file://dynamodb-filter.json
```

**Optimize:**

1. **Reduce item size** (compress, move to S3)
2. **Use batch operations** (reduce request count)
3. **Enable TTL** (auto-delete old items)
4. **Archive to S3** (use Lambda + S3 lifecycle)
5. **Switch to provisioned mode** (if workload is predictable)

## Resources

- [DynamoDB Documentation](https://docs.aws.amazon.com/dynamodb/)
- [DynamoDB Best Practices](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/best-practices.html)
- [DynamoDB Pricing](https://aws.amazon.com/dynamodb/pricing/)
- [DynamoDB Capacity Calculator](https://aws.amazon.com/dynamodb/pricing/provisioned/)
- [EdgeQuake AWS Migration Design](../../AWS_MIGRATION_DESIGN.md)

## Support

Need help? Contact:

- 📧 Email: support@edgequake.io
- 💬 [GitHub Discussions](https://github.com/edgequake/edgequake/discussions)
- 🐛 [Issue Tracker](https://github.com/edgequake/edgequake/issues)
