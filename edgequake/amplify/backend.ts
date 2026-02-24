// Required env vars:
// NAMESPACE_TABLE        - DynamoDB table for namespace registry, PK+SK schema (default: created by sandbox)
// PIPELINE_STATE_TABLE   - DynamoDB table for pipeline execution state, namespace+id schema (default: created by sandbox)
// BM25_S3_BUCKET         - S3 bucket for BM25 Iceberg table data (default: created by sandbox as edgequake-bm25-{account}-{region})
// ATHENA_BM25_DATABASE   - Glue database for BM25 Athena tables (default: created by sandbox as edgequake_bm25)
// ATHENA_WORKGROUP       - Athena workgroup, must be v3 for Iceberg (default: created by sandbox as edgequake-v3)
// PMCP_ACCOUNT_ID        - AWS account ID for pmcp.run cross-account access (default: none, same-account only)

import { defineBackend } from '@aws-amplify/backend';
import {
  aws_athena as athena,
  aws_dynamodb as dynamodb,
  aws_glue as glue,
  aws_iam as iam,
  aws_s3 as s3,
  RemovalPolicy,
  Stack,
} from 'aws-cdk-lib';
import { Construct } from 'constructs';
import { createHash } from 'node:crypto';
import { auth } from './auth/resource.js';
import { data } from './data/resource.js';

// ---------------------------------------------------------------------------
// 1. Define the Amplify Gen 2 backend
// ---------------------------------------------------------------------------

const backend = defineBackend({ auth, data });

// ---------------------------------------------------------------------------
// 2. DynamoDB namespace table
// ---------------------------------------------------------------------------
//
// Two modes:
// - NAMESPACE_TABLE env var set → reference existing external table (production)
// - NAMESPACE_TABLE not set     → create the table in the Amplify stack (sandbox)
//
// The table uses a single-table design: PK (String) + SK (String) composite key.
// Items store JSON-serialized data in a 'data' attribute.
// Key patterns: PK=NS#{slug}/SK=META|CONFIG|SCHEMA|DESCRIPTOR|LATEST_RUN,
//               PK=NAMESPACES/SK={slug} for listing.

const infraStack = backend.createStack('NamespaceInfra');

const externalTableName = process.env.NAMESPACE_TABLE;

const namespaceTable = externalTableName
  ? dynamodb.Table.fromTableName(
      infraStack,
      'NamespaceTable',
      externalTableName,
    )
  : new dynamodb.Table(infraStack, 'NamespaceTable', {
      partitionKey: { name: 'PK', type: dynamodb.AttributeType.STRING },
      sortKey: { name: 'SK', type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      removalPolicy: RemovalPolicy.DESTROY,
    });

// ---------------------------------------------------------------------------
// 2b. DynamoDB pipeline state table
// ---------------------------------------------------------------------------
//
// Stores batch pipeline execution state (job progress, document status).
// Key schema: namespace (String PK) + id (String SK) — different from the
// namespace registry table which uses PK/SK.
//
// Two modes (same pattern as namespace table):
// - PIPELINE_STATE_TABLE env var set → reference existing external table
// - PIPELINE_STATE_TABLE not set     → create in the Amplify stack (sandbox)

const externalStateTableName = process.env.PIPELINE_STATE_TABLE;

const pipelineStateTable = externalStateTableName
  ? dynamodb.Table.fromTableName(
      infraStack,
      'PipelineStateTable',
      externalStateTableName,
    )
  : new dynamodb.Table(infraStack, 'PipelineStateTable', {
      partitionKey: { name: 'namespace', type: dynamodb.AttributeType.STRING },
      sortKey: { name: 'id', type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      removalPolicy: RemovalPolicy.DESTROY,
    });

// ---------------------------------------------------------------------------
// 2c. BM25 infrastructure: S3 bucket, Glue database, Athena v3 workgroup
// ---------------------------------------------------------------------------
//
// BM25 keyword search uses Athena with Iceberg tables stored on S3.
// Resources:
// - S3 bucket: stores Iceberg table files (Parquet) and Athena query results
// - Glue database: Athena metadata catalog for BM25 tables
// - Athena v3 workgroup: required for Iceberg table support
//
// Same two-mode pattern: env var set → reference existing, not set → create.

const accountId = Stack.of(infraStack).account;
const region = Stack.of(infraStack).region;

// S3 bucket for BM25 Iceberg data + Athena query results
const externalBm25Bucket = process.env.BM25_S3_BUCKET;

const bm25Bucket = externalBm25Bucket
  ? s3.Bucket.fromBucketName(infraStack, 'Bm25Bucket', externalBm25Bucket)
  : new s3.Bucket(infraStack, 'Bm25Bucket', {
      bucketName: `edgequake-bm25-${accountId}-${region}`,
      removalPolicy: RemovalPolicy.DESTROY,
      autoDeleteObjects: true,
    });

// Glue database for Athena BM25 tables (namespace tables created at runtime by pipeline)
const externalBm25Database = process.env.ATHENA_BM25_DATABASE;
const bm25DatabaseName = externalBm25Database || 'edgequake_bm25';

if (!externalBm25Database) {
  new glue.CfnDatabase(infraStack, 'Bm25Database', {
    catalogId: accountId,
    databaseInput: {
      name: bm25DatabaseName,
      description: 'EdgeQuake BM25 inverted index tables (Iceberg)',
    },
  });
}

// Athena v3 workgroup (required for Iceberg table support)
const externalWorkgroup = process.env.ATHENA_WORKGROUP;
const athenaWorkgroupName = externalWorkgroup || 'edgequake-v3';

if (!externalWorkgroup) {
  new athena.CfnWorkGroup(infraStack, 'Bm25Workgroup', {
    name: athenaWorkgroupName,
    state: 'ENABLED',
    workGroupConfiguration: {
      engineVersion: {
        selectedEngineVersion: 'Athena engine version 3',
      },
      resultConfiguration: {
        outputLocation: `s3://${
          externalBm25Bucket || `edgequake-bm25-${accountId}-${region}`
        }/athena-results/`,
      },
    },
  });
}

// Register as AppSync data source — name must match dataSource strings in data/resource.ts
backend.data.addDynamoDbDataSource(
  'NamespaceTableDataSource', // Must match data/resource.ts handler dataSource
  namespaceTable,
);

// Expose resource names to JS resolvers via ctx.env
// Required by TransactWriteItems (which needs explicit table per item, unlike GetItem/PutItem)
backend.data.resources.cfnResources.cfnGraphqlApi.environmentVariables = {
  NAMESPACE_TABLE: namespaceTable.tableName,
  PIPELINE_STATE_TABLE: pipelineStateTable.tableName,
  BM25_S3_BUCKET: bm25Bucket.bucketName,
  ATHENA_BM25_DATABASE: bm25DatabaseName,
  ATHENA_WORKGROUP: athenaWorkgroupName,
};

// ---------------------------------------------------------------------------
// 3. Per-namespace IAM role CDK construct
// ---------------------------------------------------------------------------

/**
 * Properties for creating a per-namespace MCP IAM role.
 */
export interface McpNamespaceRoleProps {
  /** Namespace slug (DNS-safe identifier). */
  namespace: string;
  /** Deployment environment (dev | staging | prod). */
  environment: string;
  /** ARN of the DynamoDB KV table for this namespace's data. */
  kvTableArn: string;
  /** S3 Vectors bucket name for embeddings. */
  vectorBucketName: string;
  /** Neptune cluster ARN for graph queries. */
  neptuneClusterArn: string;
  /**
   * AWS account ID for pmcp.run cross-account access.
   * When undefined, only same-account access is allowed.
   */
  pmcpAccountId?: string;
  /** AWS region for resource ARN construction. */
  region: string;
  /** AWS account ID that owns the resources. */
  account: string;
}

/**
 * CDK construct that creates a per-namespace IAM role for MCP server access.
 *
 * The role is scoped to a single namespace's resources:
 * - DynamoDB: read-only with LeadingKeys condition for namespace isolation
 * - Neptune: cluster-level read-only (per-label IAM not supported by Neptune)
 * - S3 Vectors: scoped to namespace-specific vector index
 *
 * Trust policy supports both same-account and cross-account (pmcp.run) access.
 *
 * This construct is NOT instantiated by Amplify's deploy pipeline.
 * It is defined here for reuse by operators who want to pre-provision
 * namespace roles via `cdk deploy`, or by future runtime IAM role creation
 * logic in the Rust API layer.
 */
export class McpNamespaceRole extends Construct {
  /** The IAM role created for this namespace. */
  public readonly role: iam.Role;
  /** The external ID required for assume-role. */
  public readonly externalId: string;

  constructor(scope: Construct, id: string, props: McpNamespaceRoleProps) {
    super(scope, id);

    // Deterministic external ID: eq-{namespace}-{first 6 chars of sha256(namespace)}
    const hash = createHash('sha256')
      .update(props.namespace)
      .digest('hex')
      .substring(0, 6);
    this.externalId = `eq-${props.namespace}-${hash}`;

    // Build trust principals
    const principals: iam.IPrincipal[] = [
      new iam.AccountPrincipal(props.account),
    ];
    if (props.pmcpAccountId) {
      principals.push(new iam.AccountPrincipal(props.pmcpAccountId));
    }

    this.role = new iam.Role(this, 'Role', {
      roleName: `edgequake-mcp-${props.namespace}-${props.environment}`,
      assumedBy: new iam.CompositePrincipal(...principals).withConditions({
        StringEquals: {
          'sts:ExternalId': this.externalId,
        },
      }),
      description: `MCP server role for namespace "${props.namespace}" (${props.environment})`,
    });

    // --- DynamoDB: read-only with LeadingKeys condition ---
    this.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'DynamoDBNamespaceRead',
        effect: iam.Effect.ALLOW,
        actions: [
          'dynamodb:GetItem',
          'dynamodb:Query',
          'dynamodb:BatchGetItem',
        ],
        resources: [props.kvTableArn],
        conditions: {
          'ForAllValues:StringLike': {
            'dynamodb:LeadingKeys': [`${props.namespace}:*`],
          },
        },
      }),
    );

    // --- Neptune: cluster-level read-only ---
    // Note: Neptune IAM does not support per-label access control (Pitfall 3
    // in research). Namespace isolation is enforced at the application layer
    // via label-prefix filtering, not IAM.
    this.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'NeptuneReadOnly',
        effect: iam.Effect.ALLOW,
        actions: ['neptune-db:ReadDataViaQuery'],
        resources: [props.neptuneClusterArn],
      }),
    );

    // --- S3 Vectors: scoped to namespace-specific index ---
    const vectorBucketArn =
      `arn:aws:s3vectors:${props.region}:${props.account}:vector-bucket/${props.vectorBucketName}`;
    const vectorIndexArn =
      `${vectorBucketArn}/index/${props.namespace}-embeddings`;

    this.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'S3VectorsNamespaceRead',
        effect: iam.Effect.ALLOW,
        actions: [
          's3vectors:QueryVectors',
          's3vectors:GetVectors',
          's3vectors:ListVectors',
        ],
        resources: [vectorBucketArn, vectorIndexArn],
      }),
    );
  }
}

// ---------------------------------------------------------------------------
// Example: Pre-provisioning a namespace role (for operators)
// ---------------------------------------------------------------------------
//
// To pre-provision a role for a specific namespace, uncomment the following
// and run `npx ampx sandbox` or deploy via CDK:
//
// const roleStack = backend.createStack('McpRoles');
// const epsteinRole = new McpNamespaceRole(roleStack, 'EpsteinFilesRole', {
//   namespace: 'epstein-files',
//   environment: 'dev',
//   kvTableArn: 'arn:aws:dynamodb:us-east-1:123456789012:table/edgequake-kv',
//   vectorBucketName: 'edgequake-vectors',
//   neptuneClusterArn: 'arn:aws:neptune-db:us-east-1:123456789012:cluster-resource-id/*',
//   pmcpAccountId: process.env.PMCP_ACCOUNT_ID,
//   region: 'us-east-1',
//   account: '123456789012',
// });
// console.log('Role ARN:', epsteinRole.role.roleArn);
// console.log('External ID:', epsteinRole.externalId);
