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
  .slice(0, 22);
const name = `commerce-${stackSuffix}`;

const publicBucket = new aws.s3.Bucket(
  "public-bucket",
  { bucket: `${name}-public`, forceDestroy: true },
  { provider: rustack },
);

const mediaBucket = new aws.s3.Bucket(
  "media-bucket",
  { bucket: `${name}-media`, forceDestroy: true },
  { provider: rustack },
);

const privateBucket = new aws.s3.Bucket(
  "private-bucket",
  { bucket: `${name}-private`, forceDestroy: true },
  { provider: rustack },
);

const eventBucket = new aws.s3.Bucket(
  "event-bucket",
  { bucket: `${name}-events`, forceDestroy: true },
  { provider: rustack },
);

const publicVersioning = new aws.s3.BucketVersioning(
  "public-versioning",
  {
    bucket: publicBucket.bucket,
    versioningConfiguration: { status: "Enabled" },
  },
  { provider: rustack },
);

const mediaVersioning = new aws.s3.BucketVersioning(
  "media-versioning",
  {
    bucket: mediaBucket.bucket,
    versioningConfiguration: { status: "Enabled" },
  },
  { provider: rustack },
);

function siteAssetContent(index: number): string {
  return [
    "<!doctype html>",
    "<html>",
    "<head>",
    '<meta charset="utf-8">',
    `<title>Commerce Platform ${index}</title>`,
    '<meta name="cache-control" content="public">',
    "</head>",
    `<body><main>Commerce Platform Rustack fixture page ${index}</main></body>`,
    "</html>",
  ].join("");
}

const publicObjects: aws.s3.BucketObject[] = [];
for (let index = 0; index < 48; index += 1) {
  const padded = index.toString().padStart(3, "0");
  publicObjects.push(
    new aws.s3.BucketObject(
      `public-page-${padded}`,
      {
        bucket: publicBucket.bucket,
        key: `static/page-${padded}.html`,
        contentType: "text/html",
        cacheControl: "public, max-age=3600",
        content: siteAssetContent(index),
      },
      { provider: rustack, dependsOn: [publicVersioning] },
    ),
  );
}

const mediaObjects: aws.s3.BucketObject[] = [];
for (let index = 0; index < 48; index += 1) {
  const padded = index.toString().padStart(3, "0");
  mediaObjects.push(
    new aws.s3.BucketObject(
      `media-json-${padded}`,
      {
        bucket: mediaBucket.bucket,
        key: `catalog/media-${padded}.json`,
        contentType: "application/json",
        cacheControl: "public, max-age=7200",
        content: JSON.stringify({
          id: `media-${padded}`,
          title: `Product media ${padded}`,
          bytes: "x".repeat(1024 + index),
        }),
      },
      { provider: rustack, dependsOn: [mediaVersioning] },
    ),
  );
}

const privateObjects: aws.s3.BucketObject[] = [];
for (let index = 0; index < 16; index += 1) {
  const padded = index.toString().padStart(3, "0");
  privateObjects.push(
    new aws.s3.BucketObject(
      `private-report-${padded}`,
      {
        bucket: privateBucket.bucket,
        key: `reports/report-${padded}.json`,
        contentType: "application/json",
        content: JSON.stringify({ id: padded, kind: "private-report", sealed: true }),
      },
      { provider: rustack },
    ),
  );
}

function createTable(
  logicalName: string,
  tableName: string,
  streamEnabled: boolean,
): aws.dynamodb.Table {
  return new aws.dynamodb.Table(
    logicalName,
    {
      name: tableName,
      billingMode: "PAY_PER_REQUEST",
      hashKey: "pk",
      rangeKey: "sk",
      streamEnabled,
      streamViewType: streamEnabled ? "NEW_AND_OLD_IMAGES" : undefined,
      attributes: [
        { name: "pk", type: "S" },
        { name: "sk", type: "S" },
      ],
    },
    { provider: rustack },
  );
}

const catalogTable = createTable("catalog-table", `${name}-catalog`, true);
const ordersTable = createTable("orders-table", `${name}-orders`, true);
const sessionsTable = createTable("sessions-table", `${name}-sessions`, false);
const auditTable = createTable("audit-table", `${name}-audit`, true);

for (let index = 0; index < 24; index += 1) {
  const padded = index.toString().padStart(3, "0");
  new aws.dynamodb.TableItem(
    `catalog-seed-${padded}`,
    {
      tableName: catalogTable.name,
      hashKey: catalogTable.hashKey,
      rangeKey: catalogTable.rangeKey,
      item: JSON.stringify({
        pk: { S: "PRODUCT" },
        sk: { S: `SKU#${padded}` },
        title: { S: `Seed product ${padded}` },
        priceCents: { N: String(1000 + index) },
      }),
    },
    { provider: rustack },
  );
}

for (let index = 0; index < 24; index += 1) {
  const padded = index.toString().padStart(3, "0");
  new aws.dynamodb.TableItem(
    `order-seed-${padded}`,
    {
      tableName: ordersTable.name,
      hashKey: ordersTable.hashKey,
      rangeKey: ordersTable.rangeKey,
      item: JSON.stringify({
        pk: { S: "ORDER" },
        sk: { S: `ORDER#${padded}` },
        status: { S: index % 2 === 0 ? "PAID" : "PENDING" },
        amountCents: { N: String(2500 + index * 17) },
      }),
    },
    { provider: rustack },
  );
}

const queues = ["checkout", "inventory", "media", "email"].map(
  (queueName) =>
    new aws.sqs.Queue(
      `${queueName}-queue`,
      {
        name: `${name}-${queueName}`,
        visibilityTimeoutSeconds: 60,
      },
      { provider: rustack },
    ),
);

const parameters = Array.from({ length: 8 }, (_, index) => {
  const padded = index.toString().padStart(2, "0");
  return new aws.ssm.Parameter(
    `config-param-${padded}`,
    {
      name: `/commerce/${stackSuffix}/config-${padded}`,
      type: index % 3 === 0 ? "SecureString" : "String",
      value: JSON.stringify({ shard: padded, mode: "snapshot-benchmark" }),
    },
    { provider: rustack },
  );
});

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
        publicBucket.arn,
        mediaBucket.arn,
        privateBucket.arn,
        eventBucket.arn,
        catalogTable.arn,
        ordersTable.arn,
        sessionsTable.arn,
        auditTable.arn,
        queues[0].arn,
        queues[1].arn,
        queues[2].arn,
        queues[3].arn,
      ])
      .apply(
        ([
          publicArn,
          mediaArn,
          privateArn,
          eventArn,
          catalogArn,
          ordersArn,
          sessionsArn,
          auditArn,
          checkoutQueueArn,
          inventoryQueueArn,
          mediaQueueArn,
          emailQueueArn,
        ]) =>
          JSON.stringify({
            Version: "2012-10-17",
            Statement: [
              {
                Effect: "Allow",
                Action: ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
                Resource: [`${publicArn}/*`, `${mediaArn}/*`, `${privateArn}/*`, `${eventArn}/*`],
              },
              {
                Effect: "Allow",
                Action: ["dynamodb:GetItem", "dynamodb:PutItem", "dynamodb:UpdateItem", "dynamodb:Query"],
                Resource: [catalogArn, ordersArn, sessionsArn, auditArn],
              },
              {
                Effect: "Allow",
                Action: ["sqs:SendMessage", "sqs:ReceiveMessage", "sqs:DeleteMessage"],
                Resource: [checkoutQueueArn, inventoryQueueArn, mediaQueueArn, emailQueueArn],
              },
              {
                Effect: "Allow",
                Action: ["ssm:GetParameter", "ssm:GetParametersByPath"],
                Resource: "*",
              },
            ],
          }),
      ),
  },
  { provider: rustack },
);

function layerArchive(label: string): pulumi.asset.AssetArchive {
  return new pulumi.asset.AssetArchive({
    "nodejs/node_modules/runtime/index.js": new pulumi.asset.StringAsset(
      `exports.${label} = () => "${label}-ready";\n`,
    ),
  });
}

const commonLayer = new aws.lambda.LayerVersion(
  "common-layer-v1",
  {
    layerName: `${name}-common`,
    compatibleRuntimes: ["nodejs18.x"],
    description: "Shared commerce helpers",
    code: layerArchive("common"),
  },
  { provider: rustack },
);

const commonLayerV2 = new aws.lambda.LayerVersion(
  "common-layer-v2",
  {
    layerName: `${name}-common`,
    compatibleRuntimes: ["nodejs18.x"],
    description: "Shared commerce helpers v2",
    code: layerArchive("commonV2"),
  },
  { provider: rustack },
);

const observabilityLayer = new aws.lambda.LayerVersion(
  "observability-layer",
  {
    layerName: `${name}-observability`,
    compatibleRuntimes: ["nodejs18.x"],
    description: "Tracing and metrics helpers",
    code: layerArchive("observability"),
  },
  { provider: rustack },
);

const securityLayer = new aws.lambda.LayerVersion(
  "security-layer",
  {
    layerName: `${name}-security`,
    compatibleRuntimes: ["nodejs18.x"],
    description: "Request validation helpers",
    code: layerArchive("security"),
  },
  { provider: rustack },
);

function lambdaSource(handlerName: string): string {
  return [
    "exports.handler = async (event) => {",
    `  return { statusCode: 200, body: JSON.stringify({ handler: "${handlerName}", ok: true, eventType: event && event.version ? event.version : "unknown" }) };`,
    "};",
    "",
  ].join("\n");
}

const lambdaFunctions = [
  { key: "checkout", memory: 512, timeout: 20, layers: [commonLayerV2.arn, observabilityLayer.arn] },
  { key: "catalog", memory: 256, timeout: 10, layers: [commonLayer.arn, securityLayer.arn] },
  { key: "inventory", memory: 256, timeout: 15, layers: [commonLayerV2.arn, observabilityLayer.arn] },
  { key: "media", memory: 512, timeout: 30, layers: [commonLayer.arn, observabilityLayer.arn] },
  { key: "email", memory: 256, timeout: 15, layers: [commonLayer.arn, securityLayer.arn] },
  { key: "audit", memory: 256, timeout: 10, layers: [commonLayerV2.arn, securityLayer.arn] },
].map((spec) => {
  const fn = new aws.lambda.Function(
    `${spec.key}-function`,
    {
      name: `${name}-${spec.key}`,
      role: lambdaRole.arn,
      runtime: "nodejs18.x",
      handler: "index.handler",
      memorySize: spec.memory,
      timeout: spec.timeout,
      publish: true,
      layers: spec.layers,
      code: new pulumi.asset.AssetArchive({
        "index.js": new pulumi.asset.StringAsset(lambdaSource(spec.key)),
      }),
      environment: {
        variables: {
          CATALOG_TABLE: catalogTable.name,
          ORDERS_TABLE: ordersTable.name,
          SESSIONS_TABLE: sessionsTable.name,
          AUDIT_TABLE: auditTable.name,
          PUBLIC_BUCKET: publicBucket.bucket,
          MEDIA_BUCKET: mediaBucket.bucket,
          PRIVATE_BUCKET: privateBucket.bucket,
          EVENT_BUCKET: eventBucket.bucket,
          QUEUE_COUNT: String(queues.length),
        },
      },
    },
    { provider: rustack, dependsOn: [lambdaPolicy] },
  );

  const alias = new aws.lambda.Alias(
    `${spec.key}-live-alias`,
    {
      name: "live",
      functionName: fn.name,
      functionVersion: fn.version,
      description: `${spec.key} live alias`,
    },
    { provider: rustack },
  );

  return { spec, fn, alias };
});

new aws.lambda.EventSourceMapping(
  "checkout-events",
  {
    eventSourceArn: queues[0].arn,
    functionName: lambdaFunctions[0].fn.name,
    batchSize: 10,
    enabled: true,
  },
  { provider: rustack },
);

new aws.lambda.EventSourceMapping(
  "media-events",
  {
    eventSourceArn: queues[2].arn,
    functionName: lambdaFunctions[3].fn.name,
    batchSize: 10,
    enabled: true,
  },
  { provider: rustack },
);

const api = new aws.apigatewayv2.Api(
  "commerce-api",
  {
    name: `${name}-api`,
    protocolType: "HTTP",
  },
  { provider: rustack },
);

const apiIntegrations = lambdaFunctions.slice(0, 3).map(
  ({ spec, fn }) =>
    new aws.apigatewayv2.Integration(
      `${spec.key}-integration`,
      {
        apiId: api.id,
        integrationType: "AWS_PROXY",
        integrationMethod: "POST",
        integrationUri: fn.invokeArn,
        payloadFormatVersion: "2.0",
      },
      { provider: rustack },
    ),
);

apiIntegrations.forEach((integration, index) => {
  const routeKey = ["ANY /checkout/{proxy+}", "ANY /catalog/{proxy+}", "ANY /inventory/{proxy+}"][
    index
  ];
  new aws.apigatewayv2.Route(
    `api-route-${index}`,
    {
      apiId: api.id,
      routeKey,
      target: pulumi.interpolate`integrations/${integration.id}`,
    },
    { provider: rustack },
  );
});

const apiStage = new aws.apigatewayv2.Stage(
  "api-stage",
  {
    apiId: api.id,
    name: "prod",
    autoDeploy: true,
  },
  { provider: rustack },
);

lambdaFunctions.slice(0, 3).forEach(({ spec, fn }) => {
  new aws.lambda.Permission(
    `${spec.key}-api-permission`,
    {
      statementId: `${name}-${spec.key}-api`,
      action: "lambda:InvokeFunction",
      function: fn.name,
      principal: "apigateway.amazonaws.com",
      sourceArn: pulumi.interpolate`${api.executionArn}/*/*`,
    },
    { provider: rustack },
  );
});

const originAccessControl = new aws.cloudfront.OriginAccessControl(
  "origin-access-control",
  {
    name: `${name}-s3-oac`,
    description: "Commerce platform S3 origin access control",
    originAccessControlOriginType: "s3",
    signingBehavior: "always",
    signingProtocol: "sigv4",
  },
  { provider: rustack },
);

const viewerFunction = new aws.cloudfront.Function(
  "viewer-function",
  {
    name: `${name}-viewer`,
    runtime: "cloudfront-js-1.0",
    publish: true,
    code: "function handler(event) { event.request.headers['x-rustack-fixture'] = { value: 'commerce' }; return event.request; }\n",
  },
  { provider: rustack },
);

const authFunction = new aws.cloudfront.Function(
  "auth-function",
  {
    name: `${name}-auth`,
    runtime: "cloudfront-js-1.0",
    publish: true,
    code: "function handler(event) { return event.request; }\n",
  },
  { provider: rustack },
);

const staticCachePolicy = new aws.cloudfront.CachePolicy(
  "static-cache-policy",
  {
    name: `${name}-static-cache`,
    comment: "Static commerce content",
    defaultTtl: 3600,
    minTtl: 60,
    maxTtl: 86400,
    parametersInCacheKeyAndForwardedToOrigin: {
      cookiesConfig: { cookieBehavior: "none" },
      headersConfig: { headerBehavior: "none" },
      queryStringsConfig: { queryStringBehavior: "none" },
    },
  },
  { provider: rustack },
);

const mediaCachePolicy = new aws.cloudfront.CachePolicy(
  "media-cache-policy",
  {
    name: `${name}-media-cache`,
    comment: "Media metadata content",
    defaultTtl: 7200,
    minTtl: 60,
    maxTtl: 86400,
    parametersInCacheKeyAndForwardedToOrigin: {
      cookiesConfig: { cookieBehavior: "none" },
      headersConfig: { headerBehavior: "none" },
      queryStringsConfig: { queryStringBehavior: "whitelist", queryStrings: { items: ["v"] } },
    },
  },
  { provider: rustack },
);

const apiDomainName = api.apiEndpoint.apply((value) =>
  value.replace(/^https?:\/\//, "").replace(/\/$/, ""),
);

const distribution = new aws.cloudfront.Distribution(
  "commerce-distribution",
  {
    enabled: true,
    waitForDeployment: false,
    defaultRootObject: "static/page-000.html",
    comment: "Rustack commerce platform CDN",
    origins: [
      {
        originId: "public-s3",
        domainName: pulumi.interpolate`${publicBucket.bucket}.s3.amazonaws.com`,
        originAccessControlId: originAccessControl.id,
      },
      {
        originId: "media-s3",
        domainName: pulumi.interpolate`${mediaBucket.bucket}.s3.amazonaws.com`,
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
      targetOriginId: "public-s3",
      viewerProtocolPolicy: "allow-all",
      allowedMethods: ["GET", "HEAD", "OPTIONS"],
      cachedMethods: ["GET", "HEAD"],
      cachePolicyId: staticCachePolicy.id,
      forwardedValues: {
        queryString: false,
        cookies: { forward: "none" },
      },
      functionAssociations: [
        {
          eventType: "viewer-request",
          functionArn: viewerFunction.arn,
        },
      ],
    },
    orderedCacheBehaviors: [
      {
        pathPattern: "/catalog/*",
        targetOriginId: "media-s3",
        viewerProtocolPolicy: "allow-all",
        allowedMethods: ["GET", "HEAD", "OPTIONS"],
        cachedMethods: ["GET", "HEAD"],
        cachePolicyId: mediaCachePolicy.id,
        forwardedValues: {
          queryString: true,
          cookies: { forward: "none" },
        },
      },
      {
        pathPattern: "/checkout/*",
        targetOriginId: "api-gateway",
        viewerProtocolPolicy: "allow-all",
        allowedMethods: ["GET", "HEAD", "OPTIONS", "POST"],
        cachedMethods: ["GET", "HEAD"],
        forwardedValues: {
          queryString: true,
          headers: ["*"],
          cookies: { forward: "all" },
        },
        functionAssociations: [
          {
            eventType: "viewer-request",
            functionArn: authFunction.arn,
          },
        ],
      },
    ],
    restrictions: {
      geoRestriction: { restrictionType: "none" },
    },
    viewerCertificate: {
      cloudfrontDefaultCertificate: true,
    },
  },
  {
    provider: rustack,
    dependsOn: [...publicObjects, ...mediaObjects, apiStage],
  },
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

new aws.s3.BucketPolicy(
  "public-bucket-policy",
  {
    bucket: publicBucket.bucket,
    policy: cloudfrontReadPolicy(publicBucket.arn, distribution.arn),
  },
  { provider: rustack },
);

new aws.s3.BucketPolicy(
  "media-bucket-policy",
  {
    bucket: mediaBucket.bucket,
    policy: cloudfrontReadPolicy(mediaBucket.arn, distribution.arn),
  },
  { provider: rustack },
);

export const rustackEndpoint = endpoint;
export const callerAccount = caller.accountId;
export const publicBucketName = publicBucket.bucket;
export const mediaBucketName = mediaBucket.bucket;
export const privateBucketName = privateBucket.bucket;
export const eventBucketName = eventBucket.bucket;
export const catalogTableName = catalogTable.name;
export const ordersTableName = ordersTable.name;
export const sessionsTableName = sessionsTable.name;
export const auditTableName = auditTable.name;
export const checkoutQueueUrl = queues[0].url;
export const mediaQueueUrl = queues[2].url;
export const firstParameterName = parameters[0].name;
export const commonLayerArn = commonLayer.arn;
export const commonLayerV2Arn = commonLayerV2.arn;
export const observabilityLayerArn = observabilityLayer.arn;
export const securityLayerArn = securityLayer.arn;
export const checkoutFunctionArn = lambdaFunctions[0].fn.arn;
export const catalogFunctionArn = lambdaFunctions[1].fn.arn;
export const checkoutFunctionName = lambdaFunctions[0].fn.name;
export const catalogFunctionName = lambdaFunctions[1].fn.name;
export const apiEndpoint = api.apiEndpoint;
export const apiInvokeUrl = apiStage.invokeUrl;
export const cloudfrontDistributionId = distribution.id;
export const cloudfrontDomainName = distribution.domainName;
