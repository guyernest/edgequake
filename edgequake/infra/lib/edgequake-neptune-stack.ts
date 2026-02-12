import * as cdk from 'aws-cdk-lib';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as neptune from 'aws-cdk-lib/aws-neptune';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import * as logs from 'aws-cdk-lib/aws-logs';
import { Construct } from 'constructs';
import { ResolvedConfig } from './edgequake-config';

export interface EdgeQuakeNeptuneStackProps extends cdk.StackProps {
  config: ResolvedConfig;
  batchRole: iam.IRole;
  s3BucketArn: string;
}

export class EdgeQuakeNeptuneStack extends cdk.Stack {
  public readonly vpc: ec2.IVpc;
  public readonly clusterEndpoint: string;

  constructor(scope: Construct, id: string, props: EdgeQuakeNeptuneStackProps) {
    super(scope, id, props);

    const { config, batchRole, s3BucketArn } = props;
    const isProd = config.environment === 'prod';
    const ssmPrefix = `/edgequake/${config.tenantId}/${config.environment}`;

    // --- VPC ---
    if (config.existingVpcId) {
      this.vpc = ec2.Vpc.fromLookup(this, 'ExistingVpc', {
        vpcId: config.existingVpcId,
      });
    } else {
      this.vpc = new ec2.Vpc(this, 'Vpc', {
        vpcName: `edgequake-${config.tenantId}-${config.environment}`,
        maxAzs: 2,
        natGateways: config.createNatGateway ? 1 : 0,
        subnetConfiguration: [
          {
            cidrMask: 24,
            name: 'Public',
            subnetType: ec2.SubnetType.PUBLIC,
          },
          {
            cidrMask: 24,
            name: 'Private',
            subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS,
          },
        ],
      });

      // VPC Gateway Endpoints for cost optimization
      this.vpc.addGatewayEndpoint('DynamoDbEndpoint', {
        service: ec2.GatewayVpcEndpointAwsService.DYNAMODB,
      });

      this.vpc.addGatewayEndpoint('S3Endpoint', {
        service: ec2.GatewayVpcEndpointAwsService.S3,
      });
    }

    // --- Security Group ---
    const neptuneSg = new ec2.SecurityGroup(this, 'NeptuneSg', {
      vpc: this.vpc,
      securityGroupName: `edgequake-neptune-${config.tenantId}-${config.environment}`,
      description: 'Security group for EdgeQuake Neptune cluster',
      allowAllOutbound: false,
    });

    neptuneSg.addIngressRule(
      ec2.Peer.ipv4(this.vpc.vpcCidrBlock),
      ec2.Port.tcp(8182),
      'Gremlin access from VPC',
    );

    if (config.neptunePublicAccess) {
      neptuneSg.addIngressRule(
        ec2.Peer.anyIpv4(),
        ec2.Port.tcp(8182),
        'Public Gremlin access (IAM auth required)',
      );
    }

    // Neptune bulk loader needs outbound HTTPS to reach S3 via VPC gateway endpoint
    neptuneSg.addEgressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(443),
      'Allow HTTPS to S3 for bulk load',
    );

    // --- Neptune Subnet Group ---
    const subnetGroup = new neptune.CfnDBSubnetGroup(this, 'NeptuneSubnetGroup', {
      dbSubnetGroupDescription: `EdgeQuake Neptune subnet group (${config.tenantId}-${config.environment})`,
      dbSubnetGroupName: `edgequake-neptune-${config.tenantId}-${config.environment}`,
      subnetIds: this.vpc.selectSubnets({
        subnetType: config.neptunePublicAccess
          ? ec2.SubnetType.PUBLIC
          : ec2.SubnetType.PRIVATE_WITH_EGRESS,
      }).subnetIds,
    });

    // --- Neptune Cluster Parameter Group ---
    const clusterParameterGroup = new neptune.CfnDBClusterParameterGroup(this, 'NeptuneClusterParams', {
      family: 'neptune1.4',
      description: `EdgeQuake Neptune cluster parameters (${config.tenantId}-${config.environment})`,
      name: `edgequake-neptune-cluster-${config.tenantId}-${config.environment}`,
      parameters: {
        neptune_enable_audit_log: '1',
      },
    });

    // --- CloudWatch Log Group for Neptune Audit Logs ---
    const neptuneLogGroup = new logs.LogGroup(this, 'NeptuneAuditLogs', {
      logGroupName: `/aws/neptune/edgequake-${config.tenantId}-${config.environment}/audit`,
      retention: isProd ? logs.RetentionDays.THREE_MONTHS : logs.RetentionDays.ONE_WEEK,
      removalPolicy: isProd ? cdk.RemovalPolicy.RETAIN : cdk.RemovalPolicy.DESTROY,
    });

    // --- Neptune S3 Bulk Load Role ---
    const neptuneS3Role = new iam.Role(this, 'NeptuneS3LoadRole', {
      roleName: `edgequake-neptune-s3-${config.tenantId}-${config.environment}`,
      assumedBy: new iam.ServicePrincipal('rds.amazonaws.com'),
      description: `Role for Neptune to read S3 bulk load data (${config.tenantId}-${config.environment})`,
    });

    neptuneS3Role.addToPolicy(new iam.PolicyStatement({
      actions: ['s3:GetObject', 's3:ListBucket'],
      resources: [s3BucketArn, `${s3BucketArn}/*`],
    }));

    // --- Neptune Serverless Cluster ---
    const cluster = new neptune.CfnDBCluster(this, 'NeptuneCluster', {
      dbClusterIdentifier: `edgequake-${config.tenantId}-${config.environment}`,
      engineVersion: '1.4.6.0',
      dbSubnetGroupName: subnetGroup.dbSubnetGroupName,
      vpcSecurityGroupIds: [neptuneSg.securityGroupId],
      dbClusterParameterGroupName: clusterParameterGroup.name,
      iamAuthEnabled: true,
      storageEncrypted: true,
      backupRetentionPeriod: isProd ? 7 : 1,
      deletionProtection: config.enableDeletionProtection,
      serverlessScalingConfiguration: {
        minCapacity: config.neptuneMinCapacity,
        maxCapacity: config.neptuneMaxCapacity,
      },
      associatedRoles: [{ roleArn: neptuneS3Role.roleArn }],
    });

    cluster.addDependency(subnetGroup);
    cluster.addDependency(clusterParameterGroup);

    // Neptune needs at least one instance for serverless
    const instance = new neptune.CfnDBInstance(this, 'NeptuneInstance', {
      dbInstanceClass: 'db.serverless',
      dbClusterIdentifier: cluster.dbClusterIdentifier!,
      dbInstanceIdentifier: `edgequake-${config.tenantId}-${config.environment}-instance-1`,
      ...(config.neptunePublicAccess && { publiclyAccessible: true }),
    });

    instance.addDependency(cluster);

    this.clusterEndpoint = cluster.attrEndpoint;

    // --- IAM: Grant Neptune permissions to batch role ---
    const neptuneArn = `arn:aws:neptune-db:${this.region}:${this.account}:${cluster.attrClusterResourceId}/*`;

    const neptunePolicy = new iam.Policy(this, 'NeptuneAccessPolicy', {
      policyName: `edgequake-neptune-${config.tenantId}-${config.environment}`,
      statements: [
        new iam.PolicyStatement({
          actions: [
            'neptune-db:connect',
            'neptune-db:ReadDataViaQuery',
            'neptune-db:WriteDataViaQuery',
            'neptune-db:DeleteDataViaQuery',
          ],
          resources: [neptuneArn],
        }),
      ],
    });

    batchRole.attachInlinePolicy(neptunePolicy);

    // --- SSM Parameters ---
    new ssm.StringParameter(this, 'NeptuneEndpointParam', {
      parameterName: `${ssmPrefix}/neptune-endpoint`,
      stringValue: cluster.attrEndpoint,
      description: 'Neptune cluster endpoint',
    });

    new ssm.StringParameter(this, 'VpcIdParam', {
      parameterName: `${ssmPrefix}/vpc-id`,
      stringValue: this.vpc.vpcId,
      description: 'VPC ID for EdgeQuake resources',
    });

    new ssm.StringParameter(this, 'NeptuneSgIdParam', {
      parameterName: `${ssmPrefix}/neptune-security-group-id`,
      stringValue: neptuneSg.securityGroupId,
      description: 'Neptune security group ID',
    });

    new ssm.StringParameter(this, 'NeptuneS3RoleArnParam', {
      parameterName: `${ssmPrefix}/neptune-s3-role-arn`,
      stringValue: neptuneS3Role.roleArn,
      description: 'IAM role ARN for Neptune S3 bulk load',
    });

    // --- CloudFormation Outputs ---
    new cdk.CfnOutput(this, 'NeptuneClusterEndpoint', {
      value: cluster.attrEndpoint,
      description: 'Neptune cluster endpoint hostname',
    });

    new cdk.CfnOutput(this, 'NeptuneFullEndpoint', {
      value: `${cluster.attrEndpoint}:${cluster.attrPort}`,
      description: 'Neptune full endpoint (host:port)',
    });

    new cdk.CfnOutput(this, 'VpcId', {
      value: this.vpc.vpcId,
      description: 'VPC ID',
    });

    new cdk.CfnOutput(this, 'NeptuneSecurityGroupId', {
      value: neptuneSg.securityGroupId,
      description: 'Neptune security group ID',
    });

    new cdk.CfnOutput(this, 'NeptuneS3LoadRoleArn', {
      value: neptuneS3Role.roleArn,
      description: 'IAM role ARN for Neptune S3 bulk load',
    });

    // Suppress the unused variable warning - the log group is used by Neptune audit logs
    neptuneLogGroup.node.defaultChild;
  }
}
