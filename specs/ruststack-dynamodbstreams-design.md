# Rustack DynamoDB Streams: Native Rust Implementation Design

**Date:** 2026-03-19
**Status:** Draft / RFC
**Depends on:** [rustack-dynamodb-design.md](./rustack-dynamodb-design.md), [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md)
**Scope:** Add DynamoDB Streams support to Rustack -- 4 API operations (DescribeStream, GetShardIterator, GetRecords, ListStreams) plus deep integration into DynamoDB core for change data capture. Completes the DynamoDB -> Stream -> Lambda event pipeline.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Design: awsJson1.0](#5-protocol-design-awsjson10)
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

---

## 1. Executive Summary

DynamoDB Streams is the bridge between DynamoDB and event-driven architectures. It provides a time-ordered sequence of item-level changes (insert, modify, remove) on a DynamoDB table, enabling patterns like cross-region replication, materialized views, and Lambda triggers. Key points:

- **Small API surface, deep integration** -- only 4 DynamoDB Streams API operations, but requires instrumenting every DynamoDB write path (PutItem, UpdateItem, DeleteItem, BatchWriteItem) to capture old/new item images and emit change records. This cross-crate integration is the core architectural challenge.
- **Native in-memory change log** -- unlike LocalStack which uses Kinesis as a backing store (creating `__ddb_stream_TABLE_NAME` Kinesis streams and proxying all Streams API calls through Kinesis shard management), Rustack implements DynamoDB Streams natively with a per-table append-only change log. This is simpler (no cross-service dependency), faster (no Kinesis shard actor overhead for what is essentially a per-table ring buffer), and avoids the complexity of shard ID remapping.
- **Completes the event pipeline** -- with DynamoDB Streams, the DynamoDB -> Stream -> Lambda event source mapping pipeline becomes possible. This is one of the most common serverless patterns on AWS.
- **Two crate groups** -- (1) `rustack-dynamodbstreams-{model,http,core}` for the 4 Streams API operations, and (2) change capture integration in `rustack-dynamodb-core` via a `StreamEmitter` trait that DynamoDB Streams implements and DynamoDB core calls after each successful write.
- **awsJson1.0 protocol** -- same protocol as DynamoDB itself, but distinguished by the `X-Amz-Target` prefix `DynamoDBStreams_20120810.` (vs DynamoDB's `DynamoDB_20120810.`). SigV4 service name is `dynamodb` for both.
- **Estimated effort** -- 3-4 days for full implementation (4 API operations + DynamoDB change capture integration + testing).

---

## 2. Motivation

### 2.1 Why DynamoDB Streams?

DynamoDB Streams is the cornerstone of event-driven architectures built on DynamoDB. Without local Streams support, developers cannot test:

- **Lambda triggers** -- the most common DynamoDB Streams use case. A Lambda function processes each batch of stream records, enabling real-time reactions to data changes (e.g., updating search indices, sending notifications, maintaining aggregate counters).
- **Change data capture (CDC)** -- applications that need to react to DynamoDB item changes in near-real-time, such as syncing data to Elasticsearch, Redis, or external databases.
- **Cross-region replication** -- DynamoDB Global Tables are built on Streams. Local testing of replication logic requires Streams.
- **Event sourcing patterns** -- using DynamoDB as an event store with Streams as the event bus.
- **Materialized views** -- maintaining denormalized views or aggregation tables that are automatically updated when source data changes.

### 2.2 Complexity Assessment

| Dimension | DynamoDB Streams | DynamoDB | Kinesis |
|-----------|-----------------|----------|---------|
| Total operations | 4 | 66 | ~40 |
| Storage complexity | Append-only log per table | B-Tree + GSI/LSI | Actor-per-shard |
| Protocol | awsJson1.0 (reuse DDB) | awsJson1.0 | awsJson1.1 |
| Cross-service integration | Deep (instruments DDB writes) | None | None |
| Concurrency model | Read-heavy, append-only | Transactions, batch | Streaming, fan-out |
| Estimated lines of code | ~2,500 (Streams) + ~500 (DDB integration) | ~15,000 | ~8,000 |

DynamoDB Streams is small in API surface but architecturally significant because it introduces the first cross-crate data flow in Rustack: DynamoDB core must emit change records that DynamoDB Streams core consumes. This requires careful trait design to avoid tight coupling.

### 2.3 Tool Coverage

With all 4 operations implemented plus DynamoDB-side change capture, the following tools work:

| Tool | Operations Used | Notes |
|------|----------------|-------|
| AWS CLI (`aws dynamodbstreams`) | All 4 operations | Direct API testing |
| AWS CLI (`aws dynamodb` with StreamSpecification) | CreateTable, UpdateTable | Enable/disable streams |
| Terraform (`aws_dynamodb_table` with `stream_enabled`) | CreateTable + DescribeStream | Stream ARN in outputs |
| AWS CDK (`Table` with `stream`) | CreateTable + Lambda event source | Full CDC pipeline |
| aws-sdk-rust / aws-sdk-js | All 4 operations | SDK integration |
| Lambda event source mappings | GetRecords, GetShardIterator | Polling-based trigger |
| DynamoDB Local (reference) | All 4 operations | Compatibility baseline |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Full DynamoDB Streams API** -- implement all 4 operations: DescribeStream, GetShardIterator, GetRecords, ListStreams
2. **Change data capture** -- DynamoDB PutItem, UpdateItem, DeleteItem, and BatchWriteItem emit stream records with correct old/new images based on StreamViewType
3. **Stream lifecycle** -- CreateTable with StreamSpecification creates a stream; UpdateTable can enable/disable streams; DeleteTable removes the stream
4. **Four StreamViewType modes** -- KEYS_ONLY, NEW_IMAGE, OLD_IMAGE, NEW_AND_OLD_IMAGES with correct image filtering
5. **Shard iterator types** -- TRIM_HORIZON, LATEST, AT_SEQUENCE_NUMBER, AFTER_SEQUENCE_NUMBER
6. **Monotonically increasing sequence numbers** -- zero-padded string format matching real DynamoDB behavior
7. **Stream ARN format** -- `arn:aws:dynamodb:{region}:{account}:table/{table}/stream/{timestamp}` matching AWS format
8. **Smithy-generated types** -- all request/response types generated from official AWS Smithy model
9. **Same Docker image** -- single binary serves all services including DynamoDB Streams on port 4566
10. **Cross-crate integration via trait** -- clean `StreamEmitter` trait boundary between DynamoDB core and Streams core

### 3.2 Non-Goals

1. **Multiple shards per table** -- for MVP, each table has exactly one shard. Real DynamoDB automatically splits shards based on throughput. Single shard is sufficient for local development.
2. **Shard splitting/closing** -- no automatic shard rotation based on size or age. The single shard remains open indefinitely.
3. **24-hour record retention enforcement** -- real DynamoDB Streams retains records for exactly 24 hours. For local dev, records are retained until memory pressure or configurable limit (default: keep all).
4. **Iterator expiration** -- real DynamoDB expires shard iterators after 15 minutes. For local dev, iterators do not expire (configurable).
5. **Cross-region stream replication** -- no multi-region support.
6. **Lambda event source mapping integration** -- Streams provides the data; the polling/batching logic lives in Lambda. Lambda integration is a separate concern.
7. **Kinesis adapter compatibility** -- the DynamoDB Streams Kinesis Adapter (KCL-based) translates Streams into Kinesis-compatible interface. Not in scope.
8. **Data persistence across restarts** -- in-memory only, matching all other Rustack services.
9. **StreamViewType change on existing stream** -- real DynamoDB does not allow changing StreamViewType on an existing stream (must disable and re-enable). We enforce this.

---

## 4. Architecture Overview

### 4.1 Two-Component Architecture

DynamoDB Streams requires two distinct components:

1. **DynamoDB Streams API crate** (`rustack-dynamodbstreams-{model,http,core}`) -- handles the 4 Streams API operations (DescribeStream, GetShardIterator, GetRecords, ListStreams). This is a standard service crate following the same pattern as SSM, Secrets Manager, etc.

2. **Change capture integration in `rustack-dynamodb-core`** -- instruments DynamoDB write operations to emit change records. This is the architecturally significant piece: DynamoDB core must call into Streams after each successful write, passing old/new item images.

```
          AWS SDK / CLI / Terraform / Lambda ESM
                     |
                     | HTTP POST :4566
                     v
          +---------------------+
          |   Gateway Router    |  X-Amz-Target dispatch
          +--------+------------+
                   |
     +-------+----+----+--------+
     |       |         |        |
     v       v         v        v
  +-----+ +-----+ +--------+ +--------+
  | DDB | | ... | | DDB    | | ...    |
  |(J10)| |     | |Streams | |        |
  +--+--+ +-----+ |(J10)  | +--------+
     |             +---+----+
     |                 |
  +--+--+          +---+----+
  |DDB  |--------->|DDB     |
  |Core | emits    |Streams |
  |     | change   |Core    |
  +-----+ records  +--------+
     |                 |
     |  StreamEmitter  |
     |  trait callback |
     +-----------------+
```

### 4.2 Gateway Routing

DynamoDB and DynamoDB Streams are distinguished solely by the `X-Amz-Target` header prefix:

| Service | X-Amz-Target Prefix | Content-Type | SigV4 Service |
|---------|---------------------|--------------|---------------|
| DynamoDB | `DynamoDB_20120810.` | `application/x-amz-json-1.0` | `dynamodb` |
| DynamoDB Streams | `DynamoDBStreams_20120810.` | `application/x-amz-json-1.0` | `dynamodb` |

This is unambiguous: `DynamoDBStreams_20120810.` is a distinct prefix from `DynamoDB_20120810.`. The gateway registers the DynamoDB Streams router before S3 (catch-all) and its `matches()` checks for the `DynamoDBStreams_` prefix.

Note that both services share the same SigV4 service name (`dynamodb`), which means the existing auth infrastructure handles both without changes.

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-dynamodb-{model,core,http}
+-- rustack-dynamodbstreams-model        <-- NEW (auto-generated)
+-- rustack-dynamodbstreams-core         <-- NEW
+-- rustack-dynamodbstreams-http         <-- NEW
+-- ... (other services)

rustack-dynamodbstreams-http
+-- rustack-dynamodbstreams-model
+-- rustack-auth

rustack-dynamodbstreams-core
+-- rustack-core
+-- rustack-dynamodbstreams-model
+-- rustack-dynamodb-model              <-- for AttributeValue, KeySchemaElement types

rustack-dynamodb-core (MODIFIED)
+-- rustack-core
+-- rustack-dynamodb-model
+-- (NO dependency on dynamodbstreams-core; uses trait inversion)
```

The key architectural insight is that `rustack-dynamodb-core` does NOT depend on `rustack-dynamodbstreams-core`. Instead, DynamoDB core defines a `StreamEmitter` trait, and the server binary wires in the concrete implementation from DynamoDB Streams core. This follows the dependency inversion principle and keeps the two crates independently compilable.

### 4.4 Cross-Crate Integration Pattern

```
                    rustack-dynamodb-core
                    +------------------------+
                    | trait StreamEmitter {   |
                    |   fn emit_change(...)   |
                    | }                       |
                    |                         |
                    | struct RustackDynamoDB|
                    |   emitter: Option<      |
                    |     Arc<dyn StreamEmitter>|
                    |   >                     |
                    +------------+------------+
                                 ^
                                 | implements
                    +------------+------------+
                    | rustack-dynamodbstreams-core |
                    | struct DynamoDBStreamEmitter   |
                    |   impl StreamEmitter           |
                    +-------------------------------+
                                 ^
                                 | wired by
                    +------------+------------+
                    | rustack (main.rs)    |
                    | let emitter = StreamEmitter;  |
                    | dynamodb.set_emitter(emitter); |
                    +-------------------------------+
```

---

## 5. Protocol Design: awsJson1.0

### 5.1 Protocol Details

DynamoDB Streams uses `awsJson1.0`, identical to DynamoDB. The only difference is the `X-Amz-Target` prefix.

| Aspect | DynamoDB (awsJson1.0) | DynamoDB Streams (awsJson1.0) |
|--------|----------------------|-------------------------------|
| HTTP Method | POST only | POST only |
| URL Path | `/` always | `/` always |
| Content-Type | `application/x-amz-json-1.0` | `application/x-amz-json-1.0` |
| X-Amz-Target | `DynamoDB_20120810.<Op>` | `DynamoDBStreams_20120810.<Op>` |
| Request body | JSON | JSON |
| Response body | JSON | JSON |
| Error `__type` | Short name | Short name |
| Timestamp format | Epoch seconds (double) | Epoch seconds (double) |
| Auth | SigV4, service=`dynamodb` | SigV4, service=`dynamodb` |

### 5.2 What We Reuse from DynamoDB

The DynamoDB implementation provides all the infrastructure DynamoDB Streams needs:

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| JSON request deserialization | Yes | `serde_json::from_slice` with `Deserialize` derives |
| JSON response serialization | Yes | `serde_json::to_vec` with `Serialize` derives |
| `X-Amz-Target` header parsing | Yes | Same pattern, different prefix |
| JSON error formatting | Yes | Same `{"__type": "...", "Message": "..."}` format |
| SigV4 auth | Yes | Same service name `dynamodb` |
| `AttributeValue` type | Yes | Stream records reference DynamoDB `AttributeValue` |
| `KeySchemaElement` type | Yes | DescribeStream returns table's KeySchema |
| `StreamViewType` enum | Yes | Already defined in `rustack-dynamodb-model` |
| `StreamSpecification` struct | Yes | Already defined in `rustack-dynamodb-model` |

### 5.3 Wire Format Examples

**ListStreams request:**

```
POST / HTTP/1.1
Content-Type: application/x-amz-json-1.0
X-Amz-Target: DynamoDBStreams_20120810.ListStreams

{
    "TableName": "MyTable"
}
```

**ListStreams response:**

```json
{
    "Streams": [
        {
            "StreamArn": "arn:aws:dynamodb:us-east-1:000000000000:table/MyTable/stream/2026-03-19T10:00:00.000",
            "StreamLabel": "2026-03-19T10:00:00.000",
            "TableName": "MyTable"
        }
    ]
}
```

**GetRecords request:**

```
POST / HTTP/1.1
Content-Type: application/x-amz-json-1.0
X-Amz-Target: DynamoDBStreams_20120810.GetRecords

{
    "ShardIterator": "arn:aws:dynamodb:us-east-1:000000000000:table/MyTable/stream/2026-03-19T10:00:00.000|shardId-00000000-0000-0000-0000-000000000000|0",
    "Limit": 100
}
```

**GetRecords response:**

```json
{
    "Records": [
        {
            "eventID": "c81e7281-1e90-4e2e-b8b4-7c8e9e2f3a4b",
            "eventName": "INSERT",
            "eventVersion": "1.1",
            "eventSource": "aws:dynamodb",
            "awsRegion": "us-east-1",
            "dynamodb": {
                "ApproximateCreationDateTime": 1710842400,
                "Keys": {
                    "pk": {"S": "user#123"}
                },
                "NewImage": {
                    "pk": {"S": "user#123"},
                    "name": {"S": "Alice"}
                },
                "SequenceNumber": "000000000000000000001",
                "SizeBytes": 42,
                "StreamViewType": "NEW_AND_OLD_IMAGES"
            }
        }
    ],
    "NextShardIterator": "arn:aws:dynamodb:us-east-1:000000000000:table/MyTable/stream/2026-03-19T10:00:00.000|shardId-00000000-0000-0000-0000-000000000000|1"
}
```

**Error response:**

```json
{
    "__type": "ResourceNotFoundException",
    "message": "Requested resource not found: Stream: arn:aws:dynamodb:us-east-1:000000000000:table/NoSuchTable/stream/2026-03-19T10:00:00.000 not found"
}
```

---

## 6. Smithy Code Generation Strategy

### 6.1 Smithy Model

DynamoDB Streams has its own Smithy model, separate from DynamoDB. The model defines:
- 4 operations: `DescribeStream`, `GetShardIterator`, `GetRecords`, `ListStreams`
- Types specific to Streams: `Stream`, `StreamDescription`, `Shard`, `SequenceNumberRange`, `StreamRecord`, `Record`
- Shared types with DynamoDB: `AttributeValue`, `KeySchemaElement`, `StreamViewType`

**Smithy model:** `codegen/smithy-model/dynamodbstreams.json` (download from AWS)
**Service config:** `codegen/services/dynamodbstreams.toml`
**Generate:** `make codegen-dynamodbstreams`

### 6.2 Codegen TOML Configuration

```toml
[service]
name = "dynamodbstreams"
display_name = "DynamoDB Streams"
rust_prefix = "DynamoDBStreams"
namespace = "com.amazonaws.dynamodbstreams"
protocol = "awsJson1_0"

[protocol]
serde_rename = "PascalCase"
emit_serde_derives = true
target_prefix = "DynamoDBStreams_20120810"

[operations]
phase0 = [
    "DescribeStream",
    "GetShardIterator",
    "GetRecords",
    "ListStreams",
]

[errors.custom]
MissingAction = { status = 400, message = "Missing required header: X-Amz-Target" }
InvalidAction = { status = 400, message = "Operation is not supported" }

[output]
file_layout = "flat"
```

### 6.3 Generated Output

The codegen produces 6 files in `crates/rustack-dynamodbstreams-model/src/`:

| File | Contents |
|------|----------|
| `lib.rs` | Module declarations and re-exports |
| `types.rs` | Stream, StreamDescription, Shard, SequenceNumberRange, StreamRecord, Record, ShardIteratorType, etc. |
| `operations.rs` | `DynamoDBStreamsOperation` enum with `as_str()`, `from_name()`, phase methods |
| `error.rs` | `DynamoDBStreamsErrorCode` enum + `DynamoDBStreamsError` struct |
| `input.rs` | All 4 input structs with `#[serde(rename_all = "PascalCase")]` |
| `output.rs` | All 4 output structs with serde derives |

### 6.4 Service-Specific Notes

DynamoDB Streams model types reference `AttributeValue` from DynamoDB. The codegen has two options:

1. **Duplicate `AttributeValue` in the Streams model crate** -- simpler codegen, but type mismatch when passing data between DynamoDB and Streams.
2. **Re-export `AttributeValue` from `rustack-dynamodb-model`** -- requires the Streams model crate to depend on the DynamoDB model crate.

We choose option 2: `rustack-dynamodbstreams-model` depends on `rustack-dynamodb-model` and re-exports `AttributeValue`. The codegen configuration includes an import mapping that replaces the generated `AttributeValue` with a re-export. This ensures type compatibility when DynamoDB core passes item images to Streams core.

---

## 7. Crate Structure

### 7.1 `rustack-dynamodbstreams-model` (auto-generated)

```
crates/rustack-dynamodbstreams-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports + pub use rustack_dynamodb_model::AttributeValue
    +-- types.rs            # Auto-generated: Stream, StreamDescription, Shard, etc.
    +-- operations.rs       # Auto-generated: DynamoDBStreamsOperation enum
    +-- error.rs            # Auto-generated: error types + error codes
    +-- input.rs            # Auto-generated: 4 input structs
    +-- output.rs           # Auto-generated: 4 output structs
```

**Dependencies:** `serde`, `serde_json`, `rustack-dynamodb-model`

### 7.2 `rustack-dynamodbstreams-core`

```
crates/rustack-dynamodbstreams-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # DynamoDBStreamsConfig
    +-- handler.rs          # RustackDynamoDBStreamsHandler (bridges HTTP to provider)
    +-- provider.rs         # RustackDynamoDBStreams (main provider, all 4 operations)
    +-- emitter.rs          # DynamoDBStreamEmitter (implements StreamEmitter trait)
    +-- storage.rs          # StreamStore, StreamRecord, ShardRecord, StreamChangeRecord
    +-- iterator.rs         # ShardIterator encoding/decoding, iterator state
```

**Dependencies:** `rustack-core`, `rustack-dynamodbstreams-model`, `rustack-dynamodb-model`, `dashmap`, `serde_json`, `tracing`, `uuid`, `chrono`, `parking_lot`

### 7.3 `rustack-dynamodbstreams-http`

```
crates/rustack-dynamodbstreams-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # DynamoDBStreams_20120810.* target dispatch
    +-- service.rs          # DynamoDBStreamsHttpService (hyper Service impl)
    +-- dispatch.rs         # DynamoDBStreamsHandler trait + operation dispatch
    +-- body.rs             # Response body type
    +-- response.rs         # HTTP response construction
```

**Dependencies:** `rustack-dynamodbstreams-model`, `rustack-auth`, `hyper`, `http`, `serde_json`, `bytes`

This crate is structurally identical to `rustack-dynamodb-http`. The router parses `DynamoDBStreams_20120810.<Op>` instead of `DynamoDB_20120810.<Op>`.

### 7.4 Modifications to `rustack-dynamodb-core`

The DynamoDB core crate is modified to support change capture:

```
crates/rustack-dynamodb-core/
+-- src/
    +-- lib.rs              # (add stream module)
    +-- stream.rs           # NEW: StreamEmitter trait + ChangeEvent types
    +-- provider.rs         # MODIFIED: call emitter after writes
    +-- state.rs            # MODIFIED: store stream_arn on DynamoDBTable
```

The `stream.rs` module defines the `StreamEmitter` trait and `ChangeEvent` types. These types use `rustack-dynamodb-model::AttributeValue` directly, so no new dependencies are needed.

### 7.5 Workspace Changes

```toml
[workspace.dependencies]
rustack-dynamodbstreams-model = { path = "crates/rustack-dynamodbstreams-model" }
rustack-dynamodbstreams-http = { path = "crates/rustack-dynamodbstreams-http" }
rustack-dynamodbstreams-core = { path = "crates/rustack-dynamodbstreams-core" }
```

---

## 8. HTTP Layer Design

### 8.1 Router

```rust
/// DynamoDB Streams operation router.
///
/// Parses the `X-Amz-Target: DynamoDBStreams_20120810.<Op>` header to determine
/// the operation.
pub struct DynamoDBStreamsRouter;

impl DynamoDBStreamsRouter {
    pub fn resolve(target: &str) -> Result<DynamoDBStreamsOperation, DynamoDBStreamsError> {
        let op_name = target
            .strip_prefix("DynamoDBStreams_20120810.")
            .ok_or_else(|| DynamoDBStreamsError::unknown_operation(target))?;

        DynamoDBStreamsOperation::from_name(op_name)
            .ok_or_else(|| DynamoDBStreamsError::unknown_operation(op_name))
    }
}
```

### 8.2 ServiceRouter Trait Implementation

```rust
/// DynamoDB Streams service router for the gateway.
pub struct DynamoDBStreamsServiceRouter<H: DynamoDBStreamsHandler> {
    inner: DynamoDBStreamsHttpService<H>,
}

impl<H: DynamoDBStreamsHandler> ServiceRouter for DynamoDBStreamsServiceRouter<H> {
    fn name(&self) -> &'static str {
        "dynamodbstreams"
    }

    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        req.headers()
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("DynamoDBStreams_"))
    }

    fn call(
        &self,
        req: http::Request<Incoming>,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>> {
        let svc = self.inner.clone();
        Box::pin(async move {
            let resp = svc.call(req).await;
            Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
        })
    }
}
```

### 8.3 Handler Trait

```rust
/// Trait that the DynamoDB Streams business logic provider must implement.
pub trait DynamoDBStreamsHandler: Send + Sync + 'static {
    /// Handle a DynamoDB Streams operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: DynamoDBStreamsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<
        http::Response<DynamoDBStreamsResponseBody>,
        DynamoDBStreamsError,
    >> + Send>>;
}
```

---

## 9. Storage Engine Design

This is the core of the spec. The storage engine has two sides: the **change capture** side (in `rustack-dynamodb-core`) that emits records, and the **stream store** side (in `rustack-dynamodbstreams-core`) that stores and serves records.

### 9.1 StreamEmitter Trait (in `rustack-dynamodb-core`)

The `StreamEmitter` trait is defined in `rustack-dynamodb-core` and implemented by `rustack-dynamodbstreams-core`. This follows the dependency inversion principle: DynamoDB core depends only on its own trait, not on the Streams crate.

```rust
// crates/rustack-dynamodb-core/src/stream.rs

use std::collections::HashMap;
use std::sync::Arc;

use rustack_dynamodb_model::AttributeValue;
use rustack_dynamodb_model::types::StreamViewType;

/// Event name for a stream change record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeEventName {
    /// A new item was inserted.
    Insert,
    /// An existing item was modified.
    Modify,
    /// An item was removed.
    Remove,
}

impl ChangeEventName {
    /// Returns the DynamoDB Streams wire-format string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Modify => "MODIFY",
            Self::Remove => "REMOVE",
        }
    }
}

/// A change event emitted by DynamoDB core after a successful write.
///
/// Contains all the information needed to produce a DynamoDB Streams record.
/// The `stream_view_type` filtering is applied by the consumer (Streams core),
/// not the emitter. DynamoDB core always provides both old and new images when
/// available; the Streams consumer strips fields based on the table's
/// `StreamViewType` configuration.
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    /// The table name.
    pub table_name: String,
    /// The event type.
    pub event_name: ChangeEventName,
    /// The primary key attributes of the affected item.
    pub keys: HashMap<String, AttributeValue>,
    /// The item as it appeared before the write (None for INSERT).
    pub old_image: Option<HashMap<String, AttributeValue>>,
    /// The item as it appeared after the write (None for REMOVE).
    pub new_image: Option<HashMap<String, AttributeValue>>,
    /// Approximate size of the affected item in bytes.
    pub size_bytes: u64,
}

/// Trait for emitting DynamoDB change events to a stream consumer.
///
/// DynamoDB core calls `emit` after each successful write operation.
/// The implementation is provided by `rustack-dynamodbstreams-core` and
/// wired in by the server binary.
///
/// This trait is defined in `rustack-dynamodb-core` to avoid a dependency
/// from DynamoDB core on the Streams crate (dependency inversion).
pub trait StreamEmitter: Send + Sync + 'static {
    /// Emit a change event for a successful write operation.
    ///
    /// This method must not block. If the stream is disabled for the table,
    /// the implementation should silently discard the event.
    fn emit(&self, event: ChangeEvent);
}

/// A no-op emitter that discards all events.
///
/// Used when DynamoDB Streams is not enabled (feature gate off or no
/// stream configured for the table).
#[derive(Debug)]
pub struct NoopStreamEmitter;

impl StreamEmitter for NoopStreamEmitter {
    fn emit(&self, _event: ChangeEvent) {
        // Intentionally empty.
    }
}
```

### 9.2 DynamoDB Provider Modifications

The `RustackDynamoDB` provider is modified to hold an optional `StreamEmitter` and call it after each successful write.

```rust
// In crates/rustack-dynamodb-core/src/provider.rs

use crate::stream::{ChangeEvent, ChangeEventName, NoopStreamEmitter, StreamEmitter};

pub struct RustackDynamoDB {
    pub(crate) state: Arc<DynamoDBServiceState>,
    pub(crate) config: Arc<DynamoDBConfig>,
    pub(crate) emitter: Arc<dyn StreamEmitter>,
}

impl RustackDynamoDB {
    /// Create a new DynamoDB provider with no stream emitter.
    pub fn new(config: DynamoDBConfig) -> Self {
        Self {
            state: Arc::new(DynamoDBServiceState::new()),
            config: Arc::new(config),
            emitter: Arc::new(NoopStreamEmitter),
        }
    }

    /// Set the stream emitter for change data capture.
    ///
    /// Called by the server binary to wire in the DynamoDB Streams
    /// implementation.
    pub fn set_emitter(&mut self, emitter: Arc<dyn StreamEmitter>) {
        self.emitter = emitter;
    }
}
```

### 9.3 Change Capture in Write Operations

Each write operation captures the old item (before the write) and the new item (after the write), then emits a `ChangeEvent`. The key requirement is that old image capture happens **before** the write and new image capture happens **after**.

```rust
// In PutItem handler (simplified):

fn handle_put_item(&self, input: PutItemInput) -> Result<PutItemOutput, DynamoDBError> {
    let table = self.state.require_table(&input.table_name)?;

    // ... validation, condition expression evaluation ...

    // Capture old image BEFORE the write.
    let old_item = table.storage.get_item(&primary_key);

    // Perform the write.
    table.storage.put_item(primary_key.clone(), item.clone());

    // Emit stream event if stream is enabled for this table.
    if table.stream_specification.as_ref().is_some_and(|s| s.stream_enabled) {
        let event_name = if old_item.is_some() {
            ChangeEventName::Modify
        } else {
            ChangeEventName::Insert
        };

        let keys = extract_key_attributes(&item, &table.key_schema);

        self.emitter.emit(ChangeEvent {
            table_name: table.name.clone(),
            event_name,
            keys,
            old_image: old_item,
            new_image: Some(item),
            size_bytes: item_size,
        });
    }

    // ... build response ...
}

// In UpdateItem handler (simplified):

fn handle_update_item(&self, input: UpdateItemInput) -> Result<UpdateItemOutput, DynamoDBError> {
    let table = self.state.require_table(&input.table_name)?;

    // Capture old image BEFORE the update.
    let old_item = table.storage.get_item(&primary_key);

    // Apply update expressions...
    let new_item = apply_updates(old_item.as_ref(), &update_expr);

    // Perform the write.
    table.storage.put_item(primary_key.clone(), new_item.clone());

    // Emit stream event.
    if table.stream_specification.as_ref().is_some_and(|s| s.stream_enabled) {
        let event_name = if old_item.is_some() {
            ChangeEventName::Modify
        } else {
            ChangeEventName::Insert
        };

        let keys = extract_key_attributes(&new_item, &table.key_schema);

        self.emitter.emit(ChangeEvent {
            table_name: table.name.clone(),
            event_name,
            keys,
            old_image: old_item,
            new_image: Some(new_item),
            size_bytes: item_size,
        });
    }

    // ... build response ...
}

// In DeleteItem handler (simplified):

fn handle_delete_item(&self, input: DeleteItemInput) -> Result<DeleteItemOutput, DynamoDBError> {
    let table = self.state.require_table(&input.table_name)?;

    // Perform the delete (returns the old item if it existed).
    let old_item = table.storage.delete_item(&primary_key);

    // Emit stream event only if an item was actually deleted.
    if let Some(ref old) = old_item {
        if table.stream_specification.as_ref().is_some_and(|s| s.stream_enabled) {
            let keys = extract_key_attributes(old, &table.key_schema);

            self.emitter.emit(ChangeEvent {
                table_name: table.name.clone(),
                event_name: ChangeEventName::Remove,
                keys,
                old_image: old_item.clone(),
                new_image: None,
                size_bytes: item_size,
            });
        }
    }

    // ... build response ...
}

// In BatchWriteItem handler:
// Each individual PutItem/DeleteItem within the batch emits its own
// ChangeEvent. The emitter receives one event per item, not one per batch.
```

**Performance note:** Capturing the old image requires an additional read before each write. For PutItem, the storage engine's `put_item` already returns the replaced item in some implementations. For UpdateItem, the old image is naturally available from the read-modify-write cycle. For DeleteItem, the storage engine's `delete_item` returns the removed item. In practice, the overhead is minimal because the old item is often already in the read path.

### 9.4 Stream Store (in `rustack-dynamodbstreams-core`)

The `StreamStore` holds per-table change logs and serves the 4 Streams API operations.

```rust
// crates/rustack-dynamodbstreams-core/src/storage.rs

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use dashmap::DashMap;
use parking_lot::RwLock;

use rustack_dynamodb_model::AttributeValue;
use rustack_dynamodb_model::types::{KeySchemaElement, StreamViewType};

/// Top-level stream store managing all DynamoDB Streams.
///
/// Keyed by table name. Each table with streams enabled has exactly one
/// `TableStream` entry.
#[derive(Debug)]
pub struct StreamStore {
    /// Active streams keyed by table name.
    streams: DashMap<String, TableStream>,
}

/// A single DynamoDB Stream associated with a table.
///
/// Contains the stream metadata and the change log (shards).
#[derive(Debug)]
pub struct TableStream {
    /// Stream ARN: `arn:aws:dynamodb:{region}:{account}:table/{table}/stream/{label}`
    pub stream_arn: String,
    /// Table name this stream belongs to.
    pub table_name: String,
    /// Stream label: ISO 8601 timestamp when the stream was created.
    /// Format: `YYYY-MM-DDTHH:MM:SS.sss`
    pub stream_label: String,
    /// What information is captured in stream records.
    pub stream_view_type: StreamViewType,
    /// Stream status: ENABLING -> ENABLED -> DISABLING -> DISABLED.
    pub stream_status: StreamStatus,
    /// Table's key schema (needed for DescribeStream response).
    pub key_schema: Vec<KeySchemaElement>,
    /// Table ARN (for DescribeStream response).
    pub table_arn: String,
    /// The single shard for this stream (MVP: one shard per table).
    pub shard: RwLock<ShardRecord>,
}

/// Stream status lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStatus {
    /// Stream is being created.
    Enabling,
    /// Stream is active and accepting records.
    Enabled,
    /// Stream is being disabled.
    Disabling,
    /// Stream is disabled (shard is closed, still readable).
    Disabled,
}

impl StreamStatus {
    /// Returns the DynamoDB Streams wire-format string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabling => "ENABLING",
            Self::Enabled => "ENABLED",
            Self::Disabling => "DISABLING",
            Self::Disabled => "DISABLED",
        }
    }
}

/// A shard within a DynamoDB Stream.
///
/// For MVP, each table has exactly one shard. The shard is an append-only
/// log of change records with monotonically increasing sequence numbers.
#[derive(Debug)]
pub struct ShardRecord {
    /// Shard ID in DynamoDB Streams format.
    /// Format: `shardId-00000000-0000-0000-0000-000000000000`
    pub shard_id: String,
    /// Parent shard ID (None for the first shard).
    pub parent_shard_id: Option<String>,
    /// Starting sequence number (first record in this shard).
    pub starting_sequence_number: Option<String>,
    /// Ending sequence number (last record; None if shard is open).
    pub ending_sequence_number: Option<String>,
    /// Change records in chronological order.
    pub records: VecDeque<StreamChangeRecord>,
    /// Next sequence number to assign.
    pub next_sequence_number: AtomicU64,
}

impl ShardRecord {
    /// Create a new open shard with the given ID.
    pub fn new(shard_id: String) -> Self {
        Self {
            shard_id,
            parent_shard_id: None,
            starting_sequence_number: None,
            ending_sequence_number: None,
            records: VecDeque::new(),
            next_sequence_number: AtomicU64::new(1),
        }
    }

    /// Append a change record to this shard.
    ///
    /// Assigns a monotonically increasing sequence number and returns it.
    pub fn append(&mut self, mut record: StreamChangeRecord) -> String {
        let seq = self.next_sequence_number.fetch_add(1, AtomicOrdering::SeqCst);
        let seq_str = format!("{seq:021}");

        record.dynamodb.sequence_number = seq_str.clone();

        if self.starting_sequence_number.is_none() {
            self.starting_sequence_number = Some(seq_str.clone());
        }

        self.records.push_back(record);
        seq_str
    }

    /// Close this shard, setting the ending sequence number.
    pub fn close(&mut self) {
        if let Some(last) = self.records.back() {
            self.ending_sequence_number =
                Some(last.dynamodb.sequence_number.clone());
        }
    }
}

/// A single change record in a DynamoDB Stream.
///
/// Matches the `Record` structure in the DynamoDB Streams API response.
#[derive(Debug, Clone)]
pub struct StreamChangeRecord {
    /// Unique identifier for this event.
    /// Format: UUID v4.
    pub event_id: String,
    /// Type of change: INSERT, MODIFY, or REMOVE.
    pub event_name: String,
    /// Event version (always "1.1").
    pub event_version: String,
    /// Event source (always "aws:dynamodb").
    pub event_source: String,
    /// AWS region.
    pub aws_region: String,
    /// The DynamoDB-specific portion of the record.
    pub dynamodb: StreamRecordData,
}

/// The `dynamodb` field within a stream record.
///
/// Contains the actual item data (keys, images) and metadata.
#[derive(Debug, Clone)]
pub struct StreamRecordData {
    /// The primary key attributes for the affected item.
    pub keys: HashMap<String, AttributeValue>,
    /// The item as it appeared after the modification (for INSERT/MODIFY).
    /// Filtered according to StreamViewType.
    pub new_image: Option<HashMap<String, AttributeValue>>,
    /// The item as it appeared before the modification (for MODIFY/REMOVE).
    /// Filtered according to StreamViewType.
    pub old_image: Option<HashMap<String, AttributeValue>>,
    /// Monotonically increasing sequence number within the shard.
    /// Format: zero-padded 21-digit string (e.g., "000000000000000000001").
    pub sequence_number: String,
    /// Approximate size of the stream record in bytes.
    pub size_bytes: u64,
    /// The StreamViewType for this record.
    pub stream_view_type: StreamViewType,
    /// Approximate creation date/time (epoch seconds).
    pub approximate_creation_date_time: f64,
}
```

### 9.5 StreamViewType Filtering

When a change event arrives from DynamoDB core, the Streams store filters the images according to the table's `StreamViewType`:

```rust
impl TableStream {
    /// Convert a ChangeEvent from DynamoDB core into a StreamChangeRecord,
    /// applying the StreamViewType filter.
    fn create_record(
        &self,
        event: &ChangeEvent,
        region: &str,
    ) -> StreamChangeRecord {
        let (new_image, old_image) = match self.stream_view_type {
            StreamViewType::KeysOnly => (None, None),
            StreamViewType::NewImage => (event.new_image.clone(), None),
            StreamViewType::OldImage => (None, event.old_image.clone()),
            StreamViewType::NewAndOldImages => {
                (event.new_image.clone(), event.old_image.clone())
            }
        };

        StreamChangeRecord {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_name: event.event_name.as_str().to_string(),
            event_version: "1.1".to_string(),
            event_source: "aws:dynamodb".to_string(),
            aws_region: region.to_string(),
            dynamodb: StreamRecordData {
                keys: event.keys.clone(),
                new_image,
                old_image,
                sequence_number: String::new(), // assigned by ShardRecord::append
                size_bytes: event.size_bytes,
                stream_view_type: self.stream_view_type.clone(),
                approximate_creation_date_time: chrono::Utc::now().timestamp() as f64,
            },
        }
    }
}
```

### 9.6 Shard Iterator Encoding

Shard iterators are opaque tokens that encode the stream ARN, shard ID, and position. For local development, we use a simple pipe-delimited format:

```rust
// crates/rustack-dynamodbstreams-core/src/iterator.rs

/// Shard iterator types supported by DynamoDB Streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShardIteratorType {
    /// Start at the oldest record in the shard.
    TrimHorizon,
    /// Start at the newest record (records arriving after this call).
    Latest,
    /// Start at the record with the exact sequence number.
    AtSequenceNumber,
    /// Start at the record after the given sequence number.
    AfterSequenceNumber,
}

/// Encodes a shard iterator as an opaque token.
///
/// Format: `{stream_arn}|{shard_id}|{position}`
///
/// Where `position` is the 0-based index into the shard's record buffer.
pub fn encode_iterator(stream_arn: &str, shard_id: &str, position: u64) -> String {
    format!("{stream_arn}|{shard_id}|{position}")
}

/// Decodes a shard iterator token into its components.
///
/// Returns `(stream_arn, shard_id, position)` or an error if the token
/// is malformed.
pub fn decode_iterator(token: &str) -> Result<(&str, &str, u64), DynamoDBStreamsError> {
    let parts: Vec<&str> = token.splitn(3, '|').collect();
    if parts.len() != 3 {
        return Err(DynamoDBStreamsError::trimmed_data_access(
            "The shard iterator is expired or invalid.",
        ));
    }

    let position = parts[2].parse::<u64>().map_err(|_| {
        DynamoDBStreamsError::trimmed_data_access(
            "The shard iterator is expired or invalid.",
        )
    })?;

    Ok((parts[0], parts[1], position))
}
```

### 9.7 Stream ARN Format

```rust
/// Construct a DynamoDB Streams ARN.
///
/// Format: `arn:aws:dynamodb:{region}:{account}:table/{table}/stream/{label}`
///
/// The stream label is an ISO 8601 timestamp when the stream was created.
fn stream_arn(region: &str, account_id: &str, table_name: &str, stream_label: &str) -> String {
    format!(
        "arn:aws:dynamodb:{region}:{account_id}:table/{table_name}/stream/{stream_label}"
    )
}

/// Generate a stream label from the current timestamp.
///
/// Format: `YYYY-MM-DDTHH:MM:SS.sss` (ISO 8601 with milliseconds).
fn generate_stream_label() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string()
}
```

### 9.8 Record Retention

Real DynamoDB Streams retains records for exactly 24 hours. For local development, we implement configurable retention:

```rust
/// Configuration for record retention.
pub struct RetentionConfig {
    /// Maximum number of records per shard. 0 = unlimited.
    pub max_records_per_shard: usize,
    /// Maximum age of records in seconds. 0 = unlimited.
    pub max_record_age_seconds: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_records_per_shard: 0,    // Keep all
            max_record_age_seconds: 0,   // Keep forever
        }
    }
}
```

When `max_records_per_shard` is non-zero, older records are evicted from the front of the `VecDeque` when the limit is exceeded. When `max_record_age_seconds` is non-zero, records older than the threshold are lazily evicted on `GetRecords` calls.

### 9.9 Concurrency Model

The stream store uses `DashMap` for the table-to-stream mapping and `parking_lot::RwLock` for individual shard access:

- **Writes** (appending change records): acquire write lock on the shard's `RwLock`. This is called from DynamoDB write handlers, which already hold per-partition locks in `DashMap`. The shard write lock is held briefly (just the append) and does not contend with DynamoDB's partition locks.
- **Reads** (GetRecords, DescribeStream): acquire read lock on the shard's `RwLock`. Multiple concurrent readers do not block each other.
- **Stream creation/deletion** (CreateTable/UpdateTable/DeleteTable): per-entry write via `DashMap`.

This model is simple and sufficient. DynamoDB Streams for local dev does not face high concurrency; the single-shard-per-table design means no shard-level routing overhead.

---

## 10. Core Business Logic

### 10.1 DynamoDB Stream Emitter

The `DynamoDBStreamEmitter` struct in `rustack-dynamodbstreams-core` implements the `StreamEmitter` trait defined in `rustack-dynamodb-core`:

```rust
// crates/rustack-dynamodbstreams-core/src/emitter.rs

use std::sync::Arc;

use rustack_dynamodb_core::stream::{ChangeEvent, StreamEmitter};

use crate::storage::StreamStore;

/// DynamoDB Streams implementation of the StreamEmitter trait.
///
/// Receives change events from DynamoDB core and appends them to the
/// appropriate table stream in the StreamStore.
#[derive(Debug)]
pub struct DynamoDBStreamEmitter {
    store: Arc<StreamStore>,
    region: String,
}

impl DynamoDBStreamEmitter {
    /// Create a new emitter backed by the given stream store.
    pub fn new(store: Arc<StreamStore>, region: String) -> Self {
        Self { store, region }
    }
}

impl StreamEmitter for DynamoDBStreamEmitter {
    fn emit(&self, event: ChangeEvent) {
        self.store.append_change_event(&event, &self.region);
    }
}
```

### 10.2 StreamStore Operations

```rust
impl StreamStore {
    /// Create a new empty stream store.
    pub fn new() -> Self {
        Self {
            streams: DashMap::new(),
        }
    }

    /// Create a stream for a table.
    ///
    /// Called when CreateTable or UpdateTable specifies
    /// StreamSpecification.StreamEnabled = true.
    pub fn create_stream(
        &self,
        table_name: &str,
        table_arn: &str,
        key_schema: Vec<KeySchemaElement>,
        stream_view_type: StreamViewType,
        region: &str,
        account_id: &str,
    ) -> String {
        let stream_label = generate_stream_label();
        let arn = stream_arn(region, account_id, table_name, &stream_label);
        let shard_id = format!(
            "shardId-{}",
            uuid::Uuid::new_v4().to_string().replace('-', "")[..32]
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if i == 8 || i == 12 || i == 16 || i == 20 {
                        format!("-{c}")
                    } else {
                        c.to_string()
                    }
                })
                .collect::<String>()
        );

        let shard = ShardRecord::new(shard_id);

        let stream = TableStream {
            stream_arn: arn.clone(),
            table_name: table_name.to_string(),
            stream_label,
            stream_view_type,
            stream_status: StreamStatus::Enabled,
            key_schema,
            table_arn: table_arn.to_string(),
            shard: RwLock::new(shard),
        };

        self.streams.insert(table_name.to_string(), stream);
        arn
    }

    /// Disable a stream for a table.
    ///
    /// The stream remains readable but no new records are accepted.
    /// Called when UpdateTable disables streaming or DeleteTable is called.
    pub fn disable_stream(&self, table_name: &str) {
        if let Some(mut stream) = self.streams.get_mut(table_name) {
            stream.stream_status = StreamStatus::Disabled;
            stream.shard.write().close();
        }
    }

    /// Remove a stream entirely (after DeleteTable).
    pub fn remove_stream(&self, table_name: &str) {
        self.streams.remove(table_name);
    }

    /// Append a change event to a table's stream.
    ///
    /// Silently discards the event if the table has no active stream.
    pub fn append_change_event(&self, event: &ChangeEvent, region: &str) {
        if let Some(stream) = self.streams.get(&event.table_name) {
            if stream.stream_status != StreamStatus::Enabled {
                return; // Stream is disabled; discard the event.
            }

            let record = stream.create_record(event, region);
            stream.shard.write().append(record);
        }
    }

    /// Get stream info for a table (for DescribeStream).
    pub fn get_stream(&self, stream_arn: &str) -> Option<dashmap::mapref::one::Ref<'_, String, TableStream>> {
        self.streams.iter().find_map(|entry| {
            if entry.value().stream_arn == stream_arn {
                drop(entry);
                // Re-lookup by table name for proper ref lifetime
                None
            } else {
                None
            }
        });
        // Alternative: maintain a reverse index from stream_arn to table_name
        self.streams.iter().find(|entry| entry.value().stream_arn == stream_arn)
    }

    /// List all streams, optionally filtered by table name.
    pub fn list_streams(&self, table_name: Option<&str>) -> Vec<StreamSummary> {
        self.streams
            .iter()
            .filter(|entry| {
                table_name.map_or(true, |tn| entry.value().table_name == tn)
            })
            .map(|entry| {
                let stream = entry.value();
                StreamSummary {
                    stream_arn: stream.stream_arn.clone(),
                    table_name: stream.table_name.clone(),
                    stream_label: stream.stream_label.clone(),
                }
            })
            .collect()
    }
}

/// Summary information about a stream for ListStreams.
#[derive(Debug, Clone)]
pub struct StreamSummary {
    pub stream_arn: String,
    pub table_name: String,
    pub stream_label: String,
}
```

### 10.3 DynamoDB Streams API Operations

#### DescribeStream

Returns detailed metadata about a stream including shard information and key schema.

1. Parse `StreamArn` from input. If `ExclusiveStartShardId` is provided, start listing shards from that ID.
2. Look up the stream by ARN in the `StreamStore`.
3. If not found, return `ResourceNotFoundException`.
4. Build the `StreamDescription` response:
   - `StreamArn`, `StreamLabel`, `StreamStatus`, `StreamViewType`, `TableName`
   - `KeySchema` from the DynamoDB table's key schema
   - `Shards` list with shard ID, parent shard ID, and `SequenceNumberRange` (StartingSequenceNumber, EndingSequenceNumber)
   - `CreationRequestDateTime` (epoch seconds)
   - `LastEvaluatedShardId` for pagination (always null for MVP since we have one shard)

```rust
fn handle_describe_stream(
    &self,
    input: DescribeStreamInput,
) -> Result<DescribeStreamOutput, DynamoDBStreamsError> {
    let stream_arn = input.stream_arn.as_deref()
        .ok_or_else(|| DynamoDBStreamsError::validation("StreamArn is required"))?;

    let stream = self.store.get_stream_by_arn(stream_arn)
        .ok_or_else(|| DynamoDBStreamsError::resource_not_found(
            format!("Requested resource not found: Stream: {stream_arn} not found")
        ))?;

    let shard = stream.shard.read();

    let shard_desc = ShardDescription {
        shard_id: Some(shard.shard_id.clone()),
        parent_shard_id: shard.parent_shard_id.clone(),
        sequence_number_range: Some(SequenceNumberRange {
            starting_sequence_number: shard.starting_sequence_number.clone(),
            ending_sequence_number: shard.ending_sequence_number.clone(),
        }),
    };

    let description = StreamDescription {
        stream_arn: Some(stream.stream_arn.clone()),
        stream_label: Some(stream.stream_label.clone()),
        stream_status: Some(stream.stream_status.as_str().to_string()),
        stream_view_type: Some(stream.stream_view_type.clone()),
        table_name: Some(stream.table_name.clone()),
        key_schema: stream.key_schema.clone(),
        shards: vec![shard_desc],
        creation_request_date_time: None, // Optional
        last_evaluated_shard_id: None,    // Single shard, no pagination
    };

    Ok(DescribeStreamOutput {
        stream_description: Some(description),
    })
}
```

#### ListStreams

Lists all DynamoDB Streams, optionally filtered by table name.

1. If `TableName` is provided, filter streams to that table.
2. If `ExclusiveStartStreamArn` is provided, start listing from after that ARN.
3. Apply `Limit` (default 100).
4. Return stream ARNs, labels, and table names.

```rust
fn handle_list_streams(
    &self,
    input: ListStreamsInput,
) -> Result<ListStreamsOutput, DynamoDBStreamsError> {
    let mut streams = self.store.list_streams(input.table_name.as_deref());

    // Sort by stream ARN for deterministic pagination.
    streams.sort_by(|a, b| a.stream_arn.cmp(&b.stream_arn));

    // Apply ExclusiveStartStreamArn.
    if let Some(ref start_arn) = input.exclusive_start_stream_arn {
        if let Some(pos) = streams.iter().position(|s| s.stream_arn == *start_arn) {
            streams = streams.split_off(pos + 1);
        }
    }

    let limit = input.limit.map_or(100, |l| l.min(100).max(1) as usize);
    let has_more = streams.len() > limit;
    streams.truncate(limit);

    let last_arn = if has_more {
        streams.last().map(|s| s.stream_arn.clone())
    } else {
        None
    };

    let stream_items: Vec<StreamItem> = streams
        .into_iter()
        .map(|s| StreamItem {
            stream_arn: Some(s.stream_arn),
            table_name: Some(s.table_name),
            stream_label: Some(s.stream_label),
        })
        .collect();

    Ok(ListStreamsOutput {
        streams: stream_items,
        last_evaluated_stream_arn: last_arn,
    })
}
```

#### GetShardIterator

Creates an iterator pointing to a position in a shard.

1. Validate `StreamArn`, `ShardId`, `ShardIteratorType`.
2. Look up the stream and shard.
3. Determine the starting position based on iterator type:
   - `TRIM_HORIZON`: position 0 (oldest available record)
   - `LATEST`: position = current record count (next record to arrive)
   - `AT_SEQUENCE_NUMBER`: find the record with the exact sequence number
   - `AFTER_SEQUENCE_NUMBER`: find the record after the given sequence number
4. Encode the position into an opaque shard iterator token.

```rust
fn handle_get_shard_iterator(
    &self,
    input: GetShardIteratorInput,
) -> Result<GetShardIteratorOutput, DynamoDBStreamsError> {
    let stream_arn = input.stream_arn.as_deref()
        .ok_or_else(|| DynamoDBStreamsError::validation("StreamArn is required"))?;
    let shard_id = input.shard_id.as_deref()
        .ok_or_else(|| DynamoDBStreamsError::validation("ShardId is required"))?;
    let iter_type = input.shard_iterator_type.as_deref()
        .ok_or_else(|| DynamoDBStreamsError::validation("ShardIteratorType is required"))?;

    let stream = self.store.get_stream_by_arn(stream_arn)
        .ok_or_else(|| DynamoDBStreamsError::resource_not_found(
            format!("Requested resource not found: Stream: {stream_arn} not found")
        ))?;

    let shard = stream.shard.read();
    if shard.shard_id != shard_id {
        return Err(DynamoDBStreamsError::resource_not_found(
            format!("Requested resource not found: Shard: {shard_id} in Stream: {stream_arn} not found")
        ));
    }

    let position = match iter_type {
        "TRIM_HORIZON" => 0u64,
        "LATEST" => shard.records.len() as u64,
        "AT_SEQUENCE_NUMBER" => {
            let seq = input.sequence_number.as_deref()
                .ok_or_else(|| DynamoDBStreamsError::validation(
                    "SequenceNumber is required for AT_SEQUENCE_NUMBER"
                ))?;
            find_sequence_position(&shard.records, seq)?
        }
        "AFTER_SEQUENCE_NUMBER" => {
            let seq = input.sequence_number.as_deref()
                .ok_or_else(|| DynamoDBStreamsError::validation(
                    "SequenceNumber is required for AFTER_SEQUENCE_NUMBER"
                ))?;
            find_sequence_position(&shard.records, seq)? + 1
        }
        _ => {
            return Err(DynamoDBStreamsError::validation(
                format!("Invalid ShardIteratorType: {iter_type}")
            ));
        }
    };

    let token = encode_iterator(stream_arn, shard_id, position);

    Ok(GetShardIteratorOutput {
        shard_iterator: Some(token),
    })
}

/// Find the 0-based position of a record with the given sequence number.
fn find_sequence_position(
    records: &VecDeque<StreamChangeRecord>,
    sequence_number: &str,
) -> Result<u64, DynamoDBStreamsError> {
    records
        .iter()
        .position(|r| r.dynamodb.sequence_number == sequence_number)
        .map(|p| p as u64)
        .ok_or_else(|| DynamoDBStreamsError::trimmed_data_access(
            "The requested sequence number is beyond the trim horizon.",
        ))
}
```

#### GetRecords

Reads records from a shard starting at the iterator position.

1. Decode the shard iterator token to extract stream ARN, shard ID, and position.
2. Look up the stream and shard.
3. Read up to `Limit` records (default 1000, max 10000) starting at the position.
4. Encode a new iterator token pointing to the position after the last returned record.
5. If the shard is closed and all records have been read, return `NextShardIterator: null`.

```rust
fn handle_get_records(
    &self,
    input: GetRecordsInput,
) -> Result<GetRecordsOutput, DynamoDBStreamsError> {
    let token = input.shard_iterator.as_deref()
        .ok_or_else(|| DynamoDBStreamsError::validation("ShardIterator is required"))?;

    let (stream_arn, shard_id, position) = decode_iterator(token)?;

    let stream = self.store.get_stream_by_arn(stream_arn)
        .ok_or_else(|| DynamoDBStreamsError::expired_iterator(
            "The shard iterator is expired or invalid.",
        ))?;

    let shard = stream.shard.read();
    if shard.shard_id != shard_id {
        return Err(DynamoDBStreamsError::expired_iterator(
            "The shard iterator is expired or invalid.",
        ));
    }

    let limit = input.limit.map_or(1000, |l| (l as usize).min(10000).max(1));
    let start = position as usize;
    let end = (start + limit).min(shard.records.len());

    let records: Vec<RecordOutput> = shard.records
        .range(start..end)
        .map(|r| record_to_output(r))
        .collect();

    let next_position = end as u64;

    // If shard is closed and we've read all records, no next iterator.
    let next_iterator = if stream.stream_status == StreamStatus::Disabled
        && next_position >= shard.records.len() as u64
    {
        None
    } else {
        Some(encode_iterator(stream_arn, shard_id, next_position))
    };

    Ok(GetRecordsOutput {
        records,
        next_shard_iterator: next_iterator,
    })
}

/// Convert an internal StreamChangeRecord to the API output Record type.
fn record_to_output(record: &StreamChangeRecord) -> RecordOutput {
    RecordOutput {
        event_id: Some(record.event_id.clone()),
        event_name: Some(record.event_name.clone()),
        event_version: Some(record.event_version.clone()),
        event_source: Some(record.event_source.clone()),
        aws_region: Some(record.aws_region.clone()),
        dynamodb: Some(StreamRecordOutput {
            keys: if record.dynamodb.keys.is_empty() {
                None
            } else {
                Some(record.dynamodb.keys.clone())
            },
            new_image: record.dynamodb.new_image.clone(),
            old_image: record.dynamodb.old_image.clone(),
            sequence_number: Some(record.dynamodb.sequence_number.clone()),
            size_bytes: Some(record.dynamodb.size_bytes as i64),
            stream_view_type: Some(record.dynamodb.stream_view_type.clone()),
            approximate_creation_date_time: Some(
                record.dynamodb.approximate_creation_date_time,
            ),
        }),
    }
}
```

### 10.4 Stream Lifecycle

#### CreateTable with StreamSpecification

When `CreateTable` specifies `StreamSpecification.StreamEnabled = true`:

1. DynamoDB core creates the table as normal.
2. DynamoDB core calls `self.emitter` (indirectly via the server wiring) to notify Streams.
3. However, stream creation is more naturally done directly: the server binary, upon creating a DynamoDB table with stream enabled, also creates the stream in the `StreamStore`.

Alternative approach (simpler): since the server binary has access to both the DynamoDB provider and the StreamStore, it can check `CreateTableOutput` for stream specification and call `StreamStore::create_stream` directly. This avoids adding stream creation logic to the `StreamEmitter` trait.

The recommended approach uses a `StreamLifecycleManager` in the server binary:

```rust
// In rustack main.rs or a bridge module:

/// Manages the lifecycle of DynamoDB Streams alongside DynamoDB tables.
///
/// Observes DynamoDB table creation/update/deletion and creates/disables/removes
/// corresponding streams in the StreamStore.
pub struct StreamLifecycleManager {
    stream_store: Arc<StreamStore>,
    region: String,
    account_id: String,
}

impl StreamLifecycleManager {
    /// Call after a successful CreateTable or UpdateTable that enables streaming.
    pub fn on_stream_enabled(
        &self,
        table_name: &str,
        table_arn: &str,
        key_schema: Vec<KeySchemaElement>,
        stream_view_type: StreamViewType,
    ) -> String {
        self.stream_store.create_stream(
            table_name,
            table_arn,
            key_schema,
            stream_view_type,
            &self.region,
            &self.account_id,
        )
    }

    /// Call after UpdateTable that disables streaming.
    pub fn on_stream_disabled(&self, table_name: &str) {
        self.stream_store.disable_stream(table_name);
    }

    /// Call after DeleteTable.
    pub fn on_table_deleted(&self, table_name: &str) {
        self.stream_store.disable_stream(table_name);
        // Keep the stream readable for a while (real DynamoDB keeps it for 24h)
        // For local dev, we remove it immediately.
        self.stream_store.remove_stream(table_name);
    }
}
```

This keeps the `StreamEmitter` trait focused on change events only and moves lifecycle management to the application layer.

#### UpdateTable with StreamSpecification

- **Enable stream:** Same as CreateTable stream creation. The `StreamViewType` must be specified.
- **Disable stream:** Close the shard and set status to DISABLED. Records remain readable.
- **Change StreamViewType:** Not allowed. Must disable the stream first, then re-enable with the new view type. Return `ValidationException` if attempted.

#### DeleteTable

1. Delete the DynamoDB table.
2. Disable the stream (close shard, set status to DISABLED).
3. Remove the stream from the store (for local dev; real DynamoDB keeps it readable for 24 hours).

### 10.5 DynamoDB Table State Changes

The `DynamoDBTable` struct already has a `stream_specification` field. We add a `stream_arn` field to track the associated stream:

```rust
// In crates/rustack-dynamodb-core/src/state.rs (modification):

pub struct DynamoDBTable {
    // ... existing fields ...

    /// Stream specification.
    pub stream_specification: Option<StreamSpecification>,

    /// Stream ARN (set when stream is enabled).
    /// Used in DescribeTable response's LatestStreamArn field.
    pub latest_stream_arn: Option<String>,

    /// Stream label (set when stream is enabled).
    /// Used in DescribeTable response's LatestStreamLabel field.
    pub latest_stream_label: Option<String>,
}
```

The `DescribeTable` response already includes `StreamSpecification` in the output. With Streams support, it also includes `LatestStreamArn` and `LatestStreamLabel`.

### 10.6 Sequence Number Semantics

Sequence numbers are monotonically increasing u64 values per shard, formatted as 21-character zero-padded strings:

```rust
/// Format a sequence number as a zero-padded 21-character string.
///
/// Real DynamoDB uses variable-length numeric strings, but zero-padding
/// ensures correct lexicographic ordering which simplifies iterator logic.
fn format_sequence_number(seq: u64) -> String {
    format!("{seq:021}")
}
```

Properties:
- **Monotonically increasing within a shard**: each new record gets a strictly greater sequence number.
- **Not globally unique**: sequence numbers are scoped to a shard, not across shards.
- **Lexicographically orderable**: zero-padding ensures string comparison equals numeric comparison.

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// DynamoDB Streams error codes matching the AWS API.
#[derive(Debug, Clone)]
pub enum DynamoDBStreamsErrorCode {
    /// Stream or shard does not exist.
    ResourceNotFoundException,
    /// Data has been trimmed from the stream (record too old).
    TrimmedDataAccessException,
    /// Shard iterator has expired (>15 minutes old).
    ExpiredIteratorException,
    /// Internal server error.
    InternalServerError,
    /// Rate limit exceeded.
    LimitExceededException,
    /// Invalid parameter value.
    ValidationException,
}
```

### 11.2 Error Mapping

```rust
impl DynamoDBStreamsError {
    /// Map to HTTP status code, __type string, and message.
    pub fn to_error_response(&self) -> (u16, &'static str, &str) {
        match &self.code {
            DynamoDBStreamsErrorCode::ResourceNotFoundException =>
                (400, "ResourceNotFoundException", &self.message),
            DynamoDBStreamsErrorCode::TrimmedDataAccessException =>
                (400, "TrimmedDataAccessException", &self.message),
            DynamoDBStreamsErrorCode::ExpiredIteratorException =>
                (400, "ExpiredIteratorException", &self.message),
            DynamoDBStreamsErrorCode::InternalServerError =>
                (500, "InternalServerError", &self.message),
            DynamoDBStreamsErrorCode::LimitExceededException =>
                (400, "LimitExceededException", &self.message),
            DynamoDBStreamsErrorCode::ValidationException =>
                (400, "ValidationException", &self.message),
        }
    }
}
```

### 11.3 Error Response Format

```json
{
    "__type": "ResourceNotFoundException",
    "message": "Requested resource not found: Stream: arn:aws:dynamodb:us-east-1:000000000000:table/NoSuchTable/stream/2026-03-19T10:00:00.000 not found"
}
```

DynamoDB Streams uses lowercase `"message"` in error responses (same as DynamoDB, unlike Secrets Manager which uses capital `"Message"`).

---

## 12. Server Integration

### 12.1 Feature Gate

DynamoDB Streams is gated behind its own cargo feature but automatically enabled when DynamoDB is enabled:

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "dynamodbstreams", "sqs", "ssm", ...]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
dynamodbstreams = [
    "dynamodb",
    "dep:rustack-dynamodbstreams-core",
    "dep:rustack-dynamodbstreams-http",
]
```

### 12.2 Gateway Registration

DynamoDB Streams is registered as a separate service router in the gateway, distinguished from DynamoDB by the `X-Amz-Target` prefix:

```rust
// In build_services():

// ----- DynamoDB (register before S3: S3 is the catch-all) -----
#[cfg(feature = "dynamodb")]
let dynamodb_provider = if is_enabled("dynamodb") {
    let dynamodb_config = DynamoDBConfig::from_env();
    let mut provider = RustackDynamoDB::new(dynamodb_config.clone());

    // Wire in DynamoDB Streams emitter if enabled.
    #[cfg(feature = "dynamodbstreams")]
    let stream_store = if is_enabled("dynamodbstreams") {
        let store = Arc::new(StreamStore::new());
        let emitter = Arc::new(DynamoDBStreamEmitter::new(
            Arc::clone(&store),
            dynamodb_config.default_region.clone(),
        ));
        provider.set_emitter(emitter);
        Some(store)
    } else {
        None
    };

    let dynamodb_handler = RustackDynamoDBHandler::new(Arc::new(provider));
    let dynamodb_http_config = build_dynamodb_http_config(&dynamodb_config);
    let dynamodb_service =
        DynamoDBHttpService::new(Arc::new(dynamodb_handler), dynamodb_http_config);
    services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_service)));

    // Register DynamoDB Streams router.
    #[cfg(feature = "dynamodbstreams")]
    if let Some(store) = stream_store {
        let streams_config = DynamoDBStreamsConfig::from_env();
        let streams_provider = RustackDynamoDBStreams::new(store, streams_config.clone());
        let streams_handler = RustackDynamoDBStreamsHandler::new(Arc::new(streams_provider));
        let streams_http_config = build_dynamodbstreams_http_config(&streams_config);
        let streams_service = DynamoDBStreamsHttpService::new(
            Arc::new(streams_handler),
            streams_http_config,
        );
        services.push(Box::new(DynamoDBStreamsServiceRouter::new(streams_service)));
    }

    true
} else {
    false
};
```

### 12.3 Stream Lifecycle Wiring

The server binary also needs to wire stream lifecycle events (CreateTable, UpdateTable, DeleteTable) to the StreamStore. This is done via a callback or by extending the DynamoDB handler:

```rust
// Option A: Post-processing in a wrapper handler
// The DynamoDB HTTP service wraps the handler to intercept CreateTable/UpdateTable/DeleteTable
// responses and trigger stream lifecycle operations.

// Option B: StreamLifecycleManager registered on the DynamoDB provider
// The RustackDynamoDB provider holds an optional Arc<StreamLifecycleManager> and
// calls it after table operations.

// We choose Option B for cleaner separation:
impl RustackDynamoDB {
    pub fn set_stream_lifecycle_manager(&mut self, manager: Arc<StreamLifecycleManager>) {
        self.stream_lifecycle = Some(manager);
    }
}
```

In the `handle_create_table` handler:

```rust
fn handle_create_table(&self, input: CreateTableInput) -> Result<CreateTableOutput, DynamoDBError> {
    // ... existing table creation logic ...

    // Create stream if StreamSpecification is enabled.
    if let Some(ref spec) = input.stream_specification {
        if spec.stream_enabled {
            if let Some(ref manager) = self.stream_lifecycle {
                let stream_view_type = spec.stream_view_type.clone()
                    .unwrap_or(StreamViewType::NewAndOldImages);
                let stream_arn = manager.on_stream_enabled(
                    &table.name,
                    &table.arn,
                    table.key_schema_elements.clone(),
                    stream_view_type,
                );
                // Store stream_arn on the table for DescribeTable response.
                // (Requires table to be mutable or use interior mutability.)
            }
        }
    }

    // ... build response ...
}
```

### 12.4 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "running",
        "dynamodb": "running",
        "dynamodbstreams": "running",
        "sqs": "running",
        ...
    }
}
```

### 12.5 Configuration

```rust
pub struct DynamoDBStreamsConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub default_account_id: String,
    /// Maximum records per shard (0 = unlimited).
    pub max_records_per_shard: usize,
    /// Maximum record age in seconds (0 = unlimited).
    pub max_record_age_seconds: u64,
}

impl DynamoDBStreamsConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool(
                "DYNAMODBSTREAMS_SKIP_SIGNATURE_VALIDATION",
                true,
            ),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            default_account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            max_records_per_shard: env_usize("DYNAMODBSTREAMS_MAX_RECORDS_PER_SHARD", 0),
            max_record_age_seconds: env_u64("DYNAMODBSTREAMS_MAX_RECORD_AGE_SECONDS", 0),
        }
    }
}
```

### 12.6 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared) |
| `DYNAMODBSTREAMS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 for Streams |
| `DYNAMODBSTREAMS_MAX_RECORDS_PER_SHARD` | `0` | Max records per shard (0 = unlimited) |
| `DYNAMODBSTREAMS_MAX_RECORD_AGE_SECONDS` | `0` | Max record age (0 = unlimited) |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default account for ARNs |

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **StreamEmitter trait**: verify ChangeEvent is correctly constructed for INSERT/MODIFY/REMOVE operations
- **StreamStore**: create/disable/remove stream, append records, list streams
- **ShardRecord**: append records, sequence number assignment, shard close behavior
- **StreamViewType filtering**: KEYS_ONLY strips all images, NEW_IMAGE keeps only new, OLD_IMAGE keeps only old, NEW_AND_OLD_IMAGES keeps both
- **Shard iterator encoding/decoding**: round-trip encode/decode, malformed token handling
- **GetShardIterator position**: TRIM_HORIZON returns 0, LATEST returns current count, AT/AFTER_SEQUENCE_NUMBER finds correct position
- **GetRecords**: read with various limits, empty reads, closed shard behavior (null NextShardIterator)
- **Sequence numbers**: monotonically increasing, zero-padded format

### 13.2 Integration Tests: Full Pipeline

The most critical tests verify the complete DynamoDB write -> stream record -> GetRecords pipeline:

```rust
// tests/integration/dynamodbstreams_tests.rs

#[tokio::test]
#[ignore]
async fn test_should_capture_put_item_as_insert_event() {
    // 1. CreateTable with StreamSpecification(StreamEnabled=true, StreamViewType=NEW_AND_OLD_IMAGES)
    // 2. PutItem (new item)
    // 3. ListStreams -> verify stream exists
    // 4. DescribeStream -> get shard ID
    // 5. GetShardIterator(TRIM_HORIZON) -> get iterator
    // 6. GetRecords -> verify INSERT record with NewImage and no OldImage
}

#[tokio::test]
#[ignore]
async fn test_should_capture_update_item_as_modify_event() {
    // 1. CreateTable with stream enabled
    // 2. PutItem (create item)
    // 3. UpdateItem (modify item)
    // 4. GetRecords -> verify INSERT followed by MODIFY with both OldImage and NewImage
}

#[tokio::test]
#[ignore]
async fn test_should_capture_delete_item_as_remove_event() {
    // 1. CreateTable with stream enabled
    // 2. PutItem (create item)
    // 3. DeleteItem
    // 4. GetRecords -> verify INSERT followed by REMOVE with OldImage
}

#[tokio::test]
#[ignore]
async fn test_should_filter_images_by_stream_view_type_keys_only() {
    // 1. CreateTable with StreamViewType=KEYS_ONLY
    // 2. PutItem
    // 3. GetRecords -> verify record has Keys but no NewImage/OldImage
}

#[tokio::test]
#[ignore]
async fn test_should_filter_images_by_stream_view_type_new_image() {
    // 1. CreateTable with StreamViewType=NEW_IMAGE
    // 2. PutItem, then UpdateItem
    // 3. GetRecords -> verify INSERT has NewImage only, MODIFY has NewImage only
}

#[tokio::test]
#[ignore]
async fn test_should_capture_batch_write_item_events() {
    // 1. CreateTable with stream enabled
    // 2. BatchWriteItem with 3 PutRequests and 1 DeleteRequest
    // 3. GetRecords -> verify 4 records (3 INSERT + 1 REMOVE or INSERT+DELETE)
}

#[tokio::test]
#[ignore]
async fn test_should_get_shard_iterator_latest_skips_existing_records() {
    // 1. CreateTable, PutItem (record 1)
    // 2. GetShardIterator(LATEST) -> get iterator
    // 3. PutItem (record 2)
    // 4. GetRecords with LATEST iterator -> only record 2
}

#[tokio::test]
#[ignore]
async fn test_should_get_shard_iterator_at_sequence_number() {
    // 1. CreateTable, PutItem x3
    // 2. GetRecords(TRIM_HORIZON) -> get sequence numbers
    // 3. GetShardIterator(AT_SEQUENCE_NUMBER, seq=2nd record)
    // 4. GetRecords -> starts from 2nd record
}

#[tokio::test]
#[ignore]
async fn test_should_return_empty_records_when_no_new_data() {
    // 1. CreateTable, PutItem
    // 2. GetRecords -> consume all records
    // 3. GetRecords again -> empty records with NextShardIterator
}

#[tokio::test]
#[ignore]
async fn test_should_not_emit_records_when_stream_disabled() {
    // 1. CreateTable with stream enabled
    // 2. PutItem (record 1)
    // 3. UpdateTable to disable stream
    // 4. PutItem (record 2) -> should NOT appear in stream
    // 5. GetRecords -> only record 1
}

#[tokio::test]
#[ignore]
async fn test_should_describe_table_include_stream_arn() {
    // 1. CreateTable with StreamSpecification
    // 2. DescribeTable -> verify LatestStreamArn and LatestStreamLabel present
}
```

### 13.3 AWS CLI Smoke Tests

```bash
# Create table with streams enabled
aws dynamodb create-table \
    --table-name StreamTestTable \
    --attribute-definitions AttributeName=pk,AttributeType=S \
    --key-schema AttributeName=pk,KeyType=HASH \
    --billing-mode PAY_PER_REQUEST \
    --stream-specification StreamEnabled=true,StreamViewType=NEW_AND_OLD_IMAGES \
    --endpoint-url http://localhost:4566

# Put an item
aws dynamodb put-item \
    --table-name StreamTestTable \
    --item '{"pk":{"S":"item1"},"data":{"S":"hello"}}' \
    --endpoint-url http://localhost:4566

# List streams
aws dynamodbstreams list-streams \
    --table-name StreamTestTable \
    --endpoint-url http://localhost:4566

# Describe stream (use ARN from list-streams output)
aws dynamodbstreams describe-stream \
    --stream-arn "arn:aws:dynamodb:us-east-1:000000000000:table/StreamTestTable/stream/..." \
    --endpoint-url http://localhost:4566

# Get shard iterator
aws dynamodbstreams get-shard-iterator \
    --stream-arn "arn:aws:dynamodb:us-east-1:000000000000:table/StreamTestTable/stream/..." \
    --shard-id "shardId-..." \
    --shard-iterator-type TRIM_HORIZON \
    --endpoint-url http://localhost:4566

# Get records
aws dynamodbstreams get-records \
    --shard-iterator "..." \
    --endpoint-url http://localhost:4566
```

### 13.4 Third-Party Test Suites

#### 13.4.1 LocalStack DynamoDB Streams Tests

**Location:** `vendors/localstack/tests/aws/services/dynamodbstreams/`
**Coverage:** Tests the full pipeline: table creation with streams, item writes, GetRecords with all iterator types, stream disable/enable, BatchWriteItem events.

#### 13.4.2 Terraform

The `aws_dynamodb_table` resource supports `stream_enabled` and `stream_view_type`. With Streams working, Terraform can:
- Create tables with streams
- Read `stream_arn` output attribute
- Use stream ARN in Lambda event source mappings

#### 13.4.3 DynamoDB Local Reference

Amazon's DynamoDB Local includes Streams support. We can compare behavior against DynamoDB Local for edge cases (e.g., sequence number format, shard iterator expiry behavior).

### 13.5 CI Integration

```yaml
# .github/workflows/dynamodbstreams-ci.yml
name: DynamoDB Streams CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p rustack-dynamodbstreams-model
      - run: cargo test -p rustack-dynamodbstreams-core
      - run: cargo test -p rustack-dynamodbstreams-http
      - run: cargo test -p rustack-dynamodb-core  # includes stream emitter tests

  integration:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
      - run: ./target/release/rustack &
      - run: sleep 2
      - run: |
          # Full pipeline smoke test
          TABLE_ARN=$(aws dynamodb create-table --table-name test \
            --attribute-definitions AttributeName=pk,AttributeType=S \
            --key-schema AttributeName=pk,KeyType=HASH \
            --billing-mode PAY_PER_REQUEST \
            --stream-specification StreamEnabled=true,StreamViewType=NEW_AND_OLD_IMAGES \
            --endpoint-url http://localhost:4566 --query 'TableDescription.LatestStreamArn' --output text)
          aws dynamodb put-item --table-name test \
            --item '{"pk":{"S":"1"},"v":{"S":"hello"}}' \
            --endpoint-url http://localhost:4566
          STREAM_ARN=$(aws dynamodbstreams list-streams --table-name test \
            --endpoint-url http://localhost:4566 --query 'Streams[0].StreamArn' --output text)
          SHARD_ID=$(aws dynamodbstreams describe-stream --stream-arn "$STREAM_ARN" \
            --endpoint-url http://localhost:4566 --query 'StreamDescription.Shards[0].ShardId' --output text)
          ITERATOR=$(aws dynamodbstreams get-shard-iterator --stream-arn "$STREAM_ARN" \
            --shard-id "$SHARD_ID" --shard-iterator-type TRIM_HORIZON \
            --endpoint-url http://localhost:4566 --query 'ShardIterator' --output text)
          RECORDS=$(aws dynamodbstreams get-records --shard-iterator "$ITERATOR" \
            --endpoint-url http://localhost:4566 --query 'length(Records)')
          test "$RECORDS" -ge 1 || exit 1
```

---

## 14. Phased Implementation Plan

### Phase 0: Full Implementation (3-4 days)

DynamoDB Streams has only 4 operations and the implementation is not large enough to warrant multiple phases. All operations are implemented together.

**Day 1: Model + Storage + Emitter Trait**

1. Download DynamoDB Streams Smithy model, create `codegen/services/dynamodbstreams.toml`
2. Generate `rustack-dynamodbstreams-model` crate
3. Add `StreamEmitter` trait and `ChangeEvent` types to `rustack-dynamodb-core`
4. Implement `StreamStore`, `TableStream`, `ShardRecord`, `StreamChangeRecord` in `rustack-dynamodbstreams-core`
5. Implement `DynamoDBStreamEmitter` (the `StreamEmitter` impl)

**Day 2: DynamoDB Integration + Streams API**

1. Modify `RustackDynamoDB` to hold `Arc<dyn StreamEmitter>` and call it in PutItem, UpdateItem, DeleteItem, BatchWriteItem handlers
2. Add `stream_arn` / `stream_label` to `DynamoDBTable` state
3. Wire stream creation in CreateTable/UpdateTable handlers
4. Implement all 4 DynamoDB Streams operations: DescribeStream, GetShardIterator, GetRecords, ListStreams
5. Create `rustack-dynamodbstreams-http` crate with router and service

**Day 3: Server Integration + Testing**

1. Add `DynamoDBStreamsServiceRouter` to gateway
2. Wire `StreamEmitter` and `StreamLifecycleManager` in `main.rs`
3. Add feature gate `dynamodbstreams`
4. Write unit tests for all storage operations, iterator encoding, view type filtering
5. Write integration tests for the full write-to-read pipeline

**Day 4: Polish + CI**

1. Run `cargo clippy -- -D warnings`, `cargo +nightly fmt`, `cargo test`
2. AWS CLI smoke tests
3. Fix edge cases from LocalStack test comparison
4. Update health endpoint, CI workflows, README
5. Run `cargo audit` and `cargo-deny`

**Deliverable:** Full DynamoDB Streams support -- tables with streams, all 4 API operations, full write pipeline integration, AWS CLI and SDK compatibility.

---

## 15. Risk Analysis

### 15.1 Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Change capture adds latency to DynamoDB writes | Medium | Medium | The `StreamEmitter::emit` method acquires a write lock on the shard's `RwLock` briefly (just an append to a `VecDeque`). Benchmark shows <1us for typical records. For truly hot paths, could switch to lock-free `crossbeam` channel. |
| Old image capture requires extra read before write | Medium | Low | PutItem and DeleteItem already return the old item in their storage operations. UpdateItem already reads the old item as part of the read-modify-write cycle. No additional reads needed. |
| Gateway routing ambiguity between DynamoDB and DynamoDB Streams | Low | High | Prefix `DynamoDBStreams_20120810.` is a strict superset prefix check. The gateway checks DynamoDB Streams first (or DynamoDB router's `starts_with("DynamoDB_")` does not match `DynamoDBStreams_`). The `DynamoDB_` prefix check must NOT match `DynamoDBStreams_` -- verify this. |
| Shard iterator token format incompatible with AWS SDKs | Low | Medium | AWS SDKs treat iterator tokens as opaque strings. Our pipe-delimited format is valid as long as it round-trips correctly. |
| StreamViewType filtering drops images that tests expect | Medium | Medium | Carefully test all 4 view types. Common mistake: KEYS_ONLY should still include Keys, just not NewImage/OldImage. |
| Memory growth from unbounded record retention | Medium | Low | Default config retains all records. For long-running instances, configure `DYNAMODBSTREAMS_MAX_RECORDS_PER_SHARD`. Document this in README. |
| DynamoDB model crate dependency from Streams model crate | Low | Low | Required for `AttributeValue` type sharing. This is a compile-time dependency only; no runtime coupling. |
| Stream lifecycle events (create/disable) not wired correctly | Medium | Medium | The server binary wires lifecycle via `StreamLifecycleManager`. Test all paths: CreateTable+stream, UpdateTable enable/disable, DeleteTable. |

### 15.2 Dependencies

- `rustack-core` -- no changes needed
- `rustack-auth` -- no changes needed (SigV4 with service=`dynamodb`)
- `rustack-dynamodb-model` -- no changes needed (already has `StreamSpecification`, `StreamViewType`)
- `rustack-dynamodb-core` -- **modified**: adds `StreamEmitter` trait, `ChangeEvent`, emitter calls in write handlers
- `dashmap` -- already in workspace
- `parking_lot` -- already in workspace (used for shard `RwLock`)
- `uuid` -- already in workspace
- `chrono` -- already in workspace

No new external dependencies required.

### 15.3 Decision Log

| Decision | Rationale |
|----------|-----------|
| Native change log instead of Kinesis backing | LocalStack's approach (Kinesis backing) adds complexity (shard ID remapping, cross-service dependency, Kinesis actor overhead) for no benefit in local dev. A simple `VecDeque` per table is sufficient and 10x simpler. |
| `StreamEmitter` trait in `rustack-dynamodb-core` (dependency inversion) | DynamoDB core must not depend on Streams core. Defining the trait in DynamoDB core and implementing it in Streams core follows the dependency inversion principle. The server binary wires them together. |
| Single shard per table (MVP) | Real DynamoDB splits shards based on throughput. For local dev, a single shard is sufficient and avoids partition routing complexity. Multi-shard support can be added later without API changes. |
| `parking_lot::RwLock` for shard access | Shards are read-heavy (GetRecords) with infrequent writes (change events). `RwLock` allows concurrent reads. `parking_lot` is faster than `std::sync::RwLock`. |
| Pipe-delimited shard iterator token | Opaque to SDKs. Simpler than base64-encoded protobuf (which real DynamoDB uses). Easy to debug. Could add HMAC signing later if tampering is a concern. |
| `StreamLifecycleManager` in server binary (not in emitter trait) | Stream creation/deletion is a lifecycle concern, not a change event. Separating lifecycle from event emission keeps the `StreamEmitter` trait focused and testable. |
| Re-export `AttributeValue` from DynamoDB model | Ensures type compatibility when passing item images from DynamoDB core to Streams core. Avoids separate `AttributeValue` types that would need conversion. |
| Immediate stream removal on DeleteTable | Real DynamoDB keeps streams readable for 24 hours after table deletion. For local dev, immediate cleanup is simpler and avoids orphaned state. Configurable if needed later. |
| Zero-padded 21-digit sequence numbers | Matches the typical format used by real DynamoDB. Zero-padding ensures lexicographic ordering matches numeric ordering, simplifying AT_SEQUENCE_NUMBER lookups. |
