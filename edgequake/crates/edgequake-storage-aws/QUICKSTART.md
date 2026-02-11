# Quick Start Guide: S3 Vector Storage

Get started with EdgeQuake's S3 vector storage in 5 minutes.

## Prerequisites

1. **AWS Account** with S3 access
2. **Rust** 1.78 or later
3. **AWS Credentials** configured

## Step 1: Set Up AWS Credentials

Choose one method:

### Option A: AWS Profile

```bash
export AWS_PROFILE=your-profile
```

### Option B: Access Keys

```bash
export AWS_ACCESS_KEY_ID=your-key
export AWS_SECRET_ACCESS_KEY=your-secret
export AWS_REGION=us-east-1
```

### Option C: AWS CLI Configure

```bash
aws configure
```

## Step 2: Create S3 Bucket

```bash
# Create bucket
aws s3 mb s3://my-edgequake-vectors --region us-east-1

# Verify access
aws s3 ls s3://my-edgequake-vectors
```

### Optional: Enable Encryption

```bash
aws s3api put-bucket-encryption \
    --bucket my-edgequake-vectors \
    --server-side-encryption-configuration \
    '{"Rules":[{"ApplyServerSideEncryptionByDefault":{"SSEAlgorithm":"AES256"}}]}'
```

## Step 3: Add Dependency

Add to your `Cargo.toml`:

```toml
[dependencies]
edgequake-storage = { path = "../edgequake-storage" }
edgequake-storage-aws = { path = "../edgequake-storage-aws", features = ["s3-vectors"] }
tokio = { version = "1.48", features = ["full"] }
serde_json = "1.0"
```

## Step 4: Write Your First Program

Create `src/main.rs`:

```rust
use edgequake_storage::VectorStorage;
use edgequake_storage_aws::{S3Config, S3VectorStorage};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure storage
    let config = S3Config::new(
        "my-edgequake-vectors",  // Your S3 bucket
        "my-workspace",           // Namespace (workspace ID)
        1536,                     // OpenAI embedding dimension
    );

    // Create storage
    let storage = S3VectorStorage::new(config).await?;
    storage.initialize().await?;

    // Insert vectors
    let data = vec![
        (
            "doc-001".to_string(),
            vec![0.1; 1536],  // Your embedding vector
            json!({
                "title": "My First Document",
                "source": "example"
            }),
        ),
    ];

    storage.upsert(&data).await?;
    println!("✅ Inserted 1 vector");

    // Query similar vectors
    let query = vec![0.1; 1536];
    let results = storage.query(&query, 5, None).await?;

    println!("\n🔍 Found {} similar vectors:", results.len());
    for (i, result) in results.iter().enumerate() {
        println!(
            "  {}. {} (score: {:.4})",
            i + 1,
            result.id,
            result.score
        );
    }

    // Save to S3
    storage.finalize().await?;
    println!("\n💾 Saved to S3");

    Ok(())
}
```

## Step 5: Run

```bash
cargo run
```

Expected output:

```
✅ Inserted 1 vector

🔍 Found 1 similar vectors:
  1. doc-001 (score: 1.0000)

💾 Saved to S3
```

## Step 6: Verify in S3

```bash
aws s3 ls s3://my-edgequake-vectors/my-workspace/ --recursive
```

You should see:

```
my-workspace/vectors/batch-1234567890.json
my-workspace/index/hnsw.bin
my-workspace/manifest.json
```

## What's Happening?

1. **S3VectorStorage::new()** - Creates storage client, loads AWS credentials
2. **initialize()** - Checks bucket access, loads existing index from S3 (if any)
3. **upsert()** - Adds vectors to S3 batch files, updates in-memory HNSW index
4. **query()** - Searches HNSW index for similar vectors, returns top-k results
5. **finalize()** - Saves index and manifest to S3 for persistence

## Next Steps

### 1. Tune HNSW Parameters

```rust
let config = S3Config::new("bucket", "workspace", 1536)
    .with_hnsw_params(
        16,   // m: connections per layer (higher = better recall)
        200,  // ef_construction: build quality (higher = better index)
        50,   // ef_search: query quality (higher = better recall)
    );
```

**Quick Guide:**
- **Fast queries**: `m=8, ef_construction=100, ef_search=20`
- **Balanced** (default): `m=16, ef_construction=200, ef_search=50`
- **High accuracy**: `m=32, ef_construction=400, ef_search=200`

### 2. Configure Batching

```rust
let config = S3Config::new("bucket", "workspace", 1536)
    .with_batch_size(1000)     // Upload 1000 vectors per S3 object
    .with_compression(true);   // Enable gzip compression
```

### 3. Enable Caching

```rust
use std::num::NonZeroUsize;

let config = S3Config::new("bucket", "workspace", 1536);
// Default cache: 10,000 vectors in memory (LRU)
```

### 4. Query with Filters

```rust
// Only search specific document IDs
let filter_ids = vec![
    "doc-001".to_string(),
    "doc-005".to_string(),
];

let results = storage.query(&query, 10, Some(&filter_ids)).await?;
```

### 5. Use with Real Embeddings

```rust
use async_openai::{Client, types::{CreateEmbeddingRequest, EmbeddingInput}};

// Generate embedding with OpenAI
let openai = Client::new();
let request = CreateEmbeddingRequest {
    model: "text-embedding-ada-002".to_string(),
    input: EmbeddingInput::String("Your text here".to_string()),
    user: None,
    encoding_format: None,
    dimensions: None,
};

let response = openai.embeddings().create(request).await?;
let embedding = response.data[0].embedding.clone();

// Store in S3
storage.upsert(&[(
    "doc-123".to_string(),
    embedding,
    json!({"text": "Your text here"}),
)]).await?;
```

## Common Patterns

### Pattern 1: Bulk Import

```rust
// Import 10,000 vectors efficiently
let mut batch = Vec::new();

for i in 0..10_000 {
    batch.push((
        format!("doc-{}", i),
        generate_embedding(&documents[i]),
        json!({"index": i}),
    ));

    // Upload in batches of 1000
    if batch.len() >= 1000 {
        storage.upsert(&batch).await?;
        batch.clear();
    }
}

// Upload remaining
if !batch.is_empty() {
    storage.upsert(&batch).await?;
}
```

### Pattern 2: Incremental Updates

```rust
// Add new vectors daily without affecting existing ones
let new_vectors = load_new_documents().await?;
storage.upsert(&new_vectors).await?;
storage.finalize().await?;  // Saves updated index
```

### Pattern 3: Multi-Workspace

```rust
// Separate workspaces for different tenants
let workspace_a = S3VectorStorage::new(
    S3Config::new("bucket", "tenant-a", 1536)
).await?;

let workspace_b = S3VectorStorage::new(
    S3Config::new("bucket", "tenant-b", 1536)
).await?;

// Each workspace has isolated data
```

## Troubleshooting

### Issue: "Cannot access bucket"

**Solution:**

```bash
# Test AWS credentials
aws sts get-caller-identity

# Test bucket access
aws s3 ls s3://your-bucket

# Check IAM permissions (need s3:GetObject, s3:PutObject)
```

### Issue: "Dimension mismatch"

**Error:**

```
DimensionMismatch { expected: 1536, actual: 768 }
```

**Solution:** Ensure all vectors have the same dimension as configured:

```rust
let config = S3Config::new("bucket", "workspace", 768);  // Match your model
```

### Issue: Slow queries

**Solution:**

```rust
// Increase search parameter
let config = S3Config::new("bucket", "workspace", 1536)
    .with_hnsw_params(16, 200, 100);  // Increase ef_search to 100
```

### Issue: High S3 costs

**Solutions:**

1. Enable compression:
   ```rust
   let config = config.with_compression(true);
   ```

2. Set up lifecycle policy (move to Glacier after 90 days):
   ```bash
   aws s3api put-bucket-lifecycle-configuration \
       --bucket my-edgequake-vectors \
       --lifecycle-configuration file://lifecycle.json
   ```

3. Monitor with AWS Budgets:
   ```bash
   aws budgets create-budget --account-id YOUR_ACCOUNT \
       --budget file://budget.json
   ```

## Production Checklist

Before deploying to production:

- [ ] Enable S3 bucket versioning
- [ ] Configure lifecycle policies (move to Glacier after 90 days)
- [ ] Set up CloudWatch alarms for S3 costs
- [ ] Enable S3 access logging
- [ ] Use separate buckets per environment (dev/staging/prod)
- [ ] Configure IAM roles with least privilege
- [ ] Enable S3 encryption (SSE-S3 or SSE-KMS)
- [ ] Set up backup/disaster recovery
- [ ] Monitor query latency with CloudWatch
- [ ] Configure VPC endpoints for S3 (avoid internet routing)

## Cost Optimization

### 1. Use Lifecycle Policies

```json
{
  "Rules": [{
    "Id": "MoveToGlacier",
    "Status": "Enabled",
    "Transitions": [{
      "Days": 90,
      "StorageClass": "GLACIER_IR"
    }]
  }]
}
```

### 2. Monitor Costs

```bash
# Check S3 costs
aws ce get-cost-and-usage \
    --time-period Start=2024-01-01,End=2024-01-31 \
    --granularity MONTHLY \
    --metrics BlendedCost \
    --filter file://s3-filter.json
```

### 3. Optimize Batch Size

```rust
// Larger batches = fewer S3 PUTs = lower cost
let config = config.with_batch_size(5000);
```

## Resources

- [S3 Pricing](https://aws.amazon.com/s3/pricing/)
- [S3 Best Practices](https://docs.aws.amazon.com/AmazonS3/latest/userguide/best-practices.html)
- [EdgeQuake Documentation](../../README.md)
- [Migration Guide](../../AWS_MIGRATION_DESIGN.md)

## Support

Need help? Open an issue or contact:

- 📧 Email: support@edgequake.io
- 💬 [GitHub Discussions](https://github.com/edgequake/edgequake/discussions)
- 🐛 [Issue Tracker](https://github.com/edgequake/edgequake/issues)
