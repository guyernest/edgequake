// Required env vars:
// NAMESPACE_TABLE - DynamoDB table name for namespace registry (default: edgequake-namespaces)
// PMCP_ACCOUNT_ID - AWS account ID for pmcp.run cross-account access (default: none, same-account only)

import { defineBackend } from '@aws-amplify/backend';
import {
  aws_dynamodb as dynamodb,
  aws_iam as iam,
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
// 2. External DynamoDB table reference
// ---------------------------------------------------------------------------

const namespaceTableName =
  process.env.NAMESPACE_TABLE ?? 'edgequake-namespaces';

const existingInfraStack = backend.createStack('ExistingInfra');

const namespaceTable = dynamodb.Table.fromTableName(
  existingInfraStack,
  'NamespaceTable',
  namespaceTableName,
);

// Register as AppSync data source — name must match dataSource strings in data/resource.ts
backend.data.addDynamoDbDataSource(
  'NamespaceTableDataSource', // Must match data/resource.ts handler dataSource
  namespaceTable,
);

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
