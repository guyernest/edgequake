# Exercise: CDK Stack Design

::: exercise
id: ch10-cdk-stack
difficulty: intermediate
time: 30 minutes
:::

In chapter 10 you learned how EdgeQuake deploys to AWS using S3 Vectors for
embeddings, DynamoDB for key-value storage, and IAM for access control. AWS CDK
lets you define this infrastructure as TypeScript code, making deployments
reproducible, version-controlled, and testable.

In this exercise you will design a CDK stack that provisions the core storage
infrastructure for EdgeQuake: an S3 Vectors bucket with an index, a DynamoDB
table, and an IAM role with least-privilege permissions.

::: objectives
thinking:
  - Understand the CDK construct hierarchy (App, Stack, Construct)
  - Recognize why Infrastructure as Code is essential for reproducible deployments
  - Reason about least-privilege IAM policies for production services
doing:
  - Define an S3 Vectors bucket and index with correct dimensions
  - Create a DynamoDB table with appropriate key schema
  - Configure IAM roles with scoped permissions
  - Export resource ARNs via CfnOutput
:::

::: discussion
- Why does EdgeQuake use CDK (TypeScript) instead of raw CloudFormation YAML?
- What would happen if you set RemovalPolicy.DESTROY on a production DynamoDB table?
- How does the vector dimension in the CDK stack relate to the embedding model?
:::

::: starter file="lib/edgequake-storage-stack.ts"
```typescript
import * as cdk from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as s3vectors from 'aws-cdk-lib/aws-s3vectors';
import { Construct } from 'constructs';

export interface EdgeQuakeStorageProps extends cdk.StackProps {
  /** Embedding vector dimension (must match the model output). */
  vectorDimension: number;
  /** Environment name (e.g., 'dev', 'staging', 'prod'). */
  environment: string;
}

export class EdgeQuakeStorageStack extends cdk.Stack {
  /** ARN of the S3 Vectors bucket. */
  public readonly vectorBucketArn: string;
  /** Name of the DynamoDB table. */
  public readonly kvTableName: string;
  /** ARN of the application IAM role. */
  public readonly appRoleArn: string;

  constructor(scope: Construct, id: string, props: EdgeQuakeStorageProps) {
    super(scope, id, props);

    const { vectorDimension, environment } = props;

    // -------------------------------------------------------
    // 1. S3 Vectors Bucket and Index
    // -------------------------------------------------------
    // TODO: Create a CfnVectorBucket for storing embeddings.
    // The bucket name should include the environment for uniqueness.
    //
    // Then create a CfnVectorIndex on that bucket with:
    //   - dimension: vectorDimension
    //   - distanceMetric: 'cosine'
    //
    // Hint: Use s3vectors.CfnVectorBucket and s3vectors.CfnVectorIndex

    // const vectorBucket = ...
    // const vectorIndex = ...

    // -------------------------------------------------------
    // 2. DynamoDB Table for KV Storage
    // -------------------------------------------------------
    // TODO: Create a DynamoDB table for key-value storage.
    //   - Partition key: 'namespace' (string)
    //   - Sort key: 'id' (string)
    //   - Billing: PAY_PER_REQUEST for dev, PROVISIONED for prod
    //   - Removal policy: DESTROY for dev, RETAIN for prod
    //
    // Hint: Use dynamodb.Table with appropriate props.

    // const kvTable = ...

    // -------------------------------------------------------
    // 3. IAM Role for the Application
    // -------------------------------------------------------
    // TODO: Create an IAM role that the EdgeQuake application assumes.
    // It needs permissions for:
    //   - S3 Vectors: s3vectors:PutVectors, s3vectors:GetVectors,
    //     s3vectors:QueryVectors, s3vectors:DeleteVectors
    //   - DynamoDB: dynamodb:GetItem, dynamodb:PutItem,
    //     dynamodb:DeleteItem, dynamodb:Query, dynamodb:BatchWriteItem
    //
    // DO NOT grant wildcard (*) permissions. Scope each policy
    // to the specific resource ARN.

    // const appRole = ...

    // -------------------------------------------------------
    // 4. Outputs
    // -------------------------------------------------------
    // TODO: Export the resource identifiers so other stacks or
    // deployment scripts can reference them.
    //
    // new cdk.CfnOutput(this, 'VectorBucketArn', { value: ... });
    // new cdk.CfnOutput(this, 'KVTableName', { value: ... });
    // new cdk.CfnOutput(this, 'AppRoleArn', { value: ... });
  }
}
```
:::

::: starter file="test/edgequake-storage-stack.test.ts"
```typescript
import * as cdk from 'aws-cdk-lib';
import { Template } from 'aws-cdk-lib/assertions';
import { EdgeQuakeStorageStack } from '../lib/edgequake-storage-stack';

describe('EdgeQuakeStorageStack', () => {
  let template: Template;

  beforeAll(() => {
    const app = new cdk.App();
    const stack = new EdgeQuakeStorageStack(app, 'TestStack', {
      vectorDimension: 1536,
      environment: 'dev',
    });
    template = Template.fromStack(stack);
  });

  test('creates S3 Vectors bucket', () => {
    template.hasResourceProperties('AWS::S3Vectors::VectorBucket', {});
  });

  test('creates S3 Vectors index with correct dimension', () => {
    template.hasResourceProperties('AWS::S3Vectors::VectorIndex', {
      Dimension: 1536,
      DistanceMetric: 'cosine',
    });
  });

  test('creates DynamoDB table with correct key schema', () => {
    template.hasResourceProperties('AWS::DynamoDB::Table', {
      KeySchema: [
        { AttributeName: 'namespace', KeyType: 'HASH' },
        { AttributeName: 'id', KeyType: 'RANGE' },
      ],
      BillingMode: 'PAY_PER_REQUEST',
    });
  });

  test('creates IAM role with scoped permissions', () => {
    template.hasResourceProperties('AWS::IAM::Role', {
      AssumeRolePolicyDocument: {
        Statement: [
          {
            Effect: 'Allow',
            Principal: { Service: 'ecs-tasks.amazonaws.com' },
            Action: 'sts:AssumeRole',
          },
        ],
      },
    });
  });

  test('does not grant wildcard permissions', () => {
    const roles = template.findResources('AWS::IAM::Policy');
    const roleJson = JSON.stringify(roles);
    // Verify no "Action": "*" in any policy
    expect(roleJson).not.toContain('"Action":"*"');
  });

  test('exports resource identifiers', () => {
    template.hasOutput('VectorBucketArn', {});
    template.hasOutput('KVTableName', {});
    template.hasOutput('AppRoleArn', {});
  });
});
```
:::

::: hint level=1 title="S3 Vectors resources"
```typescript
const vectorBucket = new s3vectors.CfnVectorBucket(this, 'VectorBucket', {
  vectorBucketName: `edgequake-vectors-${environment}`,
});
```
The `CfnVectorIndex` references the bucket and specifies dimension and distance metric.
:::

::: hint level=2 title="DynamoDB conditional configuration"
Use a ternary to switch between dev and prod settings:
```typescript
const kvTable = new dynamodb.Table(this, 'KVTable', {
  tableName: `edgequake-kv-${environment}`,
  partitionKey: { name: 'namespace', type: dynamodb.AttributeType.STRING },
  sortKey: { name: 'id', type: dynamodb.AttributeType.STRING },
  billingMode: environment === 'prod'
    ? dynamodb.BillingMode.PROVISIONED
    : dynamodb.BillingMode.PAY_PER_REQUEST,
  removalPolicy: environment === 'prod'
    ? cdk.RemovalPolicy.RETAIN
    : cdk.RemovalPolicy.DESTROY,
});
```
:::

::: hint level=3 title="Scoped IAM policy"
```typescript
appRole.addToPolicy(new iam.PolicyStatement({
  actions: [
    'dynamodb:GetItem',
    'dynamodb:PutItem',
    'dynamodb:DeleteItem',
    'dynamodb:Query',
    'dynamodb:BatchWriteItem',
  ],
  resources: [kvTable.tableArn],
}));
```
:::

::: solution
```typescript
import * as cdk from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as s3vectors from 'aws-cdk-lib/aws-s3vectors';
import { Construct } from 'constructs';

export interface EdgeQuakeStorageProps extends cdk.StackProps {
  vectorDimension: number;
  environment: string;
}

export class EdgeQuakeStorageStack extends cdk.Stack {
  public readonly vectorBucketArn: string;
  public readonly kvTableName: string;
  public readonly appRoleArn: string;

  constructor(scope: Construct, id: string, props: EdgeQuakeStorageProps) {
    super(scope, id, props);

    const { vectorDimension, environment } = props;

    // 1. S3 Vectors Bucket and Index
    const vectorBucket = new s3vectors.CfnVectorBucket(this, 'VectorBucket', {
      vectorBucketName: `edgequake-vectors-${environment}`,
    });

    const vectorIndex = new s3vectors.CfnVectorIndex(this, 'VectorIndex', {
      vectorBucketName: vectorBucket.vectorBucketName!,
      indexName: `edgequake-embeddings-${environment}`,
      dimension: vectorDimension,
      distanceMetric: 'cosine',
    });
    vectorIndex.addDependency(vectorBucket);

    // 2. DynamoDB Table for KV Storage
    const kvTable = new dynamodb.Table(this, 'KVTable', {
      tableName: `edgequake-kv-${environment}`,
      partitionKey: { name: 'namespace', type: dynamodb.AttributeType.STRING },
      sortKey: { name: 'id', type: dynamodb.AttributeType.STRING },
      billingMode: environment === 'prod'
        ? dynamodb.BillingMode.PROVISIONED
        : dynamodb.BillingMode.PAY_PER_REQUEST,
      removalPolicy: environment === 'prod'
        ? cdk.RemovalPolicy.RETAIN
        : cdk.RemovalPolicy.DESTROY,
    });

    // 3. IAM Role for the Application
    const appRole = new iam.Role(this, 'AppRole', {
      roleName: `edgequake-app-${environment}`,
      assumedBy: new iam.ServicePrincipal('ecs-tasks.amazonaws.com'),
      description: 'IAM role for EdgeQuake application tasks',
    });

    // S3 Vectors permissions
    appRole.addToPolicy(new iam.PolicyStatement({
      actions: [
        's3vectors:PutVectors',
        's3vectors:GetVectors',
        's3vectors:QueryVectors',
        's3vectors:DeleteVectors',
      ],
      resources: [vectorBucket.attrVectorBucketArn],
    }));

    // DynamoDB permissions
    appRole.addToPolicy(new iam.PolicyStatement({
      actions: [
        'dynamodb:GetItem',
        'dynamodb:PutItem',
        'dynamodb:DeleteItem',
        'dynamodb:Query',
        'dynamodb:BatchWriteItem',
      ],
      resources: [kvTable.tableArn],
    }));

    // 4. Outputs
    this.vectorBucketArn = vectorBucket.attrVectorBucketArn;
    this.kvTableName = kvTable.tableName;
    this.appRoleArn = appRole.roleArn;

    new cdk.CfnOutput(this, 'VectorBucketArn', {
      value: vectorBucket.attrVectorBucketArn,
      description: 'ARN of the S3 Vectors bucket',
    });

    new cdk.CfnOutput(this, 'KVTableName', {
      value: kvTable.tableName,
      description: 'Name of the DynamoDB KV table',
    });

    new cdk.CfnOutput(this, 'AppRoleArn', {
      value: appRole.roleArn,
      description: 'ARN of the application IAM role',
    });
  }
}
```

### Explanation

The CDK stack provisions three resources:

- **S3 Vectors bucket + index**: The `CfnVectorBucket` creates the storage bucket,
  and `CfnVectorIndex` defines how vectors are indexed. The dimension (1536 for
  OpenAI's `text-embedding-3-small`) must match the embedding model. The distance
  metric is `cosine` because EdgeQuake converts distance to similarity with `1 - distance`.

- **DynamoDB table**: Uses composite key (`namespace` + `id`) for tenant-isolated
  key-value storage. PAY_PER_REQUEST billing avoids capacity planning for development.
  `RemovalPolicy.RETAIN` in production prevents accidental data loss when updating stacks.

- **IAM role**: Follows least-privilege by granting only the specific DynamoDB and
  S3 Vectors actions needed. The role is assumable by ECS tasks (for container deployments).
  No wildcard actions are used.

- **CfnOutput**: Exports resource identifiers so deployment scripts and other stacks
  can reference them without hardcoding ARNs.
:::

::: tests mode=local
```typescript
// Tests are in the starter code above (test/edgequake-storage-stack.test.ts).
// Run with: npx jest
//
// The snapshot test verifies the synthesized CloudFormation template
// matches expectations. If you intentionally change the stack, update
// the snapshot with: npx jest --updateSnapshot
```
:::

::: reflection
- How would you add a Neptune Serverless graph database to this stack? What
  additional IAM permissions would the application role need?
- The vector dimension is passed as a prop. What would break if you changed
  it after initial deployment with existing data?
- How would you set up separate dev/staging/prod environments using this stack?
  Would you use separate AWS accounts, separate stacks, or CDK stages?
:::
