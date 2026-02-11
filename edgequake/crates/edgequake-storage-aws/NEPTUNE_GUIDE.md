# Neptune Graph Storage Guide

Complete guide for using Amazon Neptune with EdgeQuake.

## Table of Contents

- [Overview](#overview)
- [Cost Analysis](#cost-analysis)
- [Setup Guide](#setup-guide)
- [Usage](#usage)
- [Gremlin Primer](#gremlin-primer)
- [Performance](#performance)
- [Security](#security)
- [Troubleshooting](#troubleshooting)

## Overview

Amazon Neptune is a fully managed graph database service that supports both Apache TinkerPop Gremlin and W3C SPARQL query languages. EdgeQuake uses Gremlin for property graph operations.

### Key Features

- **Serverless**: Auto-scales from 2.5 to 128 NCUs (Neptune Capacity Units)
- **Cost-effective**: Pay per query, scales to zero when idle
- **ACID**: Full transactional support
- **Highly available**: Multi-AZ deployment with failover
- **Backup**: Continuous backup to S3
- **Managed**: Automated patching, monitoring, backups

### Architecture

```
┌─────────────────────────────────────────────┐
│         EdgeQuake Application                │
│          (Rust + Gremlin Client)            │
└──────────────────┬──────────────────────────┘
                   │
                   ▼
         ┌──────────────────────┐
         │  Neptune Cluster     │
         │  (in VPC)            │
         ├──────────────────────┤
         │ Primary Instance     │
         │ (Read/Write)         │
         ├──────────────────────┤
         │ Read Replica(s)      │
         │ (Optional)           │
         └──────────────────────┘
                   │
                   ▼
         ┌──────────────────────┐
         │  Continuous Backup   │
         │  (S3)                │
         └──────────────────────┘
```

## Cost Analysis

### Neptune Serverless Pricing

**Serverless v2** (Recommended for EdgeQuake):

| Component | Price | Details |
|-----------|-------|---------|
| **Compute** | $0.12/NCU-hour | Auto-scales 2.5-128 NCUs |
| **Storage** | $0.10/GB-month | Automatic scaling |
| **I/O** | $0.20/1M requests | Read/write operations |
| **Backup** | $0.021/GB-month | Retained backups |

**Neptune Provisioned** (For predictable workloads):

| Instance Type | vCPU | RAM | Price/hour |
|--------------|------|-----|------------|
| db.t3.medium | 2 | 4 GB | $0.073 |
| db.r5.large | 2 | 16 GB | $0.29 |
| db.r5.xlarge | 4 | 32 GB | $0.58 |

### Cost Comparison: PostgreSQL AGE vs Neptune Serverless

**Scenario: 1M nodes, 5M edges, 100K queries/day**

| Component | PostgreSQL + AGE | Neptune Serverless | Savings |
|-----------|------------------|-------------------|---------|
| **Compute** | $250/month (RDS r5.large) | $72/month (6 NCU avg) | 71% |
| **Storage** | $23/month (200GB) | $20/month (200GB) | 13% |
| **I/O** | Included | $6/month (3M ops) | - |
| **Backup** | $20/month | $4.20/month | 79% |
| **Idle Cost** | $250/month | $0 (scales to zero) | 100% |
| **Total Active** | $293/month | $102.20/month | **65%** |
| **Total Idle** | $293/month | $24.20/month | **92%** |

### Cost Optimization Tips

1. **Use Serverless v2** for variable workloads
2. **Set min NCUs to 0.5** for dev/staging (scales to near-zero)
3. **Use read replicas** only for high read throughput
4. **Monitor query patterns** and optimize slow queries
5. **Delete old data** to reduce storage costs
6. **Use AWS Budgets** to set spending alerts

## Setup Guide

### Option 1: Neptune Serverless (Recommended)

#### Step 1: Create VPC Resources

```bash
# Create VPC (if you don't have one)
aws ec2 create-vpc --cidr-block 10.0.0.0/16

# Create private subnets (Neptune requires 2+ AZs)
aws ec2 create-subnet --vpc-id vpc-xxx --cidr-block 10.0.1.0/24 --availability-zone us-east-1a
aws ec2 create-subnet --vpc-id vpc-xxx --cidr-block 10.0.2.0/24 --availability-zone us-east-1b

# Create DB subnet group
aws neptune create-db-subnet-group \
    --db-subnet-group-name edgequake-neptune-subnet \
    --db-subnet-group-description "EdgeQuake Neptune subnets" \
    --subnet-ids subnet-xxx subnet-yyy
```

#### Step 2: Create Security Group

```bash
# Create security group
aws ec2 create-security-group \
    --group-name edgequake-neptune-sg \
    --description "Security group for Neptune" \
    --vpc-id vpc-xxx

# Allow inbound on port 8182 from your application
aws ec2 authorize-security-group-ingress \
    --group-id sg-xxx \
    --protocol tcp \
    --port 8182 \
    --source-group sg-yyy  # Your application security group
```

#### Step 3: Create Neptune Serverless Cluster

```bash
aws neptune create-db-cluster \
    --db-cluster-identifier edgequake-graph \
    --engine neptune \
    --serverless-v2-scaling-configuration \
        MinCapacity=2.5,MaxCapacity=128 \
    --db-subnet-group-name edgequake-neptune-subnet \
    --vpc-security-group-ids sg-xxx
```

#### Step 4: Create Neptune Instance

```bash
aws neptune create-db-instance \
    --db-instance-identifier edgequake-graph-instance \
    --db-instance-class db.serverless \
    --engine neptune \
    --db-cluster-identifier edgequake-graph
```

#### Step 5: Get Endpoint

```bash
aws neptune describe-db-clusters \
    --db-cluster-identifier edgequake-graph \
    --query 'DBClusters[0].Endpoint' \
    --output text

# Output: edgequake-graph.cluster-xxx.us-east-1.neptune.amazonaws.com
```

### Option 2: Using AWS CDK (TypeScript)

```typescript
import * as cdk from 'aws-cdk-lib';
import * as neptune from 'aws-cdk-lib/aws-neptune';
import * as ec2 from 'aws-cdk-lib/aws-ec2';

export class EdgeQuakeNeptuneStack extends cdk.Stack {
  constructor(scope: cdk.App, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    // VPC
    const vpc = new ec2.Vpc(this, 'VPC', {
      maxAzs: 2,
      natGateways: 1,
    });

    // Security group
    const sg = new ec2.SecurityGroup(this, 'NeptuneSG', {
      vpc,
      description: 'Neptune security group',
    });

    // Neptune Serverless cluster
    const cluster = new neptune.DatabaseCluster(this, 'NeptuneCluster', {
      vpc,
      instanceType: neptune.InstanceType.SERVERLESS,
      serverlessScalingConfiguration: {
        minCapacity: 2.5,
        maxCapacity: 128,
      },
      securityGroups: [sg],
    });

    // Output endpoint
    new cdk.CfnOutput(this, 'NeptuneEndpoint', {
      value: cluster.clusterEndpoint.socketAddress,
    });
  }
}
```

### Option 3: Using Terraform

```hcl
resource "aws_neptune_cluster" "edgequake" {
  cluster_identifier                  = "edgequake-graph"
  engine                              = "neptune"
  backup_retention_period             = 5
  preferred_backup_window             = "07:00-09:00"
  skip_final_snapshot                 = true
  iam_database_authentication_enabled = true
  apply_immediately                   = true

  serverlessv2_scaling_configuration {
    min_capacity = 2.5
    max_capacity = 128
  }

  vpc_security_group_ids = [aws_security_group.neptune.id]
  db_subnet_group_name   = aws_neptune_subnet_group.edgequake.name
}

resource "aws_neptune_cluster_instance" "edgequake" {
  count              = 1
  identifier         = "edgequake-graph-${count.index}"
  cluster_identifier = aws_neptune_cluster.edgequake.id
  instance_class     = "db.serverless"
  engine             = "neptune"
}

output "neptune_endpoint" {
  value = aws_neptune_cluster.edgequake.endpoint
}
```

## Usage

### Basic Example

```rust
use edgequake_storage::GraphStorage;
use edgequake_storage_aws::{NeptuneConfig, NeptuneGraphStorage};
use serde_json::json;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure
    let config = NeptuneConfig::new("cluster.region.neptune.amazonaws.com:8182")
        .with_namespace("workspace-123")
        .with_iam_auth(true);

    // Create storage
    let storage = NeptuneGraphStorage::new(config).await?;
    storage.initialize().await?;

    // Add node
    let mut props = HashMap::new();
    props.insert("name".to_string(), json!("Alice"));
    props.insert("type".to_string(), json!("person"));

    storage.upsert_node("user-001", props).await?;

    // Add edge
    let mut edge_props = HashMap::new();
    edge_props.insert("relationship".to_string(), json!("friend"));

    storage.upsert_edge("user-001", "user-002", edge_props).await?;

    // Query graph
    let graph = storage.get_knowledge_graph("user-001", 2, 100).await?;
    println!("Found {} nodes, {} edges", graph.node_count(), graph.edge_count());

    Ok(())
}
```

### Configuration Options

```rust
let config = NeptuneConfig::new("endpoint:8182")
    .with_namespace("workspace-id")      // Isolate per workspace
    .with_iam_auth(true)                 // Use IAM (recommended)
    .with_pool_size(10)                  // Connection pool size
    .with_timeout(30)                    // Query timeout (seconds)
    .with_ssl(true);                     // Enable SSL/TLS
```

### Running Examples

```bash
# Set Neptune endpoint
export NEPTUNE_ENDPOINT=my-cluster.region.neptune.amazonaws.com:8182

# Run basic example
cargo run --example neptune_graph_basic --features neptune

# Run with data clearing
CLEAR_DATA=1 cargo run --example neptune_graph_basic --features neptune
```

### Running Tests

```bash
# Set Neptune endpoint
export NEPTUNE_ENDPOINT=my-cluster.region.neptune.amazonaws.com:8182

# Run tests (single-threaded to avoid conflicts)
cargo test --features neptune -- --test-threads=1

# Run specific test
cargo test --features neptune test_neptune_node_operations -- --nocapture
```

## Gremlin Primer

### Basic Gremlin Concepts

Gremlin is a graph traversal language. Think of it as "SQL for graphs."

#### Vertices (Nodes)

```gremlin
// Add a vertex
g.addV('Person').property('name', 'Alice').property('age', 30)

// Find a vertex by property
g.V().has('name', 'Alice')

// Get all vertices
g.V()

// Update a vertex property
g.V().has('name', 'Alice').property('age', 31)

// Delete a vertex
g.V().has('name', 'Alice').drop()
```

#### Edges (Relationships)

```gremlin
// Add an edge between two vertices
g.V().has('name', 'Alice').addE('knows').to(g.V().has('name', 'Bob'))

// Find edges
g.E().hasLabel('knows')

// Get edges from a vertex
g.V().has('name', 'Alice').outE()  // Outgoing edges
g.V().has('name', 'Alice').inE()   // Incoming edges
g.V().has('name', 'Alice').bothE() // All edges

// Delete an edge
g.V().has('name', 'Alice').outE('knows').where(inV().has('name', 'Bob')).drop()
```

#### Traversals

```gremlin
// Get neighbors (1 hop)
g.V().has('name', 'Alice').out()

// Get neighbors of neighbors (2 hops)
g.V().has('name', 'Alice').out().out()

// Get all paths up to 3 hops
g.V().has('name', 'Alice').repeat(both().simplePath()).times(3)

// Get specific relationship types
g.V().has('name', 'Alice').out('knows')

// Filter by properties
g.V().has('name', 'Alice').out().has('age', gt(25))
```

#### Aggregations

```gremlin
// Count vertices
g.V().count()

// Count edges
g.E().count()

// Group by property
g.V().group().by('type').by(count())

// Get node degree (connection count)
g.V().has('name', 'Alice').bothE().count()

// Find most connected nodes
g.V().project('name', 'degree')
  .by('name')
  .by(bothE().count())
  .order().by(select('degree'), desc)
  .limit(10)
```

### Gremlin vs Cypher

If you're coming from Neo4j/Cypher, here's a comparison:

| Operation | Cypher | Gremlin |
|-----------|--------|---------|
| **Add vertex** | `CREATE (n:Person {name: 'Alice'})` | `g.addV('Person').property('name', 'Alice')` |
| **Find vertex** | `MATCH (n:Person {name: 'Alice'}) RETURN n` | `g.V().has('Person', 'name', 'Alice')` |
| **Add edge** | `MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) CREATE (a)-[:KNOWS]->(b)` | `g.V().has('name', 'Alice').addE('knows').to(g.V().has('name', 'Bob'))` |
| **Traverse** | `MATCH (a:Person {name: 'Alice'})-[:KNOWS*1..2]-(b) RETURN b` | `g.V().has('name', 'Alice').repeat(both('knows')).times(2)` |
| **Count** | `MATCH (n:Person) RETURN count(n)` | `g.V().hasLabel('Person').count()` |

### Common Patterns

#### Pattern 1: Upsert (Update or Insert)

```gremlin
g.V().has('id', 'user-123').fold()
  .coalesce(
    unfold(),                    // If exists, return it
    addV('User').property('id', 'user-123')  // Else create
  )
  .property('name', 'Alice')     // Set/update property
```

#### Pattern 2: Multi-hop Traversal with Limit

```gremlin
g.V().has('id', 'start')
  .repeat(both().simplePath())   // Avoid cycles
  .times(3)                      // Max 3 hops
  .limit(100)                    // Max 100 results
  .dedup()                       // Remove duplicates
```

#### Pattern 3: Find Shortest Path

```gremlin
g.V().has('id', 'A')
  .repeat(both().simplePath())
  .until(has('id', 'B'))
  .path()
  .limit(1)
```

#### Pattern 4: Get Subgraph

```gremlin
g.V().has('id', 'start')
  .repeat(both().simplePath())
  .times(2)
  .path()                        // Get all paths
  .by(elementMap())              // Include all properties
```

## Performance

### Query Performance

| Operation | Latency (avg) | Notes |
|-----------|---------------|-------|
| Get node by ID | 1-5ms | Indexed lookup |
| Get node neighbors | 5-15ms | Single hop |
| Traverse 2-3 hops | 20-100ms | Depends on graph density |
| Complex subgraph | 100-500ms | Multiple hops, filters |
| Full graph scan | 1-10s | Avoid in production |

### Optimization Tips

#### 1. Use Indexes

Neptune automatically indexes:
- Vertex IDs
- Edge labels
- Properties used in `has()` steps

```gremlin
// Good: Uses index
g.V().has('id', 'user-123')

// Bad: Full scan
g.V().filter(values('id').is('user-123'))
```

#### 2. Limit Early

```gremlin
// Good: Limit early
g.V().has('type', 'user').limit(100).out()

// Bad: Limit late
g.V().has('type', 'user').out().limit(100)
```

#### 3. Use `simplePath()` for Traversals

```gremlin
// Prevents infinite loops
g.V().repeat(both().simplePath()).times(3)
```

#### 4. Batch Operations

```rust
// Instead of this:
for node in nodes {
    storage.upsert_node(&node.id, node.properties).await?;
}

// Do this:
storage.upsert_nodes_batch(&nodes).await?;
```

#### 5. Use Connection Pooling

```rust
let config = NeptuneConfig::new("endpoint")
    .with_pool_size(20);  // Increase for high concurrency
```

### Monitoring

View Neptune metrics in CloudWatch:

- **GremlinRequestsPerSec**: Query throughput
- **GremlinWebSocketClientErrors**: Client errors
- **CPUUtilization**: Compute usage
- **NCUs**: Serverless capacity (scales 2.5-128)
- **VolumeReadIOPs/VolumeWriteIOPs**: Storage I/O

## Security

### IAM Authentication (Recommended)

```rust
let config = NeptuneConfig::new("endpoint")
    .with_iam_auth(true);  // Use IAM instead of passwords
```

#### IAM Policy

```json
{
    "Version": "2012-10-17",
    "Statement": [{
        "Effect": "Allow",
        "Action": [
            "neptune-db:ReadDataViaQuery",
            "neptune-db:WriteDataViaQuery",
            "neptune-db:DeleteDataViaQuery"
        ],
        "Resource": "arn:aws:neptune-db:region:account:cluster-*/*"
    }]
}
```

### Network Security

#### VPC Placement

Neptune runs in your VPC (private subnets):

```
┌─────────────────────────────────────┐
│          VPC (10.0.0.0/16)          │
│                                     │
│  ┌─────────────────────────────┐   │
│  │   Public Subnet             │   │
│  │   (NAT Gateway, ALB)        │   │
│  └─────────────────────────────┘   │
│                                     │
│  ┌─────────────────────────────┐   │
│  │   Private Subnet            │   │
│  │   ┌──────────────────────┐  │   │
│  │   │  Neptune Cluster     │  │   │
│  │   │  (No internet)       │  │   │
│  │   └──────────────────────┘  │   │
│  │   ┌──────────────────────┐  │   │
│  │   │  Application         │  │   │
│  │   │  (ECS/Lambda)        │  │   │
│  │   └──────────────────────┘  │   │
│  └─────────────────────────────┘   │
└─────────────────────────────────────┘
```

#### Security Group Rules

```bash
# Neptune security group
# Allow inbound only from application security group
aws ec2 authorize-security-group-ingress \
    --group-id sg-neptune \
    --protocol tcp \
    --port 8182 \
    --source-group sg-application
```

### Encryption

#### At Rest

```bash
# Enable encryption at cluster creation
aws neptune create-db-cluster \
    --db-cluster-identifier edgequake \
    --storage-encrypted \
    --kms-key-id arn:aws:kms:region:account:key/xxx
```

#### In Transit

SSL/TLS is enabled by default on port 8182.

```rust
let config = NeptuneConfig::new("endpoint")
    .with_ssl(true);  // Enabled by default
```

### Audit Logging

Enable CloudWatch Logs:

```bash
aws neptune modify-db-cluster \
    --db-cluster-identifier edgequake \
    --cloudwatch-logs-export-configuration '{"EnableLogTypes": ["audit"]}'
```

Logs all queries, mutations, and access patterns.

## Troubleshooting

### Issue: Cannot Connect

**Error:**
```
NeptuneError("Failed to connect: connection refused")
```

**Solutions:**

1. **Check VPC/Security Groups**:
   ```bash
   # Ensure your app is in the same VPC or has VPC peering
   # Check security group allows port 8182
   aws ec2 describe-security-groups --group-ids sg-xxx
   ```

2. **Verify endpoint**:
   ```bash
   aws neptune describe-db-clusters \
       --db-cluster-identifier edgequake \
       --query 'DBClusters[0].Endpoint'
   ```

3. **Test connectivity**:
   ```bash
   # From your application instance
   nc -zv your-cluster.region.neptune.amazonaws.com 8182
   ```

### Issue: IAM Authentication Failed

**Error:**
```
NeptuneError("IAM authentication failed")
```

**Solutions:**

1. **Check IAM role**:
   ```bash
   # Ensure your EC2/Lambda has Neptune permissions
   aws iam get-role-policy --role-name YourRole --policy-name NeptuneAccess
   ```

2. **Verify credentials**:
   ```bash
   # On EC2 instance
   aws sts get-caller-identity
   ```

3. **Enable IAM auth on cluster**:
   ```bash
   aws neptune modify-db-cluster \
       --db-cluster-identifier edgequake \
       --iam-database-authentication-enabled
   ```

### Issue: Slow Queries

**Symptoms**: Queries taking >5 seconds

**Solutions:**

1. **Check query patterns**:
   - Avoid full graph scans: `g.V()` → `g.V().has('id', 'xxx')`
   - Use `limit()` early in traversals
   - Add `simplePath()` to prevent loops

2. **Monitor NCUs** (Serverless):
   ```bash
   # Check if hitting capacity limits
   aws cloudwatch get-metric-statistics \
       --namespace AWS/Neptune \
       --metric-name ServerlessDatabaseCapacity \
       --dimensions Name=DBClusterIdentifier,Value=edgequake \
       --start-time 2024-01-01T00:00:00Z \
       --end-time 2024-01-01T23:59:59Z \
       --period 3600 \
       --statistics Average
   ```

3. **Increase capacity**:
   ```bash
   # Increase max NCUs
   aws neptune modify-db-cluster \
       --db-cluster-identifier edgequake \
       --serverless-v2-scaling-configuration MinCapacity=2.5,MaxCapacity=256
   ```

### Issue: High Costs

**Check billing**:
```bash
aws ce get-cost-and-usage \
    --time-period Start=2024-01-01,End=2024-01-31 \
    --granularity MONTHLY \
    --metrics BlendedCost \
    --filter file://neptune-filter.json
```

**Optimize:**

1. **Reduce min NCUs** (dev/staging):
   ```bash
   aws neptune modify-db-cluster \
       --db-cluster-identifier edgequake-dev \
       --serverless-v2-scaling-configuration MinCapacity=0.5,MaxCapacity=16
   ```

2. **Delete unused read replicas**
3. **Reduce backup retention**:
   ```bash
   aws neptune modify-db-cluster \
       --db-cluster-identifier edgequake \
       --backup-retention-period 3  # Down from 7 days
   ```

### Issue: Graph Data Inconsistency

**Symptoms**: Missing nodes/edges, duplicate data

**Solutions:**

1. **Check namespace isolation**:
   ```rust
   // Ensure each workspace uses unique namespace
   let config = NeptuneConfig::new("endpoint")
       .with_namespace(&format!("workspace-{}", workspace_id));
   ```

2. **Verify transactions**:
   - Neptune doesn't support multi-statement transactions
   - Use application-level retries for failed operations

3. **Clear and rebuild**:
   ```rust
   storage.clear().await?;
   // Re-import data
   ```

## Resources

- [Neptune Documentation](https://docs.aws.amazon.com/neptune/)
- [Gremlin Reference](https://tinkerpop.apache.org/docs/current/reference/)
- [Neptune Best Practices](https://docs.aws.amazon.com/neptune/latest/userguide/best-practices.html)
- [Neptune Pricing](https://aws.amazon.com/neptune/pricing/)
- [EdgeQuake AWS Migration Design](../../AWS_MIGRATION_DESIGN.md)

## Support

Need help? Contact:

- 📧 Email: support@edgequake.io
- 💬 [GitHub Discussions](https://github.com/edgequake/edgequake/discussions)
- 🐛 [Issue Tracker](https://github.com/edgequake/edgequake/issues)
