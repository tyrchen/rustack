# Rustack

A high-performance, LocalStack-compatible AWS service emulator written in Rust. **18 services, 600+ operations, <1s startup, ~8 MB Docker image.**

## Install

```bash
# From crates.io
cargo install rustack

# From source
cargo install --git https://github.com/tyrchen/rustack

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
| **S3** | 70 | REST XML |
| **DynamoDB** | 22 | awsJson 1.0 |
| **DynamoDB Streams** | 4 | awsJson 1.0 |
| **SQS** | 23 | awsJson 1.0 |
| **SSM Parameter Store** | 13 | awsJson 1.1 |
| **SNS** | 39 | awsQuery |
| **Lambda** | 45 | REST JSON |
| **EventBridge** | 48 | awsJson 1.1 |
| **CloudWatch Logs** | 42 | awsJson 1.1 |
| **KMS** | 39 | awsJson 1.0 |
| **Kinesis** | 28 | awsJson 1.1 / rpcv2Cbor |
| **Secrets Manager** | 22 | awsJson 1.1 |
| **SES** | 46 | awsQuery |
| **API Gateway V2** | 55 | REST JSON |
| **CloudWatch Metrics** | 32 | awsQuery |
| **IAM** | 96 | awsQuery |
| **STS** | 8 | awsQuery |

<details>
<summary><b>S3 operations (70)</b></summary>

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
<summary><b>DynamoDB operations (22)</b></summary>

| Category | Operations |
|----------|-----------|
| Table management | CreateTable, DeleteTable, DescribeTable, ListTables, UpdateTable |
| Item CRUD | PutItem, GetItem, DeleteItem, UpdateItem |
| Query & scan | Query, Scan |
| Batch | BatchWriteItem, BatchGetItem |
| Transactions | TransactGetItems, TransactWriteItems |

Features: condition expressions, filter expressions, projection expressions, update expressions (SET, REMOVE, ADD, DELETE), key conditions with sort key operators, consistent/eventually-consistent reads.

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
<summary><b>SNS operations (39)</b></summary>

CreateTopic, DeleteTopic, GetTopicAttributes, SetTopicAttributes, ListTopics, Publish, Subscribe, Unsubscribe, GetSubscriptionAttributes, SetSubscriptionAttributes, ListSubscriptions, ListSubscriptionsByTopic, ConfirmSubscription, TagResource, UntagResource, ListTagsForResource, CreatePlatformApplication, DeletePlatformApplication, GetPlatformApplicationAttributes, SetPlatformApplicationAttributes, ListPlatformApplications, CreatePlatformEndpoint, DeleteEndpoint, GetEndpointAttributes, SetEndpointAttributes, ListEndpointsByPlatformApplication, AddPermission, RemovePermission, CheckIfPhoneNumberIsOptedOut, GetSMSAttributes, SetSMSAttributes, ListPhoneNumbersOptedOut, OptInPhoneNumber, ListOriginationNumbers, ListSMSSandboxPhoneNumbers, CreateSMSSandboxPhoneNumber, DeleteSMSSandboxPhoneNumber, VerifySMSSandboxPhoneNumber, GetSMSSandboxAccountStatus

</details>

<details>
<summary><b>Lambda operations (45)</b></summary>

CreateFunction, DeleteFunction, GetFunction, GetFunctionConfiguration, UpdateFunctionCode, UpdateFunctionConfiguration, ListFunctions, Invoke, PublishVersion, ListVersionsByFunction, CreateAlias, DeleteAlias, GetAlias, UpdateAlias, ListAliases, CreateEventSourceMapping, DeleteEventSourceMapping, GetEventSourceMapping, UpdateEventSourceMapping, ListEventSourceMappings, TagResource, UntagResource, ListTags, AddPermission, RemovePermission, GetPolicy, PutFunctionConcurrency, DeleteFunctionConcurrency, GetFunctionConcurrency, PutFunctionEventInvokeConfig, GetFunctionEventInvokeConfig, UpdateFunctionEventInvokeConfig, DeleteFunctionEventInvokeConfig, ListFunctionEventInvokeConfigs, GetAccountSettings, CreateFunctionUrlConfig, GetFunctionUrlConfig, UpdateFunctionUrlConfig, DeleteFunctionUrlConfig, ListFunctionUrlConfigs, PutRuntimeManagementConfig, GetRuntimeManagementConfig, ListLayers, ListLayerVersions, GetLayerVersion

</details>

<details>
<summary><b>EventBridge operations (48)</b></summary>

PutEvents, PutRule, DeleteRule, DescribeRule, EnableRule, DisableRule, ListRules, ListRuleNamesByTarget, PutTargets, RemoveTargets, ListTargetsByRule, TestEventPattern, CreateEventBus, DeleteEventBus, DescribeEventBus, ListEventBuses, UpdateEventBus, TagResource, UntagResource, ListTagsForResource, PutPartnerEvents, CreatePartnerEventSource, DeletePartnerEventSource, DescribePartnerEventSource, ListPartnerEventSources, ListPartnerEventSourceAccounts, ActivateEventSource, DeactivateEventSource, DescribeEventSource, ListEventSources, CreateArchive, DeleteArchive, DescribeArchive, UpdateArchive, ListArchives, StartReplay, CancelReplay, DescribeReplay, ListReplays, CreateConnection, DeleteConnection, DescribeConnection, UpdateConnection, ListConnections, CreateApiDestination, DeleteApiDestination, DescribeApiDestination, UpdateApiDestination

</details>

<details>
<summary><b>CloudWatch Logs operations (42)</b></summary>

CreateLogGroup, DeleteLogGroup, DescribeLogGroups, CreateLogStream, DeleteLogStream, DescribeLogStreams, PutLogEvents, GetLogEvents, FilterLogEvents, PutRetentionPolicy, DeleteRetentionPolicy, TagResource, UntagResource, ListTagsForResource, PutSubscriptionFilter, DeleteSubscriptionFilter, DescribeSubscriptionFilters, PutMetricFilter, DeleteMetricFilter, DescribeMetricFilters, PutDestination, DeleteDestination, DescribeDestinations, PutDestinationPolicy, DeleteDestinationPolicy, PutQueryDefinition, DeleteQueryDefinition, DescribeQueryDefinitions, StartQuery, StopQuery, GetQueryResults, DescribeQueries, CreateExportTask, DescribeExportTasks, PutResourcePolicy, DeleteResourcePolicy, DescribeResourcePolicies, AssociateKmsKey, DisassociateKmsKey, PutDataProtectionPolicy, GetDataProtectionPolicy, DeleteDataProtectionPolicy

</details>

<details>
<summary><b>KMS operations (39)</b></summary>

CreateKey, DescribeKey, ListKeys, EnableKey, DisableKey, ScheduleKeyDeletion, CancelKeyDeletion, CreateAlias, DeleteAlias, ListAliases, UpdateAlias, Encrypt, Decrypt, ReEncrypt, GenerateDataKey, GenerateDataKeyWithoutPlaintext, GenerateDataKeyPair, GenerateDataKeyPairWithoutPlaintext, GenerateRandom, Sign, Verify, GetPublicKey, CreateGrant, RetireGrant, RevokeGrant, ListGrants, ListRetirableGrants, EnableKeyRotation, DisableKeyRotation, GetKeyRotationStatus, GetKeyPolicy, PutKeyPolicy, ListKeyPolicies, TagResource, UntagResource, ListResourceTags, UpdateKeyDescription, ReplicateKey, UpdatePrimaryRegion

</details>

<details>
<summary><b>Kinesis operations (28)</b></summary>

CreateStream, DeleteStream, DescribeStream, DescribeStreamSummary, ListStreams, PutRecord, PutRecords, GetRecords, GetShardIterator, ListShards, MergeShards, SplitShard, AddTagsToStream, RemoveTagsFromStream, ListTagsForStream, IncreaseStreamRetentionPeriod, DecreaseStreamRetentionPeriod, UpdateShardCount, EnableEnhancedMonitoring, DisableEnhancedMonitoring, StartStreamEncryption, StopStreamEncryption, RegisterStreamConsumer, DeregisterStreamConsumer, DescribeStreamConsumer, ListStreamConsumers, UpdateStreamMode, DescribeLimits

</details>

<details>
<summary><b>Secrets Manager operations (22)</b></summary>

CreateSecret, GetSecretValue, PutSecretValue, UpdateSecret, DeleteSecret, DescribeSecret, ListSecrets, RestoreSecret, RotateSecret, CancelRotateSecret, GetRandomPassword, TagResource, UntagResource, ValidateResourcePolicy, ReplicateSecretToRegions, RemoveRegionsFromReplication, StopReplicationToReplica, PutResourcePolicy, GetResourcePolicy, DeleteResourcePolicy, ListSecretVersionIds, BatchGetSecretValue

</details>

<details>
<summary><b>SES operations (46)</b></summary>

SendEmail, SendRawEmail, SendTemplatedEmail, SendBulkTemplatedEmail, VerifyEmailIdentity, VerifyEmailAddress, DeleteVerifiedEmailAddress, VerifyDomainIdentity, VerifyDomainDkim, ListIdentities, GetIdentityVerificationAttributes, GetIdentityDkimAttributes, GetIdentityNotificationAttributes, SetIdentityNotificationTopic, SetIdentityFeedbackForwardingEnabled, SetIdentityHeadersInNotificationsEnabled, GetIdentityMailFromDomainAttributes, SetIdentityMailFromDomain, DeleteIdentity, ListVerifiedEmailAddresses, GetSendQuota, GetSendStatistics, GetAccountSendingEnabled, UpdateAccountSendingEnabled, CreateTemplate, UpdateTemplate, DeleteTemplate, GetTemplate, ListTemplates, TestRenderTemplate, CreateConfigurationSet, DeleteConfigurationSet, DescribeConfigurationSet, ListConfigurationSets, CreateConfigurationSetEventDestination, DeleteConfigurationSetEventDestination, UpdateConfigurationSetEventDestination, CreateReceiptRule, DeleteReceiptRule, DescribeReceiptRule, UpdateReceiptRule, CreateReceiptRuleSet, DeleteReceiptRuleSet, DescribeActiveReceiptRuleSet, ListReceiptRuleSets, SetActiveReceiptRuleSet

</details>

<details>
<summary><b>API Gateway V2 operations (55)</b></summary>

CreateApi, GetApi, GetApis, UpdateApi, DeleteApi, CreateRoute, GetRoute, GetRoutes, UpdateRoute, DeleteRoute, CreateIntegration, GetIntegration, GetIntegrations, UpdateIntegration, DeleteIntegration, CreateIntegrationResponse, GetIntegrationResponse, GetIntegrationResponses, UpdateIntegrationResponse, DeleteIntegrationResponse, CreateStage, GetStage, GetStages, UpdateStage, DeleteStage, CreateDeployment, GetDeployment, GetDeployments, UpdateDeployment, DeleteDeployment, CreateAuthorizer, GetAuthorizer, GetAuthorizers, UpdateAuthorizer, DeleteAuthorizer, CreateRouteResponse, GetRouteResponse, GetRouteResponses, UpdateRouteResponse, DeleteRouteResponse, CreateModel, GetModel, GetModels, UpdateModel, DeleteModel, CreateDomainName, GetDomainName, GetDomainNames, UpdateDomainName, DeleteDomainName, CreateApiMapping, GetApiMapping, GetApiMappings, UpdateApiMapping, DeleteApiMapping

</details>

<details>
<summary><b>CloudWatch Metrics operations (32)</b></summary>

PutMetricData, GetMetricData, GetMetricStatistics, ListMetrics, PutMetricAlarm, DescribeAlarms, DeleteAlarms, DescribeAlarmsForMetric, SetAlarmState, EnableAlarmActions, DisableAlarmActions, DescribeAlarmHistory, PutCompositeAlarm, PutDashboard, GetDashboard, ListDashboards, DeleteDashboards, TagResource, UntagResource, ListTagsForResource, PutAnomalyDetector, DeleteAnomalyDetector, DescribeAnomalyDetectors, PutInsightRule, DeleteInsightRules, DescribeInsightRules, EnableInsightRules, DisableInsightRules, GetInsightRuleReport, GetMetricWidgetImage, ListManagedInsightRules, PutManagedInsightRules

</details>

<details>
<summary><b>IAM operations (96)</b></summary>

CreateUser, GetUser, UpdateUser, DeleteUser, ListUsers, CreateGroup, GetGroup, UpdateGroup, DeleteGroup, ListGroups, AddUserToGroup, RemoveUserFromGroup, ListGroupsForUser, CreateRole, GetRole, UpdateRole, DeleteRole, ListRoles, UpdateRoleDescription, UpdateAssumeRolePolicy, CreatePolicy, GetPolicy, DeletePolicy, ListPolicies, CreatePolicyVersion, GetPolicyVersion, DeletePolicyVersion, ListPolicyVersions, SetDefaultPolicyVersion, AttachUserPolicy, DetachUserPolicy, ListAttachedUserPolicies, AttachGroupPolicy, DetachGroupPolicy, ListAttachedGroupPolicies, AttachRolePolicy, DetachRolePolicy, ListAttachedRolePolicies, PutUserPolicy, GetUserPolicy, DeleteUserPolicy, ListUserPolicies, PutGroupPolicy, GetGroupPolicy, DeleteGroupPolicy, ListGroupPolicies, PutRolePolicy, GetRolePolicy, DeleteRolePolicy, ListRolePolicies, CreateInstanceProfile, GetInstanceProfile, DeleteInstanceProfile, ListInstanceProfiles, AddRoleToInstanceProfile, RemoveRoleFromInstanceProfile, ListInstanceProfilesForRole, CreateAccessKey, UpdateAccessKey, DeleteAccessKey, ListAccessKeys, GetAccessKeyLastUsed, CreateLoginProfile, GetLoginProfile, UpdateLoginProfile, DeleteLoginProfile, ChangePassword, UpdateAccountPasswordPolicy, GetAccountPasswordPolicy, DeleteAccountPasswordPolicy, CreateServiceLinkedRole, DeleteServiceLinkedRole, GetServiceLinkedRoleDeletionStatus, TagUser, UntagUser, TagRole, UntagRole, TagPolicy, UntagPolicy, ListUserTags, ListRoleTags, ListPolicyTags, CreateOpenIDConnectProvider, GetOpenIDConnectProvider, DeleteOpenIDConnectProvider, ListOpenIDConnectProviders, AddClientIDToOpenIDConnectProvider, RemoveClientIDFromOpenIDConnectProvider, UpdateOpenIDConnectProviderThumbprint, CreateSAMLProvider, GetSAMLProvider, UpdateSAMLProvider, DeleteSAMLProvider, ListSAMLProviders, GetAccountSummary, GetAccountAuthorizationDetails, ListEntitiesForPolicy, GenerateCredentialReport

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

Available features: `s3`, `dynamodb`, `dynamodbstreams`, `sqs`, `ssm`, `sns`, `lambda`, `events`, `logs`, `kms`, `kinesis`, `secretsmanager`, `ses`, `apigatewayv2`, `cloudwatch`, `iam`, `sts`

## GitHub Action

```yaml
steps:
  - uses: actions/checkout@v4
  - uses: tyrchen/rustack@v0
```

The action starts the server, waits for healthy, and exports `AWS_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID`, and `AWS_SECRET_ACCESS_KEY`. All subsequent AWS CLI/SDK calls use Rustack automatically.

See [action.yml](action.yml) for all inputs and outputs.

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
