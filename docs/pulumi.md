# Pulumi

Rustack can be used as a Pulumi deployment target through the standard Pulumi
AWS provider. The provider stays `@pulumi/aws`; Rustack supplies the local AWS
API endpoints.

## Quick Start

```bash
make pulumi-smoke
```

The target runs `examples/pulumi/rustack-target`, which validates STS and
creates representative resources across every currently provisionable Rustack
service.

For a larger real-world serverless topology, run:

```bash
RUSTACK_ENDPOINT=http://127.0.0.1:4567 make pulumi-hackathon-smoke
```

That target runs `examples/pulumi/hackathon-app`, which provisions a hackathon
application stack with CloudFront, S3 frontend/protected/upload buckets, API
Gateway V2, Lambda API and worker functions, DynamoDB, SQS, SSM SecureString,
IAM inline policy, CloudFront Origin Access Control, CloudFront Function token
guard, and S3 bucket policies.

## Provider Contract

Use one explicit AWS provider and pass it to every resource:

```ts
const rustack = new aws.Provider("rustack", {
  accessKey: "AKIAIOSFODNN7EXAMPLE",
  secretKey: pulumi.secret("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"),
  region: "us-east-1",
  endpoints: [{
    apigatewayv2: "http://127.0.0.1:4566",
    cloudfront: "http://127.0.0.1:4566",
    cloudwatch: "http://127.0.0.1:4566",
    cloudwatchlogs: "http://127.0.0.1:4566",
    dynamodb: "http://127.0.0.1:4566",
    eventbridge: "http://127.0.0.1:4566",
    events: "http://127.0.0.1:4566",
    iam: "http://127.0.0.1:4566",
    kinesis: "http://127.0.0.1:4566",
    kms: "http://127.0.0.1:4566",
    lambda: "http://127.0.0.1:4566",
    logs: "http://127.0.0.1:4566",
    s3: "http://127.0.0.1:4566",
    secretsmanager: "http://127.0.0.1:4566",
    ses: "http://127.0.0.1:4566",
    sesv2: "http://127.0.0.1:4566",
    sqs: "http://127.0.0.1:4566",
    sns: "http://127.0.0.1:4566",
    ssm: "http://127.0.0.1:4566",
    sts: "http://127.0.0.1:4566"
  }],
  s3UsePathStyle: true,
  skipCredentialsValidation: true,
  skipMetadataApiCheck: true,
  skipRegionValidation: true,
  skipRequestingAccountId: true,
});
```

The Pulumi AWS provider performs some credential checks during provider
initialization, before user resources are created. The smoke example skips those
bootstrap checks and then calls `aws.getCallerIdentityOutput` through the
Rustack provider to verify STS endpoint routing explicitly.

## Current Scope

The checked smoke path covers Pulumi provider initialization, explicit STS
`GetCallerIdentity`, create/read/delete for the representative resources below,
and a full cleanup:

- API Gateway V2: `aws.apigatewayv2.Api`, `aws.apigatewayv2.Stage`
- CloudFront: `aws.cloudfront.Function`
- CloudWatch Metrics: `aws.cloudwatch.MetricAlarm`
- CloudWatch Logs: `aws.cloudwatch.LogGroup`
- DynamoDB and DynamoDB Streams: `aws.dynamodb.Table` with streams enabled
- EventBridge: `aws.cloudwatch.EventBus`, `aws.cloudwatch.EventRule`
- IAM: `aws.iam.Role`
- Kinesis: `aws.kinesis.Stream`
- KMS: `aws.kms.Key`
- Lambda: `aws.lambda.Function`
- S3: `aws.s3.Bucket`, `aws.s3.BucketObject`
- Secrets Manager: `aws.secretsmanager.Secret`, `aws.secretsmanager.SecretVersion`
- SES: `aws.ses.EmailIdentity`, `aws.ses.Template`
- SNS: `aws.sns.Topic`
- SQS: `aws.sqs.Queue`
- SSM: `aws.ssm.Parameter`

The Pulumi AWS provider is based on the Terraform AWS provider, so a resource
can call APIs beyond the obvious create/read/delete operations. `make
pulumi-smoke` is the compatibility check for this provider-specific behavior.
During cleanup, SQS queue deletion can sit in the provider waiter for around two
minutes; this is expected and not a Rustack hang.

`make pulumi-hackathon-smoke` uses the same runner and deploys the richer
serverless topology. It also validates these Pulumi resources:

- API Gateway V2: `aws.apigatewayv2.Integration`, `aws.apigatewayv2.Route`
- CloudFront: `aws.cloudfront.Distribution`,
  `aws.cloudfront.OriginAccessControl`
- IAM: `aws.iam.RolePolicy`
- Lambda: `aws.lambda.EventSourceMapping`, `aws.lambda.Permission`
- S3: `aws.s3.BucketPolicy`
