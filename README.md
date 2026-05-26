# Rustack

A high-performance, LocalStack-compatible AWS service emulator written in Rust. **18 services, 779 routed operations, <1s startup, ~8 MB Docker image.**

## Install

```bash
# From crates.io
cargo install rustack-cli

# From source
cargo install --git https://github.com/tyrchen/rustack rustack-cli

# Or use Docker
docker run -p 4566:4566 ghcr.io/tyrchen/rustack:latest
```

## Quick Start

```bash
# Start the server (all 18 services on port 4566)
rustack

# Or start with specific services only
SERVICES=s3,dynamodb,sqs rustack

# Use with any AWS SDK or CLI
export AWS_ENDPOINT_URL=http://localhost:4566
export AWS_ACCESS_KEY_ID=test
export AWS_SECRET_ACCESS_KEY=test

aws s3 mb s3://my-bucket
aws s3 cp file.txt s3://my-bucket/
aws dynamodb create-table --table-name users --key-schema AttributeName=id,KeyType=HASH --attribute-definitions AttributeName=id,AttributeType=S --billing-mode PAY_PER_REQUEST
aws sqs create-queue --queue-name my-queue
```

### Docker Compose

```yaml
services:
  rustack:
    image: ghcr.io/tyrchen/rustack:latest
    ports:
      - "4566:4566"
    environment:
      - SERVICES=s3,dynamodb,sqs,lambda
      - LOG_LEVEL=info

  app:
    build: .
    depends_on:
      - rustack
    environment:
      - AWS_ENDPOINT_URL=http://rustack:4566
      - AWS_ACCESS_KEY_ID=test
      - AWS_SECRET_ACCESS_KEY=test
      - AWS_DEFAULT_REGION=us-east-1
```

## Why Rustack?

| | Rustack | LocalStack |
|---|---|---|
| **Language** | Rust (static binary) | Python |
| **Docker image** | ~8 MB (scratch) | ~475 MB / ~1.88 GB on disk |
| **Startup time** | < 1 second | 10-45s (S3 only); up to 2 min (all) |
| **Memory (idle)** | ~10 MB | ~750 MB minimum |
| **Services** | 18 | 80+ |
| **Operations** | 600+ | More per service, but behind paywall |
| **CI cold start** | Pull + ready in ~3s | Pull + ready in 30-90s |
| **Auth** | SigV4 + SigV2 + presigned URLs | SigV4 (Pro for IAM enforcement) |
| **License** | MIT, fully open source | Registration-required; free tier limited |

## Supported Services

| Service | Operations | Protocol |
|---------|-----------|----------|
| **S3** | 71 | REST XML |
| **DynamoDB** | 24 | awsJson 1.0 |
| **DynamoDB Streams** | 4 | awsJson 1.0 |
| **SQS** | 23 | awsJson 1.0 |
| **SSM Parameter Store** | 13 | awsJson 1.1 |
| **SNS** | 42 | awsQuery |
| **Lambda** | 50 | REST JSON |
| **EventBridge** | 57 | awsJson 1.1 |
| **CloudWatch Logs** | 43 | awsJson 1.1 |
| **KMS** | 39 | awsJson 1.0 |
| **Kinesis** | 29 | awsJson 1.1 / rpcv2Cbor |
| **Secrets Manager** | 23 | awsJson 1.1 |
| **SES** | 44 | awsQuery |
| **API Gateway V2** | 57 | REST JSON |
| **CloudFront** | 137 | REST XML + minimal data plane |
| **CloudWatch Metrics** | 31 | awsQuery |
| **IAM** | 84 | awsQuery |
| **STS** | 8 | awsQuery |

<details>
<summary><b>S3 operations (71)</b></summary>

| Category | Operations |
|----------|-----------|
| Bucket CRUD | CreateBucket, DeleteBucket, HeadBucket, ListBuckets, GetBucketLocation |
| Objects | PutObject, GetObject, HeadObject, DeleteObject, DeleteObjects, CopyObject, PostObject |
| Multipart | CreateMultipartUpload, UploadPart, UploadPartCopy, CompleteMultipartUpload, AbortMultipartUpload, ListParts, ListMultipartUploads |
| Listing | ListObjects, ListObjectsV2, ListObjectVersions |
| Versioning | GetBucketVersioning, PutBucketVersioning |
| Encryption | GetBucketEncryption, PutBucketEncryption, DeleteBucketEncryption |
| CORS | GetBucketCors, PutBucketCors, DeleteBucketCors |
| Lifecycle | GetBucketLifecycleConfiguration, PutBucketLifecycleConfiguration, DeleteBucketLifecycle |
| Policy | GetBucketPolicy, PutBucketPolicy, DeleteBucketPolicy, GetBucketPolicyStatus |
| Tagging | GetBucketTagging, PutBucketTagging, DeleteBucketTagging, GetObjectTagging, PutObjectTagging, DeleteObjectTagging |
| Notifications | GetBucketNotificationConfiguration, PutBucketNotificationConfiguration |
| Logging | GetBucketLogging, PutBucketLogging |
| Public Access | GetPublicAccessBlock, PutPublicAccessBlock, DeletePublicAccessBlock |
| Ownership | GetBucketOwnershipControls, PutBucketOwnershipControls, DeleteBucketOwnershipControls |
| Object Lock | GetObjectLockConfiguration, PutObjectLockConfiguration, GetObjectRetention, PutObjectRetention, GetObjectLegalHold, PutObjectLegalHold |
| Accelerate | GetBucketAccelerateConfiguration, PutBucketAccelerateConfiguration |
| Payment | GetBucketRequestPayment, PutBucketRequestPayment |
| Website | GetBucketWebsite, PutBucketWebsite, DeleteBucketWebsite |
| ACL | GetBucketAcl, PutBucketAcl, GetObjectAcl, PutObjectAcl |
| Attributes | GetObjectAttributes |

</details>

<details>
<summary><b>DynamoDB operations (24)</b></summary>

| Category | Operations |
|----------|-----------|
| Table management | CreateTable, DeleteTable, DescribeTable, ListTables, UpdateTable |
| Item CRUD | PutItem, GetItem, DeleteItem, UpdateItem |
| Query & scan | Query, Scan |
| Batch | BatchWriteItem, BatchGetItem |
| Transactions | TransactGetItems, TransactWriteItems |
| Table metadata | DescribeLimits, DescribeEndpoints |
| Tags and TTL | TagResource, UntagResource, ListTagsOfResource, DescribeTimeToLive, UpdateTimeToLive |
| Backups | DescribeContinuousBackups, UpdateContinuousBackups |

Features: condition expressions, filter expressions, projection expressions, update expressions (SET, REMOVE, ADD, DELETE), key conditions with sort key operators, consistent/eventually-consistent reads, point-in-time recovery metadata, and provider-friendly warm throughput descriptions.

</details>

<details>
<summary><b>SQS operations (23)</b></summary>

| Category | Operations |
|----------|-----------|
| Queue management | CreateQueue, DeleteQueue, GetQueueUrl, ListQueues, GetQueueAttributes, SetQueueAttributes |
| Messages | SendMessage, ReceiveMessage, DeleteMessage, ChangeMessageVisibility, PurgeQueue |
| Batch | SendMessageBatch, DeleteMessageBatch, ChangeMessageVisibilityBatch |
| Tags | TagQueue, UntagQueue, ListQueueTags |
| Permissions | AddPermission, RemovePermission |
| Dead-letter queues | ListDeadLetterSourceQueues |

Features: standard and FIFO queues, content-based deduplication, message groups, dead-letter queue redrive, long polling, visibility timeouts, message delay.

</details>

<details>
<summary><b>SSM Parameter Store operations (13)</b></summary>

| Category | Operations |
|----------|-----------|
| CRUD | PutParameter, GetParameter, GetParameters, DeleteParameter, DeleteParameters |
| Path queries | GetParametersByPath |
| Metadata | DescribeParameters, GetParameterHistory |
| Tags | AddTagsToResource, RemoveTagsFromResource, ListTagsForResource |
| Labels | LabelParameterVersion, UnlabelParameterVersion |

Features: String, StringList, SecureString types, hierarchical paths, 100-version history, version/label selectors, AllowedPattern validation.

</details>

<details>
<summary><b>SNS operations (42)</b></summary>

CreateTopic, DeleteTopic, GetTopicAttributes, SetTopicAttributes, ListTopics, Subscribe, Unsubscribe, ConfirmSubscription, GetSubscriptionAttributes, SetSubscriptionAttributes, ListSubscriptions, ListSubscriptionsByTopic, Publish, PublishBatch, AddPermission, RemovePermission, TagResource, UntagResource, ListTagsForResource, CreatePlatformApplication, DeletePlatformApplication, GetPlatformApplicationAttributes, SetPlatformApplicationAttributes, ListPlatformApplications, CreatePlatformEndpoint, DeleteEndpoint, GetEndpointAttributes, SetEndpointAttributes, ListEndpointsByPlatformApplication, CheckIfPhoneNumberIsOptedOut, GetSMSAttributes, SetSMSAttributes, ListPhoneNumbersOptedOut, OptInPhoneNumber, GetSMSSandboxAccountStatus, CreateSMSSandboxPhoneNumber, DeleteSMSSandboxPhoneNumber, VerifySMSSandboxPhoneNumber, ListSMSSandboxPhoneNumbers, ListOriginationNumbers, GetDataProtectionPolicy, PutDataProtectionPolicy

</details>

<details>
<summary><b>Lambda operations (50)</b></summary>

CreateFunction, DeleteFunction, GetFunction, GetFunctionConfiguration, GetFunctionCodeSigningConfig, UpdateFunctionCode, UpdateFunctionConfiguration, ListFunctions, Invoke, PublishVersion, ListVersionsByFunction, CreateAlias, DeleteAlias, GetAlias, UpdateAlias, ListAliases, CreateEventSourceMapping, DeleteEventSourceMapping, GetEventSourceMapping, UpdateEventSourceMapping, ListEventSourceMappings, TagResource, UntagResource, ListTags, AddPermission, RemovePermission, GetPolicy, PutFunctionConcurrency, DeleteFunctionConcurrency, GetFunctionConcurrency, PutFunctionEventInvokeConfig, GetFunctionEventInvokeConfig, UpdateFunctionEventInvokeConfig, DeleteFunctionEventInvokeConfig, ListFunctionEventInvokeConfigs, GetAccountSettings, CreateFunctionUrlConfig, GetFunctionUrlConfig, UpdateFunctionUrlConfig, DeleteFunctionUrlConfig, ListFunctionUrlConfigs, PublishLayerVersion, DeleteLayerVersion, GetLayerVersion, GetLayerVersionByArn, ListLayerVersions, ListLayers, AddLayerVersionPermission, GetLayerVersionPolicy, RemoveLayerVersionPermission

</details>

<details>
<summary><b>EventBridge operations (57)</b></summary>

CreateEventBus, DeleteEventBus, DescribeEventBus, ListEventBuses, PutRule, DeleteRule, DescribeRule, ListRules, EnableRule, DisableRule, PutTargets, RemoveTargets, ListTargetsByRule, PutEvents, TestEventPattern, TagResource, UntagResource, ListTagsForResource, PutPermission, RemovePermission, ListRuleNamesByTarget, UpdateEventBus, CreateArchive, DeleteArchive, DescribeArchive, ListArchives, UpdateArchive, StartReplay, CancelReplay, DescribeReplay, ListReplays, CreateApiDestination, DeleteApiDestination, DescribeApiDestination, ListApiDestinations, UpdateApiDestination, CreateConnection, DeleteConnection, DescribeConnection, ListConnections, UpdateConnection, DeauthorizeConnection, CreateEndpoint, DeleteEndpoint, DescribeEndpoint, ListEndpoints, UpdateEndpoint, ActivateEventSource, CreatePartnerEventSource, DeactivateEventSource, DeletePartnerEventSource, DescribeEventSource, DescribePartnerEventSource, ListEventSources, ListPartnerEventSourceAccounts, ListPartnerEventSources, PutPartnerEvents

</details>

<details>
<summary><b>CloudWatch Logs operations (43)</b></summary>

CreateLogGroup, DeleteLogGroup, DescribeLogGroups, CreateLogStream, DeleteLogStream, DescribeLogStreams, PutLogEvents, GetLogEvents, FilterLogEvents, PutRetentionPolicy, DeleteRetentionPolicy, PutMetricFilter, DeleteMetricFilter, DescribeMetricFilters, PutSubscriptionFilter, DeleteSubscriptionFilter, DescribeSubscriptionFilters, PutResourcePolicy, DeleteResourcePolicy, DescribeResourcePolicies, TagResource, UntagResource, ListTagsForResource, TagLogGroup, UntagLogGroup, ListTagsLogGroup, PutDestination, PutDestinationPolicy, DeleteDestination, DescribeDestinations, AssociateKmsKey, DisassociateKmsKey, StartQuery, StopQuery, GetQueryResults, DescribeQueries, PutQueryDefinition, DeleteQueryDefinition, DescribeQueryDefinitions, CreateExportTask, CancelExportTask, DescribeExportTasks, TestMetricFilter

</details>

<details>
<summary><b>KMS operations (39)</b></summary>

CreateKey, DescribeKey, ListKeys, EnableKey, DisableKey, ScheduleKeyDeletion, CancelKeyDeletion, CreateAlias, DeleteAlias, ListAliases, UpdateAlias, Encrypt, Decrypt, ReEncrypt, GenerateDataKey, GenerateDataKeyWithoutPlaintext, GenerateDataKeyPair, GenerateDataKeyPairWithoutPlaintext, GenerateRandom, Sign, Verify, GetPublicKey, CreateGrant, RetireGrant, RevokeGrant, ListGrants, ListRetirableGrants, EnableKeyRotation, DisableKeyRotation, GetKeyRotationStatus, GetKeyPolicy, PutKeyPolicy, ListKeyPolicies, TagResource, UntagResource, ListResourceTags, UpdateKeyDescription, ReplicateKey, UpdatePrimaryRegion

</details>

<details>
<summary><b>Kinesis operations (29)</b></summary>

CreateStream, DeleteStream, DescribeStream, DescribeStreamSummary, ListStreams, UpdateShardCount, PutRecord, PutRecords, GetRecords, GetShardIterator, ListShards, AddTagsToStream, RemoveTagsFromStream, ListTagsForStream, IncreaseStreamRetentionPeriod, DecreaseStreamRetentionPeriod, MergeShards, SplitShard, StartStreamEncryption, StopStreamEncryption, DescribeLimits, RegisterStreamConsumer, DeregisterStreamConsumer, DescribeStreamConsumer, ListStreamConsumers, SubscribeToShard, GetResourcePolicy, PutResourcePolicy, DeleteResourcePolicy

</details>

<details>
<summary><b>Secrets Manager operations (23)</b></summary>

CreateSecret, DescribeSecret, GetSecretValue, PutSecretValue, UpdateSecret, DeleteSecret, RestoreSecret, ListSecrets, ListSecretVersionIds, GetRandomPassword, TagResource, UntagResource, UpdateSecretVersionStage, RotateSecret, CancelRotateSecret, BatchGetSecretValue, GetResourcePolicy, PutResourcePolicy, DeleteResourcePolicy, ValidateResourcePolicy, ReplicateSecretToRegions, RemoveRegionsFromReplication, StopReplicationToReplica

</details>

<details>
<summary><b>SES operations (44)</b></summary>

VerifyEmailIdentity, VerifyDomainIdentity, ListIdentities, DeleteIdentity, GetIdentityVerificationAttributes, VerifyEmailAddress, DeleteVerifiedEmailAddress, ListVerifiedEmailAddresses, SendEmail, SendRawEmail, GetSendQuota, GetSendStatistics, CreateTemplate, GetTemplate, UpdateTemplate, DeleteTemplate, ListTemplates, SendTemplatedEmail, CreateConfigurationSet, DeleteConfigurationSet, DescribeConfigurationSet, ListConfigurationSets, CreateConfigurationSetEventDestination, UpdateConfigurationSetEventDestination, DeleteConfigurationSetEventDestination, CreateReceiptRuleSet, DeleteReceiptRuleSet, CreateReceiptRule, DeleteReceiptRule, DescribeReceiptRuleSet, CloneReceiptRuleSet, DescribeActiveReceiptRuleSet, SetActiveReceiptRuleSet, SetIdentityNotificationTopic, SetIdentityFeedbackForwardingEnabled, GetIdentityNotificationAttributes, VerifyDomainDkim, GetIdentityDkimAttributes, SetIdentityMailFromDomain, GetIdentityMailFromDomainAttributes, GetIdentityPolicies, PutIdentityPolicy, DeleteIdentityPolicy, ListIdentityPolicies

</details>

<details>
<summary><b>API Gateway V2 operations (57)</b></summary>

CreateApi, GetApi, UpdateApi, DeleteApi, GetApis, CreateRoute, GetRoute, UpdateRoute, DeleteRoute, GetRoutes, CreateIntegration, GetIntegration, UpdateIntegration, DeleteIntegration, GetIntegrations, CreateStage, GetStage, UpdateStage, DeleteStage, GetStages, CreateDeployment, GetDeployment, DeleteDeployment, GetDeployments, CreateRouteResponse, GetRouteResponse, DeleteRouteResponse, GetRouteResponses, CreateAuthorizer, GetAuthorizer, UpdateAuthorizer, DeleteAuthorizer, GetAuthorizers, CreateModel, GetModel, UpdateModel, DeleteModel, GetModels, GetModelTemplate, CreateDomainName, GetDomainName, UpdateDomainName, DeleteDomainName, GetDomainNames, CreateVpcLink, GetVpcLink, UpdateVpcLink, DeleteVpcLink, GetVpcLinks, TagResource, UntagResource, GetTags, CreateApiMapping, GetApiMapping, UpdateApiMapping, DeleteApiMapping, GetApiMappings

</details>

<details>
<summary><b>CloudFront operations (137)</b></summary>

| Category | Operations |
|----------|-----------|
| Distributions | CreateDistribution, CreateDistributionWithTags, GetDistribution, GetDistributionConfig, UpdateDistribution, UpdateDistributionWithStagingConfig, DeleteDistribution, CopyDistribution, ListDistributions |
| Invalidations | CreateInvalidation, GetInvalidation, ListInvalidations |
| Origins | Origin Access Control, Origin Access Identity, VPC origin, Anycast IP list, TrustStore CRUD/list operations |
| Policies and keys | CachePolicy, OriginRequestPolicy, ResponseHeadersPolicy, KeyGroup, PublicKey CRUD/list operations |
| Functions and edge data | CreateFunction, DescribeFunction, GetFunction, UpdateFunction, DeleteFunction, PublishFunction, TestFunction, ListFunctions, KeyValueStore CRUD/list operations |
| Field-level encryption | FieldLevelEncryptionConfig and FieldLevelEncryptionProfile CRUD/list operations |
| Realtime and monitoring | RealtimeLogConfig CRUD/list operations, monitoring subscriptions |
| Streaming and deployment | StreamingDistribution and ContinuousDeploymentPolicy CRUD/list operations |
| Tags and routing helpers | TagResource, UntagResource, ListTagsForResource, alias/domain conflict helpers, distribution lookup helpers, resource policy helpers |

Features: CloudFront management-plane compatibility for IaC tools plus a minimal pass-through data plane that can serve CloudFront-style requests against local S3 and HTTP origins.

</details>

<details>
<summary><b>CloudWatch Metrics operations (31)</b></summary>

PutMetricData, GetMetricData, GetMetricStatistics, ListMetrics, PutMetricAlarm, DescribeAlarms, DescribeAlarmsForMetric, DeleteAlarms, SetAlarmState, EnableAlarmActions, DisableAlarmActions, DescribeAlarmHistory, TagResource, UntagResource, ListTagsForResource, PutCompositeAlarm, PutDashboard, GetDashboard, DeleteDashboards, ListDashboards, PutInsightRule, DeleteInsightRules, DescribeInsightRules, PutAnomalyDetector, DescribeAnomalyDetectors, DeleteAnomalyDetector, PutManagedInsightRules, PutMetricStream, DeleteMetricStream, ListMetricStreams, GetMetricStream

</details>

<details>
<summary><b>IAM operations (84)</b></summary>

CreateUser, GetUser, DeleteUser, ListUsers, UpdateUser, CreateRole, GetRole, DeleteRole, ListRoles, UpdateRole, CreatePolicy, GetPolicy, DeletePolicy, ListPolicies, AttachUserPolicy, DetachUserPolicy, ListAttachedUserPolicies, AttachRolePolicy, DetachRolePolicy, ListAttachedRolePolicies, CreateAccessKey, DeleteAccessKey, ListAccessKeys, UpdateAccessKey, GetAccessKeyLastUsed, CreateGroup, GetGroup, DeleteGroup, ListGroups, UpdateGroup, AddUserToGroup, RemoveUserFromGroup, ListGroupsForUser, AttachGroupPolicy, DetachGroupPolicy, ListAttachedGroupPolicies, CreateInstanceProfile, GetInstanceProfile, DeleteInstanceProfile, ListInstanceProfiles, ListInstanceProfilesForRole, AddRoleToInstanceProfile, RemoveRoleFromInstanceProfile, CreatePolicyVersion, GetPolicyVersion, DeletePolicyVersion, ListPolicyVersions, SetDefaultPolicyVersion, PutUserPolicy, GetUserPolicy, DeleteUserPolicy, ListUserPolicies, PutRolePolicy, GetRolePolicy, DeleteRolePolicy, ListRolePolicies, PutGroupPolicy, GetGroupPolicy, DeleteGroupPolicy, ListGroupPolicies, TagUser, UntagUser, ListUserTags, TagRole, UntagRole, ListRoleTags, CreateServiceLinkedRole, DeleteServiceLinkedRole, GetServiceLinkedRoleDeletionStatus, UpdateAssumeRolePolicy, SimulatePrincipalPolicy, SimulateCustomPolicy, ListEntitiesForPolicy, GetAccountAuthorizationDetails, CreateOpenIDConnectProvider, GetOpenIDConnectProvider, DeleteOpenIDConnectProvider, ListOpenIDConnectProviders, TagPolicy, UntagPolicy, ListPolicyTags, TagInstanceProfile, UntagInstanceProfile, ListInstanceProfileTags

</details>

<details>
<summary><b>STS operations (8)</b></summary>

GetCallerIdentity, AssumeRole, GetSessionToken, AssumeRoleWithWebIdentity, GetAccessKeyInfo, DecodeAuthorizationMessage, GetFederationToken, AssumeRoleWithSAML

</details>

<details>
<summary><b>DynamoDB Streams operations (4)</b></summary>

DescribeStream, GetShardIterator, GetRecords, ListStreams

</details>

## Configuration

All settings are controlled via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address and port |
| `SERVICES` | *(empty = all)* | Comma-separated list of services to enable |
| `LOG_LEVEL` | `info` | Log level (`error`, `warn`, `info`, `debug`, `trace`) |
| `RUST_LOG` | | Fine-grained tracing filter (overrides `LOG_LEVEL`) |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `S3_VIRTUAL_HOSTING` | `true` | Enable virtual-hosted-style addressing |
| `S3_DOMAIN` | `s3.localhost.localstack.cloud` | Virtual hosting domain |
| `S3_MAX_MEMORY_OBJECT_SIZE` | `524288` | Max S3 object size (bytes) before disk spillover |
| `<SERVICE>_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification per service |

### Selective Service Enablement

**Runtime** — choose which services to start:

```bash
SERVICES=s3,dynamodb,sqs rustack
```

**Compile-time** — exclude services from the binary entirely:

```bash
cargo build -p rustack --no-default-features --features s3,dynamodb
```

Available features: `s3`, `dynamodb`, `dynamodbstreams`, `sqs`, `ssm`, `sns`, `lambda`, `events`, `logs`, `kms`, `kinesis`, `secretsmanager`, `ses`, `apigatewayv2`, `cloudfront`, `cloudfront-dataplane`, `cloudwatch`, `iam`, `sts`

## GitHub Action

```yaml
steps:
  - uses: actions/checkout@v4
  - uses: tyrchen/rustack@v0
```

The action starts the server, waits for healthy, and exports `AWS_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID`, and `AWS_SECRET_ACCESS_KEY`. All subsequent AWS CLI/SDK calls use Rustack automatically.

See [action.yml](action.yml) for all inputs and outputs.

## Pulumi

Rustack can be used as a Pulumi deployment target through the standard Pulumi
AWS provider by overriding service endpoints to `http://127.0.0.1:4566`.

```bash
make pulumi-smoke
```

The smoke target runs a TypeScript Pulumi program that validates STS and
provisions representative resources across the current Rustack service set,
including CloudFront, API Gateway V2, Lambda, DynamoDB, S3, SNS, SQS, SSM, IAM,
KMS, Kinesis, CloudWatch, CloudWatch Logs, EventBridge, Secrets Manager, SES,
and DynamoDB Streams.

For a richer serverless topology with CloudFront -> S3/API Gateway/Lambda,
DynamoDB, SQS, SSM SecureString, and bucket policies:

```bash
make pulumi-hackathon-smoke
```

See [docs/pulumi.md](docs/pulumi.md) for the provider configuration contract
and coverage details.

## Architecture

```
rustack-core              — Shared types, config, multi-account/region state
rustack-auth              — AWS SigV4/SigV2 authentication

rustack-{service}-model   — Types auto-generated from AWS Smithy models
rustack-{service}-http    — HTTP routing, protocol handling, request/response conversion
rustack-{service}-core    — Business logic, in-memory state, storage engine
```

Each service follows the same three-crate pattern. The unified server binary (`rustack`) routes requests via a `ServiceRouter` trait based on request headers.

## Development

**Prerequisites:** Rust 1.93+ (pinned in `rust-toolchain.toml`)

```bash
make build      # Compile all crates
make test       # Run unit tests (cargo nextest)
make fmt        # Format with cargo +nightly fmt
make clippy     # Lint with -D warnings
make run        # Start the server locally
```

### Integration Tests

```bash
# Terminal 1: start the server
make run

# Terminal 2: run integration tests
cargo test -p rustack-integration -- --ignored
```

## License

MIT. See [LICENSE](LICENSE.md) for details.

Copyright 2025 Tyr Chen
