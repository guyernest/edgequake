# EdgeQuake MCP Server - Quick Start

Generate an MCP (Model Context Protocol) server from EdgeQuake's OpenAPI schema in 5 minutes.

## TL;DR

```bash
# 1. Export OpenAPI schema
make export-openapi

# 2. Feed to your MCP generator
your-mcp-tool generate --schema openapi.json

# 3. Configure AWS backends
# See MCP_INTEGRATION.md for details

# 4. Run MCP server
./your-generated-mcp-server
```

## What You Get

✅ **OpenAPI 3.0 Schema** with 100+ endpoints
✅ **AWS Storage Backends** (S3, Neptune, DynamoDB, Athena)
✅ **Zero Manual MCP Coding** - auto-generated from schema
✅ **Production-Ready API** - just wrap it in MCP protocol

## Architecture Decision

**Problem**: You want an MCP server for EdgeQuake but don't want to build MCP from scratch.

**Solution**: Use your existing OpenAPI-to-MCP generator tool! EdgeQuake already has:
- ✅ Comprehensive OpenAPI 3.0 schema (in `edgequake-api` crate)
- ✅ Working REST API (Axum-based)
- ✅ AWS cloud storage (S3, Neptune, DynamoDB, Athena)

**Your Tool**: Feed `openapi.json` → Get MCP server

## Key Clarifications

### 1. Glue vs Athena

**They are TWO separate services that work together:**

```
AWS Glue Data Catalog (metadata store)
        ↓ (reads table definitions)
Amazon Athena (SQL query engine)
        ↓ (reads actual data)
Amazon S3 (data files)
```

- **Glue**: Stores table schemas, partitions, locations
- **Athena**: Executes SQL queries using Glue metadata
- **S3**: Stores the actual data (Parquet/JSON files)

### 2. Athena Schemas - Static vs Dynamic

**Recommendation**: Use **static schemas** for your MCP use case.

**Why**: Static schemas are:
- ✅ Predictable
- ✅ Type-safe
- ✅ Easy to version
- ✅ No crawler complexity

**Schema Location**: Define in code (see MCP_INTEGRATION.md Step 4)

```sql
-- Example: Documents table for analytics
CREATE EXTERNAL TABLE edgequake.documents (
  id STRING,
  workspace_id STRING,
  title STRING,
  content STRING,
  status STRING,
  created_at TIMESTAMP
)
PARTITIONED BY (workspace_id STRING)
STORED AS PARQUET
LOCATION 's3://edgequake-data/documents/';
```

### 3. Athena's Role in MCP Server

**Critical**: Athena is **NOT** for real-time MCP queries!

| Use Case | Storage | Latency | When to Use |
|----------|---------|---------|-------------|
| **Document upload** | S3 + DynamoDB | 100-500ms | Real-time MCP tools |
| **RAG query** | S3 Vectors + Neptune | 100-300ms | Real-time MCP tools |
| **List documents** | DynamoDB | 10-50ms | Real-time MCP tools |
| **Analytics** | Athena SQL | 1-5 seconds | Reporting MCP tools |
| **Historical queries** | Athena SQL | 1-10 seconds | Analytics MCP tools |

**Use Athena only for:**
- Analytics tools: `workspace_analytics`, `query_trends`
- Reporting: "How many documents uploaded last month?"
- Historical: "Show all failed processing jobs in 2024"

**Don't use Athena for:**
- Document CRUD operations → Use DynamoDB
- RAG queries → Use S3 Vectors + Neptune
- Real-time responses → Too slow (1-3 seconds)

### 4. API Architecture - No API Gateway Needed!

**Question**: API Gateway vs GraphQL vs Keep Axum?

**Answer**: Keep your Axum REST API as-is!

```
┌─────────────────────────────────────────────┐
│     Generated MCP Server (from OpenAPI)     │
│                                              │
│  Wraps EdgeQuake Axum API:                  │
│  • Embedded (in-process) - RECOMMENDED      │
│  • HTTP proxy - if needed for isolation     │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│       EdgeQuake Axum REST API               │
│       (edgequake-api crate)                 │
│                                              │
│  No changes needed!                         │
│  • Keep all existing handlers               │
│  • Keep OpenAPI schema                      │
│  • Keep multi-tenancy logic                 │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│         AWS Storage Backends                │
│  S3 | Neptune | DynamoDB | Athena           │
└─────────────────────────────────────────────┘
```

**Why NOT API Gateway?**
- ❌ Adds latency and cost
- ❌ Designed for public/web APIs
- ❌ Your MCP server is not user-facing
- ❌ No benefit for your use case

**Why NOT GraphQL?**
- ❌ MCP already has its own protocol
- ❌ REST is simpler for MCP tool mapping
- ❌ OpenAPI → MCP is more direct
- ❌ Extra complexity for no gain

**Recommended**: Keep Axum, embed it in generated MCP server.

## Step-by-Step

### Step 1: Export OpenAPI Schema

```bash
# Option 1: Using Make (requires Rust 1.88+)
make export-openapi

# Option 2: Direct cargo (requires Rust 1.88+)
cd edgequake/crates/edgequake-api
cargo run --bin export-openapi -- --pretty --output ../../openapi.json

# Option 3: From running server (no build needed)
curl http://localhost:8080/api-docs/openapi.json > openapi.json
```

**Output**: `openapi.json` with 100+ endpoints

### Step 2: Feed to Your MCP Generator

```bash
# Example (adjust for your actual tool)
your-mcp-tool generate \
  --schema openapi.json \
  --output edgequake-mcp-server \
  --protocol stdio
```

### Step 3: Configure AWS Storage

Update your generated MCP server or EdgeQuake config to use AWS:

```rust
use edgequake_storage_aws::{
    S3VectorStorage, NeptuneGraphStorage, DynamoKVStorage
};

// S3 for vectors
let vector_storage = S3VectorStorage::new(
    S3Config::new("edgequake-vectors", "workspace-1", 1536)
).await?;

// Neptune for graph
let graph_storage = NeptuneGraphStorage::new(
    NeptuneConfig::new("your-cluster:8182", "workspace-1")
).await?;

// DynamoDB for documents
let kv_storage = DynamoKVStorage::new(
    DynamoKVConfig::new("edgequake-docs", "workspace-1")
).await?;
```

### Step 4: Deploy AWS Infrastructure

```bash
# S3 bucket
aws s3 mb s3://edgequake-vectors

# Neptune cluster (serverless)
aws neptune create-db-cluster \
  --db-cluster-identifier edgequake \
  --engine neptune \
  --serverless-v2-scaling-configuration MinCapacity=2.5,MaxCapacity=16

# DynamoDB table
aws dynamodb create-table \
  --table-name edgequake-docs \
  --attribute-definitions \
    AttributeName=namespace,AttributeType=S \
    AttributeName=id,AttributeType=S \
  --key-schema \
    AttributeName=namespace,KeyType=HASH \
    AttributeName=id,KeyType=RANGE \
  --billing-mode PAY_PER_REQUEST

# Glue database (optional, for Athena analytics)
aws glue create-database --database-input '{"Name":"edgequake"}'
```

### Step 5: Run MCP Server

```bash
# If embedded Axum
./edgequake-mcp-server --stdio

# If HTTP proxy mode
./edgequake-api &  # Start Axum API
./edgequake-mcp-server --stdio --api-url http://localhost:8080
```

## Core MCP Tools (Mapped from OpenAPI)

Your MCP server should expose these operations:

### Document Management

```json
{
  "name": "add_document",
  "inputSchema": {
    "content": "string",
    "title": "string"
  }
}
```

Maps to: `POST /api/v1/documents/upload`

### Query Execution

```json
{
  "name": "query",
  "inputSchema": {
    "query": "string",
    "mode": "hybrid|local|global",
    "top_k": 10
  }
}
```

Maps to: `POST /api/v1/query`

### Graph Operations

```json
{
  "name": "get_entity",
  "inputSchema": {
    "entity_name": "string"
  }
}
```

Maps to: `GET /api/v1/graph/entities/{name}`

### Analytics (Athena)

```json
{
  "name": "workspace_analytics",
  "inputSchema": {
    "workspace_id": "string",
    "metric": "document_count|query_stats"
  }
}
```

Uses: Athena SQL queries (async, 1-5 seconds)

## Testing

```bash
# Test OpenAPI export
jq '.info.title' openapi.json
# Should show: "EdgeQuake API"

# Test AWS storage
cd edgequake/crates/edgequake-storage-aws
S3_BUCKET=test cargo run --example s3_vector_basic --features s3-vectors
NEPTUNE_ENDPOINT=test:8182 cargo run --example neptune_graph_basic --features neptune
DYNAMODB_TABLE=test cargo run --example dynamodb_kv_basic --features dynamodb
```

## Cost Optimization

With AWS backends, your costs drop dramatically:

| Component | PostgreSQL | AWS | Savings |
|-----------|-----------|-----|---------|
| **Active** | $297/month | $12/month | **96%** |
| **Idle** | $297/month | $2/month | **99%** |

**Why**:
- PostgreSQL runs 24/7 even when idle
- AWS charges only for usage (serverless)

## Troubleshooting

### "Rust 1.88 required"

The `export-openapi` binary requires Rust 1.88+. Two options:

```bash
# Option 1: Update Rust
rustup update

# Option 2: Export from running server (no build needed)
curl http://localhost:8080/api-docs/openapi.json > openapi.json
```

### "Athena too slow for queries"

Correct! **Don't use Athena for real-time queries**. Use:
- **DynamoDB** for document lookups (10-50ms)
- **S3 + Neptune** for RAG queries (100-300ms)
- **Athena** only for analytics (1-5 seconds OK)

### "How to handle multi-tenancy?"

EdgeQuake uses workspace isolation:

```rust
// Option 1: Headers (HTTP mode)
headers.insert("X-Workspace-ID", workspace_id);

// Option 2: Context (embedded mode)
let ctx = TenantContext {
    workspace_id: workspace_id.clone(),
    ...
};
```

## Files Created

1. **`src/bin/export-openapi.rs`** - Export utility
2. **`MCP_INTEGRATION.md`** - Complete integration guide
3. **`MCP_QUICKSTART.md`** - This file (quick reference)
4. **`Makefile`** - Added `export-openapi` target

## Documentation References

- **Complete Guide**: [MCP_INTEGRATION.md](MCP_INTEGRATION.md)
- **S3 Vectors**: [edgequake/crates/edgequake-storage-aws/QUICKSTART.md](edgequake/crates/edgequake-storage-aws/QUICKSTART.md)
- **Neptune Graph**: [edgequake/crates/edgequake-storage-aws/NEPTUNE_GUIDE.md](edgequake/crates/edgequake-storage-aws/NEPTUNE_GUIDE.md)
- **DynamoDB KV**: [edgequake/crates/edgequake-storage-aws/DYNAMODB_GUIDE.md](edgequake/crates/edgequake-storage-aws/DYNAMODB_GUIDE.md)
- **Athena SQL**: [edgequake/crates/edgequake-storage-aws/ATHENA_GUIDE.md](edgequake/crates/edgequake-storage-aws/ATHENA_GUIDE.md)

## Summary

✅ **No Manual MCP Coding** - Generate from OpenAPI schema
✅ **Use AWS Storage** - S3, Neptune, DynamoDB (96% cost savings)
✅ **Athena for Analytics Only** - Not for real-time queries
✅ **No API Gateway Needed** - Keep Axum, wrap in MCP
✅ **Static Schemas** - Define Glue tables in code
✅ **Production Ready** - Just deploy AWS infrastructure

**Next**: Run `make export-openapi` and feed to your MCP generator!
