#!/usr/bin/env node
import 'source-map-support/register';
import * as cdk from 'aws-cdk-lib';
import { EdgeQuakeCoreStack } from '../lib/edgequake-core-stack';
import { EdgeQuakeNeptuneStack } from '../lib/edgequake-neptune-stack';
import { resolveConfig, Environment } from '../lib/edgequake-config';

const app = new cdk.App();

// --- Read context parameters ---
const tenantId = app.node.tryGetContext('tenantId')
  || process.env.TENANT_ID
  || 'default';

const environment = (app.node.tryGetContext('environment')
  || process.env.ENVIRONMENT
  || 'dev') as Environment;

const account = app.node.tryGetContext('account')
  || process.env.CDK_DEFAULT_ACCOUNT
  || process.env.AWS_ACCOUNT_ID;

const region = app.node.tryGetContext('region')
  || process.env.CDK_DEFAULT_REGION
  || process.env.AWS_DEFAULT_REGION
  || 'us-east-1';

if (!account) {
  console.error('ERROR: AWS account not specified. Set CDK_DEFAULT_ACCOUNT or pass --context account=XXXX');
  process.exit(1);
}

// --- Build resolved config with context overrides ---
const config = resolveConfig(
  {
    tenantId,
    environment,
    dynamoTableName: app.node.tryGetContext('dynamoTable') || undefined,
    s3BucketName: app.node.tryGetContext('s3Bucket') || undefined,
    deployNeptune: contextBool(app, 'deployNeptune'),
    existingVpcId: app.node.tryGetContext('existingVpcId') || undefined,
    neptuneMinCapacity: contextInt(app, 'neptuneMinCapacity'),
    neptuneMaxCapacity: contextInt(app, 'neptuneMaxCapacity'),
    deployAthena: contextBool(app, 'deployAthena'),
  },
  account,
  region,
);

const env = { account, region };

// --- Core Stack (DynamoDB, S3, IAM, SSM) ---
const coreStack = new EdgeQuakeCoreStack(app, `EdgeQuakeCore-${tenantId}-${environment}`, {
  env,
  config,
  description: `EdgeQuake core resources for tenant ${tenantId} (${environment})`,
});

// Apply tags to core stack
applyTags(coreStack, config.tags);

// --- Neptune Stack (VPC, Neptune, Security Groups, SSM) ---
if (config.deployNeptune) {
  const neptuneStack = new EdgeQuakeNeptuneStack(app, `EdgeQuakeNeptune-${tenantId}-${environment}`, {
    env,
    config,
    batchRole: coreStack.batchRole,
    s3BucketArn: coreStack.bucket.bucketArn,
    description: `EdgeQuake Neptune/VPC resources for tenant ${tenantId} (${environment})`,
  });

  neptuneStack.addDependency(coreStack);
  applyTags(neptuneStack, config.tags);
}

app.synth();

// --- Helper functions ---

function contextBool(cdkApp: cdk.App, key: string): boolean | undefined {
  const val = cdkApp.node.tryGetContext(key);
  if (val === undefined || val === null) return undefined;
  if (typeof val === 'boolean') return val;
  return val === 'true';
}

function contextInt(cdkApp: cdk.App, key: string): number | undefined {
  const val = cdkApp.node.tryGetContext(key);
  if (val === undefined || val === null) return undefined;
  const num = parseInt(val, 10);
  return isNaN(num) ? undefined : num;
}

function applyTags(stack: cdk.Stack, tags: Record<string, string>): void {
  for (const [key, value] of Object.entries(tags)) {
    cdk.Tags.of(stack).add(key, value);
  }
}
