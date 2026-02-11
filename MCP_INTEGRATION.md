# EdgeQuake MCP Server Integration Guide

Complete guide for building an MCP (Model Context Protocol) server from EdgeQuake's OpenAPI schema using AWS cloud storage backends.

## Overview

This guide explains how to generate an MCP server from EdgeQuake's existing OpenAPI 3.0 schema, eliminating the need to build MCP from scratch. The MCP server will wrap the EdgeQuake Axum REST API and use AWS cloud storage (S3, Neptune, DynamoDB, Athena).

## Architecture

```
┌──────────────────────────────────────────────────────┐
│          MCP Server Generator Tool                   │
│  (Your existing schema-to-MCP tool)                  │
└────────────────┬─────────────────────────────────────┘
                 │
                 │ reads
                 ▼
┌──────────────────────────────────────────────────────┐
│          openapi.json                                 │
│  (EdgeQuake OpenAPI 3.0 Schema)                      │
│                                                       │
│  • 100+ REST endpoints                               │
│  • Document upload, query, graph operations          │
│  • Tenant/workspace multi-tenancy                    │
│  • Authentication (JWT, API keys)                    │
└────────────────┬─────────────────────────────────────┘
                 │
                 │ generates
                 ▼
┌──────────────────────────────────────────────────────┐
│          Generated MCP Server                         │
│  (Wraps EdgeQuake Axum API)                          │
│                                                       │
│  MCP Protocol:                                       │
│  • tools/list → discover capabilities                │
│  • tools/call → add_document, query, etc.            │
│  • resources/list → list documents                   │
└────────────────┬─────────────────────────────────────┘
                 │
                 │ calls (in-process or HTTP)
                 ▼
┌──────────────────────────────────────────────────────┐
│          EdgeQuake Axum API                           │
│  (edgequake-api crate)                               │
│                                                       │
│  Existing Handlers:                                  │
│  • POST /api/v1/documents (upload_document)          │
│  • POST /api/v1/query (execute_query)                │
│  • GET  /api/v1/documents (list_documents)           │
│  • GET  /api/v1/graph (get_graph)                    │
└────────────────┬─────────────────────────────────────┘
                 │
                 ▼
┌──────────────────────────────────────────────────────┐
│          AWS Cloud Storage                            │
│                                                       │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐    │
│  │ S3 Vectors │  │  Neptune   │  │  DynamoDB  │    │
│  │  + HNSW    │  │   Graph    │  │    KV      │    │
│  └────────────┘  └────────────┘  └────────────┘    │
│                                                       │
│  ┌────────────┐                                      │
│  │  Athena    │  (Analytics only, not real-time)    │
│  │    SQL     │                                      │
│  └────────────┘                                      │
└──────────────────────────────────────────────────────┘
```

## Step 1: Export OpenAPI Schema

### Generate the Schema File

```bash
cd edgequake/crates/edgequake-api

# Export to openapi.json (pretty formatted)
cargo run --bin export-openapi --pretty --output ../../openapi.json

# Or export to stdout
cargo run --bin export-openapi --pretty > ../../openapi.json
```

### Verify Schema Export

```bash
# Check endpoint count
jq '.paths | keys | length' openapi.json
# Should show 100+ endpoints

# Check schemas
jq '.components.schemas | keys | length' openapi.json
# Should show 100+ schema definitions

# View a sample endpoint
jq '.paths."/api/v1/query".post' openapi.json
```

## Step 2: Key Endpoints for MCP Tools

Your MCP server should expose these EdgeQuake operations as MCP tools:

### Core Document Operations

| MCP Tool | OpenAPI Endpoint | Method | Description |
|----------|------------------|--------|-------------|
| `add_document` | `/api/v1/documents/upload` | POST | Upload document (text/file) |
| `list_documents` | `/api/v1/documents` | GET | List all documents |
| `get_document` | `/api/v1/documents/{id}` | GET | Get document details |
| `delete_document` | `/api/v1/documents/{id}` | DELETE | Delete document |

### Query Operations

| MCP Tool | OpenAPI Endpoint | Method | Description |
|----------|------------------|--------|-------------|
| `query` | `/api/v1/query` | POST | Execute RAG query |
| `query_stream` | `/api/v1/query/stream` | POST | Stream query response (SSE) |

### Graph Operations

| MCP Tool | OpenAPI Endpoint | Method | Description |
|----------|------------------|--------|-------------|
| `get_graph` | `/api/v1/graph` | GET | Get knowledge graph |
| `get_entity` | `/api/v1/graph/entities/{name}` | GET | Get entity details |
| `search_entities` | `/api/v1/graph/entities` | GET | Search entities |

### Workspace Operations

| MCP Tool | OpenAPI Endpoint | Method | Description |
|----------|------------------|--------|-------------|
| `list_workspaces` | `/api/v1/tenants/{tid}/workspaces` | GET | List workspaces |
| `get_workspace` | `/api/v1/workspaces/{id}` | GET | Get workspace details |
| `workspace_stats` | `/api/v1/workspaces/{id}/stats` | GET | Get workspace statistics |

## Step 3: Configure AWS Storage Backend

### Update AppState for AWS

The EdgeQuake API uses `AppState` to manage storage backends. Configure it to use AWS:

```rust
// In your main.rs or server startup code
use edgequake_api::state::AppState;
use edgequake_storage_aws::{
    S3VectorStorage, NeptuneGraphStorage, DynamoKVStorage,
    S3Config, NeptuneConfig, DynamoKVConfig
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure S3 vector storage
    let s3_config = S3Config::new(
        "edgequake-vectors",  // S3 bucket
        "default-workspace",  // Namespace
        1536                  // Embedding dimension
    )
    .with_hnsw_params(16, 200, 50)
    .with_batch_size(1000);

    let vector_storage = Arc::new(
        S3VectorStorage::new(s3_config).await?
    );

    // Configure Neptune graph storage
    let neptune_config = NeptuneConfig::new(
        "edgequake-cluster.cluster-xxx.us-east-1.neptune.amazonaws.com:8182",
        "default-workspace"
    );

    let graph_storage = Arc::new(
        NeptuneGraphStorage::new(neptune_config).await?
    );

    // Configure DynamoDB KV storage
    let dynamo_config = DynamoKVConfig::new(
        "edgequake-documents",  // DynamoDB table
        "default-workspace"     // Namespace
    )
    .with_on_demand(true);

    let kv_storage = Arc::new(
        DynamoKVStorage::new(dynamo_config).await?
    );

    // Build AppState with AWS backends
    let state = AppState::builder()
        .vector_storage(vector_storage)
        .graph_storage(graph_storage)
        .kv_storage(kv_storage)
        .build();

    // Create Axum server
    let server = edgequake_api::Server::new(
        edgequake_api::ServerConfig::default(),
        state
    );

    // Run server (embedded in MCP or standalone)
    server.run().await?;

    Ok(())
}
```

## Step 4: Athena SQL Integration (Optional)

Athena is **NOT** for real-time MCP operations. Use it only for analytics/reporting tools.

### When to Use Athena

✅ **Good for:**
- Analytics MCP tools: `workspace_analytics`, `document_trends`
- Reporting: "Show document upload stats for last month"
- Historical queries: "Find all documents uploaded in 2024"

❌ **Not for:**
- Real-time document queries (too slow: 1-3 seconds)
- RAG query execution (use S3 vectors + Neptune instead)
- Normal MCP tool responses (users expect <100ms)

### Athena Schema Setup

If you want analytics tools, define Glue Catalog tables:

```sql
-- Create database
CREATE DATABASE IF NOT EXISTS edgequake;

-- Documents table (for analytics)
CREATE EXTERNAL TABLE edgequake.documents (
  id STRING,
  workspace_id STRING,
  title STRING,
  content STRING,
  status STRING,
  created_at TIMESTAMP,
  size_bytes BIGINT
)
PARTITIONED BY (
  workspace_id STRING,
  year STRING,
  month STRING
)
STORED AS PARQUET
LOCATION 's3://edgequake-data/documents/'
TBLPROPERTIES ('parquet.compression'='SNAPPY');

-- Vectors metadata table
CREATE EXTERNAL TABLE edgequake.vectors_metadata (
  vector_id STRING,
  document_id STRING,
  chunk_index INT,
  embedding_model STRING,
  created_at TIMESTAMP
)
PARTITIONED BY (workspace_id STRING)
STORED AS PARQUET
LOCATION 's3://edgequake-data/vectors/metadata/'
TBLPROPERTIES ('parquet.compression'='SNAPPY');

-- Query history table (for analytics)
CREATE EXTERNAL TABLE edgequake.query_history (
  query_id STRING,
  workspace_id STRING,
  query_text STRING,
  query_mode STRING,
  response_time_ms INT,
  created_at TIMESTAMP
)
PARTITIONED BY (
  workspace_id STRING,
  year STRING,
  month STRING
)
STORED AS PARQUET
LOCATION 's3://edgequake-data/query-history/'
TBLPROPERTIES ('parquet.compression'='SNAPPY');
```

### Athena MCP Tool Example

```json
{
  "name": "workspace_analytics",
  "description": "Get analytics for a workspace using Athena SQL",
  "inputSchema": {
    "type": "object",
    "properties": {
      "workspace_id": {
        "type": "string",
        "description": "Workspace UUID"
      },
      "metric": {
        "type": "string",
        "enum": ["document_count", "query_stats", "upload_trends"],
        "description": "Analytics metric to retrieve"
      }
    },
    "required": ["workspace_id", "metric"]
  }
}
```

Implementation:
```rust
use edgequake_storage_aws::{AthenaConfig, AthenaQueryEngine};

async fn workspace_analytics(workspace_id: &str, metric: &str) -> Result<Value> {
    let athena = AthenaQueryEngine::new(
        AthenaConfig::new("edgequake", "s3://athena-results/")
    ).await?;

    let query = match metric {
        "document_count" => format!(
            "SELECT COUNT(*) as count FROM documents WHERE workspace_id = '{}'",
            workspace_id
        ),
        "query_stats" => format!(
            "SELECT
               COUNT(*) as total_queries,
               AVG(response_time_ms) as avg_response_time
             FROM query_history
             WHERE workspace_id = '{}'",
            workspace_id
        ),
        "upload_trends" => format!(
            "SELECT
               DATE_TRUNC('day', created_at) as date,
               COUNT(*) as uploads
             FROM documents
             WHERE workspace_id = '{}'
             GROUP BY DATE_TRUNC('day', created_at)
             ORDER BY date DESC
             LIMIT 30",
            workspace_id
        ),
        _ => return Err("Unknown metric".into())
    };

    let results = athena.query(&query).await?;
    Ok(json!({"data": results}))
}
```

## Step 5: MCP Server Generation

### Input for Your Generator Tool

```bash
# Your MCP generator tool command (example)
your-mcp-tool generate \
  --schema openapi.json \
  --output edgequake-mcp-server \
  --protocol stdio \
  --base-url http://localhost:8080
```

### Expected MCP Tools Output

The generated MCP server should expose these tools (mapped from OpenAPI endpoints):

```json
{
  "tools": [
    {
      "name": "add_document",
      "description": "Upload a document for RAG processing",
      "inputSchema": {
        "type": "object",
        "properties": {
          "content": {"type": "string"},
          "title": {"type": "string"},
          "metadata": {"type": "object"}
        }
      }
    },
    {
      "name": "query",
      "description": "Execute a RAG query with knowledge graph",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": {"type": "string"},
          "mode": {"type": "string", "enum": ["naive", "local", "global", "hybrid"]},
          "top_k": {"type": "integer"}
        }
      }
    },
    {
      "name": "list_documents",
      "description": "List all documents in workspace",
      "inputSchema": {
        "type": "object",
        "properties": {
          "workspace_id": {"type": "string"}
        }
      }
    }
  ]
}
```

## Step 6: Deployment Options

### Option A: Embedded Axum (Recommended)

The MCP server embeds the Axum API in-process:

```rust
// MCP server calls Axum handlers directly (no HTTP overhead)
use edgequake_api::handlers;

async fn handle_mcp_tool(tool: &str, args: Value) -> Result<Value> {
    match tool {
        "add_document" => {
            let req = serde_json::from_value(args)?;
            let result = handlers::upload_document(state, req).await?;
            Ok(serde_json::to_value(result)?)
        }
        "query" => {
            let req = serde_json::from_value(args)?;
            let result = handlers::execute_query(state, req).await?;
            Ok(serde_json::to_value(result)?)
        }
        _ => Err("Unknown tool".into())
    }
}
```

**Pros:**
- Zero HTTP overhead
- Direct function calls
- Shared memory/state
- Fastest performance

**Cons:**
- Tighter coupling
- Same process lifecycle

### Option B: HTTP Proxy

MCP server calls Axum API via HTTP:

```rust
async fn handle_mcp_tool(tool: &str, args: Value) -> Result<Value> {
    let client = reqwest::Client::new();

    match tool {
        "add_document" => {
            let response = client
                .post("http://localhost:8080/api/v1/documents")
                .json(&args)
                .send()
                .await?;
            Ok(response.json().await?)
        }
        _ => Err("Unknown tool".into())
    }
}
```

**Pros:**
- Loose coupling
- Independent scaling
- Separate processes

**Cons:**
- HTTP overhead (~1-5ms)
- Network dependency
- More complex deployment

## Step 7: Multi-Tenancy Context

EdgeQuake uses workspace isolation. Your MCP server must provide context:

```rust
// Option 1: Headers (if using HTTP proxy)
let response = client
    .post("http://localhost:8080/api/v1/documents")
    .header("X-Workspace-ID", workspace_id)
    .header("X-Tenant-ID", tenant_id)
    .json(&args)
    .send()
    .await?;

// Option 2: Middleware (if embedded)
use edgequake_api::middleware::TenantContext;

let ctx = TenantContext {
    tenant_id: tenant_id.clone(),
    workspace_id: workspace_id.clone(),
    user_id: None,
};

// Inject into request
```

## Step 8: Testing

### Test OpenAPI Export

```bash
cargo run --bin export-openapi --pretty --output openapi.json
cat openapi.json | jq '.info'
```

### Test AWS Storage

```bash
# Set AWS credentials
export AWS_PROFILE=your-profile

# Test S3 vector storage
cd edgequake/crates/edgequake-storage-aws
S3_BUCKET=your-bucket cargo run --example s3_vector_basic --features s3-vectors

# Test Neptune graph storage
NEPTUNE_ENDPOINT=your-endpoint cargo run --example neptune_graph_basic --features neptune

# Test DynamoDB KV storage
DYNAMODB_TABLE=your-table cargo run --example dynamodb_kv_basic --features dynamodb
```

### Test Embedded Axum API

```bash
# Run EdgeQuake server with AWS backends
DATABASE_URL=disabled cargo run --bin edgequake-api

# Test endpoints
curl http://localhost:8080/health
curl http://localhost:8080/api-docs/openapi.json | jq '.'
```

## Step 9: Production Deployment

### Infrastructure Setup

1. **Deploy AWS Resources**
   ```bash
   # S3 bucket for vectors
   aws s3 mb s3://edgequake-vectors

   # Neptune cluster (serverless)
   aws neptune create-db-cluster \
     --db-cluster-identifier edgequake-cluster \
     --engine neptune \
     --serverless-v2-scaling-configuration MinCapacity=2.5,MaxCapacity=16

   # DynamoDB table
   aws dynamodb create-table \
     --table-name edgequake-documents \
     --attribute-definitions \
       AttributeName=namespace,AttributeType=S \
       AttributeName=id,AttributeType=S \
     --key-schema \
       AttributeName=namespace,KeyType=HASH \
       AttributeName=id,KeyType=RANGE \
     --billing-mode PAY_PER_REQUEST

   # Glue database (for Athena)
   aws glue create-database --database-input '{"Name":"edgequake"}'
   ```

2. **Configure IAM Permissions**
   - S3: read/write to vectors bucket
   - Neptune: read/write to cluster
   - DynamoDB: read/write to documents table
   - Athena: query execution and Glue access

3. **Deploy MCP Server**
   ```bash
   # Build
   cargo build --release --bin edgequake-mcp-server

   # Run
   ./target/release/edgequake-mcp-server --stdio
   ```

## Summary

### What You Have

✅ **Comprehensive OpenAPI 3.0 schema** (100+ endpoints)
✅ **Working Axum REST API** with all handlers
✅ **AWS storage backends** (S3, Neptune, DynamoDB, Athena)
✅ **OpenAPI export utility** (`export-openapi` binary)

### What You Need to Do

1. **Export OpenAPI schema** → `cargo run --bin export-openapi`
2. **Feed to MCP generator** → Your existing tool
3. **Configure AWS backends** → Set S3/Neptune/DynamoDB config
4. **Deploy infrastructure** → Create AWS resources
5. **Run MCP server** → Generated from OpenAPI

### Storage Usage Guidelines

| Storage | Use For | Don't Use For |
|---------|---------|---------------|
| **S3 Vectors** | Document embeddings, similarity search | Real-time updates |
| **Neptune Graph** | Entity relationships, graph traversal | Simple lookups |
| **DynamoDB** | Document metadata, quick lookups | Analytics |
| **Athena** | Analytics, reporting, historical queries | Real-time queries |

### Next Steps

1. Run `cargo run --bin export-openapi --output openapi.json`
2. Feed `openapi.json` to your MCP generator tool
3. Configure the generated server to use AWS storage backends
4. Test with sample documents and queries
5. Deploy to production

**Need help?** Check the AWS implementation guides:
- [S3 Vector Storage](edgequake/crates/edgequake-storage-aws/QUICKSTART.md)
- [Neptune Graph Storage](edgequake/crates/edgequake-storage-aws/NEPTUNE_GUIDE.md)
- [DynamoDB KV Storage](edgequake/crates/edgequake-storage-aws/DYNAMODB_GUIDE.md)
- [Athena SQL Queries](edgequake/crates/edgequake-storage-aws/ATHENA_GUIDE.md)
