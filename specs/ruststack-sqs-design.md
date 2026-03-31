# Rustack SQS: Native Rust Implementation Design

**Date:** 2026-03-02
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-dynamodb-design.md](./rustack-dynamodb-design.md), [SQS API Research](../docs/research/sqs-api-research.md)
**Scope:** Add native SQS support to Rustack using the same Smithy-based codegen approach as DynamoDB, with an actor-based in-memory message queue engine (no ElasticMQ/GoAWS wrapping).

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

This spec proposes adding SQS support to Rustack as a fully native Rust implementation, following the same architectural patterns established by S3 and DynamoDB. Key design decisions:

- **Native Rust message queue engine** -- unlike LocalStack which wraps ElasticMQ (a Scala/Pekko application), we build a purpose-built in-memory queue engine with actor-per-queue concurrency. This maintains the ~10MB Docker image and millisecond startup.
- **Smithy codegen reuse** -- extend the existing `codegen/` system to generate SQS model types from the official AWS SQS Smithy JSON AST (`aws/api-models-aws`), producing a `rustack-sqs-model` crate.
- **JSON protocol with awsQuery backward compatibility** -- SQS uses `awsJson1_0` with the `awsQueryCompatible` trait, identical dispatch pattern to DynamoDB (HTTP POST with `X-Amz-Target: AmazonSQS.*`). Legacy SDKs send `application/x-www-form-urlencoded` with `Action=` parameter; we must support both.
- **Actor-based concurrency** -- each queue runs as an independent actor owning its message state, communicating via `tokio::sync::mpsc` channels. Long-polling consumers wait on `tokio::sync::Notify`. This follows the actor model mandated by CLAUDE.md.
- **Shared infrastructure** -- reuse `rustack-core` (multi-account/region state), `rustack-auth` (SigV4 verification), and the DynamoDB-established `awsJson1_0` HTTP layer patterns unchanged.
- **Phased delivery** -- 4 phases from MVP (9 operations, standard queues, short polling) to full feature parity including FIFO queues, DLQ redrive, and awsQuery protocol support.

---

## 2. Motivation

### 2.1 Why SQS?

SQS is the third most-used AWS service for local development after S3 and DynamoDB. Developers need a local SQS for:

- **Event-driven architecture testing** -- test producer/consumer patterns, Lambda triggers, and async workflows without AWS costs
- **Local development** -- run full microservice stacks locally with `docker compose` where services communicate via queues
- **CI/CD pipelines** -- fast, deterministic SQS in GitHub Actions for integration tests
- **Offline development** -- work without internet connectivity
- **Dead-letter queue testing** -- validate retry logic and DLQ redrive policies locally

### 2.2 Why Not Wrap ElasticMQ?

ElasticMQ (SoftwareMill) is the most capable open-source SQS emulator. LocalStack uses it as a backend. Wrapping it has significant drawbacks:

| Issue | Impact |
|-------|--------|
| **JVM dependency** | ElasticMQ runs on JVM (Scala/Pekko), adding ~300MB to Docker image |
| **Startup time** | JVM startup takes 2-4 seconds |
| **Memory overhead** | JVM baseline memory is 80-150MB |
| **Process management** | Must proxy HTTP between Rust and JVM, manage process lifecycle |
| **Protocol gaps** | ElasticMQ supports awsQuery but JSON protocol support is recent and incomplete |
| **Behavioral differences** | ElasticMQ has known divergences from AWS SQS (FIFO semantics, error messages) |
| **Architecture mismatch** | Actor model in Pekko does not compose with our Tokio runtime |

### 2.3 Why Not Wrap GoAWS?

GoAWS (Admiral-Piett) is a lightweight Go SQS+SNS emulator but is too incomplete:

- No FIFO queue support
- Limited queue attributes (only VisibilityTimeout, ReceiveMessageWaitTimeSeconds, RedrivePolicy)
- No ChangeMessageVisibilityBatch, no tags, no message move tasks
- No JSON protocol support (awsQuery only)

### 2.4 Why Native Rust?

A native Rust implementation provides:

- **~10MB Docker image** (same as S3/DynamoDB) vs ~300MB with ElasticMQ
- **Millisecond startup** vs 2-4 seconds for JVM
- **~5MB memory baseline** vs 80-150MB for JVM
- **Full debuggability** -- we own every line of code
- **Tokio-native concurrency** -- long polling, timers, and actor channels integrate naturally with our async runtime
- **Single binary** -- no process management, no inter-process communication
- **Protocol correctness** -- support both JSON and awsQuery from day one

### 2.5 Existing Alternatives

| Implementation | Language | Image Size | JSON Protocol | FIFO | Notes |
|---------------|----------|------------|---------------|------|-------|
| ElasticMQ | Scala/JVM | ~300MB | Partial | Yes | Most mature, SoftwareMill maintained |
| GoAWS | Go | ~30MB | No | No | Lightweight but incomplete |
| LocalStack SQS | Python+ElasticMQ | ~1GB | Yes | Yes | Wraps ElasticMQ, adds multi-account |
| fake_sqs | Ruby | ~200MB | No | No | Abandoned, incomplete |
| **Rustack SQS** | **Rust** | **~10MB** | **Yes** | **Yes** | **This proposal** |

No existing Rust-based SQS emulator exists. This would be the first.

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Native Rust SQS emulator** -- no JVM, no external processes, no FFI
2. **Cover 90%+ of local development use cases** -- queue CRUD, message send/receive/delete, visibility timeout, DLQ
3. **Dual protocol support** -- `awsJson1_0` for modern SDKs and `awsQuery` (XML responses) for legacy SDKs
4. **Smithy-generated types** -- all SQS API types generated from official AWS Smithy model
5. **Actor-per-queue concurrency** -- each queue owns its messages, communicates via channels
6. **Long polling** -- hold HTTP connections and wake on message arrival or timeout
7. **FIFO queues** -- strict ordering within message groups, exactly-once deduplication
8. **Same Docker image** -- single binary serves S3, DynamoDB, and SQS on the same port (4566)
9. **GitHub Action compatibility** -- extend the existing `tyrchen/rustack` GitHub Action
10. **Pass LocalStack SQS test suite** -- validate against vendored `test_sqs.py`

### 3.2 Non-Goals

1. **KMS encryption** -- accept encryption attributes in metadata, do not perform actual encryption
2. **IAM policy enforcement** -- accept `AddPermission`/`RemovePermission` and `Policy` attribute, do not evaluate policies
3. **SQS-SNS subscriptions** -- SNS integration is a separate service; out of scope
4. **High-throughput FIFO mode** -- accept `FifoThroughputLimit` attribute, do not differentiate throughput behavior
5. **Cross-account access** -- all queues exist within a single account context
6. **CloudWatch metrics integration** -- no metrics emission
7. **Data persistence across restarts** -- in-memory only, matching S3 and DynamoDB behavior
8. **Server-side encryption at rest** -- metadata only, no actual data encryption

---

## 4. Architecture Overview

### 4.1 Layered Architecture (Mirrors S3 and DynamoDB)

```
                    AWS SDK / CLI / boto3
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  <-- Routes by X-Amz-Target or Content-Type
              |   (ServiceRouter)   |
              +--------+------------+
                       |
         +-------------+-------------+------------------+
         v                           v                  v
   +-----------+              +-----------+       +-----------+
   | S3 HTTP   |              | DDB HTTP  |       | SQS HTTP  |
   | (RestXml) |              | (JSON1.0) |       | (JSON1.0  |
   +-----------+              +-----------+       |  +Query)  |
         |                          |             +-----------+
   +-----------+              +-----------+             |
   | S3 Core   |              | DDB Core  |       +-----------+
   +-----------+              +-----------+       | SQS Core  |
         |                          |             +-----------+
         +-------------+------------+                   |
                       v                                |
              +-----------------+                       |
              | rustack-core  |  <-- Shared: multi-account/region
              | rustack-auth  |  <-- Shared: SigV4 authentication
              +-----------------+
```

### 4.2 Gateway Service Routing

SQS, DynamoDB, and S3 are distinguishable by their request signatures:

| Signal | S3 | DynamoDB | SQS (JSON) | SQS (Query) |
|--------|----|---------:|------------|-------------|
| HTTP Method | GET/PUT/DELETE/POST/HEAD | POST only | POST only | POST only |
| Content-Type | varies | `application/x-amz-json-1.0` | `application/x-amz-json-1.0` | `application/x-www-form-urlencoded` |
| `X-Amz-Target` | absent | `DynamoDB_20120810.*` | `AmazonSQS.*` | absent |
| URL path | `/{bucket}/{key}` | `/` | `/` | `/` |
| Dispatch | `Action=` form param | N/A | N/A | `Action=SendMessage` etc. |

**Routing logic** (evaluated in order):
1. If `X-Amz-Target` starts with `DynamoDB_` -- route to DynamoDB
2. If `X-Amz-Target` starts with `AmazonSQS` -- route to SQS (JSON protocol)
3. If `Content-Type` is `application/x-www-form-urlencoded` and body contains `Action=` with a recognized SQS action -- route to SQS (Query protocol)
4. Default: route to S3 (catch-all)

### 4.3 Crate Dependency Graph

```
rustack (app) <-- unified binary
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-core
+-- rustack-s3-http
+-- rustack-s3-model
+-- rustack-dynamodb-core
+-- rustack-dynamodb-http
+-- rustack-dynamodb-model
+-- rustack-sqs-core       <-- NEW
+-- rustack-sqs-http       <-- NEW
+-- rustack-sqs-model      <-- NEW (auto-generated)

rustack-sqs-http
+-- rustack-sqs-model
+-- rustack-auth

rustack-sqs-core
+-- rustack-core
+-- rustack-sqs-model
+-- rustack-auth
+-- tokio (channels, Notify, timers)
+-- dashmap

rustack-sqs-model (auto-generated, standalone)
```

---

## 5. Protocol Design

### 5.1 Dual Protocol Requirement

SQS is unique among the three services because it uses `@awsJson1_0` with the `@awsQueryCompatible` trait. This means we must handle two distinct wire formats:

| Aspect | JSON Protocol (modern) | Query Protocol (legacy) |
|--------|----------------------|------------------------|
| Content-Type | `application/x-amz-json-1.0` | `application/x-www-form-urlencoded` |
| Operation dispatch | `X-Amz-Target: AmazonSQS.SendMessage` | `Action=SendMessage` form param |
| Request body | JSON | URL-encoded form fields |
| Response body | JSON | XML |
| Error body | JSON with `__type` | XML `<ErrorResponse>` |
| Error header | `x-amzn-query-error: Code;Fault` | N/A |
| SDK versions | Rust SDK, boto3 >= 1.28.82, Java 2.x >= 2.21.19 | Older boto3, Java 1.x, PHP, older CLIs |

### 5.2 JSON Protocol Details

Request:
```http
POST / HTTP/1.1
Content-Type: application/x-amz-json-1.0
X-Amz-Target: AmazonSQS.SendMessage

{"QueueUrl":"http://localhost:4566/000000000000/my-queue","MessageBody":"hello"}
```

Success response:
```http
HTTP/1.1 200 OK
Content-Type: application/x-amz-json-1.0

{"MD5OfMessageBody":"5d41402abc4b2a76b9719d911017c592","MessageId":"uuid"}
```

Error response:
```http
HTTP/1.1 400 Bad Request
Content-Type: application/x-amz-json-1.0
x-amzn-query-error: AWS.SimpleQueueService.NonExistentQueue;Sender

{"__type":"AWS.SimpleQueueService.NonExistentQueue","message":"The specified queue does not exist."}
```

### 5.3 awsQuery Protocol Details (Legacy)

Request:
```http
POST / HTTP/1.1
Content-Type: application/x-www-form-urlencoded

Action=SendMessage&QueueUrl=http://localhost:4566/000000000000/my-queue&MessageBody=hello&Version=2012-11-05
```

Success response:
```http
HTTP/1.1 200 OK
Content-Type: text/xml

<SendMessageResponse xmlns="http://queue.amazonaws.com/doc/2012-11-05/">
  <SendMessageResult>
    <MD5OfMessageBody>5d41402abc4b2a76b9719d911017c592</MD5OfMessageBody>
    <MessageId>uuid</MessageId>
  </SendMessageResult>
  <ResponseMetadata><RequestId>uuid</RequestId></ResponseMetadata>
</SendMessageResponse>
```

Error response:
```http
HTTP/1.1 400 Bad Request
Content-Type: text/xml

<ErrorResponse xmlns="http://queue.amazonaws.com/doc/2012-11-05/">
  <Error>
    <Type>Sender</Type>
    <Code>AWS.SimpleQueueService.NonExistentQueue</Code>
    <Message>The specified queue does not exist.</Message>
  </Error>
  <RequestId>uuid</RequestId>
</ErrorResponse>
```

### 5.4 Protocol Detection Strategy

```rust
/// Determine the SQS protocol from request headers.
pub enum SqsProtocol {
    /// Modern: application/x-amz-json-1.0 with X-Amz-Target
    AwsJson1_0,
    /// Legacy: application/x-www-form-urlencoded with Action= parameter
    AwsQuery,
}

fn detect_protocol(req: &http::Request<()>) -> Option<SqsProtocol> {
    // Check for JSON protocol first (X-Amz-Target present)
    if let Some(target) = req.headers().get("x-amz-target") {
        if target.to_str().ok()?.starts_with("AmazonSQS.") {
            return Some(SqsProtocol::AwsJson1_0);
        }
    }
    // Check for Query protocol (form-urlencoded Content-Type)
    if let Some(ct) = req.headers().get("content-type") {
        if ct.to_str().ok()?.contains("x-www-form-urlencoded") {
            return Some(SqsProtocol::AwsQuery);
        }
    }
    None
}
```

### 5.5 awsQueryCompatible Error Header

For every JSON error response, we include the `x-amzn-query-error` header in the format `Code;Fault`:

- `Code`: The `@awsQueryError` code from the Smithy model (e.g., `AWS.SimpleQueueService.NonExistentQueue`)
- `Fault`: Either `Sender` (4xx) or `Receiver` (5xx)

This header allows clients using the awsQuery error parser to decode errors from JSON responses.

---

## 6. Smithy Code Generation Strategy

### 6.1 Approach: Extend Existing Multi-Service Codegen

The codegen tool already supports `--service s3` and `--service dynamodb`. SQS uses the same `@awsJson1_0` protocol as DynamoDB, so the existing JSON codegen path is directly reusable. The only addition is a new `SqsServiceConfig` implementing the `ServiceConfig` trait.

### 6.2 Changes to Codegen

```
codegen/
+-- src/
|   +-- services/
|   |   +-- mod.rs
|   |   +-- s3.rs
|   |   +-- dynamodb.rs
|   |   +-- sqs.rs            <-- NEW: SQS-specific config
+-- smithy-model/
|   +-- s3.json
|   +-- dynamodb.json
|   +-- sqs.json              <-- NEW: from aws/api-models-aws
```

### 6.3 SQS Service Configuration

```rust
pub struct SqsServiceConfig;

impl ServiceConfig for SqsServiceConfig {
    fn namespace(&self) -> &str { "com.amazonaws.sqs#" }
    fn service_name(&self) -> &str { "SQS" }
    fn target_operations(&self) -> &[&str] { &SQS_OPERATIONS }
    fn protocol(&self) -> Protocol { Protocol::AwsJson1_0 }
    // ...
}
```

### 6.4 Key Differences from DynamoDB Codegen

| Aspect | DynamoDB | SQS |
|--------|---------|-----|
| Namespace | `com.amazonaws.dynamodb#` | `com.amazonaws.sqs#` |
| Target prefix | `DynamoDB_20120810` | `AmazonSQS` |
| Operations | 66 | 23 |
| Special types | `AttributeValue` (hand-written) | None (all types are straightforward) |
| Error trait | Standard `__type` | `__type` + `x-amzn-query-error` header |
| Serde | `#[serde(rename_all = "PascalCase")]` | `#[serde(rename_all = "PascalCase")]` |

SQS has no equivalent of DynamoDB's `AttributeValue` tagged union. All SQS types are standard structs and enums that the codegen can handle automatically.

### 6.5 Smithy Model Acquisition

The SQS Smithy model is available from:

1. **aws/api-models-aws**: `models/sqs/service/2012-11-05/sqs-2012-11-05.json`
2. **smithy-rs**: Bundled in the smithy-rs codegen

We download the SQS Smithy JSON AST and place it at `codegen/smithy-model/sqs.json`.

### 6.6 Generated Types Example

```rust
/// SQS SendMessageInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SendMessageInput {
    pub queue_url: String,
    pub message_body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_deduplication_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub message_attributes: HashMap<String, MessageAttributeValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_system_attributes: Vec<MessageSystemAttributeValue>,
}

/// SQS SendMessageOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SendMessageOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(rename = "MD5OfMessageBody")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md5_of_message_body: Option<String>,
    #[serde(rename = "MD5OfMessageAttributes")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md5_of_message_attributes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<String>,
}
```

### 6.7 Makefile Integration

```makefile
codegen-sqs:
    @cd codegen && cargo run -- --service sqs
    @cargo +nightly fmt -p rustack-sqs-model

codegen: codegen-s3 codegen-dynamodb codegen-sqs
```

---

## 7. Crate Structure

### 7.1 New Crates

#### `rustack-sqs-model` (auto-generated)

```
crates/rustack-sqs-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs                    # Module re-exports
    +-- types.rs                  # Auto-generated: MessageAttributeValue, QueueAttributeName, etc.
    +-- operations.rs             # Auto-generated: SqsOperation enum
    +-- error.rs                  # Auto-generated: SqsError + error codes with awsQueryError mapping
    +-- input/
    |   +-- mod.rs
    |   +-- queue.rs              # CreateQueueInput, DeleteQueueInput, ListQueuesInput, etc.
    |   +-- message.rs            # SendMessageInput, ReceiveMessageInput, DeleteMessageInput, etc.
    |   +-- batch.rs              # SendMessageBatchInput, DeleteMessageBatchInput, etc.
    |   +-- visibility.rs         # ChangeMessageVisibilityInput, ChangeMessageVisibilityBatchInput
    |   +-- tags.rs               # TagQueueInput, UntagQueueInput, ListQueueTagsInput
    |   +-- permissions.rs        # AddPermissionInput, RemovePermissionInput
    |   +-- dlq.rs                # ListDeadLetterSourceQueuesInput, message move task inputs
    +-- output/
        +-- mod.rs
        +-- queue.rs
        +-- message.rs
        +-- batch.rs
        +-- visibility.rs
        +-- tags.rs
        +-- permissions.rs
        +-- dlq.rs
```

**Dependencies**: `serde`, `serde_json`, `bytes`, `http`

#### `rustack-sqs-http`

```
crates/rustack-sqs-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs                 # X-Amz-Target / Action dispatch -> SqsOperation
    +-- dispatch.rs               # SqsHandler trait + dispatch logic
    +-- service.rs                # Hyper Service impl for SQS
    +-- request.rs                # Protocol-aware deserialization (JSON + Query)
    +-- response.rs               # Protocol-aware serialization (JSON + XML)
    +-- error.rs                  # Error response with x-amzn-query-error header
    +-- query/
    |   +-- mod.rs
    |   +-- deserialize.rs        # awsQuery form-urlencoded -> typed input structs
    |   +-- serialize.rs          # typed output structs -> XML response
    +-- body.rs                   # Response body type
```

**Dependencies**: `rustack-sqs-model`, `rustack-auth`, `hyper`, `serde_json`, `serde_urlencoded`, `quick-xml`, `bytes`

#### `rustack-sqs-core`

```
crates/rustack-sqs-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs                 # SqsConfig
    +-- provider.rs               # RustackSqs (main provider, QueueManager actor)
    +-- error.rs                  # SqsServiceError
    +-- queue/
    |   +-- mod.rs
    |   +-- actor.rs              # QueueActor: per-queue message lifecycle management
    |   +-- standard.rs           # Standard queue message storage and delivery
    |   +-- fifo.rs               # FIFO queue: message groups, deduplication
    |   +-- attributes.rs         # Queue attribute management and validation
    |   +-- url.rs                # Queue URL/ARN generation and parsing
    +-- message/
    |   +-- mod.rs
    |   +-- storage.rs            # Message storage (VecDeque, BTreeMap for delayed)
    |   +-- inflight.rs           # In-flight tracking: receipt handle -> visibility timer
    |   +-- delay.rs              # Delay queue: BTreeMap<Instant, Message> for timed release
    |   +-- dedup.rs              # Deduplication cache (FIFO): dedup_id -> expiry
    |   +-- md5.rs                # MD5 computation for body and attributes
    +-- polling/
    |   +-- mod.rs
    |   +-- long_poll.rs          # Long polling: Notify-based consumer wakeup
    +-- dlq/
    |   +-- mod.rs
    |   +-- redrive.rs            # DLQ redrive policy: maxReceiveCount tracking
    |   +-- move_task.rs          # MessageMoveTask actor for DLQ -> source queue
    +-- ops/
        +-- mod.rs
        +-- queue.rs              # CreateQueue, DeleteQueue, GetQueueUrl, ListQueues, PurgeQueue
        +-- attributes.rs         # GetQueueAttributes, SetQueueAttributes
        +-- send.rs               # SendMessage, SendMessageBatch
        +-- receive.rs            # ReceiveMessage (short + long polling)
        +-- delete.rs             # DeleteMessage, DeleteMessageBatch
        +-- visibility.rs         # ChangeMessageVisibility, ChangeMessageVisibilityBatch
        +-- tags.rs               # TagQueue, UntagQueue, ListQueueTags
        +-- permissions.rs        # AddPermission, RemovePermission
        +-- dlq.rs                # ListDeadLetterSourceQueues, message move task ops
```

**Dependencies**: `rustack-core`, `rustack-sqs-model`, `tokio` (mpsc, Notify, time, sync), `dashmap`, `uuid`, `md-5`, `sha2`, `tracing`, `chrono`

### 7.2 Workspace Changes

```toml
# Root Cargo.toml
[workspace.dependencies]
# ... existing deps ...
rustack-sqs-model = { path = "crates/rustack-sqs-model" }
rustack-sqs-http = { path = "crates/rustack-sqs-http" }
rustack-sqs-core = { path = "crates/rustack-sqs-core" }
quick-xml = "0.37"
serde_urlencoded = "0.7"
md-5 = "0.10"
sha2 = "0.10"
```

---

## 8. HTTP Layer Design

### 8.1 SQS Router

SQS uses the same POST-to-root dispatch as DynamoDB, but with a different target prefix and dual protocol support:

```rust
/// SQS operation router.
///
/// Supports two dispatch mechanisms:
/// 1. JSON protocol: `X-Amz-Target: AmazonSQS.<OperationName>`
/// 2. Query protocol: `Action=<OperationName>` form parameter
pub struct SqsRouter;

impl SqsRouter {
    /// Resolve an HTTP request to an SQS operation and protocol.
    pub fn resolve(
        req: &http::Request<()>,
        body: &[u8],
    ) -> Result<(SqsOperation, SqsProtocol), SqsError> {
        // 1. Try JSON protocol (X-Amz-Target header)
        if let Some(target) = req.headers().get("x-amz-target") {
            let target = target.to_str().map_err(|_| SqsError::missing_action())?;
            let op_name = target
                .strip_prefix("AmazonSQS.")
                .ok_or_else(|| SqsError::unknown_operation(target))?;
            let op = SqsOperation::from_name(op_name)
                .ok_or_else(|| SqsError::unknown_operation(op_name))?;
            return Ok((op, SqsProtocol::AwsJson1_0));
        }

        // 2. Try Query protocol (Action= form parameter)
        let params = serde_urlencoded::from_bytes::<Vec<(String, String)>>(body)
            .map_err(|_| SqsError::missing_action())?;
        let action = params.iter()
            .find(|(k, _)| k == "Action")
            .map(|(_, v)| v.as_str())
            .ok_or_else(SqsError::missing_action)?;
        let op = SqsOperation::from_name(action)
            .ok_or_else(|| SqsError::unknown_operation(action))?;
        Ok((op, SqsProtocol::AwsQuery))
    }
}
```

### 8.2 Protocol-Aware Request Deserialization

```rust
/// Deserialize an SQS request from either JSON or Query format.
pub trait FromSqsRequest: Sized {
    /// Deserialize from JSON body (awsJson1_0).
    fn from_json(body: &[u8]) -> Result<Self, SqsError>;
    /// Deserialize from URL-encoded form body (awsQuery).
    fn from_query(params: &[(String, String)]) -> Result<Self, SqsError>;
}
```

For JSON protocol, this is a trivial `serde_json::from_slice`. For Query protocol, each input type needs a manual implementation that maps flat form parameters to the nested struct. The Query deseriizer handles the AWS naming convention where nested fields use dot notation (e.g., `MessageAttribute.1.Name`, `MessageAttribute.1.Value.StringValue`).

### 8.3 Protocol-Aware Response Serialization

```rust
/// Serialize an SQS response to either JSON or XML format.
pub trait IntoSqsResponse: Sized + Serialize {
    fn into_response(self, protocol: SqsProtocol) -> Result<http::Response<Bytes>, SqsError> {
        match protocol {
            SqsProtocol::AwsJson1_0 => self.into_json_response(),
            SqsProtocol::AwsQuery => self.into_xml_response(),
        }
    }
}
```

### 8.4 SqsHandler Trait

```rust
/// The boundary between HTTP and business logic.
///
/// Analogous to DynamoDBHandler but for SQS operations.
/// The handler is protocol-agnostic -- it receives typed inputs and returns typed outputs.
/// The HTTP layer handles protocol detection, deserialization, and serialization.
pub trait SqsHandler: Send + Sync + 'static {
    fn handle_operation(
        &self,
        op: SqsOperation,
        body: Bytes,
        protocol: SqsProtocol,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<Bytes>, SqsError>> + Send>>;
}
```

### 8.5 Service Integration

```rust
/// Hyper Service implementation for SQS.
pub struct SqsHttpService<H> {
    handler: Arc<H>,
    config: SqsHttpConfig,
}

pub struct SqsHttpConfig {
    pub skip_signature_validation: bool,
    pub region: String,
    pub account_id: String,
    pub credential_provider: Option<Arc<dyn CredentialProvider>>,
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

The storage engine implements a message queue with visibility timeout tracking, delay timers, and FIFO ordering. Unlike DynamoDB's indexed storage, the SQS engine is time-oriented: messages flow through states (delayed -> available -> in-flight -> deleted) based on timers.

### 9.2 Message Lifecycle State Machine

```
                  SendMessage
                      |
                      v
              +---------------+
              |   DELAYED     |  (if DelaySeconds > 0)
              |  BTreeMap by  |
              |   release_at  |
              +-------+-------+
                      | (delay expires)
                      v
              +---------------+
              |   AVAILABLE   |  (VecDeque for standard, BTreeMap<group> for FIFO)
              |   ready for   |
              |   receive     |
              +-------+-------+
                      | ReceiveMessage
                      v
              +---------------+
              |   IN-FLIGHT   |  (HashMap<ReceiptHandle, InFlightMessage>)
              |   invisible   |
              |   to others   |
              +-------+-------+
                     /|\
                    / | \
                   /  |  \
    DeleteMessage/   |   \VisibilityTimeout expires
                v    |    v
         +--------+  |  +------------+
         | DELETED |  |  | AVAILABLE  | (re-enqueued, receive_count++)
         +--------+  |  +------------+
                      |
                      | receive_count > maxReceiveCount
                      v
              +---------------+
              |   DLQ         |  (moved to dead-letter queue)
              +---------------+
```

### 9.3 Core Data Structures

```rust
/// A single SQS message.
#[derive(Debug, Clone)]
pub struct QueueMessage {
    /// Unique message identifier (UUID).
    pub message_id: String,
    /// Message body (up to 256KiB).
    pub body: String,
    /// MD5 hex digest of the body.
    pub md5_of_body: String,
    /// User-defined message attributes (up to 10).
    pub message_attributes: HashMap<String, MessageAttributeValue>,
    /// MD5 hex digest of message attributes.
    pub md5_of_message_attributes: Option<String>,
    /// System attributes tracked internally.
    pub sender_id: String,
    pub sent_timestamp: u64,
    pub approximate_receive_count: u32,
    pub approximate_first_receive_timestamp: Option<u64>,
    /// FIFO-only fields.
    pub sequence_number: Option<String>,
    pub message_group_id: Option<String>,
    pub message_deduplication_id: Option<String>,
    /// DLQ source tracking.
    pub dead_letter_queue_source_arn: Option<String>,
    /// When this message becomes available (for delayed messages).
    pub available_at: Instant,
}

/// An in-flight message with its visibility timeout.
#[derive(Debug)]
pub struct InFlightMessage {
    pub message: QueueMessage,
    pub receipt_handle: String,
    pub visible_at: Instant,
}

/// Queue metadata and attributes.
#[derive(Debug, Clone)]
pub struct QueueAttributes {
    pub delay_seconds: i32,               // 0-900, default 0
    pub maximum_message_size: i32,        // 1024-1048576, default 262144
    pub message_retention_period: i32,    // 60-1209600, default 345600 (4 days)
    pub receive_message_wait_time_seconds: i32,  // 0-20, default 0
    pub visibility_timeout: i32,          // 0-43200, default 30
    pub redrive_policy: Option<RedrivePolicy>,
    pub redrive_allow_policy: Option<RedriveAllowPolicy>,
    pub content_based_deduplication: bool, // FIFO only
    pub deduplication_scope: DeduplicationScope,  // FIFO only
    pub fifo_throughput_limit: FifoThroughputLimit, // FIFO only
    pub policy: Option<String>,           // IAM policy JSON (stored, not enforced)
    pub kms_master_key_id: Option<String>,        // Stored, not enforced
    pub kms_data_key_reuse_period_seconds: Option<i32>,
    pub sqs_managed_sse_enabled: Option<bool>,
}

/// Dead-letter queue redrive configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedrivePolicy {
    pub dead_letter_target_arn: String,
    pub max_receive_count: i32,
}
```

### 9.4 Standard Queue Storage

Standard queues provide best-effort ordering with at-least-once delivery:

```rust
/// Standard queue storage.
pub struct StandardQueueStorage {
    /// Available messages in approximate FIFO order.
    available: VecDeque<QueueMessage>,
    /// Messages not yet available (per-message delay or queue-level delay).
    delayed: BTreeMap<Instant, Vec<QueueMessage>>,
    /// Messages currently being processed by consumers.
    in_flight: HashMap<String, InFlightMessage>,  // receipt_handle -> message
    /// Approximate counters (atomics for lock-free reads).
    available_count: AtomicU32,
    in_flight_count: AtomicU32,
    delayed_count: AtomicU32,
}
```

### 9.5 FIFO Queue Storage

FIFO queues maintain strict ordering per message group with exactly-once deduplication:

```rust
/// FIFO queue storage.
pub struct FifoQueueStorage {
    /// Per-group message queues. Messages within a group are strictly ordered.
    groups: BTreeMap<String, VecDeque<QueueMessage>>,
    /// Groups with at least one message in-flight (blocked for further delivery).
    blocked_groups: HashSet<String>,
    /// Delayed messages (queue-level delay only; per-message delay not supported).
    delayed: BTreeMap<Instant, Vec<QueueMessage>>,
    /// In-flight messages.
    in_flight: HashMap<String, InFlightMessage>,
    /// Deduplication cache: dedup_id -> expiry timestamp.
    /// Entries expire after 5 minutes.
    dedup_cache: HashMap<String, Instant>,
    /// Monotonically increasing sequence number.
    next_sequence: AtomicU64,
    /// Approximate counters.
    available_count: AtomicU32,
    in_flight_count: AtomicU32,
    delayed_count: AtomicU32,
}
```

### 9.6 Queue Actor

Each queue runs as an independent actor, following the actor model pattern from CLAUDE.md:

```rust
/// Commands sent to a queue actor via its channel.
pub enum QueueCommand {
    SendMessage {
        input: SendMessageInput,
        reply: oneshot::Sender<Result<SendMessageOutput, SqsServiceError>>,
    },
    ReceiveMessage {
        input: ReceiveMessageInput,
        reply: oneshot::Sender<Result<ReceiveMessageOutput, SqsServiceError>>,
    },
    DeleteMessage {
        receipt_handle: String,
        reply: oneshot::Sender<Result<(), SqsServiceError>>,
    },
    ChangeVisibility {
        receipt_handle: String,
        visibility_timeout: i32,
        reply: oneshot::Sender<Result<(), SqsServiceError>>,
    },
    GetAttributes {
        attribute_names: Vec<String>,
        reply: oneshot::Sender<Result<HashMap<String, String>, SqsServiceError>>,
    },
    SetAttributes {
        attributes: HashMap<String, String>,
        reply: oneshot::Sender<Result<(), SqsServiceError>>,
    },
    Purge {
        reply: oneshot::Sender<Result<(), SqsServiceError>>,
    },
    Shutdown,
}

/// Per-queue actor that owns all message state.
pub struct QueueActor {
    /// Queue name and metadata.
    name: String,
    url: String,
    arn: String,
    is_fifo: bool,
    /// Queue attributes.
    attributes: QueueAttributes,
    /// Message storage (standard or FIFO).
    storage: QueueStorage,
    /// Command channel receiver.
    commands: mpsc::Receiver<QueueCommand>,
    /// Notification for long-polling consumers.
    message_notify: Arc<Notify>,
    /// Tags.
    tags: HashMap<String, String>,
    /// Timestamps.
    created_at: u64,
    last_modified_at: u64,
    last_purge_at: Option<Instant>,
    /// Shutdown signal.
    shutdown: AtomicBool,
}

enum QueueStorage {
    Standard(StandardQueueStorage),
    Fifo(FifoQueueStorage),
}
```

### 9.7 Queue Actor Event Loop

```rust
impl QueueActor {
    pub async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                // Handle incoming commands.
                Some(cmd) = self.commands.recv() => {
                    match cmd {
                        QueueCommand::Shutdown => break,
                        cmd => self.handle_command(cmd).await,
                    }
                }
                // Periodic cleanup: expired visibility, expired dedup entries,
                // promote delayed messages to available.
                _ = cleanup_interval.tick() => {
                    self.cleanup_expired_visibility();
                    self.promote_delayed_messages();
                    self.cleanup_expired_dedup_entries();
                    self.cleanup_expired_retention();
                }
            }
        }
    }
}
```

### 9.8 Long Polling Implementation

Long polling holds the HTTP connection open until messages arrive or the timeout expires. The key is the `Notify` primitive:

```rust
impl QueueActor {
    async fn handle_receive(
        &mut self,
        input: ReceiveMessageInput,
        reply: oneshot::Sender<Result<ReceiveMessageOutput, SqsServiceError>>,
    ) {
        let max_messages = input.max_number_of_messages.unwrap_or(1).min(10).max(1);
        let wait_time = input.wait_time_seconds
            .unwrap_or(self.attributes.receive_message_wait_time_seconds);

        // Try immediate receive.
        let messages = self.try_receive(max_messages);
        if !messages.is_empty() || wait_time == 0 {
            let _ = reply.send(Ok(self.build_receive_output(messages)));
            return;
        }

        // Long poll: spawn a task that waits for notification or timeout.
        let notify = Arc::clone(&self.message_notify);
        let timeout = Duration::from_secs(wait_time as u64);
        // The long-poll waiter re-sends the command after wakeup.
        // Implementation detail: store the pending reply and wake it
        // when messages arrive or timeout occurs.
        self.pending_long_polls.push(PendingLongPoll {
            reply,
            max_messages,
            deadline: Instant::now() + timeout,
            attribute_names: input.attribute_names,
            message_attribute_names: input.message_attribute_names,
        });
    }

    /// Called when a message is enqueued. Wakes all long-polling consumers.
    fn notify_consumers(&self) {
        self.message_notify.notify_waiters();
    }
}
```

### 9.9 Receipt Handle Generation

Receipt handles must be unique per receive operation and become invalid after the message is deleted or re-received:

```rust
/// Generate a receipt handle encoding the message ID and receive timestamp.
///
/// Format: base64(message_id + ":" + receive_timestamp_nanos + ":" + random_suffix)
fn generate_receipt_handle(message_id: &str) -> String {
    let nonce = uuid::Uuid::new_v4().to_string();
    let raw = format!("{}:{}:{}", message_id, Instant::now().elapsed().as_nanos(), nonce);
    base64_encode(&raw)
}
```

### 9.10 Queue URL and ARN Format

Following LocalStack conventions for local development:

```rust
/// Queue URL format: http://<host>:<port>/<account-id>/<queue-name>
fn queue_url(host: &str, port: u16, account_id: &str, queue_name: &str) -> String {
    format!("http://{}:{}/{}/{}", host, port, account_id, queue_name)
}

/// Queue ARN format: arn:aws:sqs:<region>:<account-id>:<queue-name>
fn queue_arn(region: &str, account_id: &str, queue_name: &str) -> String {
    format!("arn:aws:sqs:{}:{}:{}", region, account_id, queue_name)
}
```

Default account ID: `000000000000` (matching LocalStack convention).
Default queue URL example: `http://localhost:4566/000000000000/my-queue`.

---

## 10. Core Business Logic

### 10.1 Provider (QueueManager Actor)

```rust
/// Main SQS provider. Acts as the QueueManager actor that owns all queue actors.
pub struct RustackSqs {
    /// Queue registry: queue_name -> (QueueHandle, QueueMetadata).
    queues: DashMap<String, QueueHandle>,
    /// Configuration.
    config: Arc<SqsConfig>,
}

/// Handle to a running queue actor.
pub struct QueueHandle {
    /// Channel to send commands to the queue actor.
    sender: mpsc::Sender<QueueCommand>,
    /// Notify for long-polling wakeup (shared with actor).
    message_notify: Arc<Notify>,
    /// Queue metadata (read-only after creation).
    metadata: QueueMetadata,
    /// Actor task join handle.
    task: tokio::task::JoinHandle<()>,
}

pub struct QueueMetadata {
    pub name: String,
    pub url: String,
    pub arn: String,
    pub is_fifo: bool,
    pub created_at: u64,
}
```

### 10.2 Operations Grouped by Category

#### Queue Management (7 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreateQueue` | 0 | Medium | Spawn queue actor, validate attributes, handle idempotent creates |
| `DeleteQueue` | 0 | Low | Send Shutdown to actor, remove from registry |
| `GetQueueUrl` | 0 | Low | Lookup by name (and optional account) |
| `ListQueues` | 0 | Low | Filter by prefix, paginate with NextToken |
| `GetQueueAttributes` | 0 | Low | Forward to queue actor, return requested attributes |
| `SetQueueAttributes` | 0 | Medium | Validate attribute values, forward to queue actor |
| `PurgeQueue` | 1 | Low | Enforce 60s cooldown, clear all messages |

#### Message Operations (6 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `SendMessage` | 0 | Medium | Validate size, compute MD5, handle delay, FIFO dedup |
| `SendMessageBatch` | 1 | Medium | Up to 10 messages, per-entry success/failure, 256KiB total limit |
| `ReceiveMessage` | 0 | High | Short/long polling, visibility timeout, system attributes |
| `DeleteMessage` | 0 | Low | Validate receipt handle, remove from in-flight |
| `DeleteMessageBatch` | 1 | Low | Up to 10 deletes, per-entry results |
| `ChangeMessageVisibility` | 1 | Low | Validate receipt handle, update visibility timer |

#### Batch Visibility (1 operation)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `ChangeMessageVisibilityBatch` | 1 | Low | Up to 10 visibility changes, per-entry results |

#### Permissions (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `AddPermission` | 3 | Low | Store in Policy attribute, no enforcement |
| `RemovePermission` | 3 | Low | Remove from Policy attribute |

#### Tagging (3 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `TagQueue` | 3 | Low | Add/update tags (max 50) |
| `UntagQueue` | 3 | Low | Remove specified tag keys |
| `ListQueueTags` | 3 | Low | Return all tags |

#### Dead-Letter Queue Management (4 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `ListDeadLetterSourceQueues` | 1 | Medium | Scan all queues for matching RedrivePolicy |
| `StartMessageMoveTask` | 3 | High | Spawn background task actor with rate limiting |
| `CancelMessageMoveTask` | 3 | Medium | Send cancel signal to move task actor |
| `ListMessageMoveTasks` | 3 | Low | Return recent task status |

### 10.3 CreateQueue Logic

```rust
impl RustackSqs {
    pub async fn create_queue(
        &self,
        input: CreateQueueInput,
    ) -> Result<CreateQueueOutput, SqsServiceError> {
        let queue_name = &input.queue_name;
        let is_fifo = queue_name.ends_with(".fifo");

        // Validate queue name: 1-80 chars, alphanumeric + hyphens + underscores.
        validate_queue_name(queue_name)?;

        // Validate FIFO-specific attributes.
        if is_fifo {
            validate_fifo_attributes(&input.attributes)?;
        }

        // Idempotent create: if queue exists with same attributes, return existing URL.
        // If queue exists with different attributes, return QueueNameExists error.
        if let Some(existing) = self.queues.get(queue_name) {
            if attributes_match(&existing, &input.attributes) {
                return Ok(CreateQueueOutput { queue_url: existing.metadata.url.clone() });
            }
            return Err(SqsServiceError::QueueAlreadyExists {
                name: queue_name.clone(),
            });
        }

        // Build queue attributes with defaults.
        let attributes = QueueAttributes::from_input(input.attributes, is_fifo)?;
        let url = queue_url(&self.config.host, self.config.port,
                           &self.config.account_id, queue_name);
        let arn = queue_arn(&self.config.region, &self.config.account_id, queue_name);

        // Spawn queue actor.
        let (sender, receiver) = mpsc::channel(256);
        let notify = Arc::new(Notify::new());
        let actor = QueueActor::new(
            queue_name.clone(), url.clone(), arn.clone(),
            is_fifo, attributes, receiver, Arc::clone(&notify),
            input.tags.unwrap_or_default(),
        );
        let task = tokio::spawn(actor.run());

        let handle = QueueHandle {
            sender, message_notify: notify,
            metadata: QueueMetadata {
                name: queue_name.clone(), url: url.clone(), arn,
                is_fifo, created_at: now_epoch_seconds(),
            },
            task,
        };
        self.queues.insert(queue_name.clone(), handle);

        Ok(CreateQueueOutput { queue_url: url })
    }
}
```

### 10.4 ReceiveMessage Logic (Short + Long Polling)

```rust
impl QueueActor {
    fn try_receive(&mut self, max_messages: i32) -> Vec<ReceivedMessage> {
        let mut result = Vec::new();
        let visibility_timeout = Duration::from_secs(
            self.attributes.visibility_timeout as u64
        );

        match &mut self.storage {
            QueueStorage::Standard(storage) => {
                while result.len() < max_messages as usize {
                    match storage.available.pop_front() {
                        Some(mut msg) => {
                            msg.approximate_receive_count += 1;
                            if msg.approximate_first_receive_timestamp.is_none() {
                                msg.approximate_first_receive_timestamp =
                                    Some(now_epoch_millis());
                            }
                            // Check DLQ redrive.
                            if let Some(ref policy) = self.attributes.redrive_policy {
                                if msg.approximate_receive_count > policy.max_receive_count as u32 {
                                    self.move_to_dlq(msg, &policy.dead_letter_target_arn);
                                    continue;
                                }
                            }
                            let receipt_handle = generate_receipt_handle(&msg.message_id);
                            let in_flight = InFlightMessage {
                                message: msg.clone(),
                                receipt_handle: receipt_handle.clone(),
                                visible_at: Instant::now() + visibility_timeout,
                            };
                            storage.in_flight.insert(receipt_handle.clone(), in_flight);
                            result.push(ReceivedMessage { message: msg, receipt_handle });
                        }
                        None => break,
                    }
                }
            }
            QueueStorage::Fifo(storage) => {
                // Deliver from non-blocked groups in order.
                for (group_id, messages) in storage.groups.iter_mut() {
                    if storage.blocked_groups.contains(group_id) {
                        continue;
                    }
                    if result.len() >= max_messages as usize {
                        break;
                    }
                    if let Some(mut msg) = messages.pop_front() {
                        msg.approximate_receive_count += 1;
                        let receipt_handle = generate_receipt_handle(&msg.message_id);
                        storage.blocked_groups.insert(group_id.clone());
                        storage.in_flight.insert(receipt_handle.clone(), InFlightMessage {
                            message: msg.clone(),
                            receipt_handle: receipt_handle.clone(),
                            visible_at: Instant::now() + visibility_timeout,
                        });
                        result.push(ReceivedMessage { message: msg, receipt_handle });
                    }
                }
            }
        }
        result
    }
}
```

### 10.5 Visibility Timeout Expiry

The cleanup loop in the queue actor periodically checks for expired in-flight messages:

```rust
impl QueueActor {
    fn cleanup_expired_visibility(&mut self) {
        let now = Instant::now();
        let storage = match &mut self.storage {
            QueueStorage::Standard(s) => {
                let expired: Vec<String> = s.in_flight.iter()
                    .filter(|(_, ifm)| ifm.visible_at <= now)
                    .map(|(handle, _)| handle.clone())
                    .collect();
                for handle in expired {
                    if let Some(ifm) = s.in_flight.remove(&handle) {
                        // Re-enqueue the message (at-least-once delivery).
                        s.available.push_back(ifm.message);
                        self.message_notify.notify_waiters();
                    }
                }
            }
            QueueStorage::Fifo(s) => {
                let expired: Vec<String> = s.in_flight.iter()
                    .filter(|(_, ifm)| ifm.visible_at <= now)
                    .map(|(handle, _)| handle.clone())
                    .collect();
                for handle in expired {
                    if let Some(ifm) = s.in_flight.remove(&handle) {
                        let group_id = ifm.message.message_group_id.clone()
                            .unwrap_or_default();
                        // Re-enqueue to group and unblock.
                        s.groups.entry(group_id.clone())
                            .or_default()
                            .push_front(ifm.message);
                        s.blocked_groups.remove(&group_id);
                        self.message_notify.notify_waiters();
                    }
                }
            }
        };
    }
}
```

### 10.6 FIFO Deduplication

```rust
impl FifoQueueStorage {
    /// Check deduplication and insert message if not a duplicate.
    /// Returns true if the message was accepted (not a duplicate).
    fn send_with_dedup(
        &mut self,
        msg: QueueMessage,
        dedup_id: &str,
        dedup_scope: DeduplicationScope,
    ) -> Result<bool, SqsServiceError> {
        let cache_key = match dedup_scope {
            DeduplicationScope::Queue => dedup_id.to_string(),
            DeduplicationScope::MessageGroup => {
                format!("{}:{}", msg.message_group_id.as_deref().unwrap_or(""), dedup_id)
            }
        };

        // Check 5-minute dedup window.
        if let Some(expiry) = self.dedup_cache.get(&cache_key) {
            if *expiry > Instant::now() {
                // Duplicate within window: accept but do not deliver.
                return Ok(false);
            }
        }

        // Insert dedup entry with 5-minute TTL.
        self.dedup_cache.insert(cache_key, Instant::now() + Duration::from_secs(300));

        // Assign sequence number and enqueue.
        let group_id = msg.message_group_id.clone().unwrap_or_default();
        self.groups.entry(group_id).or_default().push_back(msg);
        Ok(true)
    }
}
```

### 10.7 MD5 Computation

SQS requires MD5 hashes for message bodies and attributes. The attribute MD5 follows a specific binary encoding:

```rust
/// Compute MD5 of message body.
fn md5_of_body(body: &str) -> String {
    format!("{:x}", md5::compute(body.as_bytes()))
}

/// Compute MD5 of message attributes following AWS specification.
/// Attributes are sorted by name, then each is encoded as:
///   length(name) as 4-byte big-endian + UTF-8(name)
///   length(data_type) as 4-byte big-endian + UTF-8(data_type)
///   1 byte transport type (1=String, 2=Binary)
///   length(value) as 4-byte big-endian + value_bytes
fn md5_of_message_attributes(
    attrs: &HashMap<String, MessageAttributeValue>,
) -> Option<String> {
    if attrs.is_empty() {
        return None;
    }
    let mut sorted: Vec<_> = attrs.iter().collect();
    sorted.sort_by_key(|(k, _)| *k);

    let mut hasher = md5::Context::new();
    for (name, value) in sorted {
        // Encode name
        hasher.consume(&(name.len() as u32).to_be_bytes());
        hasher.consume(name.as_bytes());
        // Encode data type
        hasher.consume(&(value.data_type.len() as u32).to_be_bytes());
        hasher.consume(value.data_type.as_bytes());
        // Encode transport type + value
        if let Some(ref string_value) = value.string_value {
            hasher.consume(&[1u8]); // STRING type
            hasher.consume(&(string_value.len() as u32).to_be_bytes());
            hasher.consume(string_value.as_bytes());
        } else if let Some(ref binary_value) = value.binary_value {
            hasher.consume(&[2u8]); // BINARY type
            hasher.consume(&(binary_value.len() as u32).to_be_bytes());
            hasher.consume(binary_value);
        }
    }
    Some(format!("{:x}", hasher.compute()))
}
```

---

## 11. Error Handling

### 11.1 SQS Error Codes

SQS errors use a different naming convention than DynamoDB. Most errors use the `AWS.SimpleQueueService.` prefix.

```rust
/// SQS error codes with awsQueryCompatible mapping.
pub enum SqsErrorCode {
    /// Queue does not exist.
    NonExistentQueue,
    /// Queue already exists with different attributes.
    QueueAlreadyExists,
    /// Queue deleted within last 60 seconds.
    QueueDeletedRecently,
    /// Invalid parameter value.
    InvalidParameterValue,
    /// Required parameter missing.
    MissingParameter,
    /// Invalid attribute name.
    InvalidAttributeName,
    /// Invalid attribute value.
    InvalidAttributeValue,
    /// Message is not currently in flight.
    MessageNotInflight,
    /// Receipt handle is invalid.
    ReceiptHandleIsInvalid,
    /// Batch request contains no entries.
    EmptyBatchRequest,
    /// More than 10 entries in batch.
    TooManyEntriesInBatchRequest,
    /// Duplicate IDs in batch request.
    BatchEntryIdsNotDistinct,
    /// Batch request exceeds size limit.
    BatchRequestTooLong,
    /// Invalid batch entry ID format.
    InvalidBatchEntryId,
    /// Another purge within 60 seconds.
    PurgeQueueInProgress,
    /// Queue limit exceeded.
    OverLimit,
    /// Message move task not found.
    ResourceNotFoundException,
    /// Unsupported operation for queue type.
    UnsupportedOperation,
    /// Internal server error.
    InternalError,
}
```

### 11.2 Error Type Mapping

Each error code maps to a JSON `__type` string and an `x-amzn-query-error` header value:

```rust
impl SqsErrorCode {
    /// JSON `__type` field value.
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::NonExistentQueue =>
                "AWS.SimpleQueueService.NonExistentQueue",
            Self::QueueAlreadyExists =>
                "QueueAlreadyExists",
            Self::QueueDeletedRecently =>
                "AWS.SimpleQueueService.QueueDeletedRecently",
            Self::PurgeQueueInProgress =>
                "AWS.SimpleQueueService.PurgeQueueInProgress",
            Self::ReceiptHandleIsInvalid =>
                "ReceiptHandleIsInvalid",
            // ... other mappings from Smithy model
        }
    }

    /// HTTP status code.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::NonExistentQueue => 400,
            Self::PurgeQueueInProgress => 403,
            Self::ResourceNotFoundException => 404,
            Self::InternalError => 500,
            _ => 400,
        }
    }

    /// Fault type for x-amzn-query-error header.
    pub fn fault(&self) -> &'static str {
        match self {
            Self::InternalError => "Receiver",
            _ => "Sender",
        }
    }

    /// Full x-amzn-query-error header value: "Code;Fault".
    pub fn query_error_header(&self) -> String {
        format!("{};{}", self.error_type(), self.fault())
    }
}
```

### 11.3 Error Response Formatting

```rust
/// Format an SQS error response for JSON protocol.
fn json_error_response(error: &SqsError) -> http::Response<Bytes> {
    let body = serde_json::json!({
        "__type": error.code.error_type(),
        "message": error.message,
    });

    http::Response::builder()
        .status(error.code.status_code())
        .header("content-type", "application/x-amz-json-1.0")
        .header("x-amzn-query-error", error.code.query_error_header())
        .body(Bytes::from(serde_json::to_vec(&body).expect("JSON serialization")))
        .expect("valid error response")
}

/// Format an SQS error response for Query protocol.
fn query_error_response(error: &SqsError) -> http::Response<Bytes> {
    let xml = format!(
        r#"<ErrorResponse xmlns="http://queue.amazonaws.com/doc/2012-11-05/">
  <Error>
    <Type>{}</Type>
    <Code>{}</Code>
    <Message>{}</Message>
  </Error>
  <RequestId>{}</RequestId>
</ErrorResponse>"#,
        error.code.fault(),
        error.code.error_type(),
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
/// Domain-level errors for SQS business logic.
#[derive(Debug, thiserror::Error)]
pub enum SqsServiceError {
    #[error("The specified queue does not exist")]
    QueueNotFound { name: String },
    #[error("A queue already exists with the same name and different attributes")]
    QueueAlreadyExists { name: String },
    #[error("Queue recently deleted, must wait 60 seconds")]
    QueueDeletedRecently { name: String },
    #[error("Invalid parameter: {message}")]
    InvalidParameter { message: String },
    #[error("Message body must not be empty")]
    EmptyMessageBody,
    #[error("Message body exceeds maximum size of {max} bytes")]
    MessageTooLarge { size: usize, max: usize },
    #[error("Receipt handle is invalid")]
    InvalidReceiptHandle,
    #[error("Message is not in flight")]
    MessageNotInFlight,
    #[error("PurgeQueue called within 60 seconds of previous purge")]
    PurgeQueueInProgress,
    #[error("Batch request must contain 1-10 entries")]
    InvalidBatchSize { count: usize },
    #[error("Duplicate IDs in batch request")]
    BatchIdsNotDistinct,
    #[error("FIFO queue name must end with .fifo")]
    InvalidFifoQueueName,
    #[error("MessageGroupId required for FIFO queue")]
    MissingMessageGroupId,
    #[error("Per-message delay not supported for FIFO queues")]
    FifoDelayNotSupported,
    #[error("Internal error: {message}")]
    Internal { message: String },
}
```

---

## 12. Server Integration

### 12.1 SQS ServiceRouter

```rust
#[cfg(feature = "sqs")]
mod sqs_router {
    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the SQS service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `AmazonSQS`
    /// OR whose Content-Type is form-urlencoded with a recognized SQS Action.
    pub struct SqsServiceRouter<H: SqsHandler> {
        inner: SqsHttpService<H>,
    }

    impl<H: SqsHandler> ServiceRouter for SqsServiceRouter<H> {
        fn name(&self) -> &'static str { "sqs" }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            // JSON protocol: X-Amz-Target starts with "AmazonSQS"
            if let Some(target) = req.headers().get("x-amz-target") {
                if let Ok(s) = target.to_str() {
                    if s.starts_with("AmazonSQS") {
                        return true;
                    }
                }
            }
            // Query protocol: Content-Type is form-urlencoded
            // (body parsing for Action= deferred to handler;
            //  we match on Content-Type + POST method + path=/)
            if *req.method() == http::Method::POST && req.uri().path() == "/" {
                if let Some(ct) = req.headers().get("content-type") {
                    if let Ok(s) = ct.to_str() {
                        if s.contains("x-www-form-urlencoded") {
                            // Cannot inspect body here (Incoming is not buffered).
                            // Route all form-urlencoded POSTs to / through SQS.
                            // If Action is not an SQS action, SQS handler returns
                            // appropriate error; gateway can fall through.
                            return true;
                        }
                    }
                }
            }
            false
        }

        fn call(&self, req: http::Request<Incoming>)
            -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
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

### 12.2 Feature Gate

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http", "dep:rustack-s3-model"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
sqs = ["dep:rustack-sqs-core", "dep:rustack-sqs-http"]
```

### 12.3 Gateway Registration Order

Services are registered in specificity order: most specific first, catch-all last.

```rust
fn build_gateway(config: &ServerConfig) -> GatewayService {
    let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

    #[cfg(feature = "dynamodb")]
    services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_service)));

    #[cfg(feature = "sqs")]
    services.push(Box::new(SqsServiceRouter::new(sqs_service)));

    #[cfg(feature = "s3")]
    services.push(Box::new(S3ServiceRouter::new(s3_service))); // catch-all, must be last

    GatewayService::new(services)
}
```

### 12.4 Configuration

```rust
/// SQS configuration.
pub struct SqsConfig {
    pub skip_signature_validation: bool,
    pub default_region: String,
    pub account_id: String,
    pub host: String,
    pub port: u16,
}

impl SqsConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("SQS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            host: env_str("GATEWAY_HOST", "localhost"),
            port: env_u16("GATEWAY_PORT", 4566),
        }
    }
}
```

### 12.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared with S3/DynamoDB) |
| `SQS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default AWS account ID for queue URLs |

### 12.6 Health Endpoint

The existing health endpoint automatically includes SQS when the feature is enabled:

```json
GET /_localstack/health

{
    "services": {
        "s3": "running",
        "dynamodb": "running",
        "sqs": "running"
    }
}
```

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **Queue actor**: Test message lifecycle (send, receive, delete, visibility expiry)
- **Standard queue storage**: Test FIFO-ish ordering, at-least-once delivery, re-enqueue on visibility expiry
- **FIFO queue storage**: Test message group ordering, group blocking/unblocking, deduplication window
- **Long polling**: Test immediate return when messages available, timeout when empty, wake on message arrival
- **Receipt handle**: Test generation, validation, invalidation after delete or re-receive
- **MD5 computation**: Test body MD5 and attribute MD5 against known AWS values
- **Queue attributes**: Test validation ranges, defaults, FIFO-specific attributes
- **Protocol detection**: Test JSON vs Query protocol routing
- **Query deserialization**: Test form-urlencoded parsing with nested attribute notation
- **XML serialization**: Test XML response format matches AWS output

### 13.2 Integration Tests with aws-sdk-sqs

```rust
// tests/integration/sqs_tests.rs
#[tokio::test]
#[ignore]
async fn test_sqs_standard_queue_lifecycle() {
    let client = aws_sdk_sqs::Client::new(&config);

    // Create queue
    let create = client.create_queue()
        .queue_name("test-queue")
        .send().await.unwrap();
    let queue_url = create.queue_url().unwrap();

    // Send message
    let send = client.send_message()
        .queue_url(queue_url)
        .message_body("hello world")
        .send().await.unwrap();
    assert!(send.message_id().is_some());

    // Receive message
    let recv = client.receive_message()
        .queue_url(queue_url)
        .max_number_of_messages(1)
        .send().await.unwrap();
    let messages = recv.messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body().unwrap(), "hello world");

    // Delete message
    client.delete_message()
        .queue_url(queue_url)
        .receipt_handle(messages[0].receipt_handle().unwrap())
        .send().await.unwrap();

    // Delete queue
    client.delete_queue().queue_url(queue_url).send().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_sqs_fifo_queue_ordering() {
    // Test strict ordering within message groups
    // Test group blocking during in-flight
    // Test deduplication within 5-minute window
}

#[tokio::test]
#[ignore]
async fn test_sqs_long_polling() {
    // Test that ReceiveMessage with WaitTimeSeconds blocks until
    // message arrives or timeout expires
}
```

### 13.3 Third-Party Test Suites

#### 13.3.1 LocalStack Test Suite (Primary)

The most comprehensive open-source SQS test suite. Already vendored at `vendors/localstack/tests/aws/services/sqs/`.

- **`test_sqs.py`** -- main comprehensive test module, likely hundreds of test cases covering standard queues, FIFO queues, batch operations, DLQ, visibility timeout, long polling, attributes, tags, permissions
- **`test_sqs_developer_api.py`** -- developer API tests
- **`test_sqs_move_task.py`** -- message move task operation tests
- **Framework**: pytest with snapshot testing

Adaptation strategy: same approach as DynamoDB -- run the Python test suite against Rustack's SQS endpoint, track pass/fail counts, progressively fix failures.

```makefile
test-sqs-localstack:
    @cd vendors/localstack && python -m pytest tests/aws/services/sqs/test_sqs.py \
        --endpoint-url=http://localhost:4566 -v
```

#### 13.3.2 ElasticMQ Test Suite (Secondary Validation)

- **Repository**: https://github.com/softwaremill/elasticmq
- **Location**: `rest/rest-sqs-testing-amazon-java-sdk/src/test/scala/org/elasticmq/rest/sqs/`
- **Language**: Scala using AWS Java SDK
- **Coverage**: Queue CRUD, message send/receive, visibility timeout, FIFO queues, content-based deduplication, message delay, DLQ, long polling, tags, batch operations

Adaptation strategy: extract test cases and translate to Rust integration tests or run via Java SDK test harness.

#### 13.3.3 AWS SDK Integration Tests

Write targeted tests using `aws-sdk-sqs` Rust crate:

```rust
// Test each operation against known AWS behavior
// Focus on edge cases: empty queues, max batch sizes, invalid parameters,
// FIFO dedup window, visibility timeout expiry, long poll timeout
```

#### 13.3.4 AWS CLI Smoke Tests

Shell-based end-to-end tests for CI:

```bash
#!/bin/bash
# Basic SQS CLI smoke test
ENDPOINT="--endpoint-url http://localhost:4566"
QUEUE_URL=$(aws sqs create-queue $ENDPOINT --queue-name test-queue \
    --query QueueUrl --output text)
aws sqs send-message $ENDPOINT --queue-url "$QUEUE_URL" --message-body "hello"
RECEIPT=$(aws sqs receive-message $ENDPOINT --queue-url "$QUEUE_URL" \
    --query 'Messages[0].ReceiptHandle' --output text)
aws sqs delete-message $ENDPOINT --queue-url "$QUEUE_URL" --receipt-handle "$RECEIPT"
aws sqs delete-queue $ENDPOINT --queue-url "$QUEUE_URL"
```

### 13.4 Makefile Targets

```makefile
test-sqs: test-sqs-unit test-sqs-integration

test-sqs-unit:
    @cargo test -p rustack-sqs-model -p rustack-sqs-core -p rustack-sqs-http

test-sqs-integration:
    @cargo test -p integration-tests -- sqs --ignored

test-sqs-cli:
    @./tests/sqs-cli-smoke.sh

test-sqs-localstack:
    @cd vendors/localstack && python -m pytest tests/aws/services/sqs/ -v
```

---

## 14. Phased Implementation Plan

### Phase 0: MVP (9 Operations -- Standard Queues, Short Polling)

**Goal**: Cover the most common local development use case: create a queue, send messages, receive messages, delete messages.
**Estimated scope**: ~5,000-7,000 lines of Rust code across 3 new crates.

#### Step 0.1: Codegen Extension
- Add `SqsServiceConfig` to codegen `services/` module
- Download SQS Smithy model JSON from `aws/api-models-aws`
- Generate `rustack-sqs-model` crate (operations enum, input/output structs, error codes)
- Generate serde derives with `#[serde(rename_all = "PascalCase")]`

#### Step 0.2: HTTP Layer (JSON Protocol Only)
- Implement `SqsRouter` (X-Amz-Target: AmazonSQS.* dispatch)
- Implement `SqsHttpService` (hyper Service)
- Implement JSON request deserialization
- Implement JSON response serialization
- Implement JSON error formatting with `x-amzn-query-error` header

#### Step 0.3: Queue Actor Infrastructure
- Implement `QueueActor` with command channel and event loop
- Implement `StandardQueueStorage` (VecDeque for available, HashMap for in-flight)
- Implement message lifecycle: send -> available -> in-flight -> deleted
- Implement receipt handle generation
- Implement visibility timeout tracking with periodic cleanup
- Implement MD5 computation for body and attributes

#### Step 0.4: Core Operations (9 ops)
- `CreateQueue` / `DeleteQueue` / `GetQueueUrl` / `ListQueues` (queue management)
- `SendMessage` / `ReceiveMessage` (short polling only) / `DeleteMessage` (message lifecycle)
- `GetQueueAttributes` / `SetQueueAttributes` (basic attributes: VisibilityTimeout, DelaySeconds, MaximumMessageSize, MessageRetentionPeriod)

#### Step 0.5: Server Integration
- Implement `SqsServiceRouter` with gateway registration
- Add `sqs` cargo feature gate
- Register SQS before S3 in gateway (SQS uses specific matching, S3 is catch-all)
- Update health endpoint

#### Step 0.6: Testing
- Unit tests for queue actor, message storage, MD5 computation
- Integration tests with aws-sdk-sqs
- CLI smoke tests
- Update Makefile with SQS test targets

### Phase 1: Batch Operations, Long Polling, DLQ (6 Operations + Features)

**Goal**: Production-ready standard queues with batch operations, long polling, and dead-letter queues.

- `SendMessageBatch` / `DeleteMessageBatch` (batch message operations with per-entry results)
- `ChangeMessageVisibility` / `ChangeMessageVisibilityBatch` (visibility timeout adjustment)
- `PurgeQueue` (with 60-second cooldown enforcement)
- `ListDeadLetterSourceQueues` (scan queues for matching RedrivePolicy)
- **Long polling**: `WaitTimeSeconds` on ReceiveMessage, `ReceiveMessageWaitTimeSeconds` queue attribute
- **Dead-letter queues**: `RedrivePolicy` attribute, `maxReceiveCount` tracking, automatic DLQ routing
- **Message delay**: `DelaySeconds` on queue and per-message, delayed message promotion
- **Message retention**: Automatic purge of messages exceeding `MessageRetentionPeriod`

### Phase 2: FIFO Queues and Deduplication

**Goal**: Full FIFO queue support with message groups and exactly-once deduplication.

- `FifoQueueStorage` implementation (per-group BTreeMap, group blocking)
- Message group ordering: strict FIFO within groups, parallel across groups
- Group blocking: while a message from a group is in-flight, no more messages from that group are delivered
- Deduplication: 5-minute sliding window, content-based (SHA-256) or explicit `MessageDeduplicationId`
- `DeduplicationScope`: `queue` vs `messageGroup`
- Sequence number assignment (monotonically increasing per queue)
- FIFO-specific validations: name must end with `.fifo`, per-message delay not supported

### Phase 3: Tags, Permissions, Message Move Tasks, awsQuery Protocol

**Goal**: Feature completeness and legacy protocol support.

- `TagQueue` / `UntagQueue` / `ListQueueTags` (cost allocation tags, max 50)
- `AddPermission` / `RemovePermission` (store in Policy attribute, no enforcement)
- `StartMessageMoveTask` / `CancelMessageMoveTask` / `ListMessageMoveTasks` (DLQ redrive)
- **awsQuery protocol support**: `application/x-www-form-urlencoded` request parsing, XML response serialization
- Query protocol deserialization: flat form params with dot-notation for nested fields
- XML response templates for all 23 operations

---

## 15. Risk Analysis

### 15.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Long polling correctness under concurrent access | High | High | Extensive concurrency tests, use `tokio::sync::Notify` which is well-tested, test with multiple concurrent consumers |
| awsQuery protocol complexity | Medium | Medium | Defer to Phase 3, focus on JSON protocol first; most modern SDKs use JSON |
| FIFO message group blocking edge cases | Medium | High | Test against ElasticMQ behavior, extensive property-based tests with `proptest` |
| Deduplication cache memory growth | Medium | Low | Periodic cleanup in actor event loop, bounded by 5-minute window |
| Gateway routing ambiguity for Query protocol | Medium | Medium | Query protocol (form-urlencoded POST to /) could conflict with S3 POST uploads; use Action= parameter sniffing |
| Receipt handle validation across re-receives | Medium | Medium | Encode receive-timestamp in handle, invalidate old handles properly |
| MD5 attribute hash computation correctness | Medium | High | Test against known AWS SDK computed values, match binary encoding exactly |
| Actor channel backpressure under high load | Low | Medium | Use bounded channels (256 capacity), return throttling errors if full |

### 15.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| awsQuery protocol more complex than expected | High | Medium | Defer to Phase 3; each operation needs custom query param mapping |
| Users demand FIFO before batch/DLQ | Medium | Low | Phases can be reordered based on demand |
| Feature creep (SNS integration, Lambda triggers) | High | Medium | Strict non-goals boundary, return NotImplemented for unsupported ops |
| LocalStack test suite expects non-standard behavior | Medium | Medium | Track divergences, prefer AWS-correct behavior over LocalStack compat |

### 15.3 Behavioral Differences

Our implementation will intentionally differ from ElasticMQ and LocalStack in some areas:

| Behavior | ElasticMQ | LocalStack | Rustack | Justification |
|----------|-----------|------------|-----------|---------------|
| Queue URL format | `http://host:port/queue/name` | `http://host:port/acct/name` | `http://host:port/acct/name` | Match LocalStack/AWS convention |
| Error message text | Custom messages | Custom messages | Match AWS messages | Better SDK compatibility |
| CreateQueue idempotency | Lenient | Lenient | Strict (same attrs = ok, diff = error) | Match AWS behavior |
| Purge cooldown | Not enforced | 60 seconds | 60 seconds | Match AWS behavior |
| FIFO throughput limits | Not enforced | Not enforced | Not enforced | Not needed for local dev |
| Queue deletion cooldown | Not enforced | 60 seconds | 60 seconds | Match AWS behavior |
| Max in-flight messages | Not enforced | Not enforced | Not enforced initially | Low priority for local dev |

---

## 16. Open Questions

### 16.1 awsQuery Protocol Priority

Should awsQuery protocol support be in Phase 1 instead of Phase 3? Some older CI environments may use legacy SDKs.

**Recommendation**: Keep in Phase 3. The vast majority of local development uses modern SDKs (Rust SDK, boto3 >= 1.28, Java SDK v2) which all default to JSON protocol. If demand emerges, we can pull it forward.

### 16.2 Queue URL Resolution

Should we support queue URLs in the `QueueUrl` parameter that reference this server by different hostnames (e.g., `http://sqs.us-east-1.localhost:4566/000000000000/q`)? AWS SDKs construct URLs with region-specific hostnames.

**Recommendation**: Accept any queue URL and extract only the account-id and queue-name segments. Ignore the host/port portion. This provides maximum compatibility with different SDK configurations.

### 16.3 Long Polling Implementation Detail

Should long polling be implemented within the queue actor event loop (storing pending replies), or as a separate waiter task per HTTP connection?

**Recommendation**: Store pending long-poll replies within the queue actor. The actor's event loop already ticks every second for cleanup; it can check pending polls in the same tick, avoiding the complexity of spawning per-connection tasks.

### 16.4 Query Protocol Routing Ambiguity

When a `POST /` request arrives with `Content-Type: application/x-www-form-urlencoded`, how do we distinguish SQS Query protocol from other potential services?

**Recommendation**: In the SQS handler (not the gateway router), read the `Action=` parameter. If the action is not a recognized SQS operation, return an error. The gateway router should match on `Content-Type: x-www-form-urlencoded` + `POST /` and route to SQS tentatively. If SQS rejects it, the request falls through to S3. Alternatively, buffer the body at the gateway level and check `Action=` before routing. The buffering approach is cleaner but requires reading the body before routing.

### 16.5 Message Size Enforcement

Should we enforce the 256KiB message size limit strictly, or be lenient for local development?

**Recommendation**: Enforce strictly. Message size validation is cheap and catches real bugs in application code. Developers want their local environment to catch the same errors as production.

---

## Appendix A: SQS vs DynamoDB Implementation Effort Comparison

| Component | DynamoDB Lines | SQS Est. | Ratio | Notes |
|-----------|---------------|---------|-------|-------|
| Model (codegen output) | ~4,000 | ~2,500 | 0.6x | Fewer operations (23 vs 66), simpler types |
| JSON serde | ~200 | ~200 | 1.0x | Same approach, serde derives |
| HTTP routing | ~100 | ~300 | 3.0x | Dual protocol adds complexity |
| Query protocol serde | N/A | ~1,500 | New | Form-urlencoded parsing + XML responses |
| Auth integration | ~100 | ~100 | 1.0x | SigV4 only, identical |
| Core business logic | ~6,000 | ~4,000 | 0.7x | Simpler (no expression language) |
| Storage engine | ~2,500 | ~2,000 | 0.8x | Queue storage simpler than B-Tree indexing |
| Actor infrastructure | N/A | ~1,000 | New | Actor per queue, channel management |
| Long polling | N/A | ~500 | New | Notify-based wakeup |
| Expression parser | ~2,500 | N/A | N/A | SQS has no expression language |
| **Total** | **~15,400** | **~12,100** | **0.8x** | |

SQS is simpler than DynamoDB primarily because there is no expression language. The main new complexity is dual protocol support (JSON + awsQuery) and the actor-based concurrency model for long polling.

## Appendix B: SQS Error Codes and HTTP Status Codes

| Error Code | HTTP Status | When |
|-----------|------------|------|
| `AWS.SimpleQueueService.NonExistentQueue` | 400 | Queue does not exist |
| `QueueAlreadyExists` | 400 | Queue exists with different attributes |
| `AWS.SimpleQueueService.QueueDeletedRecently` | 400 | Queue deleted within 60 seconds |
| `InvalidParameterValue` | 400 | Invalid parameter value |
| `MissingParameter` | 400 | Required parameter missing |
| `InvalidAttributeName` | 400 | Invalid attribute name |
| `InvalidAttributeValue` | 400 | Invalid attribute value |
| `MessageNotInflight` | 400 | Message not currently in flight |
| `ReceiptHandleIsInvalid` | 400 | Invalid receipt handle |
| `EmptyBatchRequest` | 400 | Batch contains no entries |
| `TooManyEntriesInBatchRequest` | 400 | More than 10 batch entries |
| `BatchEntryIdsNotDistinct` | 400 | Duplicate IDs in batch |
| `BatchRequestTooLong` | 400 | Batch exceeds 256KiB |
| `InvalidBatchEntryId` | 400 | Invalid batch entry ID format |
| `AWS.SimpleQueueService.PurgeQueueInProgress` | 403 | Purge within 60 seconds |
| `OverLimit` | 403 | Queue limit exceeded |
| `UnsupportedOperation` | 400 | Not supported for queue type |
| `ResourceNotFoundException` | 404 | Message move task not found |
| `InternalError` | 500 | Internal server error |

## Appendix C: SQS Constraints and Limits

| Resource | Limit | Enforced in Rustack? |
|----------|-------|----------------------|
| Max message size | 256 KiB | Yes |
| Max message retention | 14 days | Yes |
| Min message retention | 60 seconds | Yes |
| Max visibility timeout | 12 hours (43,200s) | Yes |
| Max long-poll wait | 20 seconds | Yes |
| Max delay | 15 minutes (900s) | Yes |
| Max batch size | 10 messages | Yes |
| Max batch request size | 256 KiB total | Yes |
| Max message attributes | 10 per message | Yes |
| Max queues per account | 1,000,000 | No (unbounded) |
| Max in-flight (standard) | 120,000 | No (unbounded) |
| Max in-flight (FIFO) | 20,000 | No (unbounded) |
| Max tags per queue | 50 | Yes |
| Queue name length | 1-80 characters | Yes |
| Message dedup window | 5 minutes | Yes |
| Purge cooldown | 60 seconds | Yes |
