# Rustack Pulumi Target Design

**Date:** 2026-05-25
**Status:** Implemented
**Scope:** Make Rustack usable as a Pulumi deployment target through the Pulumi AWS provider, with an executable smoke test and documented compatibility contract.

---

## 1. Executive Summary

Rustack should become a Pulumi target by presenting itself as a local AWS-compatible endpoint to the existing Pulumi AWS provider. This is the pragmatic path because Pulumi already models AWS resources and the provider exposes endpoint overrides for AWS service clients. Rustack does not need a native Pulumi provider for the first milestone.

The implemented shape is:

- `examples/pulumi/rustack-target`: TypeScript Pulumi program using `@pulumi/aws`.
- `scripts/pulumi-rustack-smoke.sh`: end-to-end smoke runner that builds/starts Rustack, runs Pulumi with a local backend, deploys resources, prints outputs, and destroys them.
- `make pulumi-smoke`: repository-level target for local and CI usage.
- `.github/workflows/pulumi-test.yml`: pull-request smoke workflow.
- `docs/pulumi.md`: concise user guide and provider contract.

## 2. Background

Pulumi providers are resource plugins that translate Pulumi resource operations into provider API calls. The Pulumi AWS provider supports explicit provider instances, which lets a program configure credentials, region, endpoint overrides, and provider behavior per resource group. The AWS provider also exposes `endpoints`, `s3UsePathStyle`, `skipMetadataApiCheck`, `skipRegionValidation`, `skipCredentialsValidation`, and `skipRequestingAccountId`.

Rustack already implements the key bootstrap APIs Pulumi needs:

- STS `GetCallerIdentity` for account discovery and credential validation.
- IAM awsQuery routing for IAM-backed resource workflows.
- S3 path-style operations.
- SQS/SNS/DynamoDB management APIs, including the read-after-write fields the
  Pulumi AWS provider waits on.
- Gateway routing by SigV4 service name for awsQuery services sharing `POST /`.

## 3. Decision

Use Pulumi AWS Classic (`@pulumi/aws`) as the frontend and point every supported AWS service endpoint to Rustack:

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
    sns: "http://127.0.0.1:4566",
    sqs: "http://127.0.0.1:4566",
    ssm: "http://127.0.0.1:4566",
    sts: "http://127.0.0.1:4566",
  }],
  s3UsePathStyle: true,
  skipCredentialsValidation: true,
  skipMetadataApiCheck: true,
  skipRegionValidation: true,
  skipRequestingAccountId: true,
});
```

This decision keeps Rustack aligned with Terraform, CDK, AWS SDKs, and AWS CLI behavior: all clients speak AWS APIs, Rustack serves AWS-compatible APIs.

## 4. Goals

1. Provide a runnable Pulumi project that deploys real resources into Rustack.
2. Validate Rustack STS through an explicit Pulumi data-source call after provider initialization.
3. Cover provider initialization, STS data-source routing, and a representative CRUD path for every currently provisionable Rustack service.
4. Make the smoke test self-contained: it can build/start Rustack, create a temporary Pulumi backend, deploy, destroy, and clean state.
5. Document the provider configuration users need to copy into their own Pulumi programs.
6. Keep the implementation independent from Pulumi Cloud credentials by using a local file backend.

## 5. Non-Goals

1. Do not build a native `pulumi-resource-rustack` provider in this milestone.
2. Do not fork or wrap the Pulumi AWS provider.
3. Do not promise that every `@pulumi/aws` resource works; compatibility is bounded by Rustack's implemented AWS API surface and the provider's read-after-write calls.
4. Do not use Pulumi Deployments/Pulumi Cloud as the default test path.
5. Do not persist Rustack data across smoke-test runs.
6. Do not simulate non-provisioning data planes as Pulumi-managed resources. For example, CloudFront's local data plane is exercised by HTTP requests, not by a Pulumi resource.

## 6. Provider Contract

### Required Pulumi Settings

| Setting | Value | Reason |
|---------|-------|--------|
| `accessKey` | `AKIAIOSFODNN7EXAMPLE` or caller-provided | Uses a realistic local access key shape accepted by the Pulumi AWS provider. |
| `secretKey` | `wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY` or caller-provided secret | Uses a realistic local secret key shape accepted by the Pulumi AWS provider. |
| `region` | `us-east-1` default | Matches Rustack default region. |
| `endpoints` | one object with supported service URLs | Forces the provider to call Rustack instead of AWS. |
| `s3UsePathStyle` | `true` | Avoids bucket-name virtual host DNS requirements. |
| `skipMetadataApiCheck` | `true` | Rustack is not EC2 IMDS. |
| `skipRegionValidation` | `true` | Avoids AWS public region metadata assumptions. |
| `skipCredentialsValidation` | `true` | Avoids provider bootstrap checks that can validate credentials before endpoint behavior is observable. |
| `skipRequestingAccountId` | `true` | Avoids implicit provider account lookup during configuration. |

### Endpoint Map

All endpoints use the same gateway URL in normal local mode:

| Pulumi endpoint key | Rustack service |
|---------------------|-----------------|
| `s3`, `s3api` | S3 |
| `dynamodb` | DynamoDB |
| `sqs` | SQS |
| `sns` | SNS |
| `iam` | IAM |
| `sts` | STS |
| `lambda` | Lambda |
| `eventbridge`, `events` | EventBridge |
| `cloudwatch` | CloudWatch Metrics |
| `cloudwatchlogs`, `logs` | CloudWatch Logs |
| `kms` | KMS |
| `kinesis` | Kinesis |
| `secretsmanager` | Secrets Manager |
| `ses`, `sesv2` | SES |
| `apigatewayv2` | API Gateway V2 |
| `cloudfront` | CloudFront |

The example sets all currently relevant keys so users can extend the program without revisiting the provider block.

## 7. Smoke Stack

The checked Pulumi stack creates:

- `aws.getCallerIdentityOutput`
- `aws.apigatewayv2.Api`
- `aws.apigatewayv2.Stage`
- `aws.cloudfront.Function`
- `aws.cloudwatch.EventBus`
- `aws.cloudwatch.EventRule`
- `aws.cloudwatch.LogGroup`
- `aws.cloudwatch.MetricAlarm`
- `aws.dynamodb.Table` with `streamEnabled`
- `aws.iam.Role`
- `aws.kinesis.Stream`
- `aws.kms.Key`
- `aws.lambda.Function`
- `aws.s3.Bucket`
- `aws.s3.BucketObject`
- `aws.secretsmanager.Secret`
- `aws.secretsmanager.SecretVersion`
- `aws.ses.EmailIdentity`
- `aws.ses.Template`
- `aws.sns.Topic`
- `aws.sqs.Queue`
- `aws.ssm.Parameter`

This set is intentionally representative rather than exhaustive. It proves that
each current Rustack service with a Pulumi-provisionable control-plane resource
can be targeted by the Pulumi AWS provider. Adding a new Rustack service or a
new high-value resource type should extend this stack and require a passing
`make pulumi-smoke` run.

### Compatibility Matrix

| Rustack service | Pulumi coverage | Notes |
|-----------------|-----------------|-------|
| API Gateway V2 | `aws.apigatewayv2.Api`, `aws.apigatewayv2.Stage` | Covers HTTP API and stage CRUD. |
| CloudFront | `aws.cloudfront.Function` | Covers function create/read/publish/delete routes. Distribution resources are broader than the current smoke. |
| CloudWatch | `aws.cloudwatch.MetricAlarm` | Covers monitoring awsQuery routing. |
| DynamoDB | `aws.dynamodb.Table` | Covers table waiter, PITR read path, tags, warm throughput response fields, and delete. |
| DynamoDB Streams | `aws.dynamodb.Table` with streams enabled | Streams are provisioned through the table's stream specification. |
| EventBridge | `aws.cloudwatch.EventBus`, `aws.cloudwatch.EventRule` | Pulumi uses the historical `cloudwatch` namespace for EventBridge resources. |
| IAM | `aws.iam.Role` | Covers awsQuery IAM routing and policy document handling. |
| Kinesis | `aws.kinesis.Stream` | Covers stream create/read/delete waiters. |
| KMS | `aws.kms.Key` | Covers key lifecycle used by local IaC tests. |
| Lambda | `aws.lambda.Function` | Covers archive upload, function read, concurrency read, event invoke config read, and code signing config read. |
| Logs | `aws.cloudwatch.LogGroup` | Covers log group create/read/delete. |
| S3 | `aws.s3.Bucket`, `aws.s3.BucketObject` | Requires path-style addressing. |
| Secrets Manager | `aws.secretsmanager.Secret`, `aws.secretsmanager.SecretVersion` | Covers secret metadata and version lifecycle. |
| SES | `aws.ses.EmailIdentity`, `aws.ses.Template` | Covers SES query routing and template CRUD. |
| SNS | `aws.sns.Topic` | Covers provider-managed topic attributes and default policy. |
| SQS | `aws.sqs.Queue` | Delete cleanup waits around two minutes due the Terraform AWS provider waiter. |
| SSM | `aws.ssm.Parameter` | Empty `AllowedPattern` from the provider is treated as unset. |
| STS | `aws.getCallerIdentityOutput` | Data source coverage, not a provisioned resource. |

## 8. Script Contract

`scripts/pulumi-rustack-smoke.sh`:

1. Requires `cargo`, `curl`, `npm`, and `pulumi`.
2. Checks `RUSTACK_ENDPOINT`, defaulting to `http://127.0.0.1:4566`.
3. Starts Rustack automatically only for the default local endpoint.
4. Uses a temporary Pulumi file backend unless `PULUMI_STATE_DIR` is supplied.
5. Uses `PULUMI_CONFIG_PASSPHRASE=""` for local encrypted config.
6. Runs `npm ci` when `package-lock.json` exists.
7. Runs `npm run typecheck`.
8. Creates a stack, sets provider config, runs `pulumi up --yes --skip-preview`.
9. Prints stack outputs as JSON.
10. Destroys resources, removes the temporary stack, stops the child Rustack process, and removes temporary state on exit.

The cleanup path destroys resources after a successful `pulumi up`. If
`pulumi up` fails, the script still removes the temporary stack, stops any child
Rustack process, and removes temporary state; for the default auto-started path,
Rustack's in-memory resources are discarded with the process. SQS queue delete
confirmation can take around two minutes because the Terraform AWS provider
requires repeated not-found confirmations before completing the delete.

## 9. Compatibility Risks

### Provider Read-After-Write Expansion

The Pulumi AWS provider is based on the Terraform AWS provider. A resource may call additional AWS APIs during create, read, update, or delete beyond the obvious API names. A Rustack service can appear implemented for direct SDK usage but still fail a Pulumi resource if the provider performs an unsupported read-after-write check.

Mitigation: every resource added to the example must be validated by `make pulumi-smoke`.

### Service Routing Ambiguity

Several AWS services use awsQuery over `POST /` with form-encoded bodies. Rustack routes these by SigV4 signing service names. Pulumi requests must be signed with service names Rustack recognizes, such as `sts`, `iam`, `sns`, `ses`, and `monitoring`.

Mitigation: keep STS/IAM/CloudWatch/SES/SNS routers registered before the S3 catch-all and keep endpoint keys aligned with Pulumi AWS provider signing names.

### S3 Addressing

Virtual-hosted S3 addressing requires local DNS names that are not guaranteed on every developer or CI machine.

Mitigation: force `s3UsePathStyle: true`.

### Account ID Discovery

Some local AWS emulators recommend `skipCredentialsValidation` and `skipRequestingAccountId`. Rustack implements STS, but provider initialization still has to be compatible with the Pulumi AWS provider's credential bootstrap behavior.

In practice, the Pulumi AWS provider validates provider configuration before any resource CRUD. Endpoint overrides are configured on the provider, but the provider's credential bootstrap path may still fail before Rustack-specific behavior is useful to the user.

Mitigation: set both skip flags to true for provider initialization and add an explicit `aws.getCallerIdentityOutput` invocation with the Rustack provider. This keeps provider startup compatible while still validating STS routing through Pulumi.

### Credential Shape

The Pulumi AWS provider validates static credential shape before making resource calls. Very short local placeholders like `test/test` fail during provider initialization.

Mitigation: use AWS documentation-style fake credentials by default while keeping all calls pointed at Rustack.

## 10. Compatibility Fixes Implemented

The full smoke stack exposed provider-specific behavior that direct SDK tests did
not cover. The implementation now includes these compatibility fixes:

1. SSM treats an empty `AllowedPattern` as absent because the Pulumi AWS provider sends an empty string for the default case.
2. SNS accepts and round-trips provider-managed topic feedback attributes, returns a default JSON policy, and maps `FifoThroughputLimit` to Rustack's stored FIFO throughput scope.
3. S3 returns `ObjectLockConfigurationNotFoundError` for buckets without object lock configuration, matching the AWS error name expected by the provider.
4. DynamoDB table descriptions include `WarmThroughput`, and Rustack implements `DescribeContinuousBackups` and `UpdateContinuousBackups` for the provider's PITR read path.
5. CloudFront routes `DescribeFunction` and `GetFunction` using the AWS paths emitted by the provider.
6. Lambda implements `GetFunctionCodeSigningConfig`; functions without a code signing config return a successful response with an empty ARN, which is what the Terraform AWS provider expects.

## 11. Future Work

1. Add a machine-readable compatibility matrix generated from successful Pulumi smoke runs.
2. Extend the smoke stack when Rustack adds new provisionable services or high-value resource types such as broader CloudFront distribution coverage.
3. Consider a small helper package only if users repeatedly need a typed Rustack provider factory; do not build a native provider unless Rustack exposes non-AWS resources that Pulumi should manage directly.

## 12. References

- Pulumi AWS provider `aws.Provider` endpoint and validation options: https://www.pulumi.com/registry/packages/aws/api-docs/provider/
- Pulumi resource providers concept: https://www.pulumi.com/docs/iac/concepts/providers/
- Pulumi provider resource option inheritance: https://www.pulumi.com/docs/iac/concepts/resources/options/providers/
