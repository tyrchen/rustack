import * as aws from "@pulumi/aws";
import * as pulumi from "@pulumi/pulumi";

const config = new pulumi.Config();

const endpoint =
  config.get("endpoint") ?? process.env.RUSTACK_ENDPOINT ?? "http://127.0.0.1:4566";
const region = config.get("region") ?? process.env.AWS_DEFAULT_REGION ?? "us-east-1";
const accessKey =
  config.get("accessKey") ?? process.env.AWS_ACCESS_KEY_ID ?? "AKIAIOSFODNN7EXAMPLE";
const secretKey =
  config.getSecret("secretKey") ??
  pulumi.secret(
    process.env.AWS_SECRET_ACCESS_KEY ?? "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
  );

const endpointOverrides: aws.types.input.ProviderEndpoint[] = [
  {
    apigatewayv2: endpoint,
    cloudfront: endpoint,
    cloudwatch: endpoint,
    cloudwatchlogs: endpoint,
    dynamodb: endpoint,
    eventbridge: endpoint,
    events: endpoint,
    iam: endpoint,
    kinesis: endpoint,
    kms: endpoint,
    lambda: endpoint,
    logs: endpoint,
    s3: endpoint,
    secretsmanager: endpoint,
    ses: endpoint,
    sesv2: endpoint,
    sns: endpoint,
    sqs: endpoint,
    ssm: endpoint,
    sts: endpoint,
  },
];

const rustack = new aws.Provider("rustack", {
  accessKey,
  secretKey,
  region,
  endpoints: endpointOverrides,
  maxRetries: 1,
  s3UsePathStyle: true,
  skipCredentialsValidation: true,
  skipMetadataApiCheck: true,
  skipRegionValidation: true,
  skipRequestingAccountId: true,
});

const caller = aws.getCallerIdentityOutput({}, { provider: rustack });

const name = pulumi.interpolate`rustack-pulumi-${pulumi
  .getStack()
  .toLowerCase()
  .replace(/[^a-z0-9-]/g, "-")}`;

const queue = new aws.sqs.Queue(
  "queue",
  {
    name,
  },
  { provider: rustack },
);

export const rustackEndpoint = endpoint;
export const callerAccount = caller.accountId;
export const queueUrl = queue.url;
