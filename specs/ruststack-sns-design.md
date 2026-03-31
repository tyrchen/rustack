# Rustack SNS: Native Rust Implementation Design

**Date:** 2026-03-06
**Status:** Draft / RFC
**Depends on:** [rustack-sqs-design.md](./rustack-sqs-design.md), [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-dynamodb-design.md](./rustack-dynamodb-design.md)
**Scope:** Add native SNS support to Rustack using the same Smithy-based codegen and gateway routing patterns established by S3, DynamoDB, SQS, and SSM. SNS is unique in that it requires cross-service integration with the existing SQS implementation for SNS-to-SQS fan-out.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Design](#5-protocol-design)
6. [Smithy Code Generation Strategy](#6-smithy-code-generation-strategy)
7. [Crate Structure](#7-crate-structure)
8. [HTTP Layer Design](#8-http-layer-design)
9. [Storage Engine Design](#9-storage-engine-design)
10. [Core Business Logic](#10-core-business-logic)
11. [Error Handling](#11-error-handling)
12. [Server Integration](#12-server-integration)
13. [Testing Strategy](#13-testing-strategy)
14. [Phased Implementation Plan](#14-phased-implementation-plan)
15. [Risk Analysis](#15-risk-analysis)
16. [Open Questions](#16-open-questions)

---

## 1. Executive Summary

This spec proposes adding SNS support to Rustack as a fully native Rust implementation, following the same architectural patterns established by S3, DynamoDB, SQS, and SSM. Key design decisions:

- **Native Rust pub/sub engine** -- unlike LocalStack which implements SNS in Python with complex provider classes, we build a purpose-built in-memory topic/subscription engine. This maintains the ~10MB Docker image and millisecond startup.
- **Smithy codegen reuse** -- extend the existing `codegen/` system to generate SNS model types from the official AWS SNS Smithy JSON AST (`aws/api-models-aws`), producing a `rustack-sns-model` crate.
- **awsQuery protocol** -- SNS uses the `@awsQuery` Smithy protocol (API version `2010-03-31`), with `application/x-www-form-urlencoded` request bodies and XML responses. This is the *only* protocol SNS supports -- unlike SQS, SNS has no JSON protocol alternative. Modern AWS SDKs still use awsQuery for SNS.
- **Cross-service SNS-to-SQS fan-out** -- the core differentiating feature of SNS for local development. When a message is published to an SNS topic, subscribed SQS queues receive the message. This requires a well-defined integration boundary between `rustack-sns-core` and `rustack-sqs-core`.
- **Message filtering** -- subscription filter policies allow fine-grained message routing based on message attributes or body content. This is a critical feature for microservice testing.
- **FIFO topics** -- paired with FIFO SQS queues, FIFO topics provide strict ordering and deduplication. Building on the existing FIFO SQS support.
- **Shared infrastructure** -- reuse `rustack-core` (multi-account/region state), `rustack-auth` (SigV4 verification), and the gateway routing pattern unchanged.
- **Phased delivery** -- 4 phases from MVP (topic CRUD, publish, SQS subscriptions) to full feature parity including FIFO topics, HTTP/HTTPS subscriptions, platform applications, and filter policies.

---

## 2. Motivation

### 2.1 Why SNS?

SNS is the fifth most-used AWS service for local development, and the essential complement to SQS. The SNS+SQS pattern (fan-out) is the standard event-driven architecture primitive in AWS:

- **Event-driven microservices** -- services publish domain events to SNS topics, and downstream services subscribe via SQS queues. Testing this pattern locally requires both services.
- **Fan-out pattern** -- a single SNS publish delivers to multiple SQS queues, Lambda functions, or HTTP endpoints. Without local SNS, developers cannot test fan-out logic.
- **Message filtering** -- SNS filter policies route messages to specific subscribers based on attributes, reducing consumer-side filtering. This is a common production pattern that must be testable locally.
- **FIFO ordering** -- SNS FIFO topics paired with SQS FIFO queues provide strictly ordered event delivery. Critical for financial and ordering systems.
- **CI/CD pipelines** -- fast, deterministic SNS+SQS in GitHub Actions for integration tests
- **Offline development** -- work without internet connectivity

### 2.2 Why Not Wrap an Existing Implementation?

| Implementation | Language | Image Size | awsQuery | Fan-out | Filter Policies | FIFO | Notes |
|---------------|----------|------------|----------|---------|-----------------|------|-------|
| GoAWS | Go | ~30MB | Yes | SQS only | Exact match only | No | Limited, SQS+SNS combined |
| s12v/sns | Scala/JVM | ~300MB | Partial | SQS, HTTP, RabbitMQ | No | No | Abandoned since 2019 |
| fake_sns | Ruby | ~200MB | Partial | SQS, HTTP | No | No | Minimal, Ruby-only test helper |
| LocalStack SNS | Python | ~1GB | Yes | Full | Full | Yes | Most complete, heavy |
| **Rustack SNS** | **Rust** | **~10MB** | **Yes** | **SQS, HTTP** | **Full** | **Yes** | **This proposal** |

All existing alternatives are either too incomplete (GoAWS lacks filter policies and FIFO), too heavy (s12v/sns requires JVM), abandoned (fake_sns), or part of a monolithic system (LocalStack). No Rust-based SNS emulator exists.

### 2.3 Why Native Rust?

A native Rust implementation provides:

- **~10MB Docker image** (same binary as S3/DynamoDB/SQS/SSM) vs ~300MB+ with JVM alternatives
- **Millisecond startup** vs 2-4 seconds for JVM
- **~5MB memory baseline** vs 80-150MB for JVM
- **Direct SQS integration** -- SNS-to-SQS fan-out calls directly into `rustack-sqs-core` without HTTP round-trips or inter-process communication
- **Tokio-native concurrency** -- async message delivery, HTTP endpoint callbacks, and timers integrate naturally with our async runtime
- **Single binary** -- no process management, no inter-process communication
- **Full debuggability** -- we own every line of code

### 2.4 Tool Coverage

With SNS implemented, the following common patterns work out of the box:

| Pattern / Tool | Operations Required | Phase Available |
|---------------|-------------------|-----------------|
| AWS CLI (`aws sns`) | All topic/subscription CRUD + publish | Phase 0 |
| SNS+SQS fan-out | CreateTopic, Subscribe (sqs), Publish | Phase 0 |
| Message filtering | SetSubscriptionAttributes (FilterPolicy) | Phase 1 |
| FIFO topic ordering | CreateTopic (.fifo), Publish with MessageGroupId | Phase 2 |
| Terraform | Topic/subscription CRUD + tags | Phase 1 |
| AWS CDK | Topic/subscription CRUD + tags | Phase 1 |
| Serverless Framework | SNS triggers, SQS subscriptions | Phase 0 |
| HTTP webhooks | Subscribe (http/https), Publish | Phase 2 |
| Platform push notifications | CreatePlatformApplication, Publish | Phase 3 (stub) |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Native Rust SNS emulator** -- no JVM, no external processes, no FFI
2. **Cover 90%+ of local development use cases** -- topic CRUD, subscriptions, publish, SQS fan-out, message filtering
3. **awsQuery protocol support** -- full `application/x-www-form-urlencoded` request parsing and XML response serialization (the only SNS protocol)
4. **Smithy-generated types** -- all SNS API types generated from official AWS Smithy model
5. **SNS-to-SQS fan-out** -- when a message is published to a topic, deliver to all subscribed SQS queues via direct in-process integration with `rustack-sqs-core`
6. **Message filtering** -- support filter policies on subscriptions with `MessageAttributes` and `MessageBody` scopes
7. **FIFO topics** -- strict ordering within message groups, exactly-once deduplication, paired with FIFO SQS queues
8. **Same Docker image** -- single binary serves S3, DynamoDB, SQS, SSM, and SNS on the same port (4566)
9. **GitHub Action compatibility** -- extend the existing `tyrchen/rustack` GitHub Action
10. **Pass LocalStack SNS test suite** -- validate against vendored `test_sns.py` (180 tests) and `test_sns_filter_policy.py` (22 tests)

### 3.2 Non-Goals

1. **Real SMS delivery** -- accept Publish to phone numbers, store messages for retrospection, do not send actual SMS
2. **Real email delivery** -- accept email/email-json subscriptions, do not send actual emails
3. **KMS encryption** -- accept `KmsMasterKeyId` attribute, do not perform actual encryption
4. **IAM policy enforcement** -- accept `AddPermission`/`RemovePermission` and `Policy` attribute, do not evaluate policies
5. **Lambda invocation** -- accept Lambda subscriptions, do not invoke actual Lambda functions (future: integrate with local Lambda runtime)
6. **Firehose delivery** -- accept Firehose subscriptions, do not deliver to actual Firehose streams
7. **CloudWatch metrics** -- no metrics emission
8. **Data protection policies** -- accept `PutDataProtectionPolicy`/`GetDataProtectionPolicy`, store policy JSON, do not enforce content scanning
9. **SMS sandbox management** -- accept SMS sandbox operations, return stub responses
10. **Cross-account access** -- all topics exist within a single account context
11. **Data persistence across restarts** -- in-memory only, matching all other Rustack services
12. **Message delivery retry with backoff** -- for HTTP/HTTPS endpoints, attempt delivery once with a short timeout; do not implement the full exponential backoff retry policy
13. **Subscription confirmation for SQS/Lambda** -- auto-confirm internal subscriptions (matching LocalStack behavior)

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                    AWS SDK / CLI / boto3
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  <-- Routes by X-Amz-Target, Content-Type, Action=
              |   (ServiceRouter)   |
              +--------+------------+
                       |
         +------+------+------+------+------+
         v      v             v      v      v
   +------+ +------+    +------+ +------+ +------+
   | S3   | | DDB  |    | SQS  | | SSM  | | SNS  |
   | HTTP | | HTTP |    | HTTP | | HTTP | | HTTP  |
   +------+ +------+    +------+ +------+ +------+
      |        |            |       |        |
   +------+ +------+    +------+ +------+ +------+
   | S3   | | DDB  |    | SQS  | | SSM  | | SNS  |
   | Core | | Core |    | Core | | Core | | Core  |
   +------+ +------+    +------+ +------+ +------+
      |        |            |       |        |
      +--------+------+----+-------+---------+
                       |            |
                +------+------+     |
                | rustack-  |     | SNS->SQS fan-out:
                | core + auth |     | SNS Core calls SQS
                +-------------+     | Core directly
                                    v
                           +----------------+
                           | SqsPublisher   |
                           | (trait in sns- |
                           |  core, impl    |
                           |  wraps SQS)    |
                           +----------------+
```

### 4.2 SNS-to-SQS Integration Architecture

The critical design challenge is the cross-service fan-out. When SNS publishes a message, it must deliver to subscribed SQS queues. We solve this with a **trait-based abstraction**:

```rust
/// Trait for delivering messages to SQS queues.
///
/// This abstraction decouples `rustack-sns-core` from `rustack-sqs-core`.
/// In production, the implementation wraps `RustackSqs::send_message()`.
/// In tests, a mock implementation captures delivered messages.
#[async_trait]
pub trait SqsPublisher: Send + Sync + 'static {
    /// Deliver a message to an SQS queue.
    async fn send_message(
        &self,
        queue_arn: &str,
        message_body: &str,
        message_attributes: &HashMap<String, MessageAttributeValue>,
        message_group_id: Option<&str>,
        message_deduplication_id: Option<&str>,
    ) -> Result<(), SnsDeliveryError>;
}
```

The server binary wires `RustackSqs` into the `SqsPublisher` implementation at startup:

```rust
/// Production SQS publisher that delegates to the SQS provider.
pub struct RustackSqsPublisher {
    sqs: Arc<RustackSqs>,
    config: SqsPublisherConfig,
}

#[async_trait]
impl SqsPublisher for RustackSqsPublisher {
    async fn send_message(
        &self,
        queue_arn: &str,
        message_body: &str,
        message_attributes: &HashMap<String, MessageAttributeValue>,
        message_group_id: Option<&str>,
        message_deduplication_id: Option<&str>,
    ) -> Result<(), SnsDeliveryError> {
        let queue_url = self.arn_to_queue_url(queue_arn);
        let input = SendMessageInput {
            queue_url,
            message_body: message_body.to_string(),
            message_group_id: message_group_id.map(String::from),
            message_deduplication_id: message_deduplication_id.map(String::from),
            // Convert SNS MessageAttributeValue -> SQS MessageAttributeValue
            message_attributes: convert_attributes(message_attributes),
            ..Default::default()
        };
        self.sqs.send_message(input).await.map_err(|e| {
            SnsDeliveryError::SqsDeliveryFailed {
                queue_arn: queue_arn.to_string(),
                source: e.to_string(),
            }
        })?;
        Ok(())
    }
}
```

### 4.3 Gateway Service Routing

SNS requests are distinguished from other services by their characteristics:

| Signal | S3 | DynamoDB | SQS (JSON) | SSM | SNS |
|--------|----|---------:|-----------|-----|-----|
| HTTP Method | GET/PUT/DELETE/POST | POST | POST | POST | POST |
| Content-Type | varies | `application/x-amz-json-1.0` | `application/x-amz-json-1.0` | `application/x-amz-json-1.1` | `application/x-www-form-urlencoded` |
| `X-Amz-Target` | absent | `DynamoDB_20120810.*` | `AmazonSQS.*` | `AmazonSSM.*` | absent |
| URL path | `/{bucket}/{key}` | `/` | `/` | `/` | `/` |
| Dispatch | N/A | N/A | N/A | N/A | `Action=CreateTopic` etc. |

**Routing logic** (evaluated in order):
1. If `X-Amz-Target` starts with `DynamoDB_` -- route to DynamoDB
2. If `X-Amz-Target` starts with `AmazonSQS` -- route to SQS (JSON protocol)
3. If `X-Amz-Target` starts with `AmazonSSM.` -- route to SSM
4. If `Content-Type` is `application/x-www-form-urlencoded` and `POST /`:
   - Parse `Action=` parameter from body
   - If Action is a recognized SNS operation -- route to SNS
   - If Action is a recognized SQS operation -- route to SQS (Query protocol)
   - Otherwise, fall through to S3
5. Default: route to S3 (catch-all)

**Important**: SNS and SQS both support `awsQuery` with `application/x-www-form-urlencoded`. The gateway must buffer the body for `POST /` with form-urlencoded Content-Type and inspect the `Action=` parameter to distinguish SNS from SQS. SNS operations have different names than SQS operations (e.g., `CreateTopic` vs `CreateQueue`, `Publish` vs `SendMessage`), so there is no ambiguity in Action names.

### 4.4 Crate Dependency Graph

```
rustack (app) <-- unified binary
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-sns-core       <-- NEW
+-- rustack-sns-http       <-- NEW
+-- rustack-sns-model      <-- NEW (auto-generated)

rustack-sns-http
+-- rustack-sns-model
+-- rustack-auth
+-- quick-xml (XML response serialization)
+-- serde_urlencoded (form request deserialization)

rustack-sns-core
+-- rustack-core
+-- rustack-sns-model
+-- dashmap
+-- tokio
+-- serde_json (for filter policy evaluation, message JSON wrapping)
+-- regex (for filter policy pattern matching)

rustack (wiring layer)
+-- rustack-sns-core
+-- rustack-sqs-core       <-- for SqsPublisher integration
```

Note: `rustack-sns-core` does NOT depend on `rustack-sqs-core`. The `SqsPublisher` trait is defined in `rustack-sns-core`, and the concrete implementation wrapping `RustackSqs` lives in the server binary (or a thin integration crate). This keeps the dependency graph clean and testable.

---

## 5. Protocol Design

### 5.1 awsQuery Protocol (The Only SNS Protocol)

Unlike SQS (which supports both awsJson1_0 and awsQuery) and DynamoDB/SSM (which use awsJson), SNS uses **only** the `@awsQuery` protocol with API version `2010-03-31`. All AWS SDKs send SNS requests as `application/x-www-form-urlencoded` and receive XML responses.

| Aspect | SNS (awsQuery) |
|--------|---------------|
| Content-Type (request) | `application/x-www-form-urlencoded` |
| Content-Type (response) | `text/xml` |
| HTTP Method | POST |
| URL Path | `/` |
| Operation dispatch | `Action=<OperationName>` form parameter |
| Request body | URL-encoded form fields |
| Response body | XML |
| Error body | XML `<ErrorResponse>` |
| API version | `2010-03-31` |
| XML namespace | `http://sns.amazonaws.com/doc/2010-03-31/` |

### 5.2 Request Format

```http
POST / HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Authorization: AWS4-HMAC-SHA256 ...

Action=Publish
&TopicArn=arn%3Aaws%3Asns%3Aus-east-1%3A000000000000%3Amy-topic
&Message=Hello%20World
&Version=2010-03-31
```

### 5.3 Success Response Format

```http
HTTP/1.1 200 OK
Content-Type: text/xml

<PublishResponse xmlns="http://sns.amazonaws.com/doc/2010-03-31/">
  <PublishResult>
    <MessageId>567910cd-659e-55d4-8ccb-5aaf14679dc0</MessageId>
  </PublishResult>
  <ResponseMetadata>
    <RequestId>d74b8436-ae13-5ab4-a9ff-ce54dfea72a0</RequestId>
  </ResponseMetadata>
</PublishResponse>
```

### 5.4 Error Response Format

```http
HTTP/1.1 404 Not Found
Content-Type: text/xml

<ErrorResponse xmlns="http://sns.amazonaws.com/doc/2010-03-31/">
  <Error>
    <Type>Sender</Type>
    <Code>NotFound</Code>
    <Message>Topic does not exist</Message>
  </Error>
  <RequestId>d74b8436-ae13-5ab4-a9ff-ce54dfea72a0</RequestId>
</ErrorResponse>
```

### 5.5 Form Parameter Encoding Conventions

SNS uses a specific convention for encoding complex parameters in form bodies:

**Flat parameters:**
```
Action=CreateTopic&Name=my-topic&Version=2010-03-31
```

**Map parameters (Attributes):**
```
Attributes.entry.1.key=DisplayName&Attributes.entry.1.value=My+Topic
&Attributes.entry.2.key=FifoTopic&Attributes.entry.2.value=true
```

**List parameters (Tags):**
```
Tags.member.1.Key=env&Tags.member.1.Value=prod
&Tags.member.2.Key=team&Tags.member.2.Value=platform
```

**Message attributes:**
```
MessageAttributes.entry.1.Name=type
&MessageAttributes.entry.1.Value.DataType=String
&MessageAttributes.entry.1.Value.StringValue=order.created
```

**Batch entries (PublishBatch):**
```
PublishBatchRequestEntries.member.1.Id=msg1
&PublishBatchRequestEntries.member.1.Message=Hello
&PublishBatchRequestEntries.member.2.Id=msg2
&PublishBatchRequestEntries.member.2.Message=World
```

### 5.6 Protocol Detection at Gateway

```rust
/// Check if a form-urlencoded body contains an SNS Action parameter.
fn is_sns_action(body: &[u8]) -> bool {
    // Parse enough of the body to find Action=
    let params = serde_urlencoded::from_bytes::<Vec<(String, String)>>(body)
        .unwrap_or_default();
    params.iter()
        .find(|(k, _)| k == "Action")
        .map(|(_, v)| SNS_ACTIONS.contains(&v.as_str()))
        .unwrap_or(false)
}

const SNS_ACTIONS: &[&str] = &[
    "AddPermission", "CheckIfPhoneNumberIsOptedOut", "ConfirmSubscription",
    "CreatePlatformApplication", "CreatePlatformEndpoint", "CreateSMSSandboxPhoneNumber",
    "CreateTopic", "DeleteEndpoint", "DeletePlatformApplication",
    "DeleteSMSSandboxPhoneNumber", "DeleteTopic", "GetDataProtectionPolicy",
    "GetEndpointAttributes", "GetPlatformApplicationAttributes", "GetSMSAttributes",
    "GetSMSSandboxAccountStatus", "GetSubscriptionAttributes", "GetTopicAttributes",
    "ListEndpointsByPlatformApplication", "ListOriginationNumbers",
    "ListPhoneNumbersOptedOut", "ListPlatformApplications",
    "ListSMSSandboxPhoneNumbers", "ListSubscriptions", "ListSubscriptionsByTopic",
    "ListTagsForResource", "ListTopics", "OptInPhoneNumber", "Publish",
    "PublishBatch", "PutDataProtectionPolicy", "RemovePermission",
    "SetEndpointAttributes", "SetPlatformApplicationAttributes", "SetSMSAttributes",
    "SetSubscriptionAttributes", "SetTopicAttributes", "Subscribe",
    "TagResource", "Unsubscribe", "UntagResource", "VerifySMSSandboxPhoneNumber",
];
```

---

## 6. Smithy Code Generation Strategy

### 6.1 Approach: Extend Existing Multi-Service Codegen

The codegen tool already supports S3, DynamoDB, SQS, and SSM. SNS uses the `@awsQuery` protocol, which is different from the `@awsJson1_0`/`@awsJson1_1` protocols used by the other services. However, the Smithy model defines the same type shapes (structures, enums, lists, maps) regardless of protocol. The codegen generates the model types; the HTTP layer handles protocol-specific serialization.

### 6.2 Changes to Codegen

```
codegen/
+-- src/
|   +-- main.rs
|   +-- codegen.rs            # ServiceConfig trait + shared codegen
|   +-- model.rs
|   +-- shapes.rs
+-- smithy-model/
|   +-- s3.json
|   +-- sns.json              <-- NEW: from aws/api-models-aws
```

### 6.3 SNS Service Configuration

```rust
pub struct SnsServiceConfig;

const SNS_OPERATIONS: &[&str] = &[
    // Topic management (7)
    "CreateTopic",
    "DeleteTopic",
    "GetTopicAttributes",
    "SetTopicAttributes",
    "ListTopics",
    "AddPermission",
    "RemovePermission",
    // Subscription management (7)
    "Subscribe",
    "Unsubscribe",
    "ConfirmSubscription",
    "GetSubscriptionAttributes",
    "SetSubscriptionAttributes",
    "ListSubscriptions",
    "ListSubscriptionsByTopic",
    // Publishing (2)
    "Publish",
    "PublishBatch",
    // Tagging (3)
    "TagResource",
    "UntagResource",
    "ListTagsForResource",
    // Platform applications (8)
    "CreatePlatformApplication",
    "DeletePlatformApplication",
    "GetPlatformApplicationAttributes",
    "SetPlatformApplicationAttributes",
    "ListPlatformApplications",
    "CreatePlatformEndpoint",
    "DeleteEndpoint",
    "GetEndpointAttributes",
    "SetEndpointAttributes",
    "ListEndpointsByPlatformApplication",
    // SMS (9)
    "CheckIfPhoneNumberIsOptedOut",
    "GetSMSAttributes",
    "SetSMSAttributes",
    "ListPhoneNumbersOptedOut",
    "OptInPhoneNumber",
    "GetSMSSandboxAccountStatus",
    "CreateSMSSandboxPhoneNumber",
    "DeleteSMSSandboxPhoneNumber",
    "VerifySMSSandboxPhoneNumber",
    "ListSMSSandboxPhoneNumbers",
    "ListOriginationNumbers",
    // Data protection (2)
    "GetDataProtectionPolicy",
    "PutDataProtectionPolicy",
];

impl ServiceConfig for SnsServiceConfig {
    fn namespace(&self) -> &str { "com.amazonaws.sns#" }
    fn service_name(&self) -> &str { "SNS" }
    fn target_operations(&self) -> &[&str] { &SNS_OPERATIONS }
    fn protocol(&self) -> Protocol { Protocol::AwsQuery }
    // ...
}
```

### 6.4 Key Differences from Other Services

| Aspect | DynamoDB/SSM | SQS | SNS |
|--------|-------------|-----|-----|
| Protocol | awsJson1.0/1.1 | awsJson1.0 + awsQuery | awsQuery only |
| Namespace | `com.amazonaws.dynamodb#` / `com.amazonaws.ssm#` | `com.amazonaws.sqs#` | `com.amazonaws.sns#` |
| Target prefix | `DynamoDB_20120810` / `AmazonSSM` | `AmazonSQS` | N/A (Action= param) |
| Operations | 66 / 13 | 23 | 42 |
| Request format | JSON | JSON + form-urlencoded | form-urlencoded only |
| Response format | JSON | JSON + XML | XML only |
| Special types | `AttributeValue` | None | None |
| Serde | `#[serde(rename_all = "PascalCase")]` | Same | Same |

SNS has no equivalent of DynamoDB's `AttributeValue`. All types are standard structs and enums. The codegen can handle them automatically.

### 6.5 Smithy Model Acquisition

The SNS Smithy model is available from:

1. **aws/api-models-aws**: `models/sns/service/2010-03-31/sns-2010-03-31.json`
2. **smithy-rs**: Bundled in the smithy-rs codegen

We download the SNS Smithy JSON AST and place it at `codegen/smithy-model/sns.json`.

### 6.6 Generated Types Example

```rust
/// SNS CreateTopicInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateTopicInput {
    pub name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_protection_policy: Option<String>,
}

/// SNS PublishInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublishInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_structure: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub message_attributes: HashMap<String, MessageAttributeValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_deduplication_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_group_id: Option<String>,
}

/// SNS MessageAttributeValue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MessageAttributeValue {
    pub data_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_value: Option<Vec<u8>>,
}
```

### 6.7 Makefile Integration

```makefile
codegen-sns:
    @cd codegen && cargo run -- --service sns
    @cargo +nightly fmt -p rustack-sns-model

codegen: codegen-s3 codegen-dynamodb codegen-sqs codegen-ssm codegen-sns
```

---

## 7. Crate Structure

### 7.1 `rustack-sns-model` (auto-generated)

```
crates/rustack-sns-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs                    # Module re-exports
    +-- types.rs                  # Auto-generated: Tag, MessageAttributeValue, etc.
    +-- operations.rs             # Auto-generated: SnsOperation enum (42 variants)
    +-- error.rs                  # Auto-generated: SnsError + error codes
    +-- input/
    |   +-- mod.rs
    |   +-- topic.rs              # CreateTopicInput, DeleteTopicInput, etc.
    |   +-- subscription.rs       # SubscribeInput, UnsubscribeInput, etc.
    |   +-- publish.rs            # PublishInput, PublishBatchInput
    |   +-- tags.rs               # TagResourceInput, UntagResourceInput, etc.
    |   +-- platform.rs           # CreatePlatformApplicationInput, etc.
    |   +-- sms.rs                # GetSMSAttributesInput, etc.
    |   +-- permissions.rs        # AddPermissionInput, RemovePermissionInput
    |   +-- data_protection.rs    # GetDataProtectionPolicyInput, etc.
    +-- output/
        +-- mod.rs
        +-- topic.rs
        +-- subscription.rs
        +-- publish.rs
        +-- tags.rs
        +-- platform.rs
        +-- sms.rs
        +-- permissions.rs
        +-- data_protection.rs
```

**Dependencies**: `serde`, `serde_json`

### 7.2 `rustack-sns-core`

```
crates/rustack-sns-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs                 # SnsConfig
    +-- provider.rs               # RustackSns (main provider, all operation handlers)
    +-- error.rs                  # SnsServiceError
    +-- publisher.rs              # SqsPublisher trait, HttpPublisher trait
    +-- state.rs                  # TopicStore (DashMap<topic_arn, TopicRecord>)
    +-- topic/
    |   +-- mod.rs
    |   +-- record.rs             # TopicRecord: attributes, subscriptions, tags
    |   +-- attributes.rs         # Topic attribute management and validation
    |   +-- arn.rs                # Topic ARN generation and parsing
    +-- subscription/
    |   +-- mod.rs
    |   +-- record.rs             # SubscriptionRecord: protocol, endpoint, attributes
    |   +-- confirmation.rs       # Subscription confirmation state machine
    |   +-- protocols.rs          # Protocol enum (sqs, http, https, email, sms, etc.)
    +-- delivery/
    |   +-- mod.rs
    |   +-- fanout.rs             # Fan-out: publish to all matching subscriptions
    |   +-- sqs.rs                # SQS delivery: JSON message wrapping
    |   +-- http.rs               # HTTP/HTTPS delivery: POST with SNS message format
    |   +-- filter.rs             # Filter policy evaluation engine
    |   +-- message.rs            # SNS message envelope (JSON wrapping for SQS/HTTP)
    +-- filter/
    |   +-- mod.rs
    |   +-- policy.rs             # FilterPolicy struct and parsing
    |   +-- evaluator.rs          # Filter evaluation: exact, prefix, suffix, numeric, exists, etc.
    |   +-- validation.rs         # Filter policy validation (complexity limits)
    +-- platform/
    |   +-- mod.rs
    |   +-- application.rs        # PlatformApplication stub
    |   +-- endpoint.rs           # PlatformEndpoint stub
    +-- sms/
    |   +-- mod.rs
    |   +-- stub.rs               # SMS operation stubs (store messages for retrospection)
    +-- ops/
        +-- mod.rs
        +-- topic.rs              # CreateTopic, DeleteTopic, GetTopicAttributes, SetTopicAttributes, ListTopics
        +-- subscription.rs       # Subscribe, Unsubscribe, ConfirmSubscription, Get/Set/List
        +-- publish.rs            # Publish, PublishBatch
        +-- tags.rs               # TagResource, UntagResource, ListTagsForResource
        +-- permissions.rs        # AddPermission, RemovePermission
        +-- platform.rs           # Platform application/endpoint operations
        +-- sms.rs                # SMS operations
        +-- data_protection.rs    # Data protection policy operations
```

**Dependencies**: `rustack-core`, `rustack-sns-model`, `dashmap`, `tokio`, `serde_json`, `regex`, `uuid`, `tracing`, `chrono`, `async-trait`

### 7.3 `rustack-sns-http`

```
crates/rustack-sns-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs                 # Action= dispatch -> SnsOperation
    +-- dispatch.rs               # SnsHandler trait + operation dispatch
    +-- service.rs                # Hyper Service impl for SNS
    +-- request.rs                # Form-urlencoded deserialization
    +-- response.rs               # XML response serialization
    +-- error.rs                  # XML error response formatting
    +-- query/
    |   +-- mod.rs
    |   +-- deserialize.rs        # awsQuery form params -> typed input structs
    |   +-- serialize.rs          # typed output structs -> XML response
    |   +-- params.rs             # Nested param parsing (Attributes.entry.1.key)
```

**Dependencies**: `rustack-sns-model`, `rustack-auth`, `hyper`, `serde_urlencoded`, `quick-xml`, `bytes`, `uuid`

### 7.4 Workspace Changes

```toml
# Root Cargo.toml
[workspace.dependencies]
# ... existing deps ...
rustack-sns-model = { path = "crates/rustack-sns-model" }
rustack-sns-http = { path = "crates/rustack-sns-http" }
rustack-sns-core = { path = "crates/rustack-sns-core" }

# Testing
aws-sdk-sns = "1.x"
```

---

## 8. HTTP Layer Design

### 8.1 SNS Router

SNS uses `Action=` parameter dispatch from form-urlencoded bodies:

```rust
/// SNS operation router.
///
/// Parses the `Action=<OperationName>` form parameter to determine the operation.
pub struct SnsRouter;

impl SnsRouter {
    /// Resolve a form-urlencoded body to an SNS operation.
    pub fn resolve(params: &[(String, String)]) -> Result<SnsOperation, SnsError> {
        let action = params
            .iter()
            .find(|(k, _)| k == "Action")
            .map(|(_, v)| v.as_str())
            .ok_or_else(SnsError::missing_action)?;

        SnsOperation::from_name(action)
            .ok_or_else(|| SnsError::invalid_action(action))
    }
}
```

### 8.2 Form-Urlencoded Request Deserialization

Each input type needs a custom deserializer that maps flat form parameters (with dot-notation for nesting) to the typed struct:

```rust
/// Deserialize an SNS request from URL-encoded form parameters.
pub trait FromSnsRequest: Sized {
    /// Deserialize from form parameters.
    fn from_params(params: &[(String, String)]) -> Result<Self, SnsError>;
}

/// Example: CreateTopicInput deserialization.
impl FromSnsRequest for CreateTopicInput {
    fn from_params(params: &[(String, String)]) -> Result<Self, SnsError> {
        let name = get_required_param(params, "Name")?;
        let attributes = parse_attributes_map(params, "Attributes")?;
        let tags = parse_tag_list(params, "Tags")?;
        let data_protection_policy = get_optional_param(params, "DataProtectionPolicy");

        Ok(Self {
            name,
            attributes,
            tags,
            data_protection_policy,
        })
    }
}
```

### 8.3 Nested Parameter Parsing Utilities

```rust
/// Parse an `Attributes.entry.N.key` / `Attributes.entry.N.value` map.
fn parse_attributes_map(
    params: &[(String, String)],
    prefix: &str,
) -> Result<HashMap<String, String>, SnsError> {
    let mut result = HashMap::new();
    for n in 1..=100 {
        let key_param = format!("{prefix}.entry.{n}.key");
        let value_param = format!("{prefix}.entry.{n}.value");
        match (find_param(params, &key_param), find_param(params, &value_param)) {
            (Some(k), Some(v)) => { result.insert(k.to_string(), v.to_string()); }
            (None, None) => break,
            _ => return Err(SnsError::invalid_parameter("Incomplete attribute entry")),
        }
    }
    Ok(result)
}

/// Parse a `Tags.member.N.Key` / `Tags.member.N.Value` list.
fn parse_tag_list(
    params: &[(String, String)],
    prefix: &str,
) -> Result<Vec<Tag>, SnsError> {
    let mut result = Vec::new();
    for n in 1..=50 {
        let key_param = format!("{prefix}.member.{n}.Key");
        let value_param = format!("{prefix}.member.{n}.Value");
        match (find_param(params, &key_param), find_param(params, &value_param)) {
            (Some(k), Some(v)) => {
                result.push(Tag { key: k.to_string(), value: v.to_string() });
            }
            (None, None) => break,
            _ => return Err(SnsError::invalid_parameter("Incomplete tag entry")),
        }
    }
    Ok(result)
}

/// Parse MessageAttributes from form params.
fn parse_message_attributes(
    params: &[(String, String)],
    prefix: &str,
) -> Result<HashMap<String, MessageAttributeValue>, SnsError> {
    let mut result = HashMap::new();
    for n in 1..=10 {
        let name_param = format!("{prefix}.entry.{n}.Name");
        let data_type_param = format!("{prefix}.entry.{n}.Value.DataType");
        let string_value_param = format!("{prefix}.entry.{n}.Value.StringValue");
        let binary_value_param = format!("{prefix}.entry.{n}.Value.BinaryValue");

        let name = match find_param(params, &name_param) {
            Some(n) => n.to_string(),
            None => break,
        };
        let data_type = get_required_param_named(params, &data_type_param)?;
        let string_value = find_param(params, &string_value_param).map(String::from);
        let binary_value = find_param(params, &binary_value_param)
            .and_then(|v| base64::decode(v).ok());

        result.insert(name, MessageAttributeValue {
            data_type,
            string_value,
            binary_value,
        });
    }
    Ok(result)
}
```

### 8.4 XML Response Serialization

```rust
/// Serialize an SNS response to XML.
pub trait IntoSnsXmlResponse: Sized {
    /// The XML element name for the response wrapper (e.g., "PublishResponse").
    fn response_element() -> &'static str;

    /// The XML element name for the result wrapper (e.g., "PublishResult").
    fn result_element() -> &'static str;

    /// Write the result fields as XML.
    fn write_xml(&self, writer: &mut XmlWriter) -> Result<(), SnsError>;

    /// Build the complete XML response.
    fn into_xml_response(self) -> Result<http::Response<Bytes>, SnsError> {
        let mut writer = XmlWriter::new();
        writer.start_element(Self::response_element());
        writer.attribute("xmlns", "http://sns.amazonaws.com/doc/2010-03-31/");

        writer.start_element(Self::result_element());
        self.write_xml(&mut writer)?;
        writer.end_element(); // result

        writer.start_element("ResponseMetadata");
        writer.write_element("RequestId", &uuid::Uuid::new_v4().to_string());
        writer.end_element(); // ResponseMetadata

        writer.end_element(); // response

        Ok(http::Response::builder()
            .status(200)
            .header("content-type", "text/xml")
            .body(Bytes::from(writer.into_string()))
            .expect("valid XML response"))
    }
}
```

### 8.5 SnsHandler Trait

```rust
/// The boundary between HTTP and business logic.
///
/// Protocol-agnostic: receives typed inputs and returns typed outputs.
/// The HTTP layer handles awsQuery deserialization and XML serialization.
pub trait SnsHandler: Send + Sync + 'static {
    fn handle_operation(
        &self,
        op: SnsOperation,
        params: Vec<(String, String)>,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<Bytes>, SnsError>> + Send>>;
}
```

### 8.6 Service Integration

```rust
/// Hyper Service implementation for SNS.
pub struct SnsHttpService<H> {
    handler: Arc<H>,
    config: SnsHttpConfig,
}

pub struct SnsHttpConfig {
    pub skip_signature_validation: bool,
    pub region: String,
    pub account_id: String,
    pub credential_provider: Option<Arc<dyn CredentialProvider>>,
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

The SNS storage engine manages topics, subscriptions, platform applications, and SMS state. Unlike the SQS engine (which is time-oriented with message lifecycles), the SNS engine is relationship-oriented: topics own subscriptions, and publishing triggers fan-out to all matching subscriptions.

### 9.2 Core Data Structures

```rust
/// A single SNS topic.
#[derive(Debug, Clone)]
pub struct TopicRecord {
    /// Topic ARN (globally unique identifier).
    pub arn: String,
    /// Topic name.
    pub name: String,
    /// Whether this is a FIFO topic (.fifo suffix).
    pub is_fifo: bool,
    /// Topic attributes.
    pub attributes: TopicAttributes,
    /// Subscriptions attached to this topic.
    pub subscriptions: Vec<SubscriptionRecord>,
    /// Tags.
    pub tags: HashMap<String, String>,
    /// Data protection policy JSON (stored, not enforced).
    pub data_protection_policy: Option<String>,
    /// Timestamps.
    pub created_at: u64,
    /// Monotonically increasing subscription counter for unique ARN generation.
    pub subscription_counter: u64,
}

/// Topic attributes.
#[derive(Debug, Clone)]
pub struct TopicAttributes {
    pub display_name: String,
    /// IAM policy JSON (stored, not enforced).
    pub policy: Option<String>,
    /// Delivery policy JSON for HTTP/S endpoints.
    pub delivery_policy: Option<String>,
    /// Effective delivery policy (inherits defaults if not set).
    pub effective_delivery_policy: Option<String>,
    /// KMS key for encryption (stored, not enforced).
    pub kms_master_key_id: Option<String>,
    /// Signature version for HTTP/S subscription messages (1 or 2).
    pub signature_version: String,
    /// FIFO-only: content-based deduplication.
    pub content_based_deduplication: bool,
    /// FIFO-only: throughput limit (not enforced).
    pub fifo_throughput_limit: Option<String>,
    /// Owner account ID.
    pub owner: String,
}

/// A subscription attached to a topic.
#[derive(Debug, Clone)]
pub struct SubscriptionRecord {
    /// Subscription ARN.
    pub arn: String,
    /// Topic ARN this subscription belongs to.
    pub topic_arn: String,
    /// Protocol: sqs, http, https, email, email-json, sms, application, lambda, firehose.
    pub protocol: SubscriptionProtocol,
    /// Endpoint: queue ARN, URL, email address, phone number, etc.
    pub endpoint: String,
    /// Owner account ID.
    pub owner: String,
    /// Whether the subscription is confirmed.
    pub confirmed: bool,
    /// Confirmation token (for HTTP/HTTPS endpoints).
    pub confirmation_token: Option<String>,
    /// Subscription attributes.
    pub attributes: SubscriptionAttributes,
}

/// Subscription attributes.
#[derive(Debug, Clone, Default)]
pub struct SubscriptionAttributes {
    /// Filter policy JSON.
    pub filter_policy: Option<String>,
    /// Filter policy scope: "MessageAttributes" (default) or "MessageBody".
    pub filter_policy_scope: FilterPolicyScope,
    /// Raw message delivery (skips JSON wrapping for SQS/HTTP).
    pub raw_message_delivery: bool,
    /// Redrive policy for undeliverable messages (DLQ).
    pub redrive_policy: Option<String>,
    /// Delivery policy override for HTTP/S.
    pub delivery_policy: Option<String>,
    /// Subscription role ARN (for Firehose).
    pub subscription_role_arn: Option<String>,
}

/// Subscription protocol enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionProtocol {
    Sqs,
    Http,
    Https,
    Email,
    EmailJson,
    Sms,
    Application,
    Lambda,
    Firehose,
}

/// Filter policy scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FilterPolicyScope {
    #[default]
    MessageAttributes,
    MessageBody,
}
```

### 9.3 Topic Store

```rust
/// Global topic store. Thread-safe via DashMap.
#[derive(Debug)]
pub struct TopicStore {
    /// topic_arn -> TopicRecord
    topics: DashMap<String, TopicRecord>,
    /// subscription_arn -> topic_arn (reverse index for fast subscription lookup)
    subscription_index: DashMap<String, String>,
}

impl TopicStore {
    pub fn new() -> Self {
        Self {
            topics: DashMap::new(),
            subscription_index: DashMap::new(),
        }
    }

    /// Look up a topic by ARN.
    pub fn get_topic(&self, arn: &str) -> Option<dashmap::mapref::one::Ref<'_, String, TopicRecord>> {
        self.topics.get(arn)
    }

    /// Look up a topic by ARN, mutable.
    pub fn get_topic_mut(&self, arn: &str) -> Option<dashmap::mapref::one::RefMut<'_, String, TopicRecord>> {
        self.topics.get_mut(arn)
    }

    /// Find the topic ARN for a subscription ARN.
    pub fn find_topic_for_subscription(&self, sub_arn: &str) -> Option<String> {
        self.subscription_index.get(sub_arn).map(|r| r.clone())
    }

    /// Insert a topic.
    pub fn insert_topic(&self, topic: TopicRecord) {
        self.topics.insert(topic.arn.clone(), topic);
    }

    /// Remove a topic and all its subscriptions.
    pub fn remove_topic(&self, arn: &str) -> Option<TopicRecord> {
        if let Some((_, topic)) = self.topics.remove(arn) {
            for sub in &topic.subscriptions {
                self.subscription_index.remove(&sub.arn);
            }
            Some(topic)
        } else {
            None
        }
    }

    /// Add a subscription to a topic.
    pub fn add_subscription(&self, topic_arn: &str, sub: SubscriptionRecord) -> Result<(), SnsServiceError> {
        let mut topic = self.topics.get_mut(topic_arn)
            .ok_or_else(|| SnsServiceError::TopicNotFound { arn: topic_arn.to_string() })?;
        self.subscription_index.insert(sub.arn.clone(), topic_arn.to_string());
        topic.subscriptions.push(sub);
        Ok(())
    }

    /// Remove a subscription by ARN.
    pub fn remove_subscription(&self, sub_arn: &str) -> Result<(), SnsServiceError> {
        let topic_arn = self.subscription_index.remove(sub_arn)
            .map(|(_, v)| v)
            .ok_or_else(|| SnsServiceError::SubscriptionNotFound { arn: sub_arn.to_string() })?;
        if let Some(mut topic) = self.topics.get_mut(&topic_arn) {
            topic.subscriptions.retain(|s| s.arn != sub_arn);
        }
        Ok(())
    }
}
```

### 9.4 Platform Application Store

```rust
/// Platform application storage (stub implementation).
#[derive(Debug)]
pub struct PlatformStore {
    /// platform_application_arn -> PlatformApplicationRecord
    applications: DashMap<String, PlatformApplicationRecord>,
    /// endpoint_arn -> PlatformEndpointRecord
    endpoints: DashMap<String, PlatformEndpointRecord>,
}

#[derive(Debug, Clone)]
pub struct PlatformApplicationRecord {
    pub arn: String,
    pub name: String,
    pub platform: String,
    pub attributes: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PlatformEndpointRecord {
    pub arn: String,
    pub application_arn: String,
    pub token: String,
    pub custom_user_data: Option<String>,
    pub attributes: HashMap<String, String>,
    pub enabled: bool,
}
```

### 9.5 SMS State Store

```rust
/// SMS state storage (stub implementation for retrospection).
#[derive(Debug)]
pub struct SmsStore {
    /// Global SMS attributes.
    pub attributes: parking_lot::RwLock<HashMap<String, String>>,
    /// Sent SMS messages (for retrospection endpoint).
    pub sent_messages: parking_lot::RwLock<Vec<SentSmsMessage>>,
    /// Opted-out phone numbers.
    pub opted_out: parking_lot::RwLock<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct SentSmsMessage {
    pub phone_number: String,
    pub message: String,
    pub message_id: String,
    pub timestamp: u64,
}
```

### 9.6 SNS Message Envelope

When SNS delivers a message to SQS or HTTP endpoints, it wraps the original message in a JSON envelope (unless `RawMessageDelivery` is enabled):

```rust
/// SNS message envelope for SQS and HTTP delivery.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SnsMessageEnvelope {
    #[serde(rename = "Type")]
    pub message_type: String,  // "Notification"
    pub message_id: String,
    pub topic_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub message: String,
    pub timestamp: String,  // ISO 8601
    pub signature_version: String,
    pub signature: String,  // Stub: base64-encoded placeholder
    pub signing_cert_url: String,
    pub unsubscribe_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_attributes: Option<HashMap<String, SnsMessageAttributeEnvelope>>,
}

/// Message attribute in the envelope format.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SnsMessageAttributeEnvelope {
    #[serde(rename = "Type")]
    pub data_type: String,
    #[serde(rename = "Value")]
    pub value: String,
}
```

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main SNS provider. Owns all topic, subscription, and platform state.
#[derive(Debug)]
pub struct RustackSns {
    /// Topic and subscription storage.
    topics: TopicStore,
    /// Platform application/endpoint storage.
    platforms: PlatformStore,
    /// SMS state storage.
    sms: SmsStore,
    /// SQS publisher for fan-out delivery.
    sqs_publisher: Arc<dyn SqsPublisher>,
    /// HTTP client for HTTP/HTTPS endpoint delivery.
    http_publisher: Arc<dyn HttpPublisher>,
    /// Configuration.
    config: Arc<SnsConfig>,
}
```

### 10.2 Operations Grouped by Category

#### Topic Management (5 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreateTopic` | 0 | Medium | Validate name, attributes, idempotent create, FIFO detection |
| `DeleteTopic` | 0 | Low | Remove topic and all subscriptions, idempotent (no error on missing) |
| `GetTopicAttributes` | 0 | Low | Return all topic attributes |
| `SetTopicAttributes` | 0 | Low | Validate and update single attribute |
| `ListTopics` | 0 | Low | Paginate with NextToken |

#### Subscription Management (7 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `Subscribe` | 0 | Medium | Validate protocol/endpoint, auto-confirm SQS/Lambda, require confirmation for HTTP |
| `Unsubscribe` | 0 | Low | Remove subscription, idempotent |
| `ConfirmSubscription` | 1 | Medium | Validate token, transition subscription to confirmed |
| `GetSubscriptionAttributes` | 0 | Low | Return all subscription attributes |
| `SetSubscriptionAttributes` | 0 | Medium | Validate FilterPolicy, RawMessageDelivery, RedrivePolicy |
| `ListSubscriptions` | 0 | Low | Paginate all subscriptions across all topics |
| `ListSubscriptionsByTopic` | 0 | Low | Paginate subscriptions for a specific topic |

#### Publishing (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `Publish` | 0 | High | Fan-out to all matching subscriptions, filter evaluation, message wrapping |
| `PublishBatch` | 1 | High | Up to 10 messages, per-entry success/failure, fan-out each |

#### Permissions (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `AddPermission` | 2 | Low | Store in Policy attribute, no enforcement |
| `RemovePermission` | 2 | Low | Remove from Policy attribute |

#### Tagging (3 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `TagResource` | 1 | Low | Add/update tags (max 50), validate no duplicate keys |
| `UntagResource` | 1 | Low | Remove specified tag keys |
| `ListTagsForResource` | 1 | Low | Return all tags |

#### Platform Applications (10 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreatePlatformApplication` | 3 | Low | Store attributes, validate platform name |
| `DeletePlatformApplication` | 3 | Low | Remove application and all endpoints |
| `GetPlatformApplicationAttributes` | 3 | Low | Return attributes |
| `SetPlatformApplicationAttributes` | 3 | Low | Update attributes |
| `ListPlatformApplications` | 3 | Low | Paginate |
| `CreatePlatformEndpoint` | 3 | Low | Create endpoint with token |
| `DeleteEndpoint` | 3 | Low | Remove endpoint |
| `GetEndpointAttributes` | 3 | Low | Return attributes |
| `SetEndpointAttributes` | 3 | Low | Update attributes |
| `ListEndpointsByPlatformApplication` | 3 | Low | Paginate |

#### SMS (11 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CheckIfPhoneNumberIsOptedOut` | 3 | Low | Check opted-out list |
| `GetSMSAttributes` | 3 | Low | Return SMS attributes |
| `SetSMSAttributes` | 3 | Low | Update SMS attributes |
| `ListPhoneNumbersOptedOut` | 3 | Low | Return opted-out list |
| `OptInPhoneNumber` | 3 | Low | Remove from opted-out list |
| `GetSMSSandboxAccountStatus` | 3 | Low | Return stub status |
| `CreateSMSSandboxPhoneNumber` | 3 | Low | Stub |
| `DeleteSMSSandboxPhoneNumber` | 3 | Low | Stub |
| `VerifySMSSandboxPhoneNumber` | 3 | Low | Stub |
| `ListSMSSandboxPhoneNumbers` | 3 | Low | Stub |
| `ListOriginationNumbers` | 3 | Low | Stub |

#### Data Protection (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `GetDataProtectionPolicy` | 2 | Low | Return stored policy JSON |
| `PutDataProtectionPolicy` | 2 | Low | Store policy JSON, no enforcement |

### 10.3 CreateTopic Logic

```rust
impl RustackSns {
    pub fn create_topic(
        &self,
        input: CreateTopicInput,
    ) -> Result<CreateTopicOutput, SnsServiceError> {
        let name = &input.name;
        let is_fifo = name.ends_with(".fifo");

        // Validate topic name: 1-256 chars, alphanumeric + hyphens + underscores.
        validate_topic_name(name)?;

        // Validate FIFO-specific attributes.
        if is_fifo {
            validate_fifo_topic_attributes(&input.attributes)?;
        }

        let arn = topic_arn(
            &self.config.region,
            &self.config.account_id,
            name,
        );

        // Idempotent create: if topic exists with same name, return existing ARN.
        // If topic exists but tags differ, return error (AWS behavior).
        if let Some(existing) = self.topics.get_topic(&arn) {
            if !input.tags.is_empty() {
                // AWS returns error if different tags are provided on duplicate create
                let existing_tags: Vec<_> = existing.tags.iter()
                    .map(|(k, v)| Tag { key: k.clone(), value: v.clone() })
                    .collect();
                if tags_differ(&existing_tags, &input.tags) {
                    return Err(SnsServiceError::InvalidParameter {
                        message: "Invalid parameter: Tags Reason: can't add tags \
                                  to a topic with different tags".to_string(),
                    });
                }
            }
            return Ok(CreateTopicOutput { topic_arn: arn });
        }

        // Build topic attributes with defaults.
        let attributes = TopicAttributes::from_input(
            &input.attributes,
            is_fifo,
            &self.config.account_id,
        )?;

        let tags: HashMap<String, String> = input
            .tags
            .into_iter()
            .map(|t| (t.key, t.value))
            .collect();

        let topic = TopicRecord {
            arn: arn.clone(),
            name: name.clone(),
            is_fifo,
            attributes,
            subscriptions: Vec::new(),
            tags,
            data_protection_policy: input.data_protection_policy,
            created_at: now_epoch_seconds(),
            subscription_counter: 0,
        };

        self.topics.insert_topic(topic);
        Ok(CreateTopicOutput { topic_arn: arn })
    }
}
```

### 10.4 Subscribe Logic

```rust
impl RustackSns {
    pub fn subscribe(
        &self,
        input: SubscribeInput,
    ) -> Result<SubscribeOutput, SnsServiceError> {
        let topic_arn = &input.topic_arn;
        let protocol = SubscriptionProtocol::from_str(&input.protocol)?;
        let endpoint = input.endpoint.as_deref()
            .ok_or_else(|| SnsServiceError::InvalidParameter {
                message: "Endpoint is required".to_string(),
            })?;

        // Validate that the topic exists.
        let mut topic = self.topics.get_topic_mut(topic_arn)
            .ok_or_else(|| SnsServiceError::TopicNotFound {
                arn: topic_arn.clone(),
            })?;

        // Validate protocol-endpoint compatibility.
        validate_endpoint_for_protocol(&protocol, endpoint)?;

        // Check for duplicate subscription (same protocol + endpoint = idempotent).
        if let Some(existing) = topic.subscriptions.iter()
            .find(|s| s.protocol == protocol && s.endpoint == endpoint)
        {
            return Ok(SubscribeOutput {
                subscription_arn: Some(existing.arn.clone()),
            });
        }

        // Generate subscription ARN.
        topic.subscription_counter += 1;
        let sub_id = uuid::Uuid::new_v4();
        let sub_arn = format!("{}:{}", topic_arn, sub_id);

        // Auto-confirm for internal protocols (SQS, Lambda, Firehose).
        // HTTP/HTTPS require explicit confirmation.
        let auto_confirm = matches!(
            protocol,
            SubscriptionProtocol::Sqs
                | SubscriptionProtocol::Lambda
                | SubscriptionProtocol::Firehose
                | SubscriptionProtocol::Email
                | SubscriptionProtocol::EmailJson
                | SubscriptionProtocol::Sms
                | SubscriptionProtocol::Application
        );

        let confirmation_token = if !auto_confirm {
            Some(uuid::Uuid::new_v4().to_string())
        } else {
            None
        };

        // Parse subscription attributes from input.
        let sub_attributes = SubscriptionAttributes::from_input(&input.attributes)?;

        let subscription = SubscriptionRecord {
            arn: sub_arn.clone(),
            topic_arn: topic_arn.clone(),
            protocol,
            endpoint: endpoint.to_string(),
            owner: self.config.account_id.clone(),
            confirmed: auto_confirm,
            confirmation_token,
            attributes: sub_attributes,
        };

        self.topics.subscription_index.insert(sub_arn.clone(), topic_arn.clone());
        topic.subscriptions.push(subscription);

        // For HTTP/HTTPS endpoints, send subscription confirmation request.
        // (Phase 2: async POST to endpoint with SubscriptionConfirmation message)

        let response_arn = if auto_confirm {
            Some(sub_arn)
        } else {
            // "pending confirmation" is returned as the ARN
            Some("PendingConfirmation".to_string())
        };

        Ok(SubscribeOutput {
            subscription_arn: response_arn,
        })
    }
}
```

### 10.5 Publish Logic (Fan-Out)

```rust
impl RustackSns {
    pub async fn publish(
        &self,
        input: PublishInput,
    ) -> Result<PublishOutput, SnsServiceError> {
        // Resolve the target topic.
        let topic_arn = input.topic_arn.as_deref()
            .or(input.target_arn.as_deref())
            .ok_or_else(|| SnsServiceError::InvalidParameter {
                message: "One of TopicArn or TargetArn must be specified".to_string(),
            })?;

        // Validate message size (256 KiB).
        validate_message_size(&input.message)?;

        // If MessageStructure is "json", validate that default key exists.
        if input.message_structure.as_deref() == Some("json") {
            validate_json_message_structure(&input.message)?;
        }

        let topic = self.topics.get_topic(topic_arn)
            .ok_or_else(|| SnsServiceError::TopicNotFound {
                arn: topic_arn.to_string(),
            })?;

        // FIFO topic validation.
        if topic.is_fifo {
            if input.message_group_id.is_none() {
                return Err(SnsServiceError::InvalidParameter {
                    message: "MessageGroupId is required for FIFO topics".to_string(),
                });
            }
            // Deduplication: check if MessageDeduplicationId or content-based.
            if input.message_deduplication_id.is_none()
                && !topic.attributes.content_based_deduplication
            {
                return Err(SnsServiceError::InvalidParameter {
                    message: "MessageDeduplicationId is required when \
                              ContentBasedDeduplication is not enabled".to_string(),
                });
            }
        }

        let message_id = uuid::Uuid::new_v4().to_string();

        // Fan-out to all confirmed subscriptions.
        let subscriptions: Vec<SubscriptionRecord> = topic.subscriptions
            .iter()
            .filter(|s| s.confirmed)
            .cloned()
            .collect();

        // Drop the topic lock before async delivery.
        drop(topic);

        for sub in &subscriptions {
            // Evaluate filter policy.
            if let Some(ref filter_json) = sub.attributes.filter_policy {
                let matches = evaluate_filter_policy(
                    filter_json,
                    &sub.attributes.filter_policy_scope,
                    &input.message_attributes,
                    &input.message,
                )?;
                if !matches {
                    continue;
                }
            }

            // Resolve the message for this protocol.
            let resolved_message = if input.message_structure.as_deref() == Some("json") {
                resolve_json_message(&input.message, &sub.protocol)?
            } else {
                input.message.clone()
            };

            // Deliver based on protocol.
            match sub.protocol {
                SubscriptionProtocol::Sqs => {
                    let envelope = if sub.attributes.raw_message_delivery {
                        resolved_message
                    } else {
                        build_sns_envelope(
                            &message_id,
                            topic_arn,
                            input.subject.as_deref(),
                            &resolved_message,
                            &input.message_attributes,
                        )
                    };
                    self.sqs_publisher.send_message(
                        &sub.endpoint,
                        &envelope,
                        &if sub.attributes.raw_message_delivery {
                            convert_sns_to_sqs_attributes(&input.message_attributes)
                        } else {
                            HashMap::new()
                        },
                        input.message_group_id.as_deref(),
                        input.message_deduplication_id.as_deref(),
                    ).await.ok(); // Log but don't fail on delivery errors
                }
                SubscriptionProtocol::Http | SubscriptionProtocol::Https => {
                    let envelope = build_sns_envelope(
                        &message_id,
                        topic_arn,
                        input.subject.as_deref(),
                        &resolved_message,
                        &input.message_attributes,
                    );
                    self.http_publisher.post(&sub.endpoint, &envelope).await.ok();
                }
                SubscriptionProtocol::Sms => {
                    // Store for retrospection.
                    self.sms.record_sent_message(
                        &sub.endpoint,
                        &resolved_message,
                        &message_id,
                    );
                }
                _ => {
                    // Email, Lambda, Firehose, Application: log and skip.
                    tracing::debug!(
                        subscription_arn = %sub.arn,
                        protocol = ?sub.protocol,
                        "skipping delivery for unsupported protocol"
                    );
                }
            }
        }

        Ok(PublishOutput {
            message_id: Some(message_id),
            sequence_number: None, // Set for FIFO topics
        })
    }
}
```

### 10.6 Filter Policy Evaluation

```rust
/// Evaluate a subscription filter policy against message attributes or body.
///
/// Returns `true` if the message matches the filter policy.
pub fn evaluate_filter_policy(
    filter_json: &str,
    scope: &FilterPolicyScope,
    message_attributes: &HashMap<String, MessageAttributeValue>,
    message_body: &str,
) -> Result<bool, SnsServiceError> {
    let policy: serde_json::Value = serde_json::from_str(filter_json)
        .map_err(|e| SnsServiceError::InvalidParameter {
            message: format!("Invalid filter policy: {e}"),
        })?;

    match scope {
        FilterPolicyScope::MessageAttributes => {
            evaluate_policy_against_attributes(&policy, message_attributes)
        }
        FilterPolicyScope::MessageBody => {
            // Parse message body as JSON and evaluate policy against it.
            let body: serde_json::Value = serde_json::from_str(message_body)
                .unwrap_or(serde_json::Value::Null);
            evaluate_policy_against_json(&policy, &body)
        }
    }
}

/// Evaluate a filter policy against message attributes.
///
/// The policy is a JSON object where each key is an attribute name
/// and each value is an array of acceptable values (with operators).
///
/// Supported operators:
/// - Exact match: `["value1", "value2"]`
/// - Prefix: `[{"prefix": "order."}]`
/// - Suffix: `[{"suffix": ".created"}]`
/// - Numeric: `[{"numeric": [">=", 100, "<=", 200]}]`
/// - Exists: `[{"exists": true}]` or `[{"exists": false}]`
/// - Anything-but: `[{"anything-but": "value"}]`
/// - IP address: `[{"cidr": "10.0.0.0/8"}]`
fn evaluate_policy_against_attributes(
    policy: &serde_json::Value,
    attributes: &HashMap<String, MessageAttributeValue>,
) -> Result<bool, SnsServiceError> {
    let policy_obj = policy.as_object()
        .ok_or_else(|| SnsServiceError::InvalidParameter {
            message: "Filter policy must be a JSON object".to_string(),
        })?;

    // All keys in the policy must match (AND logic).
    for (key, conditions) in policy_obj {
        let attr_value = attributes.get(key);
        let conditions = conditions.as_array()
            .ok_or_else(|| SnsServiceError::InvalidParameter {
                message: format!("Filter conditions for '{key}' must be an array"),
            })?;

        // At least one condition must match (OR logic).
        let any_match = conditions.iter().any(|condition| {
            evaluate_single_condition(condition, attr_value)
        });

        if !any_match {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Evaluate a single filter condition against an attribute value.
fn evaluate_single_condition(
    condition: &serde_json::Value,
    attr_value: Option<&MessageAttributeValue>,
) -> bool {
    match condition {
        // Exact string match.
        serde_json::Value::String(expected) => {
            attr_value
                .and_then(|v| v.string_value.as_deref())
                .is_some_and(|v| v == expected.as_str())
        }
        // Exact numeric match.
        serde_json::Value::Number(expected) => {
            attr_value
                .and_then(|v| v.string_value.as_deref())
                .and_then(|v| v.parse::<f64>().ok())
                .is_some_and(|v| {
                    expected.as_f64().is_some_and(|e| (v - e).abs() < f64::EPSILON)
                })
        }
        // Boolean exact match.
        serde_json::Value::Bool(expected) => {
            attr_value
                .and_then(|v| v.string_value.as_deref())
                .and_then(|v| v.parse::<bool>().ok())
                .is_some_and(|v| v == *expected)
        }
        // Operator object: prefix, suffix, numeric, exists, anything-but, cidr.
        serde_json::Value::Object(obj) => {
            if let Some(prefix) = obj.get("prefix").and_then(|v| v.as_str()) {
                return attr_value
                    .and_then(|v| v.string_value.as_deref())
                    .is_some_and(|v| v.starts_with(prefix));
            }
            if let Some(suffix) = obj.get("suffix").and_then(|v| v.as_str()) {
                return attr_value
                    .and_then(|v| v.string_value.as_deref())
                    .is_some_and(|v| v.ends_with(suffix));
            }
            if let Some(exists) = obj.get("exists").and_then(|v| v.as_bool()) {
                return exists == attr_value.is_some();
            }
            if let Some(numeric_arr) = obj.get("numeric").and_then(|v| v.as_array()) {
                return evaluate_numeric_condition(numeric_arr, attr_value);
            }
            if let Some(anything_but) = obj.get("anything-but") {
                return evaluate_anything_but(anything_but, attr_value);
            }
            false
        }
        _ => false,
    }
}
```

### 10.7 Topic ARN and Subscription ARN Format

```rust
/// Topic ARN format: arn:aws:sns:<region>:<account-id>:<topic-name>
fn topic_arn(region: &str, account_id: &str, topic_name: &str) -> String {
    format!("arn:aws:sns:{region}:{account_id}:{topic_name}")
}

/// Subscription ARN format: arn:aws:sns:<region>:<account-id>:<topic-name>:<uuid>
/// (The topic ARN + a UUID suffix)
fn subscription_arn(topic_arn: &str) -> String {
    format!("{topic_arn}:{}", uuid::Uuid::new_v4())
}
```

---

## 11. Error Handling

### 11.1 SNS Error Codes

SNS uses simple error codes without the `AWS.SimpleNotificationService.` prefix used by SQS.

```rust
/// SNS error codes.
pub enum SnsErrorCode {
    /// Topic does not exist.
    NotFound,
    /// Topic already exists (with different attributes).
    TopicAlreadyExists,
    /// Invalid parameter value.
    InvalidParameter,
    /// Invalid parameter value (more specific).
    InvalidParameterValue,
    /// Authorization error.
    AuthorizationError,
    /// Internal error.
    InternalError,
    /// Throttled.
    Throttled,
    /// Subscription limit exceeded.
    SubscriptionLimitExceeded,
    /// Topic limit exceeded.
    TopicLimitExceeded,
    /// Filter policy limit exceeded.
    FilterPolicyLimitExceeded,
    /// Invalid security (authentication failure).
    InvalidSecurity,
    /// Endpoint disabled.
    EndpointDisabled,
    /// Platform application disabled.
    PlatformApplicationDisabled,
    /// KMS error.
    KMSDisabled,
    /// Tag policy violation.
    TagPolicy,
    /// Stale tag.
    StaleTag,
    /// Tag limit exceeded.
    TagLimitExceeded,
    /// Concurrent access.
    ConcurrentAccess,
    /// Validation exception.
    ValidationException,
    /// Invalid batch entry ID.
    BatchEntryIdsNotDistinct,
    /// Batch request too long.
    BatchRequestTooLong,
    /// Empty batch request.
    EmptyBatchRequest,
    /// Too many entries in batch.
    TooManyEntriesInBatchRequest,
}
```

### 11.2 Error Type Mapping

```rust
impl SnsErrorCode {
    /// The error code string for XML responses.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "NotFound",
            Self::InvalidParameter => "InvalidParameter",
            Self::InvalidParameterValue => "InvalidParameterValue",
            Self::AuthorizationError => "AuthorizationError",
            Self::InternalError => "InternalError",
            Self::Throttled => "Throttled",
            Self::SubscriptionLimitExceeded => "SubscriptionLimitExceeded",
            Self::TopicLimitExceeded => "TopicLimitExceeded",
            Self::FilterPolicyLimitExceeded => "FilterPolicyLimitExceeded",
            Self::EndpointDisabled => "EndpointDisabled",
            Self::ValidationException => "ValidationException",
            Self::BatchEntryIdsNotDistinct => "BatchEntryIdsNotDistinct",
            Self::EmptyBatchRequest => "EmptyBatchRequest",
            Self::TooManyEntriesInBatchRequest => "TooManyEntriesInBatchRequest",
            _ => "InternalError",
        }
    }

    /// HTTP status code.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::InternalError => 500,
            Self::Throttled => 429,
            _ => 400,
        }
    }

    /// Fault type: "Sender" (4xx) or "Receiver" (5xx).
    pub fn fault(&self) -> &'static str {
        match self {
            Self::InternalError => "Receiver",
            Self::Throttled => "Receiver",
            _ => "Sender",
        }
    }
}
```

### 11.3 Error Response Formatting

```rust
/// Format an SNS error as an XML response.
fn xml_error_response(error: &SnsError) -> http::Response<Bytes> {
    let xml = format!(
        r#"<ErrorResponse xmlns="http://sns.amazonaws.com/doc/2010-03-31/">
  <Error>
    <Type>{}</Type>
    <Code>{}</Code>
    <Message>{}</Message>
  </Error>
  <RequestId>{}</RequestId>
</ErrorResponse>"#,
        error.code.fault(),
        error.code.code(),
        xml_escape(&error.message),
        uuid::Uuid::new_v4(),
    );

    http::Response::builder()
        .status(error.code.status_code())
        .header("content-type", "text/xml")
        .body(Bytes::from(xml))
        .expect("valid error response")
}
```

### 11.4 Service Error Enum

```rust
/// Domain-level errors for SNS business logic.
#[derive(Debug, thiserror::Error)]
pub enum SnsServiceError {
    #[error("Topic does not exist: {arn}")]
    TopicNotFound { arn: String },
    #[error("Subscription does not exist: {arn}")]
    SubscriptionNotFound { arn: String },
    #[error("Invalid parameter: {message}")]
    InvalidParameter { message: String },
    #[error("Invalid ARN: {arn}")]
    InvalidArn { arn: String },
    #[error("Topic name is invalid: {name}")]
    InvalidTopicName { name: String },
    #[error("Message is too large: {size} bytes (max 262144)")]
    MessageTooLarge { size: usize },
    #[error("Message structure json is invalid: {message}")]
    InvalidMessageStructure { message: String },
    #[error("Filter policy is invalid: {message}")]
    InvalidFilterPolicy { message: String },
    #[error("Endpoint disabled: {arn}")]
    EndpointDisabled { arn: String },
    #[error("Platform application not found: {arn}")]
    PlatformApplicationNotFound { arn: String },
    #[error("Platform endpoint not found: {arn}")]
    PlatformEndpointNotFound { arn: String },
    #[error("Batch entry IDs not distinct")]
    BatchIdsNotDistinct,
    #[error("Batch request must contain 1-10 entries")]
    InvalidBatchSize { count: usize },
    #[error("Duplicate tag keys in request")]
    DuplicateTagKeys,
    #[error("Authorization error: {message}")]
    AuthorizationError { message: String },
    #[error("Internal error: {message}")]
    Internal { message: String },
}
```

---

## 12. Server Integration

### 12.1 SNS ServiceRouter

```rust
#[cfg(feature = "sns")]
mod sns_router {
    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the SNS service.
    ///
    /// Matches requests whose Content-Type is form-urlencoded and whose
    /// body contains an Action= parameter that is a recognized SNS action.
    ///
    /// Because matching requires reading the body (to check Action=), the
    /// gateway must buffer form-urlencoded POST requests before routing.
    pub struct SnsServiceRouter<H: SnsHandler> {
        inner: SnsHttpService<H>,
    }

    impl<H: SnsHandler> ServiceRouter for SnsServiceRouter<H> {
        fn name(&self) -> &'static str { "sns" }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            // SNS uses form-urlencoded POST to /
            if *req.method() != http::Method::POST {
                return false;
            }
            // Check Content-Type header for form-urlencoded
            req.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.contains("x-www-form-urlencoded"))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}
```

**Important routing note**: Both SNS and SQS (Query protocol) use `application/x-www-form-urlencoded` POST to `/`. The gateway cannot distinguish them without reading the body. Two approaches:

1. **Approach A (recommended)**: Register SNS as the handler for all `x-www-form-urlencoded POST /` requests. Inside the SNS HTTP service, parse `Action=` and delegate to SQS if the action is a known SQS action. This avoids body buffering at the gateway level.

2. **Approach B**: Buffer the body at the gateway level and inspect `Action=` before routing. Cleaner separation but requires body cloning.

We recommend **Approach A** with a combined `QueryServiceRouter` that dispatches to either SNS or SQS based on the `Action=` parameter:

```rust
/// Routes form-urlencoded requests to SNS or SQS based on Action parameter.
pub struct QueryServiceRouter {
    sns: SnsHttpService,
    sqs: SqsHttpService,
}

impl ServiceRouter for QueryServiceRouter {
    fn name(&self) -> &'static str { "sns" }

    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        *req.method() == http::Method::POST
            && req.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.contains("x-www-form-urlencoded"))
    }

    fn call(&self, req: http::Request<Incoming>) -> /* ... */ {
        // Buffer body, parse Action=, dispatch to SNS or SQS
    }
}
```

### 12.2 Feature Gate

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "sns"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http", "dep:rustack-s3-model"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
sqs = ["dep:rustack-sqs-core", "dep:rustack-sqs-http"]
ssm = ["dep:rustack-ssm-core", "dep:rustack-ssm-http"]
sns = ["dep:rustack-sns-core", "dep:rustack-sns-http", "sqs"]
```

Note: `sns` feature implies `sqs` because SNS-to-SQS fan-out requires the SQS provider.

### 12.3 Gateway Registration Order

Services are registered in specificity order: most specific first, catch-all last.

```rust
fn build_gateway(config: &ServerConfig) -> GatewayService {
    let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

    #[cfg(feature = "dynamodb")]
    services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_service)));

    // SQS JSON protocol (X-Amz-Target: AmazonSQS.*)
    #[cfg(feature = "sqs")]
    services.push(Box::new(SqsServiceRouter::new(sqs_service)));

    #[cfg(feature = "ssm")]
    services.push(Box::new(SsmServiceRouter::new(ssm_service)));

    // SNS + SQS Query protocol (form-urlencoded POST to /)
    // This router handles both SNS awsQuery and SQS awsQuery requests.
    #[cfg(feature = "sns")]
    services.push(Box::new(QueryServiceRouter::new(sns_service, sqs_query_service)));

    #[cfg(feature = "s3")]
    services.push(Box::new(S3ServiceRouter::new(s3_service))); // catch-all, must be last

    GatewayService::new(services)
}
```

### 12.4 SNS-SQS Wiring

The server binary creates the `SqsPublisher` bridge at startup:

```rust
#[cfg(all(feature = "sns", feature = "sqs"))]
{
    let sqs_provider = Arc::clone(&sqs_provider);
    let sqs_publisher = Arc::new(RustackSqsPublisher::new(
        sqs_provider,
        SqsPublisherConfig {
            region: sns_config.default_region.clone(),
            account_id: sns_config.account_id.clone(),
            host: sns_config.host.clone(),
            port: sns_config.port,
        },
    ));

    let sns_provider = RustackSns::new(
        sns_config.clone(),
        sqs_publisher,
        http_publisher,
    );
}
```

### 12.5 Configuration

```rust
/// SNS configuration.
pub struct SnsConfig {
    pub skip_signature_validation: bool,
    pub default_region: String,
    pub account_id: String,
    pub host: String,
    pub port: u16,
}

impl SnsConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("SNS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            host: env_str("GATEWAY_HOST", "localhost"),
            port: env_u16("GATEWAY_PORT", 4566),
        }
    }
}
```

### 12.6 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared with all services) |
| `SNS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default AWS account ID |

### 12.7 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "running",
        "dynamodb": "running",
        "sqs": "running",
        "ssm": "running",
        "sns": "running"
    }
}
```

### 12.8 Retrospection Endpoints

LocalStack provides retrospection endpoints for inspecting delivered messages. We implement these for compatibility:

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/_aws/sns/platform-endpoint-messages` | GET | List messages sent to platform endpoints |
| `/_aws/sns/sms-messages` | GET | List SMS messages sent |
| `/_aws/sns/subscription-tokens` | GET | List pending confirmation tokens |

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **Topic store**: Test CRUD operations, idempotent create, subscription management
- **Subscription protocols**: Test protocol parsing, endpoint validation
- **Filter policy evaluation**: Test all operators (exact, prefix, suffix, numeric, exists, anything-but, cidr) against message attributes and body
- **Filter policy validation**: Test complexity limits, invalid policy detection
- **Message envelope**: Test JSON wrapping for SQS delivery, raw message delivery mode
- **Form parameter parsing**: Test nested parameter parsing (Attributes.entry.N, Tags.member.N, MessageAttributes.entry.N)
- **XML serialization**: Test XML response format matches AWS output for all operations
- **Topic name validation**: Test valid/invalid names, FIFO suffix detection
- **ARN generation/parsing**: Test topic and subscription ARN format

### 13.2 Integration Tests with aws-sdk-sns

```rust
// tests/integration/sns_tests.rs
#[tokio::test]
#[ignore]
async fn test_sns_topic_lifecycle() {
    let client = aws_sdk_sns::Client::new(&config);

    // Create topic
    let create = client.create_topic()
        .name("test-topic")
        .send().await.unwrap();
    let topic_arn = create.topic_arn().unwrap();

    // Get attributes
    let attrs = client.get_topic_attributes()
        .topic_arn(topic_arn)
        .send().await.unwrap();
    assert!(attrs.attributes().contains_key("TopicArn"));

    // Delete topic
    client.delete_topic().topic_arn(topic_arn).send().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_sns_sqs_fanout() {
    let sns = aws_sdk_sns::Client::new(&config);
    let sqs = aws_sdk_sqs::Client::new(&config);

    // Create topic and queue
    let topic = sns.create_topic().name("fanout-topic").send().await.unwrap();
    let queue = sqs.create_queue().queue_name("fanout-queue").send().await.unwrap();
    let queue_url = queue.queue_url().unwrap();
    let queue_arn = format!(
        "arn:aws:sqs:us-east-1:000000000000:fanout-queue"
    );

    // Subscribe queue to topic
    let sub = sns.subscribe()
        .topic_arn(topic.topic_arn().unwrap())
        .protocol("sqs")
        .endpoint(&queue_arn)
        .send().await.unwrap();

    // Publish message
    sns.publish()
        .topic_arn(topic.topic_arn().unwrap())
        .message("hello from SNS")
        .send().await.unwrap();

    // Receive from SQS
    let recv = sqs.receive_message()
        .queue_url(queue_url)
        .wait_time_seconds(5)
        .send().await.unwrap();
    let messages = recv.messages();
    assert_eq!(messages.len(), 1);
    // Message body is an SNS envelope JSON
    let envelope: serde_json::Value = serde_json::from_str(
        messages[0].body().unwrap()
    ).unwrap();
    assert_eq!(envelope["Message"], "hello from SNS");
}

#[tokio::test]
#[ignore]
async fn test_sns_filter_policy() {
    // Test that filter policies correctly route messages
    // to only matching subscriptions
}

#[tokio::test]
#[ignore]
async fn test_sns_fifo_topic() {
    // Test FIFO topic with message group ordering
    // and deduplication
}
```

### 13.3 Third-Party Test Suites

#### 13.3.1 LocalStack Test Suite (Primary)

The most comprehensive open-source SNS test suite. Already vendored at `vendors/localstack/tests/aws/services/sns/`.

- **`test_sns.py`** -- 158 test functions across 20 test classes covering:
  - `TestSNSTopicCrud` -- topic create, delete, attributes, idempotency, FIFO, permissions
  - `TestSNSTopicCrudV2` -- additional topic CRUD scenarios
  - `TestSNSPublishCrud` -- publish by ARN, target ARN, phone number, message structure json, message size limits
  - `TestSNSSubscriptionCrud` -- subscribe, unsubscribe, confirm, attributes, filter policy, idempotency
  - `TestSNSSubscriptionSQS` -- SNS-to-SQS delivery, batch, message attributes, raw message delivery, message structure json, signature verification
  - `TestSNSSubscriptionSQSFifo` -- FIFO topic to FIFO queue, ordering, deduplication, DLQ
  - `TestSNSSubscriptionLambda` -- Lambda trigger, DLQ redrive
  - `TestSNSSubscriptionHttp` -- HTTP endpoint subscription, confirmation, delivery
  - `TestSNSSubscriptionSES` -- SES email subscription (stub)
  - `TestSNSSubscriptionFirehose` -- Firehose subscription (stub)
  - `TestSNSPlatformApplicationCrud` -- platform app CRUD
  - `TestSNSPlatformEndpointCrud` -- platform endpoint CRUD
  - `TestSNSPlatformEndpoint` -- endpoint publish, disabled endpoint
  - `TestSNSSMS` -- SMS publish, attributes, opt-out
  - `TestSNSMultiAccounts` -- cross-account access
  - `TestSNSMultiRegions` -- cross-region delivery
  - `TestSNSPublishDelivery` -- delivery verification
  - `TestSNSRetrospectionEndpoints` -- platform/SMS retrospection endpoints

- **`test_sns_filter_policy.py`** -- 22 test functions covering:
  - Filter policy scope (MessageAttributes, MessageBody)
  - Nested property matching
  - Exact match, exists, prefix/suffix operators
  - Numeric conditions
  - IP address/CIDR matching
  - Anything-but operator
  - Complex payloads with arrays and nested objects
  - Policy validation and complexity limits

- **Framework**: pytest with snapshot testing (`snapshot.match()`)

**Adaptation strategy**: Same approach as SQS -- run the Python test suite against Rustack's SNS endpoint, track pass/fail counts, progressively fix failures.

```makefile
test-sns-localstack:
    @cd vendors/localstack && python -m pytest tests/aws/services/sns/test_sns.py \
        --endpoint-url=http://localhost:4566 -v

test-sns-filter-localstack:
    @cd vendors/localstack && python -m pytest tests/aws/services/sns/test_sns_filter_policy.py \
        --endpoint-url=http://localhost:4566 -v
```

**Expected initial pass rate**: ~30-40% in Phase 0 (topic CRUD, basic publish/subscribe/SQS delivery). Many tests require Lambda, HTTP endpoints, or platform applications which are Phase 2-3.

#### 13.3.2 Moto SNS Test Suite (Secondary Validation)

- **Repository**: https://github.com/getmoto/moto
- **Location**: `tests/test_sns/`
- **Language**: Python with pytest
- **Files**:
  - `test_topics.py` -- topic CRUD, attributes, permissions, tags
  - `test_subscriptions.py` -- subscription CRUD, filter policies, confirmation
  - `test_publishing.py` -- publish to topics, message attributes, message structure json
  - `test_publish_batch.py` -- batch publish operations
  - `test_application.py` -- platform application/endpoint CRUD
  - `test_http_message_verification.py` -- HTTP subscription message signature verification
  - `test_server.py` -- server endpoint testing
  - `test_sns_cloudformation.py` -- CloudFormation integration (not applicable)
- **Running**: Can be adapted to run against Rustack by pointing boto3 at our endpoint

Moto is the second-most comprehensive SNS mock after LocalStack. While it is primarily an in-process mock (not a server), its test cases document expected AWS behavior and can be ported to integration tests.

**Adaptation strategy**: Extract test cases from moto and translate to either Rust integration tests or standalone Python scripts that hit our endpoint.

#### 13.3.3 GoAWS Smoke Tests

- **Repository**: https://github.com/Admiral-Piett/goaws
- **Coverage**: Basic SNS operations (CreateTopic, ListTopics, DeleteTopic, Subscribe, Publish, Unsubscribe)
- **Limitations**: No filter policies, no FIFO, no batch operations
- **Utility**: Useful only for basic smoke testing. GoAWS Postman collection can be adapted.

#### 13.3.4 s12v/sns Cucumber Tests

- **Repository**: https://github.com/s12v/sns
- **Language**: Scala with Cucumber BDD features, validated against AWS Ruby and PHP SDKs
- **Coverage**: CreateTopic, Subscribe, Publish with SQS/HTTP/File endpoints
- **Utility**: Provides behavior-driven test scenarios that document expected API behavior. Can serve as a reference for edge case documentation.

#### 13.3.5 AWS SDK Integration Tests

Write targeted tests using `aws-sdk-sns` Rust crate:

```rust
// Test each operation against known AWS behavior
// Focus on edge cases: empty topics, max subscriptions, invalid ARNs,
// filter policy evaluation, FIFO dedup window, message structure json
```

#### 13.3.6 AWS CLI Smoke Tests

Shell-based end-to-end tests for CI:

```bash
#!/bin/bash
# Basic SNS+SQS fan-out CLI smoke test
ENDPOINT="--endpoint-url http://localhost:4566"

# Create topic and queue
TOPIC_ARN=$(aws sns create-topic $ENDPOINT --name test-topic \
    --query TopicArn --output text)
QUEUE_URL=$(aws sqs create-queue $ENDPOINT --queue-name test-queue \
    --query QueueUrl --output text)
QUEUE_ARN="arn:aws:sqs:us-east-1:000000000000:test-queue"

# Subscribe queue to topic
aws sns subscribe $ENDPOINT --topic-arn "$TOPIC_ARN" \
    --protocol sqs --notification-endpoint "$QUEUE_ARN"

# Publish message
aws sns publish $ENDPOINT --topic-arn "$TOPIC_ARN" --message "hello from SNS"

# Receive from SQS
MSG=$(aws sqs receive-message $ENDPOINT --queue-url "$QUEUE_URL" \
    --wait-time-seconds 5 --query 'Messages[0].Body' --output text)
echo "$MSG" | python3 -c "import sys,json; print(json.load(sys.stdin)['Message'])"

# Cleanup
aws sns delete-topic $ENDPOINT --topic-arn "$TOPIC_ARN"
aws sqs delete-queue $ENDPOINT --queue-url "$QUEUE_URL"
```

### 13.4 Makefile Targets

```makefile
test-sns: test-sns-unit test-sns-integration

test-sns-unit:
    @cargo test -p rustack-sns-model -p rustack-sns-core -p rustack-sns-http

test-sns-integration:
    @cargo test -p integration-tests -- sns --ignored

test-sns-cli:
    @./tests/sns-cli-smoke.sh

test-sns-localstack:
    @cd vendors/localstack && python -m pytest tests/aws/services/sns/ -v
```

---

## 14. Phased Implementation Plan

### Phase 0: MVP (12 Operations -- Topic CRUD, Subscribe, Publish, SQS Fan-out)

**Goal**: Cover the most common local development use case: create a topic, subscribe an SQS queue, publish messages, and receive them from SQS.
**Estimated scope**: ~6,000-8,000 lines of Rust code across 3 new crates.

#### Step 0.1: Codegen Extension
- Add `SnsServiceConfig` to codegen
- Download SNS Smithy model JSON from `aws/api-models-aws`
- Generate `rustack-sns-model` crate (operations enum, input/output structs, error codes)
- Generate serde derives with `#[serde(rename_all = "PascalCase")]`

#### Step 0.2: HTTP Layer (awsQuery Protocol)
- Implement `SnsRouter` (Action= dispatch)
- Implement `SnsHttpService` (hyper Service)
- Implement form-urlencoded request deserialization with nested parameter parsing
- Implement XML response serialization
- Implement XML error response formatting

#### Step 0.3: Storage Engine
- Implement `TopicStore` (DashMap-based topic/subscription storage)
- Implement `TopicRecord`, `SubscriptionRecord` data structures
- Implement topic ARN and subscription ARN generation

#### Step 0.4: Core Operations (12 ops)
- `CreateTopic` / `DeleteTopic` / `GetTopicAttributes` / `SetTopicAttributes` / `ListTopics` (topic management)
- `Subscribe` / `Unsubscribe` / `GetSubscriptionAttributes` / `SetSubscriptionAttributes` (subscription management)
- `ListSubscriptions` / `ListSubscriptionsByTopic` (listing)
- `Publish` (fan-out to SQS only, no filtering)

#### Step 0.5: SQS Integration
- Define `SqsPublisher` trait in `rustack-sns-core`
- Implement `RustackSqsPublisher` in server binary wrapping `RustackSqs`
- Implement SNS message envelope (JSON wrapping for SQS delivery)
- Implement `RawMessageDelivery` attribute support
- Wire SNS and SQS providers together in `main.rs`

#### Step 0.6: Server Integration
- Implement `SnsServiceRouter` (or `QueryServiceRouter` for SNS+SQS query dispatch)
- Add `sns` cargo feature gate
- Handle form-urlencoded routing disambiguation (SNS vs SQS query protocol)
- Update health endpoint
- Update `is_compiled_in()` and `parse_enabled_services()`

#### Step 0.7: Testing
- Unit tests for topic store, subscription management, XML serialization, form parsing
- Integration tests with aws-sdk-sns + aws-sdk-sqs for fan-out
- CLI smoke tests
- Update Makefile with SNS test targets

### Phase 1: Filter Policies, Tags, PublishBatch, ConfirmSubscription

**Goal**: Production-ready SNS with message filtering, tagging, and batch publishing.

- `PublishBatch` (up to 10 messages, per-entry success/failure)
- `ConfirmSubscription` (token-based confirmation for HTTP/HTTPS)
- `TagResource` / `UntagResource` / `ListTagsForResource` (tags, max 50)
- **Filter policy evaluation**: exact match, prefix, suffix, numeric, exists, anything-but, cidr
- **Filter policy scope**: `MessageAttributes` (default) and `MessageBody`
- **Filter policy validation**: complexity limits, correct JSON structure
- **MessageStructure=json**: protocol-specific message resolution from JSON structure
- Run LocalStack `test_sns_filter_policy.py` suite

### Phase 2: HTTP Subscriptions, FIFO Topics, Permissions, Data Protection

**Goal**: Advanced features for more complex testing scenarios.

- **HTTP/HTTPS endpoint subscriptions**:
  - Send `SubscriptionConfirmation` POST to endpoint
  - Send `Notification` POST for published messages
  - Implement `HttpPublisher` trait with reqwest or hyper client
- **FIFO topics**:
  - Topic name must end with `.fifo`
  - `MessageGroupId` required on Publish
  - `MessageDeduplicationId` or content-based deduplication
  - Fan-out to FIFO SQS queues with group ID and dedup ID propagation
  - Sequence number assignment
- `AddPermission` / `RemovePermission` (store in Policy, no enforcement)
- `GetDataProtectionPolicy` / `PutDataProtectionPolicy` (store, no enforcement)

### Phase 3: Platform Applications, SMS, Retrospection Endpoints

**Goal**: Feature completeness for the full SNS API surface.

- Platform application CRUD (10 operations): `CreatePlatformApplication`, `DeletePlatformApplication`, `GetPlatformApplicationAttributes`, `SetPlatformApplicationAttributes`, `ListPlatformApplications`, `CreatePlatformEndpoint`, `DeleteEndpoint`, `GetEndpointAttributes`, `SetEndpointAttributes`, `ListEndpointsByPlatformApplication`
- SMS operations (11 operations): `CheckIfPhoneNumberIsOptedOut`, `GetSMSAttributes`, `SetSMSAttributes`, `ListPhoneNumbersOptedOut`, `OptInPhoneNumber`, `GetSMSSandboxAccountStatus`, `CreateSMSSandboxPhoneNumber`, `DeleteSMSSandboxPhoneNumber`, `VerifySMSSandboxPhoneNumber`, `ListSMSSandboxPhoneNumbers`, `ListOriginationNumbers`
- Retrospection endpoints: `/_aws/sns/platform-endpoint-messages`, `/_aws/sns/sms-messages`, `/_aws/sns/subscription-tokens`
- Publish to platform endpoints (store for retrospection)
- Publish to SMS (store for retrospection)

---

## 15. Risk Analysis

### 15.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| awsQuery form parsing complexity (nested dot-notation) | High | High | Build comprehensive parser with unit tests; reference SQS query impl; test against AWS SDK behavior |
| XML response format divergence from AWS | Medium | High | Compare responses against real AWS or LocalStack snapshots; use snapshot testing |
| SNS-to-SQS integration correctness (message envelope format) | High | High | Test with aws-sdk-sqs receiving SNS messages; compare envelope JSON with real AWS output |
| Filter policy evaluation edge cases | High | Medium | Port moto and LocalStack filter tests; test all operators individually and combined |
| Gateway routing ambiguity (SNS vs SQS awsQuery) | Medium | High | Use Action= parameter disambiguation; maintain separate action name sets |
| Message delivery ordering in fan-out | Medium | Medium | Deliver sequentially to subscriptions for deterministic testing; document non-guarantee |
| HTTP subscription confirmation flow | Medium | Medium | Defer to Phase 2; auto-confirm all subscriptions initially |
| FIFO topic-to-FIFO queue propagation correctness | Medium | High | Test MessageGroupId and DeduplicationId passthrough; reuse SQS FIFO tests |

### 15.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Platform application operations more complex than expected | Low | Low | These are CRUD-only stubs; no actual push notification delivery |
| LocalStack test suite uses internal LocalStack APIs | High | Medium | Skip tests that depend on LocalStack internals (provider imports, internal endpoints) |
| Filter policy operator complexity exceeds estimate | Medium | Medium | Phase 1 can be split into sub-phases; start with exact match only |
| Users demand Lambda trigger integration | Medium | Low | Out of scope; document as future work when Lambda runtime is added |
| Message signing/verification for HTTP subscriptions | Medium | Low | Use stub signatures initially; Phase 2 can add proper signing |

### 15.3 Dependency Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| SNS requires SQS to be stable and complete | Low | High | SQS is already implemented and passing tests; SNS integration is additive |
| Codegen tool may not handle awsQuery-specific Smithy traits | Medium | Medium | awsQuery types are structurally identical to awsJson types; only the HTTP layer differs |
| SNS Smithy model not available in expected format | Low | Medium | Fall back to manual type definitions if needed; SNS has only ~42 operations |

### 15.4 Behavioral Differences

| Behavior | AWS | LocalStack | Rustack | Justification |
|----------|-----|------------|-----------|---------------|
| SQS subscription confirmation | Auto-confirmed | Auto-confirmed | Auto-confirmed | Standard for local dev |
| HTTP subscription confirmation | Requires ConfirmSubscription | Sends POST, requires confirm | Phase 0: auto-confirm; Phase 2: proper flow | Simplify MVP |
| Message signing | RSA signature | Stub signature | Stub signature | Not needed for local dev |
| SMS delivery | Actual SMS | Stub (retrospection) | Stub (retrospection) | No real SMS for local dev |
| Lambda invocation | Invokes Lambda | Invokes local Lambda | Not implemented | Future work |
| Platform push | Sends push notification | Stub (retrospection) | Stub (retrospection) | No real push for local dev |
| Delete non-existent topic | No error (idempotent) | No error | No error | Match AWS behavior |
| Filter policy complexity limit | 150 combinations | Enforced | Enforced | Match AWS behavior |
| Message size limit | 256 KiB | Enforced | Enforced | Catch real bugs locally |

---

## 16. Open Questions

### 16.1 Query Protocol Routing Strategy

Should we use Approach A (combined `QueryServiceRouter` for SNS+SQS) or Approach B (body buffering at gateway)?

**Recommendation**: Approach A. A combined router avoids changing the `ServiceRouter` trait to support body access. The router buffers the form body, checks `Action=`, and dispatches to the correct service. This is a localized change in the server binary.

### 16.2 SQS Publisher Trait Location

Should the `SqsPublisher` trait live in `rustack-sns-core` or in a shared `rustack-core` crate?

**Recommendation**: Define in `rustack-sns-core`. The trait is specific to SNS delivery semantics. If we later add other cross-service integrations (e.g., S3 event notifications to SNS), we can extract a shared trait then. YAGNI for now.

### 16.3 HTTP Subscription Delivery in MVP

Should HTTP/HTTPS subscription delivery be included in Phase 0?

**Recommendation**: No. Phase 0 focuses on SQS fan-out, which covers the vast majority of local development use cases. HTTP subscriptions add complexity (async HTTP client, subscription confirmation flow, delivery retry) that can wait for Phase 2. In Phase 0, HTTP subscriptions are accepted but delivery is a no-op with a debug log.

### 16.4 Message Signing

Should we implement proper SNS message signing for SQS/HTTP deliveries?

**Recommendation**: No. Use a deterministic stub signature (e.g., base64-encoded placeholder). Real message signature verification requires generating RSA key pairs and signing with the SNS certificate. This is unnecessary for local development and would add complexity. If a user needs signature verification testing, they can disable verification on their end.

### 16.5 Concurrency Model

Should topics use an actor-per-topic model (like SQS queues) or a shared DashMap approach?

**Recommendation**: Use DashMap (shared state) rather than actors. Unlike SQS queues which have complex stateful message lifecycles (visibility timeouts, in-flight tracking, long polling), SNS topics are primarily read-heavy with simple publish-and-deliver semantics. There is no message retention, no in-flight state, and no long polling. A DashMap with per-topic entries is simpler and sufficient. The fan-out delivery itself is async but stateless.

---
