export type Environment = 'dev' | 'staging' | 'prod';

export interface EdgeQuakeConfig {
  tenantId: string;
  environment: Environment;
  dynamoTableName?: string;
  s3BucketName?: string;
  vectorBucketName?: string;
  vectorIndexName?: string;
  vectorDimension?: number;
  deployNeptune?: boolean;
  neptuneMinCapacity?: number;
  neptuneMaxCapacity?: number;
  existingVpcId?: string;
  createNatGateway?: boolean;
  deployAthena?: boolean;
  neptunePublicAccess?: boolean;
  enableDeletionProtection?: boolean;
  additionalTags?: Record<string, string>;
}

export interface ResolvedConfig {
  tenantId: string;
  environment: Environment;
  dynamoTableName: string;
  s3BucketName: string;
  vectorBucketName: string;
  vectorIndexName: string;
  vectorDimension: number;
  deployNeptune: boolean;
  neptuneMinCapacity: number;
  neptuneMaxCapacity: number;
  existingVpcId?: string;
  createNatGateway: boolean;
  deployAthena: boolean;
  neptunePublicAccess: boolean;
  enableDeletionProtection: boolean;
  tags: Record<string, string>;
}

export function getDefaultConfig(
  tenantId: string,
  environment: Environment,
  account: string,
  _region: string,
): ResolvedConfig {
  const isProd = environment === 'prod';

  return {
    tenantId,
    environment,
    dynamoTableName: `edgequake-kv-${tenantId}-${environment}`,
    s3BucketName: `edgequake-graph-${tenantId}-${environment}-${account}`,
    vectorBucketName: `edgequake-vectors-${tenantId}-${environment}`,
    vectorIndexName: 'embeddings',
    vectorDimension: 1536,
    deployNeptune: true,
    neptuneMinCapacity: 1,
    neptuneMaxCapacity: isProd ? 32 : 16,
    createNatGateway: true,
    deployAthena: false,
    neptunePublicAccess: !isProd,
    enableDeletionProtection: isProd,
    tags: {
      project: 'graphrag',
      Tenant: tenantId,
      Environment: environment,
      ManagedBy: 'CDK',
    },
  };
}

export function resolveConfig(
  partial: EdgeQuakeConfig,
  account: string,
  region: string,
): ResolvedConfig {
  const defaults = getDefaultConfig(partial.tenantId, partial.environment, account, region);

  return {
    ...defaults,
    ...(partial.dynamoTableName && { dynamoTableName: partial.dynamoTableName }),
    ...(partial.s3BucketName && { s3BucketName: partial.s3BucketName }),
    ...(partial.vectorBucketName && { vectorBucketName: partial.vectorBucketName }),
    ...(partial.vectorIndexName && { vectorIndexName: partial.vectorIndexName }),
    ...(partial.vectorDimension !== undefined && { vectorDimension: partial.vectorDimension }),
    ...(partial.deployNeptune !== undefined && { deployNeptune: partial.deployNeptune }),
    ...(partial.neptuneMinCapacity !== undefined && { neptuneMinCapacity: partial.neptuneMinCapacity }),
    ...(partial.neptuneMaxCapacity !== undefined && { neptuneMaxCapacity: partial.neptuneMaxCapacity }),
    ...(partial.existingVpcId && { existingVpcId: partial.existingVpcId }),
    ...(partial.createNatGateway !== undefined && { createNatGateway: partial.createNatGateway }),
    ...(partial.deployAthena !== undefined && { deployAthena: partial.deployAthena }),
    ...(partial.neptunePublicAccess !== undefined && { neptunePublicAccess: partial.neptunePublicAccess }),
    ...(partial.enableDeletionProtection !== undefined && { enableDeletionProtection: partial.enableDeletionProtection }),
    tags: {
      ...defaults.tags,
      ...(partial.additionalTags || {}),
    },
  };
}
