import * as cdk from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as s3vectors from 'aws-cdk-lib/aws-s3vectors';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import * as glue from 'aws-cdk-lib/aws-glue';
import { Construct } from 'constructs';
import { ResolvedConfig } from './edgequake-config';

export interface EdgeQuakeCoreStackProps extends cdk.StackProps {
  config: ResolvedConfig;
}

export class EdgeQuakeCoreStack extends cdk.Stack {
  public readonly table: dynamodb.Table;
  public readonly bucket: s3.Bucket;
  public readonly vectorBucket: s3vectors.CfnVectorBucket;
  public readonly vectorIndex: s3vectors.CfnIndex;
  public readonly batchRole: iam.Role;

  constructor(scope: Construct, id: string, props: EdgeQuakeCoreStackProps) {
    super(scope, id, props);

    const { config } = props;
    const isProd = config.environment === 'prod';
    const ssmPrefix = `/edgequake/${config.tenantId}/${config.environment}`;

    // --- DynamoDB Table ---
    this.table = new dynamodb.Table(this, 'KvTable', {
      tableName: config.dynamoTableName,
      partitionKey: { name: 'namespace', type: dynamodb.AttributeType.STRING },
      sortKey: { name: 'id', type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      pointInTimeRecoverySpecification: { pointInTimeRecoveryEnabled: true },
      encryption: dynamodb.TableEncryption.AWS_MANAGED,
      removalPolicy: isProd ? cdk.RemovalPolicy.RETAIN : cdk.RemovalPolicy.DESTROY,
    });

    // --- S3 Bucket ---
    const bucketProps: s3.BucketProps = {
      bucketName: config.s3BucketName,
      encryption: s3.BucketEncryption.S3_MANAGED,
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      enforceSSL: true,
      removalPolicy: isProd ? cdk.RemovalPolicy.RETAIN : cdk.RemovalPolicy.DESTROY,
      autoDeleteObjects: !isProd,
    };

    if (isProd) {
      Object.assign(bucketProps, {
        versioned: true,
        lifecycleRules: [
          {
            noncurrentVersionExpiration: cdk.Duration.days(30),
          },
        ],
      });
    }

    this.bucket = new s3.Bucket(this, 'VectorBucket', bucketProps);

    // --- S3 Vectors Bucket ---
    this.vectorBucket = new s3vectors.CfnVectorBucket(this, 'S3VectorsBucket', {
      vectorBucketName: config.vectorBucketName,
      encryptionConfiguration: { sseType: 'AES256' },
    });
    if (!isProd) {
      this.vectorBucket.applyRemovalPolicy(cdk.RemovalPolicy.DESTROY);
    }

    // --- S3 Vectors Index ---
    this.vectorIndex = new s3vectors.CfnIndex(this, 'S3VectorsIndex', {
      vectorBucketName: config.vectorBucketName,
      indexName: config.vectorIndexName,
      dataType: 'float32',
      dimension: config.vectorDimension,
      distanceMetric: 'cosine',
    });
    this.vectorIndex.addDependency(this.vectorBucket);

    // --- IAM Role ---
    this.batchRole = new iam.Role(this, 'BatchRole', {
      roleName: `edgequake-batch-${config.tenantId}-${config.environment}`,
      assumedBy: new iam.CompositePrincipal(
        new iam.ServicePrincipal('ec2.amazonaws.com'),
        new iam.ServicePrincipal('ecs-tasks.amazonaws.com'),
        new iam.AccountPrincipal(this.account),
      ),
      description: `EdgeQuake batch/API role for tenant ${config.tenantId} (${config.environment})`,
    });

    // DynamoDB permissions
    this.table.grantReadWriteData(this.batchRole);

    // S3 permissions (legacy bucket)
    this.bucket.grantReadWrite(this.batchRole);

    // S3 Vectors permissions
    this.batchRole.addToPolicy(new iam.PolicyStatement({
      actions: [
        's3vectors:CreateVectorBucket', 's3vectors:GetVectorBucket',
        's3vectors:CreateIndex', 's3vectors:GetIndex', 's3vectors:DeleteIndex',
        's3vectors:PutVectors', 's3vectors:GetVectors',
        's3vectors:QueryVectors', 's3vectors:DeleteVectors',
        's3vectors:ListVectors', 's3vectors:ListIndexes',
      ],
      resources: [
        `arn:aws:s3vectors:${this.region}:${this.account}:vector-bucket/${config.vectorBucketName}`,
        `arn:aws:s3vectors:${this.region}:${this.account}:vector-bucket/${config.vectorBucketName}/*`,
      ],
    }));

    // SSM read permissions
    this.batchRole.addToPolicy(new iam.PolicyStatement({
      actions: ['ssm:GetParameter', 'ssm:GetParameters', 'ssm:GetParametersByPath'],
      resources: [
        `arn:aws:ssm:${this.region}:${this.account}:parameter${ssmPrefix}/*`,
      ],
    }));

    // CloudWatch Logs permissions
    this.batchRole.addToPolicy(new iam.PolicyStatement({
      actions: [
        'logs:CreateLogGroup',
        'logs:CreateLogStream',
        'logs:PutLogEvents',
      ],
      resources: [
        `arn:aws:logs:${this.region}:${this.account}:log-group:/edgequake/*`,
      ],
    }));

    // --- Optional Athena ---
    if (config.deployAthena) {
      const athenaBucket = new s3.Bucket(this, 'AthenaResultsBucket', {
        bucketName: `edgequake-athena-${config.tenantId}-${config.environment}-${this.account}`,
        encryption: s3.BucketEncryption.S3_MANAGED,
        blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
        enforceSSL: true,
        removalPolicy: cdk.RemovalPolicy.DESTROY,
        autoDeleteObjects: true,
        lifecycleRules: [
          { expiration: cdk.Duration.days(30) },
        ],
      });

      athenaBucket.grantReadWrite(this.batchRole);

      new glue.CfnDatabase(this, 'GlueDatabase', {
        catalogId: this.account,
        databaseInput: {
          name: `edgequake_${config.tenantId}_${config.environment}`,
          description: `EdgeQuake Glue database for tenant ${config.tenantId}`,
        },
      });

      this.batchRole.addToPolicy(new iam.PolicyStatement({
        actions: [
          'athena:StartQueryExecution',
          'athena:GetQueryExecution',
          'athena:GetQueryResults',
          'athena:StopQueryExecution',
          'glue:GetDatabase',
          'glue:GetTable',
          'glue:GetTables',
          'glue:GetPartitions',
        ],
        resources: ['*'],
      }));

      new ssm.StringParameter(this, 'AthenaResultsBucketParam', {
        parameterName: `${ssmPrefix}/athena-results-bucket`,
        stringValue: athenaBucket.bucketName,
        description: 'Athena query results S3 bucket',
      });
    }

    // --- SSM Parameters ---
    new ssm.StringParameter(this, 'DynamoTableParam', {
      parameterName: `${ssmPrefix}/dynamo-table`,
      stringValue: this.table.tableName,
      description: 'EdgeQuake DynamoDB table name',
    });

    new ssm.StringParameter(this, 'S3BucketParam', {
      parameterName: `${ssmPrefix}/s3-bucket`,
      stringValue: this.bucket.bucketName,
      description: 'EdgeQuake S3 vector storage bucket (legacy)',
    });

    new ssm.StringParameter(this, 'VectorBucketParam', {
      parameterName: `${ssmPrefix}/vector-bucket`,
      stringValue: config.vectorBucketName,
      description: 'EdgeQuake S3 Vectors bucket name',
    });

    new ssm.StringParameter(this, 'VectorIndexParam', {
      parameterName: `${ssmPrefix}/vector-index`,
      stringValue: config.vectorIndexName,
      description: 'EdgeQuake S3 Vectors index name',
    });

    new ssm.StringParameter(this, 'BatchRoleArnParam', {
      parameterName: `${ssmPrefix}/batch-role-arn`,
      stringValue: this.batchRole.roleArn,
      description: 'EdgeQuake batch/API IAM role ARN',
    });

    // --- CloudFormation Outputs ---
    new cdk.CfnOutput(this, 'DynamoTableName', {
      value: this.table.tableName,
      description: 'DynamoDB table name',
    });

    new cdk.CfnOutput(this, 'S3BucketName', {
      value: this.bucket.bucketName,
      description: 'S3 bucket name (legacy)',
    });

    new cdk.CfnOutput(this, 'VectorBucketName', {
      value: config.vectorBucketName,
      description: 'S3 Vectors bucket name',
    });

    new cdk.CfnOutput(this, 'VectorIndexName', {
      value: config.vectorIndexName,
      description: 'S3 Vectors index name',
    });

    new cdk.CfnOutput(this, 'BatchRoleArn', {
      value: this.batchRole.roleArn,
      description: 'Batch/API IAM role ARN',
    });
  }
}
