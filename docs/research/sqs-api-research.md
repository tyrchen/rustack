# SQS API Comprehensive Research

**Date:** 2026-03-02
**Purpose:** Comprehensive analysis of the AWS SQS API surface, protocol, Smithy model, features, implementation challenges, and available test suites for building a Rust-based local SQS implementation.

---

## Table of Contents

1. [Protocol: AWS JSON 1.0 with awsQuery Compatibility](#1-protocol-aws-json-10-with-awsquery-compatibility)
2. [Smithy Model and Code Generation](#2-smithy-model-and-code-generation)
3. [Complete API Operations (23 Operations)](#3-complete-api-operations-23-operations)
4. [Queue Types and Semantics](#4-queue-types-and-semantics)
5. [Queue Attributes](#5-queue-attributes)
6. [Message Attributes and System Attributes](#6-message-attributes-and-system-attributes)
7. [SQS Features Deep Dive](#7-sqs-features-deep-dive)
8. [Queue URL and ARN Formats](#8-queue-url-and-arn-formats)
9. [Error Handling](#9-error-handling)
10. [Constraints and Limits](#10-constraints-and-limits)
11. [Third-Party Test Suites](#11-third-party-test-suites)
12. [Implementation Challenges](#12-implementation-challenges)
13. [Key Differences from DynamoDB Implementation](#13-key-differences-from-dynamodb-implementation)
14. [Implementation Priority Matrix](#14-implementation-priority-matrix)
15. [Architecture Considerations for RustStack](#15-architecture-considerations-for-ruststack)

---

## 1. Protocol: AWS JSON 1.0 with awsQuery Compatibility

SQS uses the `@awsJson1_0` Smithy protocol with the `@awsQueryCompatible` trait. This is the same protocol as DynamoDB, which means the ruststack infrastructure for DynamoDB HTTP routing and JSON serde can be largely reused.

### 1.1 Protocol History

- **Original protocol**: `awsQuery` (XML-based request/response)
- **Migration announced**: July 2023 (preview), November 2023 (GA)
- **Current protocol**: `awsJson1_0` with backward compatibility for `awsQuery` clients
- **Performance improvement**: Up to 23% reduction in end-to-end message processing latency for 5KB payloads

### 1.2 Request Format (JSON Protocol)

All SQS requests are **HTTP POST** to the root path (`/`). Operation dispatch is done via the `X-Amz-Target` header.

```http
POST / HTTP/1.1
Host: sqs.<region>.amazonaws.com
Content-Type: application/x-amz-json-1.0
X-Amz-Target: AmazonSQS.SendMessage
Authorization: AWS4-HMAC-SHA256 Credential=<...>, SignedHeaders=<...>, Signature=<...>
X-Amz-Date: 20260302T120000Z
Content-Length: <n>

{
    "QueueUrl": "https://sqs.us-east-1.amazonaws.com/123456789012/MyQueue",
    "MessageBody": "Hello World"
}
```

### 1.3 Response Format (JSON Protocol)

Successful responses return JSON:

```http
HTTP/1.1 200 OK
Content-Type: application/x-amz-json-1.0

{
    "MD5OfMessageBody": "...",
    "MessageId": "..."
}
```

### 1.4 Error Responses (JSON Protocol)

Errors include a `__type` field in the body and optionally the `x-amzn-query-error` header for backward compatibility:

```http
HTTP/1.1 400 Bad Request
Content-Type: application/x-amz-json-1.0
x-amzn-query-error: AWS.SimpleQueueService.NonExistentQueue;Sender

{
    "__type": "AWS.SimpleQueueService.NonExistentQueue",
    "message": "The specified queue does not exist."
}
```

### 1.5 The awsQueryCompatible Trait

The `awsQueryCompatible` trait allows services to backward-compatibly migrate from `awsQuery` to `awsJson1_0`. It:

- Adds the `x-amzn-query-error` header to error responses in the format `Code;Fault`
- `Code` is the custom error identifier from the `awsQueryError` trait
- `Fault` is either `Sender` or `Receiver`
- Allows clients expecting either protocol format to interpret errors correctly

### 1.6 Legacy awsQuery Request Format

Older SDKs still send `awsQuery` format requests:

```http
POST / HTTP/1.1
Host: sqs.us-east-1.amazonaws.com
Content-Type: application/x-www-form-urlencoded

Action=SendMessage
&QueueUrl=https://sqs.us-east-1.amazonaws.com/123456789012/MyQueue
&MessageBody=Hello+World
&Version=2012-11-05
```

Responses for `awsQuery` are XML-based. **An implementation must support both protocols** to be compatible with all SDK versions.

### 1.7 SDK Minimum Versions for JSON Protocol

| Language | Minimum Version |
|----------|-----------------|
| Python (boto3) | 1.28.82 |
| Java 2.x | 2.21.19 |
| Node.js v3.x | v3.447.0 |
| Go 1.x | v1.47.7 |
| .NET | 3.7.681.0 |
| Rust (aws-sdk-sqs) | Uses JSON by default |

---

## 2. Smithy Model and Code Generation

### 2.1 Official Model Location

The official Smithy model for SQS is hosted in the **aws/api-models-aws** GitHub repository:

- **Repository**: https://github.com/aws/api-models-aws
- **Path**: `models/sqs/service/2012-11-05/sqs-2012-11-05.json`
- **API Version**: 2012-11-05
- **Format**: Smithy JSON AST

### 2.2 Smithy Traits on the SQS Service

```
@awsJson1_0
@awsQueryCompatible
@service(sdkId: "SQS")
service AmazonSQS {
    version: "2012-11-05"
    operations: [... 23 operations ...]
}
```

Key traits:
- `@awsJson1_0` - Primary protocol
- `@awsQueryCompatible` - Backward compatibility with awsQuery clients
- `X-Amz-Target` prefix: `AmazonSQS`

### 2.3 AWS SDK for Rust

- **Crate**: `aws-sdk-sqs` on crates.io
- **Docs**: https://docs.rs/aws-sdk-sqs
- **Code generation**: Via smithy-rs from Smithy models
- **Source code structure** (`sdk/sqs/src/`):
  - `client/` - Client implementation
  - `operation/` - One submodule per API operation (input/output/error types)
  - `types/` - Data type definitions
  - `protocol_serde/` - JSON serialization/deserialization
  - `json_errors.rs` - JSON error handling
  - `aws_query_compatible_errors.rs` - awsQuery backward-compatible error support

### 2.4 Code Generation for RustStack

Since SQS uses the same `@awsJson1_0` protocol as DynamoDB, the existing codegen approach from the DynamoDB implementation should be adaptable. The codegen would:

1. Parse the `sqs-2012-11-05.json` Smithy model
2. Generate Rust types for all input/output/error shapes
3. Generate JSON serde implementations
4. The HTTP routing layer can be shared with DynamoDB (same POST-to-root with X-Amz-Target dispatch pattern)

---

## 3. Complete API Operations (23 Operations)

### 3.1 Queue Management (7 operations)

| Operation | Description |
|-----------|-------------|
| **CreateQueue** | Creates a new standard or FIFO queue with specified attributes and tags |
| **DeleteQueue** | Deletes the queue specified by the QueueUrl, regardless of contents |
| **GetQueueUrl** | Returns the URL of an existing queue by name (and optionally account ID) |
| **GetQueueAttributes** | Gets attributes for the specified queue |
| **SetQueueAttributes** | Sets the value of one or more queue attributes |
| **ListQueues** | Returns a list of queue URLs, optionally filtered by prefix |
| **PurgeQueue** | Deletes all messages in a queue (60-second cooldown between calls) |

### 3.2 Message Operations (6 operations)

| Operation | Description |
|-----------|-------------|
| **SendMessage** | Delivers a message to the specified queue |
| **SendMessageBatch** | Delivers up to 10 messages to the specified queue in a single request |
| **ReceiveMessage** | Retrieves 1-10 messages from the queue, with optional long polling |
| **DeleteMessage** | Deletes a specific message from the queue using its receipt handle |
| **DeleteMessageBatch** | Deletes up to 10 messages from the queue in a single request |
| **ChangeMessageVisibility** | Changes the visibility timeout of a received message |

### 3.3 Batch Visibility (1 operation)

| Operation | Description |
|-----------|-------------|
| **ChangeMessageVisibilityBatch** | Changes the visibility timeout of up to 10 messages in a single request |

### 3.4 Permissions (2 operations)

| Operation | Description |
|-----------|-------------|
| **AddPermission** | Adds a permission to a queue for a specific principal and actions |
| **RemovePermission** | Revokes any permissions in the queue policy matching the specified label |

### 3.5 Tagging (3 operations)

| Operation | Description |
|-----------|-------------|
| **TagQueue** | Adds cost allocation tags to the specified queue |
| **UntagQueue** | Removes cost allocation tags from the specified queue |
| **ListQueueTags** | Lists all cost allocation tags for the specified queue |

### 3.6 Dead-Letter Queue Management (4 operations)

| Operation | Description |
|-----------|-------------|
| **ListDeadLetterSourceQueues** | Returns a list of queues that have a dead-letter queue configured pointing to this queue |
| **StartMessageMoveTask** | Starts an asynchronous task to move messages from a DLQ to a destination queue |
| **CancelMessageMoveTask** | Cancels a specified message move task |
| **ListMessageMoveTasks** | Gets the most recent message move tasks for a specified source queue |

---

## 4. Queue Types and Semantics

### 4.1 Standard Queues

- **Delivery guarantee**: At-least-once delivery (messages may be delivered more than once)
- **Ordering**: Best-effort ordering (messages generally delivered in order sent, but not guaranteed)
- **Throughput**: Nearly unlimited messages per second
- **In-flight messages limit**: 120,000

### 4.2 FIFO Queues

- **Delivery guarantee**: Exactly-once processing (within the queue scope)
- **Ordering**: Strict FIFO ordering within message groups
- **Throughput**: 300 messages/second without batching, 3,000/sec with batching (default); up to 70,000/sec in high-throughput mode
- **In-flight messages limit**: 20,000
- **Naming**: Must end with `.fifo` suffix
- **Message groups**: Support multiple independent ordering groups via `MessageGroupId`
- **Deduplication**: 5-minute deduplication window via `MessageDeduplicationId` or content-based deduplication (SHA-256 of body)

### 4.3 Key Semantic Differences

| Aspect | Standard | FIFO |
|--------|----------|------|
| Delivery | At-least-once | Exactly-once processing |
| Ordering | Best-effort | Strict within message group |
| Throughput | ~Unlimited | 300-70,000 msg/sec |
| Per-message delay | Supported | NOT supported |
| Deduplication | None | 5-min dedup window |
| Naming | Any valid name | Must end with `.fifo` |

---

## 5. Queue Attributes

### 5.1 Settable Attributes

| Attribute | Type | Default | Range | Description |
|-----------|------|---------|-------|-------------|
| `DelaySeconds` | Integer | 0 | 0-900 (15 min) | Default delay for messages in the queue |
| `MaximumMessageSize` | Integer | 262,144 | 1,024-1,048,576 | Max message size in bytes (1KiB-1MiB) |
| `MessageRetentionPeriod` | Integer | 345,600 | 60-1,209,600 | How long to retain messages (1 min-14 days) |
| `Policy` | JSON String | | | IAM policy document |
| `ReceiveMessageWaitTimeSeconds` | Integer | 0 | 0-20 | Default long-polling wait time |
| `VisibilityTimeout` | Integer | 30 | 0-43,200 | Visibility timeout in seconds (0-12 hours) |
| `RedrivePolicy` | JSON String | | | DLQ configuration: `{deadLetterTargetArn, maxReceiveCount}` |
| `RedriveAllowPolicy` | JSON String | | | Permissions for which queues can use this as DLQ |
| `KmsMasterKeyId` | String | | | KMS CMK ID for encryption |
| `KmsDataKeyReusePeriodSeconds` | Integer | 300 | 60-86,400 | How long to reuse data keys |
| `SqsManagedSseEnabled` | Boolean | | | Enable SQS-managed SSE |

### 5.2 FIFO-Only Settable Attributes

| Attribute | Type | Default | Description |
|-----------|------|---------|-------------|
| `ContentBasedDeduplication` | Boolean | false | Use SHA-256 of message body as dedup ID |
| `DeduplicationScope` | String | `queue` | `messageGroup` or `queue` |
| `FifoThroughputLimit` | String | `perQueue` | `perQueue` or `perMessageGroupId` |

### 5.3 Read-Only Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `ApproximateNumberOfMessages` | Integer | Approx number of messages available |
| `ApproximateNumberOfMessagesNotVisible` | Integer | Messages in flight |
| `ApproximateNumberOfMessagesDelayed` | Integer | Messages delayed and not yet available |
| `CreatedTimestamp` | Integer | Queue creation time (epoch seconds) |
| `LastModifiedTimestamp` | Integer | Last attribute change time (epoch seconds) |
| `QueueArn` | String | Queue's Amazon Resource Name |
| `FifoQueue` | Boolean | Whether this is a FIFO queue |

---

## 6. Message Attributes and System Attributes

### 6.1 User Message Attributes

Messages can carry up to 10 custom attributes. Each attribute has:
- **Name**: String (up to 256 chars, alphanumeric and `.`, `-`, `_`)
- **DataType**: `String`, `Number`, `Binary` (with optional custom type suffix like `String.custom`)
- **Value**: StringValue or BinaryValue

### 6.2 System Attributes (Read-Only on Receive)

| Attribute | Description |
|-----------|-------------|
| `SenderId` | IAM principal ID of the sender |
| `SentTimestamp` | Epoch milliseconds when message was sent |
| `ApproximateReceiveCount` | Number of times message has been received |
| `ApproximateFirstReceiveTimestamp` | Epoch milliseconds of first receive |
| `SequenceNumber` | FIFO only - large increasing number for ordering |
| `MessageDeduplicationId` | FIFO only - deduplication token |
| `MessageGroupId` | FIFO only - message group identifier |
| `AWSTraceHeader` | X-Ray trace header |
| `DeadLetterQueueSourceArn` | ARN of source queue if redriven from DLQ |

### 6.3 Message Structure on Receive

```json
{
    "Messages": [
        {
            "MessageId": "uuid",
            "ReceiptHandle": "opaque-string",
            "MD5OfBody": "md5-hex",
            "Body": "message content",
            "Attributes": {
                "SenderId": "...",
                "SentTimestamp": "1234567890123",
                "ApproximateReceiveCount": "1",
                "ApproximateFirstReceiveTimestamp": "1234567890123"
            },
            "MessageAttributes": {
                "CustomAttr": {
                    "StringValue": "value",
                    "DataType": "String"
                }
            },
            "MD5OfMessageAttributes": "md5-hex"
        }
    ]
}
```

---

## 7. SQS Features Deep Dive

### 7.1 Visibility Timeout

- When a consumer receives a message, it becomes invisible to other consumers for `VisibilityTimeout` seconds
- Default: 30 seconds, Range: 0-43,200 (12 hours)
- Can be changed per-message via `ChangeMessageVisibility`
- If not deleted before timeout expires, message becomes visible again
- In-flight messages count toward the queue's in-flight limit (120,000 standard, 20,000 FIFO)

### 7.2 Long Polling

- Set via `WaitTimeSeconds` on `ReceiveMessage` (0-20 seconds) or `ReceiveMessageWaitTimeSeconds` on the queue
- Short polling (default, WaitTimeSeconds=0): queries a subset of servers, returns immediately even if empty
- Long polling (WaitTimeSeconds > 0): queries all servers, waits for messages to arrive up to the timeout
- Implementation: TCP connection held open; SQS responds immediately when messages arrive, or returns empty after timeout
- Reduces empty responses and cost

### 7.3 Delay Queues

- **Queue-level delay**: `DelaySeconds` attribute (0-900 seconds / 15 minutes)
- **Per-message delay** (message timers): `DelaySeconds` parameter on `SendMessage` (overrides queue setting)
- **FIFO limitation**: Per-message delay is NOT supported in FIFO queues; only queue-level delay
- Delayed messages count toward `ApproximateNumberOfMessagesDelayed`
- Standard queue: changing queue delay does NOT affect already-queued messages
- FIFO queue: changing queue delay IS retroactive

### 7.4 Dead-Letter Queues (DLQ)

- Configured via `RedrivePolicy` attribute: `{"deadLetterTargetArn": "arn:...", "maxReceiveCount": 10}`
- When `ApproximateReceiveCount` exceeds `maxReceiveCount`, message moves to DLQ
- DLQ must be the same queue type (standard -> standard, FIFO -> FIFO)
- DLQ must be in the same AWS account and region
- `RedriveAllowPolicy` controls which queues can designate this queue as their DLQ
  - `allowAll` (default), `denyAll`, or `byQueue` with up to 10 `sourceQueueArns`

### 7.5 DLQ Redrive (Message Move Tasks)

Added in June 2023 via three new APIs:

- **StartMessageMoveTask**: Moves messages from DLQ back to source queue or custom destination
  - `SourceArn`: DLQ ARN (required)
  - `DestinationArn`: Target ARN (optional; defaults to original source queue)
  - `MaxNumberOfMessagesPerSecond`: Rate limit (max 500 msg/sec)
- **CancelMessageMoveTask**: Cancels an in-progress move task
- **ListMessageMoveTasks**: Lists recent move tasks for a source queue
- Constraints: Max 36 hours runtime, max 100 active tasks per account

### 7.6 FIFO Queues - Message Groups

- `MessageGroupId` (required for FIFO sends): Groups related messages
- Messages within the same group are processed strictly in order
- Messages from different groups can be processed in parallel
- While a message in a group is in-flight, no other messages from that group are delivered
- Group blocking: if one message fails processing, it blocks all subsequent messages in the same group

### 7.7 FIFO Queues - Deduplication

- **5-minute deduplication window**: Duplicate messages with the same deduplication ID within 5 minutes are accepted but not delivered
- Two deduplication methods:
  1. **Explicit**: Provide `MessageDeduplicationId` on each send
  2. **Content-based**: Enable `ContentBasedDeduplication` on queue; uses SHA-256 of message body
- `DeduplicationScope`: `queue` (dedup across entire queue) or `messageGroup` (dedup within group only)

### 7.8 Batch Operations

- `SendMessageBatch`: Up to 10 messages per request (max 256KB total)
- `DeleteMessageBatch`: Up to 10 messages per request
- `ChangeMessageVisibilityBatch`: Up to 10 messages per request
- Batch responses include per-message success/failure results

### 7.9 Purge Queue

- Deletes all messages in the queue
- **60-second cooldown**: Cannot call `PurgeQueue` again on the same queue within 60 seconds
- Messages already in flight may not be purged immediately
- Message deletion takes up to 60 seconds to complete

### 7.10 Queue Tags

- Key-value pairs for cost allocation
- Up to 50 tags per queue
- Tag keys: 1-128 Unicode chars
- Tag values: 0-256 Unicode chars

### 7.11 Permissions (Resource-Based Policy)

- `AddPermission` / `RemovePermission` provide a simplified API over the IAM policy
- Underlying `Policy` attribute holds the full IAM policy JSON document
- For a local implementation, basic permission checking may be deferred

---

## 8. Queue URL and ARN Formats

### 8.1 Queue URL

```
https://sqs.<region>.amazonaws.com/<account-id>/<queue-name>
```

Examples:
- Standard: `https://sqs.us-east-1.amazonaws.com/123456789012/MyQueue`
- FIFO: `https://sqs.us-east-1.amazonaws.com/123456789012/MyQueue.fifo`

### 8.2 Queue ARN

```
arn:aws:sqs:<region>:<account-id>:<queue-name>
```

Example: `arn:aws:sqs:us-east-1:123456789012:MyQueue`

### 8.3 Queue Naming Rules

- Standard: 1-80 characters, alphanumeric, hyphens, underscores
- FIFO: Same rules but must end with `.fifo` (suffix counts toward 80-char limit)

---

## 9. Error Handling

### 9.1 Common Error Types

| Error | HTTP Status | Description |
|-------|-------------|-------------|
| `AWS.SimpleQueueService.NonExistentQueue` | 400 | Queue does not exist |
| `QueueAlreadyExists` | 400 | Queue with same name exists with different attributes |
| `QueueDeletedRecently` | 400 | Queue deleted within last 60 seconds |
| `InvalidParameterValue` | 400 | Invalid parameter value |
| `MissingParameter` | 400 | Required parameter missing |
| `InvalidAttributeName` | 400 | Invalid attribute name |
| `InvalidAttributeValue` | 400 | Invalid attribute value |
| `MessageNotInflight` | 400 | Message is not currently in flight |
| `ReceiptHandleIsInvalid` | 400 | Invalid receipt handle |
| `EmptyBatchRequest` | 400 | Batch request contains no entries |
| `TooManyEntriesInBatchRequest` | 400 | More than 10 entries in batch |
| `BatchEntryIdsNotDistinct` | 400 | Duplicate IDs in batch request |
| `BatchRequestTooLong` | 400 | Batch request exceeds size limit |
| `InvalidBatchEntryId` | 400 | Invalid batch entry ID format |
| `PurgeQueueInProgress` | 403 | Another purge within 60 seconds |
| `QueueNameExists` | 400 | Queue already exists with different attributes |
| `UnsupportedOperation` | 400 | Operation not supported for queue type |
| `OverLimit` | 403 | Queue limit exceeded |
| `ResourceNotFoundException` | 404 | Message move task not found |
| `InternalError` | 500 | Internal server error |

### 9.2 Error Response Format (JSON)

```json
{
    "__type": "AWS.SimpleQueueService.NonExistentQueue",
    "message": "The specified queue does not exist for this wsdl version."
}
```

With `x-amzn-query-error: AWS.SimpleQueueService.NonExistentQueue;Sender` header for backward compatibility.

---

## 10. Constraints and Limits

| Resource | Limit |
|----------|-------|
| Max message size | 256 KiB (262,144 bytes) |
| Max message retention | 14 days |
| Min message retention | 60 seconds |
| Max visibility timeout | 12 hours (43,200 seconds) |
| Max long-poll wait | 20 seconds |
| Max delay | 15 minutes (900 seconds) |
| Max batch size | 10 messages |
| Max batch request size | 256 KiB total |
| Max message attributes | 10 per message |
| Max in-flight (standard) | 120,000 |
| Max in-flight (FIFO) | 20,000 |
| Max queues per account | 1,000,000 (default) |
| Max tags per queue | 50 |
| Queue name length | 1-80 characters |
| Message dedup window | 5 minutes |
| Purge cooldown | 60 seconds |
| FIFO throughput (no batch) | 300 msg/sec |
| FIFO throughput (batch) | 3,000 msg/sec |
| FIFO high throughput | 70,000 msg/sec |
| Max message move tasks | 100 per account |
| Message move max rate | 500 msg/sec |
| Message move max duration | 36 hours |
| ReceiveMessage MaxMessages | 1-10 |

---

## 11. Third-Party Test Suites

### 11.1 ElasticMQ (SoftwareMill)

- **Repository**: https://github.com/softwaremill/elasticmq
- **Language**: Scala (Akka/Pekko-based)
- **Test suite location**: `rest/rest-sqs-testing-amazon-java-sdk/src/test/scala/org/elasticmq/rest/sqs/`
- **Test modules**:
  - `AmazonJavaSdkTestSuite.scala` - Main test suite using AWS Java SDK
  - Integration tests in `integration/` subdirectory
  - `common-test` module with shared utilities
  - Performance tests in `performance-tests/`
- **Features tested**: Queue CRUD, message send/receive, visibility timeout, FIFO queues, content-based deduplication, message delay, DLQ, long polling, queue tags, batch operations
- **Supported operations**: CreateQueue, DeleteQueue, ListQueues, GetQueueUrl, GetQueueAttributes, SetQueueAttributes, SendMessage, SendMessageBatch, ReceiveMessage, DeleteMessage, DeleteMessageBatch, ChangeMessageVisibility, ChangeMessageVisibilityBatch, PurgeQueue, TagQueue, UntagQueue, ListQueueTags, AddPermission, RemovePermission
- **Notable**: Supports both awsQuery and JSON protocols. Throughput: ~2,540+ messages/sec

### 11.2 GoAWS (Admiral-Piett)

- **Repository**: https://github.com/Admiral-Piett/goaws
- **Language**: Go
- **Supported SQS operations**: ListQueues, CreateQueue, DeleteQueue, GetQueueUrl, PurgeQueue, SendMessage, SendMessageBatch, ReceiveMessage, DeleteMessage, DeleteMessageBatch, ChangeMessageVisibility, GetQueueAttributes, SetQueueAttributes (limited)
- **Unsupported**: ChangeMessageVisibilityBatch, ListDeadLetterSourceQueues, ListQueueTags, RemovePermission, TagQueue, UntagQueue
- **Supported attributes**: VisibilityTimeout, ReceiveMessageWaitTimeSeconds, RedrivePolicy (only)
- **Testing**: Uses `go test -cover -race`; also has Postman collection
- **Limitation**: No FIFO support, limited queue attributes

### 11.3 LocalStack

- **Repository**: https://github.com/localstack/localstack
- **Language**: Python
- **Test location**: `tests/aws/services/sqs/`
- **Test files**:
  - `test_sqs.py` - Main comprehensive test module (likely hundreds of test cases)
  - `test_sqs_developer_api.py` - Tests for developer API
  - `test_sqs_move_task.py` - Tests for message move task operations
  - `resource_providers/` - CloudFormation resource provider tests
- **Framework**: pytest with snapshot testing
- **JSON protocol support**: Added in LocalStack v3.1 (PR #9710)
- **Most comprehensive**: Likely the most thorough open-source SQS test suite available

### 11.4 fake_sqs (Ruby)

- **Repository**: https://github.com/iain/fake_sqs (original), https://github.com/lyft/fake_sqs (Lyft fork)
- **Language**: Ruby
- **Features**: In-memory or on-disk storage, test integration via TestIntegration class
- **Testing**: Unit tests and acceptance tests
- **Limitation**: Older project, may not support JSON protocol or newer features

### 11.5 Microcks

- **Repository**: https://microcks.io/
- **Description**: Open source cloud-native API mocking and testing tool with SQS/SNS support
- **Focus**: Primarily async API testing rather than full SQS compatibility

### 11.6 AWS SDK Test Suites

- **aws-sdk-go-v2**: Integration tests available in `service/sqs/` directory, with interface-based mocking via `sqsiface`
- **aws-sdk-rust**: Generated from Smithy models; protocol-level tests but focused on serialization correctness
- **aws-sdk-java**: ElasticMQ's AmazonJavaSdkTestSuite uses this SDK extensively

### 11.7 Recommended Test Strategy for RustStack

1. **Primary**: Adapt LocalStack's `test_sqs.py` as the compatibility test suite (similar to what was done for DynamoDB)
2. **Secondary**: Port ElasticMQ's AmazonJavaSdkTestSuite tests as a second validation layer
3. **Smoke tests**: Use AWS CLI with `--endpoint-url` for quick manual testing
4. **SDK integration**: Test with `aws-sdk-sqs` Rust crate pointing to local endpoint

---

## 12. Implementation Challenges

### 12.1 Dual Protocol Support (Hardest)

**Challenge**: Must support both `awsJson1_0` AND `awsQuery` (XML) protocols for backward compatibility.

- JSON protocol: `Content-Type: application/x-amz-json-1.0` with `X-Amz-Target: AmazonSQS.<Op>`
- Query protocol: `Content-Type: application/x-www-form-urlencoded` with `Action=<Op>`
- Detection: Check Content-Type header to determine protocol
- Responses: JSON for JSON requests, XML for Query requests
- Error handling: Include `x-amzn-query-error` header on JSON error responses

### 12.2 Long Polling (Hard)

**Challenge**: ReceiveMessage with `WaitTimeSeconds > 0` must hold the HTTP connection open and respond immediately when messages arrive.

- Requires async notification mechanism (e.g., `tokio::sync::Notify` or channel per waiting consumer)
- Must handle multiple concurrent long-polling consumers on the same queue
- Must timeout correctly after WaitTimeSeconds
- Must cancel waiting when the client disconnects
- Standard queue: query all messages; FIFO: respect message group ordering

### 12.3 FIFO Message Group Blocking (Hard)

**Challenge**: Messages within the same group must be processed strictly in order. While a message from a group is in-flight, no more messages from that group should be delivered.

- Must track in-flight messages per message group
- Must block delivery of subsequent messages in the same group
- Unblocking when: message is deleted, or visibility timeout expires
- Must handle 120,000-message lookahead window for finding available groups

### 12.4 Deduplication Window (Medium)

**Challenge**: Must maintain a 5-minute sliding window of deduplication IDs.

- Store dedup ID -> timestamp mapping
- Periodically clean up expired entries (older than 5 minutes)
- `DeduplicationScope` can be `queue` or `messageGroup`
- Content-based dedup: compute SHA-256 of message body on send

### 12.5 Visibility Timeout Management (Medium)

**Challenge**: Track per-message visibility timers that can be changed dynamically.

- Each received message gets a timer based on queue's VisibilityTimeout
- Timer can be extended via `ChangeMessageVisibility` (and batch variant)
- When timer expires, message becomes visible again
- Must generate unique receipt handles per receive (old handles become invalid)
- Must track `ApproximateReceiveCount` and trigger DLQ redrive at `maxReceiveCount`

### 12.6 Message Delay (Medium)

**Challenge**: Messages with delay must not be visible until delay expires.

- Queue-level delay: `DelaySeconds` attribute
- Per-message delay: `DelaySeconds` parameter on SendMessage (not FIFO)
- FIFO retroactive behavior: changing queue delay affects already-queued messages
- Must track delayed message count for `ApproximateNumberOfMessagesDelayed`

### 12.7 DLQ Redrive (Message Move Tasks) (Medium)

**Challenge**: Background task management for moving messages between queues.

- Asynchronous background task with progress tracking
- Rate limiting (MaxNumberOfMessagesPerSecond)
- Cancellation support
- Task listing and status reporting
- 36-hour max runtime

### 12.8 Receipt Handle Management (Medium)

**Challenge**: Receipt handles are opaque strings that are valid only for the specific receive event.

- Must be unique per receive operation
- Must become invalid after the message is deleted or re-received
- Must be used for delete and change-visibility operations
- Old receipt handles should return appropriate errors

### 12.9 Approximate Counters (Easy-Medium)

**Challenge**: Several attributes are "approximate" counts that don't need to be perfectly accurate.

- `ApproximateNumberOfMessages`: Available messages
- `ApproximateNumberOfMessagesNotVisible`: In-flight messages
- `ApproximateNumberOfMessagesDelayed`: Delayed messages
- These can use atomic counters or periodic computation

### 12.10 Purge Queue Cooldown (Easy)

**Challenge**: Must enforce 60-second cooldown between PurgeQueue calls.

- Track last purge timestamp per queue
- Return `PurgeQueueInProgress` error if called within 60 seconds

---

## 13. Key Differences from DynamoDB Implementation

| Aspect | DynamoDB | SQS |
|--------|----------|-----|
| Protocol | `awsJson1_0` only | `awsJson1_0` + `awsQuery` (dual) |
| Target header | `DynamoDB_20120810.<Op>` | `AmazonSQS.<Op>` |
| Operations | 66 | 23 |
| Stateful behavior | Table CRUD, Items | Message lifecycle (visibility, delay, dedup) |
| Time sensitivity | Low (TTL is optional) | High (visibility timeouts, delays, long polling) |
| Concurrency model | Read/Write per item | Producer/Consumer with visibility tracking |
| Background tasks | TTL expiry, Streams | Long polling, message move, delay timers, DLQ redrive |
| Unique challenge | Expression language, LSI/GSI | Long polling HTTP hold, FIFO ordering |

---

## 14. Implementation Priority Matrix

### Phase 1: Core Operations (MVP)

| Priority | Operation | Notes |
|----------|-----------|-------|
| P0 | CreateQueue | Standard queues first |
| P0 | DeleteQueue | |
| P0 | GetQueueUrl | |
| P0 | ListQueues | |
| P0 | SendMessage | |
| P0 | ReceiveMessage | Short polling first |
| P0 | DeleteMessage | |
| P0 | GetQueueAttributes | |
| P0 | SetQueueAttributes | Basic attributes |

### Phase 2: Batch & Visibility

| Priority | Operation | Notes |
|----------|-----------|-------|
| P1 | SendMessageBatch | |
| P1 | DeleteMessageBatch | |
| P1 | ChangeMessageVisibility | |
| P1 | ChangeMessageVisibilityBatch | |
| P1 | PurgeQueue | With 60s cooldown |
| P1 | Long Polling | WaitTimeSeconds support |

### Phase 3: FIFO & DLQ

| Priority | Operation | Notes |
|----------|-----------|-------|
| P2 | FIFO queue support | MessageGroupId, deduplication |
| P2 | Dead-letter queues | RedrivePolicy, maxReceiveCount |
| P2 | Message delay | Per-queue and per-message |
| P2 | ListDeadLetterSourceQueues | |

### Phase 4: Tags, Permissions & Move Tasks

| Priority | Operation | Notes |
|----------|-----------|-------|
| P3 | TagQueue / UntagQueue / ListQueueTags | |
| P3 | AddPermission / RemovePermission | |
| P3 | StartMessageMoveTask | |
| P3 | CancelMessageMoveTask | |
| P3 | ListMessageMoveTasks | |

### Phase 5: Advanced

| Priority | Feature | Notes |
|----------|---------|-------|
| P4 | awsQuery protocol support | For older SDK compatibility |
| P4 | KMS encryption attributes | Metadata only, no real encryption |
| P4 | High-throughput FIFO mode | |
| P4 | Full IAM policy evaluation | |

---

## 15. Architecture Considerations for RustStack

### 15.1 Crate Structure

Following the existing pattern:
- `ruststack-sqs-model` - Auto-generated types from Smithy model (via codegen)
- `ruststack-sqs-core` - Business logic (queue management, message lifecycle)
- `ruststack-sqs-http` - HTTP routing, protocol handling (JSON + Query)
- `apps/ruststack-sqs-server` - Binary server

### 15.2 Core Data Structures

```
Queue {
    name: String,
    url: String,
    arn: String,
    attributes: QueueAttributes,
    is_fifo: bool,
    messages: VecDeque<Message>,          // Available messages
    delayed_messages: BTreeMap<Instant, Message>,  // Delay timer
    in_flight: HashMap<ReceiptHandle, InFlightMessage>,  // Visibility tracking
    dedup_cache: HashMap<String, Instant>,  // 5-min dedup window (FIFO)
    message_groups: HashMap<String, MessageGroupState>,  // FIFO group tracking
    tags: HashMap<String, String>,
    last_purge: Option<Instant>,
    created_at: Instant,
    modified_at: Instant,
}

Message {
    message_id: String,  // UUID
    body: String,
    attributes: HashMap<String, String>,   // System attributes
    message_attributes: HashMap<String, MessageAttribute>,  // User attributes
    md5_of_body: String,
    md5_of_attributes: String,
    sent_timestamp: u64,
    receive_count: u32,
    sequence_number: Option<u64>,  // FIFO only
    message_group_id: Option<String>,  // FIFO only
    dedup_id: Option<String>,  // FIFO only
    delay_until: Option<Instant>,
}
```

### 15.3 Concurrency Model (Actor-based)

Following CLAUDE.md guidance for Actor model:

- **QueueManager Actor**: Owns all queues, handles create/delete/list
- **Queue Actor** (per queue): Owns messages, handles send/receive/delete
  - Uses `tokio::sync::mpsc` for command channel
  - Long-polling consumers wait on `tokio::sync::Notify`
  - Timer wheel or `tokio::time::sleep_until` for visibility timeouts and delays
  - `DashMap` for in-flight message tracking
- **MessageMoveTask Actor**: Background task for DLQ redrive

### 15.4 Long Polling Implementation

```rust
async fn receive_message(queue: &Queue, wait_time: Duration) -> Vec<Message> {
    // Try immediate receive
    if let messages = queue.try_receive() && !messages.is_empty() {
        return messages;
    }

    // Long poll: wait for notification or timeout
    tokio::select! {
        _ = queue.message_notify.notified() => {
            queue.try_receive()
        }
        _ = tokio::time::sleep(wait_time) => {
            Vec::new()
        }
    }
}
```

### 15.5 Protocol Detection and Routing

```rust
async fn handle_request(req: Request) -> Response {
    let content_type = req.headers().get("content-type");
    match content_type {
        Some("application/x-amz-json-1.0") => {
            // JSON protocol: dispatch via X-Amz-Target
            let target = req.headers().get("x-amz-target");
            handle_json_request(target, req.body()).await
        }
        Some("application/x-www-form-urlencoded") => {
            // Query protocol: dispatch via Action parameter
            let params = parse_form_urlencoded(req.body());
            let action = params.get("Action");
            handle_query_request(action, params).await
        }
        _ => error_response(400, "Unsupported Content-Type")
    }
}
```

---

## Sources

- [AWS API Models Repository](https://github.com/aws/api-models-aws)
- [AWS SQS API Reference](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/APIReference/Welcome.html)
- [AWS SQS Developer Guide](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/welcome.html)
- [AWS SQS JSON Protocol FAQ](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-json-faqs.html)
- [AWS SQS JSON Protocol Requests](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-making-api-requests-json.html)
- [AWS JSON 1.0 Protocol - Smithy 2.0](https://smithy.io/2.0/aws/protocols/aws-json-1_0-protocol.html)
- [AWS Query Protocol - Smithy 2.0](https://smithy.io/2.0/aws/protocols/aws-query-protocol.html)
- [ElasticMQ Repository](https://github.com/softwaremill/elasticmq)
- [GoAWS Repository](https://github.com/Admiral-Piett/goaws)
- [LocalStack Repository](https://github.com/localstack/localstack)
- [fake_sqs Repository](https://github.com/iain/fake_sqs)
- [aws-sdk-sqs Rust Crate](https://docs.rs/aws-sdk-sqs)
- [AWS SDK for Rust GitHub](https://github.com/awslabs/aws-sdk-rust)
- [smithy-rs GitHub](https://github.com/smithy-lang/smithy-rs)
- [SQS Visibility Timeout](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-visibility-timeout.html)
- [SQS Long Polling](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-short-and-long-polling.html)
- [SQS FIFO Queues](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-fifo-queues.html)
- [SQS Dead Letter Queues](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-dead-letter-queues.html)
- [SQS Delay Queues](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-delay-queues.html)
- [SQS High Throughput FIFO](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/high-throughput-fifo.html)
- [LocalStack SQS JSON Protocol Issue #10821](https://github.com/localstack/localstack/issues/10821)
- [AWS SDK Rust SQS Query Protocol Discussion #1156](https://github.com/awslabs/aws-sdk-rust/discussions/1156)
- [SQS FIFO Challenges - Ably Blog](https://ably.com/blog/sqs-fifo-queues-message-ordering-and-exactly-once-processing-guaranteed)
- [SQS DLQ Redrive Announcement](https://aws.amazon.com/blogs/compute/introducing-amazon-simple-queue-service-dead-letter-queue-redrive-to-source-queues/)
