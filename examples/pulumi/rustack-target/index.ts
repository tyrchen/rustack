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

const stackSuffix = pulumi
  .getStack()
  .toLowerCase()
  .replace(/[^a-z0-9-]/g, "-")
  .slice(0, 32);
const name = `rustack-pulumi-${stackSuffix}`;

const queue = new aws.sqs.Queue(
  "queue",
  {
    name,
  },
  { provider: rustack },
);

const topic = new aws.sns.Topic(
  "topic",
  {
    name: `${name}-topic`,
  },
  { provider: rustack },
);

const bucket = new aws.s3.Bucket(
  "bucket",
  {
    bucket: `${name}-bucket`,
    forceDestroy: true,
  },
  { provider: rustack },
);

const object = new aws.s3.BucketObject(
  "object",
  {
    bucket: bucket.bucket,
    key: "hello.txt",
    content: "hello from pulumi",
  },
  { provider: rustack },
);

const table = new aws.dynamodb.Table(
  "table",
  {
    name: `${name}-table`,
    billingMode: "PAY_PER_REQUEST",
    hashKey: "id",
    streamEnabled: true,
    streamViewType: "NEW_AND_OLD_IMAGES",
    attributes: [
      {
        name: "id",
        type: "S",
      },
    ],
  },
  { provider: rustack },
);

const parameter = new aws.ssm.Parameter(
  "parameter",
  {
    name: `/rustack/pulumi/${stackSuffix}`,
    type: "String",
    value: "ready",
  },
  { provider: rustack },
);

const logGroup = new aws.cloudwatch.LogGroup(
  "log-group",
  {
    name: `/rustack/pulumi/${stackSuffix}`,
    retentionInDays: 1,
  },
  { provider: rustack },
);

const stream = new aws.kinesis.Stream(
  "stream",
  {
    name: `${name}-stream`,
    shardCount: 1,
    streamModeDetails: {
      streamMode: "PROVISIONED",
    },
  },
  { provider: rustack },
);

const key = new aws.kms.Key(
  "key",
  {
    description: `${name} key`,
    deletionWindowInDays: 7,
  },
  { provider: rustack },
);

const secret = new aws.secretsmanager.Secret(
  "secret",
  {
    name: `${name}-secret`,
    recoveryWindowInDays: 0,
  },
  { provider: rustack },
);

const secretVersion = new aws.secretsmanager.SecretVersion(
  "secret-version",
  {
    secretId: secret.id,
    secretString: JSON.stringify({ status: "ready" }),
  },
  { provider: rustack },
);

const role = new aws.iam.Role(
  "role",
  {
    name: `${name}-role`,
    assumeRolePolicy: JSON.stringify({
      Version: "2012-10-17",
      Statement: [
        {
          Effect: "Allow",
          Principal: { Service: "lambda.amazonaws.com" },
          Action: "sts:AssumeRole",
        },
      ],
    }),
  },
  { provider: rustack },
);

const eventBus = new aws.cloudwatch.EventBus(
  "event-bus",
  {
    name: `${name}-bus`,
  },
  { provider: rustack },
);

const eventRule = new aws.cloudwatch.EventRule(
  "event-rule",
  {
    name: `${name}-rule`,
    eventBusName: eventBus.name,
    eventPattern: JSON.stringify({ source: ["rustack.pulumi"] }),
  },
  { provider: rustack },
);

const alarm = new aws.cloudwatch.MetricAlarm(
  "alarm",
  {
    name: `${name}-alarm`,
    comparisonOperator: "GreaterThanThreshold",
    evaluationPeriods: 1,
    metricName: "Provisioned",
    namespace: "Rustack/Pulumi",
    period: 60,
    statistic: "Sum",
    threshold: 1,
    actionsEnabled: false,
  },
  { provider: rustack },
);

const emailIdentity = new aws.ses.EmailIdentity(
  "email-identity",
  {
    email: `pulumi-${stackSuffix}@example.com`,
  },
  { provider: rustack },
);

const emailTemplate = new aws.ses.Template(
  "email-template",
  {
    name: `${name}-template`.slice(0, 64),
    subject: "Rustack Pulumi",
    text: "ready",
    html: "<strong>ready</strong>",
  },
  { provider: rustack },
);

const lambdaFunction = new aws.lambda.Function(
  "lambda-function",
  {
    name: `${name}-lambda`,
    role: role.arn,
    runtime: "nodejs18.x",
    handler: "index.handler",
    memorySize: 128,
    timeout: 3,
    code: new pulumi.asset.AssetArchive({
      "index.js": new pulumi.asset.StringAsset(
        'exports.handler = async () => ({ statusCode: 200, body: "ready" });\n',
      ),
    }),
  },
  { provider: rustack },
);

const api = new aws.apigatewayv2.Api(
  "api",
  {
    name: `${name}-api`,
    protocolType: "HTTP",
  },
  { provider: rustack },
);

const apiStage = new aws.apigatewayv2.Stage(
  "api-stage",
  {
    apiId: api.id,
    name: "dev",
    autoDeploy: true,
  },
  { provider: rustack },
);

const cloudfrontFunction = new aws.cloudfront.Function(
  "cloudfront-function",
  {
    name: `${name}-cf-fn`,
    runtime: "cloudfront-js-1.0",
    code: "function handler(event) { return event.request; }\n",
    publish: true,
  },
  { provider: rustack },
);

export const rustackEndpoint = endpoint;
export const callerAccount = caller.accountId;
export const queueUrl = queue.url;
export const topicArn = topic.arn;
export const bucketName = bucket.bucket;
export const objectKey = object.key;
export const tableName = table.name;
export const tableStreamArn = table.streamArn;
export const parameterName = parameter.name;
export const logGroupName = logGroup.name;
export const streamName = stream.name;
export const keyArn = key.arn;
export const secretArn = secret.arn;
export const secretVersionId = secretVersion.versionId;
export const roleArn = role.arn;
export const eventBusArn = eventBus.arn;
export const eventRuleArn = eventRule.arn;
export const alarmName = alarm.name;
export const emailIdentityName = emailIdentity.email;
export const emailTemplateName = emailTemplate.name;
export const lambdaFunctionArn = lambdaFunction.arn;
export const apiEndpoint = api.apiEndpoint;
export const apiStageInvokeUrl = apiStage.invokeUrl;
export const cloudfrontFunctionArn = cloudfrontFunction.arn;
