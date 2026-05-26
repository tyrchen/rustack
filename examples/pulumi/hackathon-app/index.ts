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
    lambda: endpoint,
    logs: endpoint,
    s3: endpoint,
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
  .slice(0, 24);
const name = `hackathon-${stackSuffix}`;

const siteBucket = new aws.s3.Bucket(
  "site-bucket",
  {
    bucket: `${name}-site`,
    forceDestroy: true,
  },
  { provider: rustack },
);

const protectedBucket = new aws.s3.Bucket(
  "protected-bucket",
  {
    bucket: `${name}-protected`,
    forceDestroy: true,
  },
  { provider: rustack },
);

const uploadBucket = new aws.s3.Bucket(
  "upload-bucket",
  {
    bucket: `${name}-uploads`,
    forceDestroy: true,
  },
  { provider: rustack },
);

const siteIndex = new aws.s3.BucketObject(
  "site-index",
  {
    bucket: siteBucket.bucket,
    key: "index.html",
    contentType: "text/html",
    content: [
      "<!doctype html>",
      "<html>",
      "<head><meta charset=\"utf-8\"><title>Hackathon</title></head>",
      "<body><main id=\"app\">Hackathon app deployed by Rustack</main>",
      "<script src=\"/assets/app.js\"></script></body>",
      "</html>",
    ].join(""),
  },
  { provider: rustack },
);

const siteScript = new aws.s3.BucketObject(
  "site-script",
  {
    bucket: siteBucket.bucket,
    key: "assets/app.js",
    contentType: "application/javascript",
    content: "document.getElementById('app').dataset.ready = 'true';\n",
  },
  { provider: rustack },
);

const protectedSeed = new aws.s3.BucketObject(
  "protected-seed",
  {
    bucket: protectedBucket.bucket,
    key: "protected/private-seed.json",
    contentType: "application/json",
    content: JSON.stringify({ judges: ["alice", "bob"], rubric: "private" }),
  },
  { provider: rustack },
);

const projectsTable = new aws.dynamodb.Table(
  "projects-table",
  {
    name: `${name}-projects`,
    billingMode: "PAY_PER_REQUEST",
    hashKey: "pk",
    rangeKey: "sk",
    streamEnabled: true,
    streamViewType: "NEW_AND_OLD_IMAGES",
    attributes: [
      { name: "pk", type: "S" },
      { name: "sk", type: "S" },
    ],
  },
  { provider: rustack },
);

const imageQueue = new aws.sqs.Queue(
  "image-queue",
  {
    name: `${name}-images`,
    visibilityTimeoutSeconds: 60,
  },
  { provider: rustack },
);

const tokenParameter = new aws.ssm.Parameter(
  "token-parameter",
  {
    name: `/hackathon/${stackSuffix}/protected-token`,
    type: "SecureString",
    value: JSON.stringify({ token: "local-dev-token", audience: "judges" }),
  },
  { provider: rustack },
);

const lambdaRole = new aws.iam.Role(
  "lambda-role",
  {
    name: `${name}-lambda-role`,
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

const lambdaPolicy = new aws.iam.RolePolicy(
  "lambda-policy",
  {
    name: `${name}-lambda-policy`,
    role: lambdaRole.name,
    policy: pulumi
      .all([
        siteBucket.arn,
        protectedBucket.arn,
        uploadBucket.arn,
        imageQueue.arn,
        projectsTable.arn,
        tokenParameter.arn,
      ])
      .apply(([siteArn, protectedArn, uploadArn, queueArn, tableArn, parameterArn]) =>
        JSON.stringify({
          Version: "2012-10-17",
          Statement: [
            {
              Effect: "Allow",
              Action: ["s3:GetObject", "s3:PutObject"],
              Resource: [`${siteArn}/*`, `${protectedArn}/*`, `${uploadArn}/*`],
            },
            {
              Effect: "Allow",
              Action: ["sqs:SendMessage", "sqs:ReceiveMessage", "sqs:DeleteMessage"],
              Resource: queueArn,
            },
            {
              Effect: "Allow",
              Action: ["dynamodb:GetItem", "dynamodb:PutItem", "dynamodb:UpdateItem", "dynamodb:Query"],
              Resource: tableArn,
            },
            {
              Effect: "Allow",
              Action: ["ssm:GetParameter"],
              Resource: parameterArn,
            },
          ],
        }),
      ),
  },
  { provider: rustack },
);

const apiHandlerSource = [
  "const AWS = require('aws-sdk');",
  "const dynamodb = new AWS.DynamoDB.DocumentClient();",
  "const s3 = new AWS.S3();",
  "const sqs = new AWS.SQS();",
  "const tableName = process.env.TABLE_NAME;",
  "const uploadBucket = process.env.UPLOAD_BUCKET;",
  "const queueUrl = process.env.IMAGE_QUEUE_URL;",
  "function respond(statusCode, body) {",
  "  return { statusCode, headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) };",
  "}",
  "exports.handler = async (event) => {",
  "  const path = event.rawPath || event.path || '/';",
  "  const method = event.requestContext && event.requestContext.http ? event.requestContext.http.method : event.httpMethod;",
  "  if (path.startsWith('/api/uploads') && method === 'POST') {",
  "    const body = JSON.parse(event.body || '{}');",
  "    const id = body.id || String(Date.now());",
  "    const key = 'incoming/' + id + '.json';",
  "    await s3.putObject({ Bucket: uploadBucket, Key: key, Body: JSON.stringify(body), ContentType: 'application/json' }).promise();",
  "    await sqs.sendMessage({ QueueUrl: queueUrl, MessageBody: JSON.stringify({ id, key }) }).promise();",
  "    return respond(202, { id, key, status: 'queued' });",
  "  }",
  "  if (path.startsWith('/api/projects') && method === 'POST') {",
  "    const body = JSON.parse(event.body || '{}');",
  "    const id = body.id || String(Date.now());",
  "    await dynamodb.put({ TableName: tableName, Item: { pk: 'PROJECT', sk: id, title: body.title || 'Untitled' } }).promise();",
  "    return respond(201, { id });",
  "  }",
  "  return respond(200, { ok: true, service: 'hackathon-api' });",
  "};",
  "",
].join("\n");

const workerHandlerSource = [
  "const AWS = require('aws-sdk');",
  "const dynamodb = new AWS.DynamoDB.DocumentClient();",
  "const s3 = new AWS.S3();",
  "const tableName = process.env.TABLE_NAME;",
  "const uploadBucket = process.env.UPLOAD_BUCKET;",
  "exports.handler = async (event) => {",
  "  for (const record of event.Records || []) {",
  "    const message = JSON.parse(record.body || '{}');",
  "    const processedKey = 'processed/' + (message.id || 'image') + '.json';",
  "    await s3.putObject({ Bucket: uploadBucket, Key: processedKey, Body: JSON.stringify({ source: message.key, status: 'processed' }), ContentType: 'application/json' }).promise();",
  "    await dynamodb.put({ TableName: tableName, Item: { pk: 'IMAGE', sk: message.id || processedKey, status: 'processed', key: processedKey } }).promise();",
  "  }",
  "  return { batchItemFailures: [] };",
  "};",
  "",
].join("\n");

const apiFunction = new aws.lambda.Function(
  "api-function",
  {
    name: `${name}-api`,
    role: lambdaRole.arn,
    runtime: "nodejs18.x",
    handler: "index.handler",
    memorySize: 256,
    timeout: 15,
    code: new pulumi.asset.AssetArchive({
      "index.js": new pulumi.asset.StringAsset(apiHandlerSource),
    }),
    environment: {
      variables: {
        TABLE_NAME: projectsTable.name,
        SITE_BUCKET: siteBucket.bucket,
        PROTECTED_BUCKET: protectedBucket.bucket,
        UPLOAD_BUCKET: uploadBucket.bucket,
        IMAGE_QUEUE_URL: imageQueue.url,
        TOKEN_PARAMETER: tokenParameter.name,
      },
    },
  },
  { provider: rustack, dependsOn: [lambdaPolicy] },
);

const workerFunction = new aws.lambda.Function(
  "worker-function",
  {
    name: `${name}-worker`,
    role: lambdaRole.arn,
    runtime: "nodejs18.x",
    handler: "index.handler",
    memorySize: 512,
    timeout: 30,
    code: new pulumi.asset.AssetArchive({
      "index.js": new pulumi.asset.StringAsset(workerHandlerSource),
    }),
    environment: {
      variables: {
        TABLE_NAME: projectsTable.name,
        UPLOAD_BUCKET: uploadBucket.bucket,
      },
    },
  },
  { provider: rustack, dependsOn: [lambdaPolicy] },
);

const imageWorkerMapping = new aws.lambda.EventSourceMapping(
  "image-worker-mapping",
  {
    eventSourceArn: imageQueue.arn,
    functionName: workerFunction.name,
    batchSize: 5,
    enabled: true,
  },
  { provider: rustack },
);

const api = new aws.apigatewayv2.Api(
  "http-api",
  {
    name: `${name}-api`,
    protocolType: "HTTP",
  },
  { provider: rustack },
);

const apiIntegration = new aws.apigatewayv2.Integration(
  "api-integration",
  {
    apiId: api.id,
    integrationType: "AWS_PROXY",
    integrationMethod: "POST",
    integrationUri: apiFunction.invokeArn,
    payloadFormatVersion: "2.0",
  },
  { provider: rustack },
);

const apiRoute = new aws.apigatewayv2.Route(
  "api-route",
  {
    apiId: api.id,
    routeKey: "ANY /api/{proxy+}",
    target: pulumi.interpolate`integrations/${apiIntegration.id}`,
  },
  { provider: rustack },
);

const apiStage = new aws.apigatewayv2.Stage(
  "api-stage",
  {
    apiId: api.id,
    name: "prod",
    autoDeploy: true,
  },
  { provider: rustack, dependsOn: [apiRoute] },
);

const apiPermission = new aws.lambda.Permission(
  "api-permission",
  {
    statementId: `${name}-api-gateway`,
    action: "lambda:InvokeFunction",
    function: apiFunction.name,
    principal: "apigateway.amazonaws.com",
    sourceArn: pulumi.interpolate`${api.executionArn}/*/*`,
  },
  { provider: rustack },
);

const originAccessControl = new aws.cloudfront.OriginAccessControl(
  "origin-access-control",
  {
    name: `${name}-s3-oac`,
    description: "Rustack hackathon app S3 origin access control",
    originAccessControlOriginType: "s3",
    signingBehavior: "always",
    signingProtocol: "sigv4",
  },
  { provider: rustack },
);

const tokenFunction = new aws.cloudfront.Function(
  "token-function",
  {
    name: `${name}-token`,
    runtime: "cloudfront-js-1.0",
    publish: true,
    code: [
      "function handler(event) {",
      "  var request = event.request;",
      "  var headers = request.headers;",
      "  if (!headers['x-hackathon-token'] || headers['x-hackathon-token'].value !== 'local-dev-token') {",
      "    return { statusCode: 403, statusDescription: 'Forbidden' };",
      "  }",
      "  return request;",
      "}",
      "",
    ].join("\n"),
  },
  { provider: rustack },
);

const apiDomainName = api.apiEndpoint.apply((value) =>
  value.replace(/^https?:\/\//, "").replace(/\/$/, ""),
);

const distribution = new aws.cloudfront.Distribution(
  "app-distribution",
  {
    enabled: true,
    waitForDeployment: false,
    defaultRootObject: "index.html",
    comment: "Rustack hackathon app frontend, API, protected data, and image pipeline",
    origins: [
      {
        originId: "site-s3",
        domainName: pulumi.interpolate`${siteBucket.bucket}.s3.amazonaws.com`,
        originAccessControlId: originAccessControl.id,
      },
      {
        originId: "protected-s3",
        domainName: pulumi.interpolate`${protectedBucket.bucket}.s3.amazonaws.com`,
        originAccessControlId: originAccessControl.id,
      },
      {
        originId: "api-gateway",
        domainName: apiDomainName,
        customOriginConfig: {
          httpPort: 80,
          httpsPort: 443,
          originProtocolPolicy: "https-only",
          originSslProtocols: ["TLSv1.2"],
        },
      },
    ],
    defaultCacheBehavior: {
      targetOriginId: "site-s3",
      viewerProtocolPolicy: "allow-all",
      allowedMethods: ["GET", "HEAD", "OPTIONS"],
      cachedMethods: ["GET", "HEAD"],
      forwardedValues: {
        queryString: false,
        cookies: { forward: "none" },
      },
    },
    orderedCacheBehaviors: [
      {
        pathPattern: "/api/*",
        targetOriginId: "api-gateway",
        viewerProtocolPolicy: "allow-all",
        allowedMethods: ["GET", "HEAD", "OPTIONS", "PUT", "POST", "PATCH", "DELETE"],
        cachedMethods: ["GET", "HEAD", "OPTIONS"],
        forwardedValues: {
          queryString: true,
          headers: ["*"],
          cookies: { forward: "all" },
        },
      },
      {
        pathPattern: "/protected/*",
        targetOriginId: "protected-s3",
        viewerProtocolPolicy: "allow-all",
        allowedMethods: ["GET", "HEAD", "OPTIONS"],
        cachedMethods: ["GET", "HEAD"],
        forwardedValues: {
          queryString: false,
          headers: ["x-hackathon-token"],
          cookies: { forward: "none" },
        },
        functionAssociations: [
          {
            eventType: "viewer-request",
            functionArn: tokenFunction.arn,
          },
        ],
      },
    ],
    restrictions: {
      geoRestriction: {
        restrictionType: "none",
      },
    },
    viewerCertificate: {
      cloudfrontDefaultCertificate: true,
    },
  },
  { provider: rustack, dependsOn: [siteIndex, siteScript, protectedSeed, apiPermission] },
);

function cloudfrontReadPolicy(
  bucketArn: pulumi.Output<string>,
  distributionArn: pulumi.Output<string>,
): pulumi.Output<string> {
  return pulumi.all([bucketArn, distributionArn]).apply(([arn, distributionArnValue]) =>
    JSON.stringify({
      Version: "2012-10-17",
      Statement: [
        {
          Effect: "Allow",
          Principal: { Service: "cloudfront.amazonaws.com" },
          Action: "s3:GetObject",
          Resource: `${arn}/*`,
          Condition: {
            StringEquals: {
              "AWS:SourceArn": distributionArnValue,
            },
          },
        },
      ],
    }),
  );
}

const siteBucketPolicy = new aws.s3.BucketPolicy(
  "site-bucket-policy",
  {
    bucket: siteBucket.bucket,
    policy: cloudfrontReadPolicy(siteBucket.arn, distribution.arn),
  },
  { provider: rustack },
);

const protectedBucketPolicy = new aws.s3.BucketPolicy(
  "protected-bucket-policy",
  {
    bucket: protectedBucket.bucket,
    policy: cloudfrontReadPolicy(protectedBucket.arn, distribution.arn),
  },
  { provider: rustack },
);

export const rustackEndpoint = endpoint;
export const callerAccount = caller.accountId;
export const frontendBucketName = siteBucket.bucket;
export const frontendIndexKey = siteIndex.key;
export const protectedBucketName = protectedBucket.bucket;
export const protectedSeedKey = protectedSeed.key;
export const uploadBucketName = uploadBucket.bucket;
export const projectsTableName = projectsTable.name;
export const projectsTableStreamArn = projectsTable.streamArn;
export const imageQueueUrl = imageQueue.url;
export const tokenParameterName = tokenParameter.name;
export const apiFunctionArn = apiFunction.arn;
export const workerFunctionArn = workerFunction.arn;
export const imageWorkerMappingUuid = imageWorkerMapping.uuid;
export const apiEndpoint = api.apiEndpoint;
export const apiStageInvokeUrl = apiStage.invokeUrl;
export const cloudfrontDistributionId = distribution.id;
export const cloudfrontDomainName = distribution.domainName;
export const tokenFunctionArn = tokenFunction.arn;
export const siteBucketPolicyId = siteBucketPolicy.id;
export const protectedBucketPolicyId = protectedBucketPolicy.id;
