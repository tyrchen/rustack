# Rustack Kinesis Data Streams: Native Rust Implementation Design

**Date:** 2026-03-06
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-dynamodb-design.md](./rustack-dynamodb-design.md), [rustack-sqs-design.md](./rustack-sqs-design.md)
**Scope:** Add native Kinesis Data Streams support to Rustack using the same Smithy-based codegen approach as DynamoDB/SQS/SSM, with an actor-per-shard in-memory streaming engine. Supports awsJson1.1 protocol with CBOR wire format compatibility.

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
9. [Shard Engine Design](#9-shard-engine-design)
10. [Storage Engine Design](#10-storage-engine-design)
11. [Core Business Logic](#11-core-business-logic)
12. [Error Handling](#12-error-handling)
13. [Server Integration](#13-server-integration)
14. [Testing Strategy](#14-testing-strategy)
15. [Phased Implementation Plan](#15-phased-implementation-plan)
16. [Risk Analysis](#16-risk-analysis)

---

## 1. Executive Summary

This spec proposes adding Kinesis Data Streams support to Rustack as a fully native Rust implementation. Key design decisions:

- **Native Rust streaming engine** -- unlike LocalStack which wraps kinesis-mock (a Scala/Http4s application) or kinesalite (a Node.js/LevelDB application), we build a purpose-built in-memory streaming engine with actor-per-shard concurrency. This maintains the ~10MB Docker image and millisecond startup.
- **Smithy codegen reuse** -- extend the existing `codegen/` system to generate Kinesis model types from the official AWS Kinesis Smithy JSON AST, producing a `rustack-kinesis-model` crate.
- **awsJson1.1 protocol with CBOR support** -- Kinesis uses `awsJson1_1` (same as SSM), dispatched via `X-Amz-Target: Kinesis_20131202.*`. The AWS Java SDK sends CBOR-encoded requests (`application/x-amz-cbor-1.1`); we must support both JSON and CBOR wire formats.
- **Actor-per-shard concurrency** -- each shard runs as an independent actor owning its record storage, sequence number generator, and iterator state. This follows the actor model established by SQS's actor-per-queue pattern.
- **MD5-based partition key routing** -- the core data model. Partition keys are MD5-hashed to 128-bit integers. Each shard owns a contiguous range of the hash key space `[0, 2^128)`. Records are routed to the shard whose range contains the hash.
- **Shard iterator state machine** -- five iterator types (TRIM_HORIZON, LATEST, AT_SEQUENCE_NUMBER, AFTER_SEQUENCE_NUMBER, AT_TIMESTAMP) with opaque iterator tokens that encode position within a shard's record log.
- **Phased delivery** -- 4 phases from MVP (stream CRUD, put/get records, shard iterators) to full feature parity including shard splitting/merging, enhanced fan-out, and CBOR protocol support.

---

## 2. Motivation

### 2.1 Why Kinesis?

Kinesis Data Streams is the core real-time streaming primitive on AWS. Developers need a local Kinesis for:

- **Event-driven architecture testing** -- test producer/consumer patterns, event sourcing, and real-time pipelines without AWS costs
- **KCL/KPL testing** -- the Kinesis Client Library (KCL) uses DynamoDB for checkpointing and Kinesis for reading; with both services in Rustack, full KCL integration tests become possible locally
- **Lambda event source mappings** -- Lambda triggers from Kinesis are one of the most common serverless patterns; local Kinesis enables testing these flows
- **Apache Flink / Spark Streaming** -- local Kinesis enables testing streaming analytics pipelines
- **CI/CD pipelines** -- fast, deterministic Kinesis in GitHub Actions for integration tests
- **Offline development** -- work without internet connectivity

### 2.2 Why Not Wrap kinesis-mock?

kinesis-mock (etspaceman) is the backend LocalStack uses for Kinesis emulation. Wrapping it has significant drawbacks:

| Issue | Impact |
|-------|--------|
| **JVM dependency** | kinesis-mock runs on JVM (Scala/Http4s/Cats-effect), adding ~300MB to Docker image |
| **Startup time** | JVM startup takes 2-4 seconds |
| **Memory overhead** | JVM baseline memory is 80-150MB |
| **Process management** | Must proxy HTTP between Rust and JVM, manage process lifecycle |
| **Limited fan-out** | kinesis-mock only supports polling consumers; no SubscribeToShard |
| **Architecture mismatch** | Cats-effect IO runtime does not compose with our Tokio runtime |

### 2.3 Why Not Wrap kinesalite?

kinesalite (mhart) is a lightweight Node.js Kinesis emulator built on LevelDB:

- No enhanced fan-out (SubscribeToShard)
- No stream consumers (RegisterStreamConsumer/DeregisterStreamConsumer)
- No UpdateShardCount or UpdateStreamMode
- Last commit January 2023; maintenance appears dormant
- Node.js process adds ~80MB memory and requires npm

### 2.4 Why Native Rust?

A native Rust implementation provides:

- **~10MB Docker image** (same as S3/DynamoDB/SQS/SSM) vs ~300MB with kinesis-mock
- **Millisecond startup** vs 2-4 seconds for JVM
- **~5MB memory baseline** vs 80-150MB for JVM
- **Full debuggability** -- we own every line of code
- **Tokio-native concurrency** -- shard actors, timers, and record retention integrate naturally with our async runtime
- **Single binary** -- no process management, no inter-process communication
- **CBOR support** -- can implement CBOR natively without Jackson dependency

### 2.5 Existing Alternatives

| Implementation | Language | Image Size | CBOR | Enhanced Fan-out | Notes |
|---------------|----------|------------|------|-----------------|-------|
| kinesis-mock | Scala/JVM | ~300MB | Yes | No | Used by LocalStack |
| kinesalite | Node.js | ~80MB | No | No | Dormant since 2023 |
| LocalStack Kinesis | Python+kinesis-mock | ~1GB | Yes | Partial | Wraps kinesis-mock |
| **Rustack Kinesis** | **Rust** | **~10MB** | **Yes** | **Deferred** | **This proposal** |

No existing Rust-based Kinesis emulator exists. This would be the first.

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Native Rust Kinesis emulator** -- no JVM, no external processes, no FFI
2. **Cover 90%+ of local development use cases** -- stream CRUD, put/get records, shard iterators, tags
3. **Dual wire format** -- `awsJson1.1` (JSON) for all SDKs, `application/x-amz-cbor-1.1` (CBOR) for Java SDK
4. **Smithy-generated types** -- all Kinesis API types generated from official AWS Smithy model
5. **Actor-per-shard concurrency** -- each shard owns its records, communicates via channels
6. **Correct shard model** -- MD5 partition key hashing, hash key range partitioning, sequence number generation
7. **All 5 shard iterator types** -- TRIM_HORIZON, LATEST, AT_SEQUENCE_NUMBER, AFTER_SEQUENCE_NUMBER, AT_TIMESTAMP
8. **Record retention** -- configurable 24h-365d retention with background cleanup
9. **Shard splitting and merging** -- support resharding for testing scaling logic
10. **Same Docker image** -- single binary serves S3, DynamoDB, SQS, SSM, and Kinesis on port 4566
11. **KCL compatibility** -- with Rustack DynamoDB for lease tables, enable full KCL testing
12. **Pass LocalStack Kinesis test suite** -- validate against vendored `test_kinesis.py`

### 3.2 Non-Goals

1. **Real KMS encryption** -- accept encryption operations in metadata, do not perform actual encryption
2. **Enhanced fan-out (SubscribeToShard)** -- requires HTTP/2 event streaming; deferred to future phase
3. **CloudWatch metrics integration** -- EnableEnhancedMonitoring/DisableEnhancedMonitoring accepted but no-op
4. **Throughput enforcement** -- accept limits but do not throttle (1MB/s write, 2MB/s read per shard)
5. **Cross-account access** -- all streams exist within a single account context
6. **Data persistence across restarts** -- in-memory only, matching other Rustack services
7. **Server-side encryption at rest** -- metadata only, no actual data encryption
8. **Resource policies enforcement** -- accept Put/Get/DeleteResourcePolicy, store but do not enforce
9. **Account-level settings** -- DescribeAccountSettings/UpdateAccountSettings accepted but static defaults
10. **Warm throughput** -- UpdateStreamWarmThroughput accepted but no-op
11. **Max record size updates** -- UpdateMaxRecordSize accepted but no-op

---

## 4. Architecture Overview

### 4.1 Layered Architecture (Mirrors S3, DynamoDB, SQS, SSM)

```
                    AWS SDK / CLI / KCL / KPL
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  Routes by X-Amz-Target
              |   (ServiceRouter)   |
              +--------+------------+
                       |
       +-------+-------+-------+-------+--------+
       v       v       v       v       v        v
   +------+ +------+ +------+ +------+ +--------+
   |  S3  | | DDB  | | SQS  | | SSM  | |Kinesis |
   | HTTP | | HTTP | | HTTP | | HTTP | | HTTP   |
   +--+---+ +--+---+ +--+---+ +--+---+ +---+---+
      |        |        |        |         |
   +--+---+ +--+---+ +--+---+ +--+---+ +---+---+
   |  S3  | | DDB  | | SQS  | | SSM  | |Kinesis|
   | Core | | Core | | Core | | Core | | Core  |
   +--+---+ +--+---+ +--+---+ +--+---+ +---+---+
      |        |        |        |         |
      +--------+--------+--------+---------+
                       |
              +-----------------+
              | rustack-core  |  Shared: multi-account/region
              | rustack-auth  |  Shared: SigV4 authentication
              +-----------------+
```

### 4.2 Gateway Service Routing

Kinesis requests are distinguished by the `X-Amz-Target` header prefix:

| Service | X-Amz-Target Prefix | Content-Type |
|---------|---------------------|--------------|
| DynamoDB | `DynamoDB_20120810.` | `application/x-amz-json-1.0` |
| SQS | `AmazonSQS.` | `application/x-amz-json-1.0` |
| SSM | `AmazonSSM.` | `application/x-amz-json-1.1` |
| **Kinesis** | **`Kinesis_20131202.`** | **`application/x-amz-json-1.1`** or **`application/x-amz-cbor-1.1`** |
| S3 | *(absent)* | varies |

Routing logic: check `X-Amz-Target` header. If prefix is `Kinesis_20131202.`, route to Kinesis. This is unambiguous and follows the same pattern as DynamoDB, SQS, and SSM.

### 4.3 Crate Dependency Graph

```
rustack (app) <-- unified binary
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-kinesis-core       <-- NEW
+-- rustack-kinesis-http       <-- NEW
+-- rustack-kinesis-model      <-- NEW (auto-generated)

rustack-kinesis-http
+-- rustack-kinesis-model
+-- rustack-auth

rustack-kinesis-core
+-- rustack-core
+-- rustack-kinesis-model
+-- rustack-kinesis-http
+-- rustack-auth
+-- tokio (channels, timers)
+-- dashmap
+-- md-5 (partition key hashing)

rustack-kinesis-model (auto-generated, standalone)
```

---

## 5. Protocol Design

### 5.1 Dual Wire Format Requirement

Kinesis uses `awsJson1.1` as its Smithy protocol, but the AWS Java SDK (and KCL) sends requests using CBOR encoding instead of JSON. We must handle both:

| Aspect | JSON Wire Format | CBOR Wire Format |
|--------|-----------------|-----------------|
| Content-Type | `application/x-amz-json-1.1` | `application/x-amz-cbor-1.1` |
| Operation dispatch | `X-Amz-Target: Kinesis_20131202.PutRecord` | Same header |
| Request body | JSON | CBOR (RFC 7049) |
| Response body | JSON | CBOR |
| Error body | JSON with `__type` | CBOR with `__type` |
| SDK versions | Rust SDK, Python boto3, Go SDK, JS SDK | Java SDK v1/v2 (default), KCL |
| Disable CBOR | N/A | `-Dcom.amazonaws.sdk.disableCbor=true` or `AWS_CBOR_DISABLE=true` |

### 5.2 JSON Protocol Details

Request:
```http
POST / HTTP/1.1
Content-Type: application/x-amz-json-1.1
X-Amz-Target: Kinesis_20131202.PutRecord

{"StreamName":"my-stream","Data":"SGVsbG8=","PartitionKey":"pk1"}
```

Success response:
```http
HTTP/1.1 200 OK
Content-Type: application/x-amz-json-1.1

{"ShardId":"shardId-000000000000","SequenceNumber":"49543463076548007379340356408662344608690478424072724482","EncryptionType":"NONE"}
```

Error response:
```http
HTTP/1.1 400 Bad Request
Content-Type: application/x-amz-json-1.1

{"__type":"ResourceNotFoundException","message":"Stream my-stream under account 000000000000 not found."}
```

### 5.3 CBOR Protocol Details

CBOR requests use the same `X-Amz-Target` header but encode the body as CBOR instead of JSON. The CBOR encoding follows these rules:

- Strings map to CBOR text strings (major type 3)
- Blob fields (e.g., `Data` in PutRecord) map to CBOR byte strings (major type 2)
- Timestamps map to CBOR semantic tag 1 (epoch-based date/time) with integer values -- **not** floating-point (AWS SDKs reject float timestamps)
- All other types follow standard CBOR/JSON equivalence

### 5.4 Wire Format Detection

```rust
/// Determine the Kinesis wire format from request headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KinesisWireFormat {
    /// Standard JSON: application/x-amz-json-1.1
    Json,
    /// CBOR: application/x-amz-cbor-1.1
    Cbor,
}

fn detect_wire_format(req: &http::Request<()>) -> KinesisWireFormat {
    req.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map_or(KinesisWireFormat::Json, |ct| {
            if ct.contains("cbor") {
                KinesisWireFormat::Cbor
            } else {
                KinesisWireFormat::Json
            }
        })
}
```

### 5.5 CBOR Timestamp Encoding

A critical correctness requirement: timestamps in CBOR responses MUST use integer epoch seconds, not floating-point. The AWS Java SDK rejects floating-point CBOR timestamps. The CBOR byte sequence for a timestamp must be:

```
0xc1        -- semantic tag 1 (epoch-based date/time)
0x1a/0x1b   -- unsigned integer (not 0xfb which is float64)
```

This is verified in LocalStack's test suite (`test_subscribe_to_shard_with_at_timestamp_cbor`).

---

## 6. Smithy Code Generation Strategy

### 6.1 Universal Codegen

The `rustack-kinesis-model` crate is generated from the official AWS Smithy JSON AST using the universal codegen tool at `codegen/`. The codegen reads a TOML service configuration and the Smithy model to produce all model types with correct serde attributes.

**Smithy model:** `codegen/smithy-model/kinesis.json` (500KB, namespace `com.amazonaws.kinesis`, 29 operations)
**Service config:** `codegen/services/kinesis.toml`
**Generate:** `make codegen-kinesis`

### 6.2 Generated Output

The codegen produces 6 files in `crates/rustack-kinesis-model/src/`:

| File | Contents |
|------|----------|
| `lib.rs` | Module declarations and re-exports |
| `types.rs` | Shared types (enums and structs) with serde derives |
| `operations.rs` | `KinesisOperation` enum with `as_str()`, `from_name()`, phase methods |
| `error.rs` | `KinesisErrorCode` enum + `KinesisError` struct + `kinesis_error!` macro |
| `input.rs` | All input structs with `#[serde(rename_all = "PascalCase")]` |
| `output.rs` | All output structs with serde derives |

### 6.3 Service-Specific Notes

Kinesis uses `awsJson1.1` but also supports CBOR wire format. The codegen generates JSON serde types; CBOR support needs to be handled in the HTTP layer. The `Data` field in records is a blob (base64-encoded in JSON, raw bytes in CBOR) generated as `bytes::Bytes`.

See [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md) for full codegen architecture details.

---

## 7. Crate Structure

### 7.1 New Crates

#### `rustack-kinesis-model` (auto-generated)

```
crates/rustack-kinesis-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs                    # Module re-exports
    +-- types.rs                  # HashKeyRange, SequenceNumberRange, Shard, Record, etc.
    +-- operations.rs             # KinesisOperation enum
    +-- error.rs                  # KinesisError + error codes
    +-- input/
    |   +-- mod.rs
    |   +-- stream.rs             # CreateStreamInput, DeleteStreamInput, etc.
    |   +-- record.rs             # PutRecordInput, PutRecordsInput, GetRecordsInput
    |   +-- shard.rs              # GetShardIteratorInput, ListShardsInput, MergeShardsInput, SplitShardInput
    |   +-- consumer.rs           # RegisterStreamConsumerInput, DeregisterStreamConsumerInput
    |   +-- tags.rs               # AddTagsToStreamInput, RemoveTagsFromStreamInput
    |   +-- encryption.rs         # StartStreamEncryptionInput, StopStreamEncryptionInput
    |   +-- retention.rs          # IncreaseStreamRetentionPeriodInput, DecreaseStreamRetentionPeriodInput
    |   +-- policy.rs             # PutResourcePolicyInput, GetResourcePolicyInput, DeleteResourcePolicyInput
    +-- output/
        +-- mod.rs
        +-- stream.rs
        +-- record.rs
        +-- shard.rs
        +-- consumer.rs
        +-- tags.rs
        +-- encryption.rs
        +-- retention.rs
        +-- policy.rs
```

**Dependencies**: `serde`, `serde_json`, `bytes`, `http`

#### `rustack-kinesis-http`

```
crates/rustack-kinesis-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs                 # X-Amz-Target: Kinesis_20131202.* dispatch
    +-- dispatch.rs               # KinesisHandler trait + dispatch logic
    +-- service.rs                # Hyper Service impl for Kinesis
    +-- request.rs                # Wire-format-aware deserialization (JSON + CBOR)
    +-- response.rs               # Wire-format-aware serialization (JSON + CBOR)
    +-- error.rs                  # Error response formatting
    +-- cbor.rs                   # CBOR serialization/deserialization helpers
    +-- body.rs                   # Response body type
```

**Dependencies**: `rustack-kinesis-model`, `rustack-auth`, `hyper`, `serde_json`, `ciborium` (CBOR), `bytes`

#### `rustack-kinesis-core`

```
crates/rustack-kinesis-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs                 # KinesisConfig
    +-- provider.rs               # RustackKinesis (main provider, StreamManager)
    +-- error.rs                  # KinesisServiceError
    +-- stream/
    |   +-- mod.rs
    |   +-- state.rs              # StreamState enum (CREATING, ACTIVE, DELETING, UPDATING)
    |   +-- metadata.rs           # StreamMetadata: name, ARN, shard count, retention, mode
    +-- shard/
    |   +-- mod.rs
    |   +-- actor.rs              # ShardActor: per-shard record storage and iterator management
    |   +-- hash.rs               # MD5 partition key hashing, hash key range logic
    |   +-- iterator.rs           # ShardIterator types and token generation/resolution
    |   +-- sequence.rs           # Sequence number generation (128-bit monotonic)
    |   +-- split_merge.rs        # Shard splitting and merging logic
    +-- record/
    |   +-- mod.rs
    |   +-- storage.rs            # Record log: Vec<StoredRecord> with sequence numbers
    |   +-- retention.rs          # Background retention cleanup (expired records)
    +-- consumer/
    |   +-- mod.rs
    |   +-- registry.rs           # Enhanced fan-out consumer registration
    +-- ops/
        +-- mod.rs
        +-- stream.rs             # CreateStream, DeleteStream, DescribeStream, DescribeStreamSummary, ListStreams
        +-- record.rs             # PutRecord, PutRecords, GetRecords
        +-- shard.rs              # GetShardIterator, ListShards, MergeShards, SplitShard, UpdateShardCount
        +-- consumer.rs           # RegisterStreamConsumer, DeregisterStreamConsumer, ListStreamConsumers, DescribeStreamConsumer
        +-- tags.rs               # AddTagsToStream, RemoveTagsFromStream, ListTagsForStream
        +-- retention.rs          # IncreaseStreamRetentionPeriod, DecreaseStreamRetentionPeriod
        +-- encryption.rs         # StartStreamEncryption, StopStreamEncryption (metadata only)
        +-- policy.rs             # PutResourcePolicy, GetResourcePolicy, DeleteResourcePolicy (store only)
        +-- monitoring.rs         # EnableEnhancedMonitoring, DisableEnhancedMonitoring (no-op)
        +-- mode.rs               # UpdateStreamMode (store only)
```

**Dependencies**: `rustack-core`, `rustack-kinesis-model`, `rustack-kinesis-http`, `rustack-auth`, `tokio` (mpsc, time, sync), `dashmap`, `md-5`, `uuid`, `tracing`, `chrono`, `thiserror`, `anyhow`

### 7.2 Workspace Changes

```toml
# Root Cargo.toml
[workspace.dependencies]
# ... existing deps ...
rustack-kinesis-model = { path = "crates/rustack-kinesis-model" }
rustack-kinesis-http = { path = "crates/rustack-kinesis-http" }
rustack-kinesis-core = { path = "crates/rustack-kinesis-core" }
ciborium = "0.2"               # CBOR serialization
ciborium-ll = "0.2"            # Low-level CBOR for timestamp control
```

---

## 8. HTTP Layer Design

### 8.1 Kinesis Router

Kinesis uses the same POST-to-root dispatch as DynamoDB/SSM, with `Kinesis_20131202` as the target prefix:

```rust
//! Kinesis operation router.
//!
//! Dispatches requests based on the `X-Amz-Target` header:
//! X-Amz-Target: Kinesis_20131202.PutRecord

const TARGET_PREFIX: &str = "Kinesis_20131202.";

/// Resolve an HTTP request to a Kinesis operation.
pub fn resolve_operation(
    headers: &http::HeaderMap,
) -> Result<KinesisOperation, KinesisError> {
    let target = headers
        .get("x-amz-target")
        .ok_or_else(KinesisError::missing_target)?
        .to_str()
        .map_err(|_| KinesisError::missing_target())?;

    let op_name = target
        .strip_prefix(TARGET_PREFIX)
        .ok_or_else(|| KinesisError::unknown_operation(target))?;

    KinesisOperation::from_name(op_name)
        .ok_or_else(|| KinesisError::unknown_operation(op_name))
}
```

### 8.2 Wire-Format-Aware Request Deserialization

```rust
/// Deserialize a Kinesis request from either JSON or CBOR.
pub fn deserialize_request<T: DeserializeOwned>(
    body: &[u8],
    wire_format: KinesisWireFormat,
) -> Result<T, KinesisError> {
    match wire_format {
        KinesisWireFormat::Json => {
            serde_json::from_slice(body)
                .map_err(|e| KinesisError::serialization(e.to_string()))
        }
        KinesisWireFormat::Cbor => {
            ciborium::from_reader(body)
                .map_err(|e| KinesisError::serialization(e.to_string()))
        }
    }
}
```

### 8.3 Wire-Format-Aware Response Serialization

```rust
/// Serialize a Kinesis response to either JSON or CBOR.
pub fn serialize_response<T: Serialize>(
    value: &T,
    wire_format: KinesisWireFormat,
) -> Result<(Bytes, &'static str), KinesisError> {
    match wire_format {
        KinesisWireFormat::Json => {
            let body = serde_json::to_vec(value)
                .map_err(|e| KinesisError::serialization(e.to_string()))?;
            Ok((Bytes::from(body), "application/x-amz-json-1.1"))
        }
        KinesisWireFormat::Cbor => {
            let mut buf = Vec::new();
            ciborium::into_writer(value, &mut buf)
                .map_err(|e| KinesisError::serialization(e.to_string()))?;
            Ok((Bytes::from(buf), "application/x-amz-cbor-1.1"))
        }
    }
}
```

### 8.4 KinesisHandler Trait

```rust
/// The boundary between HTTP and business logic.
///
/// Analogous to SsmHandler/SqsHandler but for Kinesis operations.
/// The handler is wire-format-agnostic -- it receives typed inputs and
/// returns typed outputs. The HTTP layer handles format detection,
/// deserialization, and serialization.
pub trait KinesisHandler: Send + Sync + 'static {
    fn handle_operation(
        &self,
        op: KinesisOperation,
        body: Bytes,
        wire_format: KinesisWireFormat,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<Bytes>, KinesisError>> + Send>>;
}
```

### 8.5 Service Integration

```rust
/// Hyper Service implementation for Kinesis.
#[derive(Clone)]
pub struct KinesisHttpService<H> {
    handler: Arc<H>,
    config: KinesisHttpConfig,
}

pub struct KinesisHttpConfig {
    pub skip_signature_validation: bool,
    pub region: String,
    pub account_id: String,
    pub credential_provider: Option<Arc<dyn CredentialProvider>>,
}
```

---

## 9. Shard Engine Design

This is the core complexity of the Kinesis implementation. The shard engine manages the mapping between partition keys and shards, sequence number generation, and iterator state.

### 9.1 Hash Key Space

The Kinesis hash key space is the full range of 128-bit unsigned integers: `[0, 2^128 - 1]`, which equals `[0, 340282366920938463463374607431768211455]`.

Each shard owns a contiguous, non-overlapping sub-range. For a stream with N shards, the space is divided evenly:

```
N=4 shards:
  shard-0: [0, 85070591730234615865843651857942052863]
  shard-1: [85070591730234615865843651857942052864, 170141183460469231731687303715884105727]
  shard-2: [170141183460469231731687303715884105728, 255211775190703847597530955573826158591]
  shard-3: [255211775190703847597530955573826158592, 340282366920938463463374607431768211455]
```

### 9.2 Partition Key Hashing

When a record is written with a partition key, the key is MD5-hashed to determine shard placement:

```rust
use md5::{Digest, Md5};

/// Hash key type: 128-bit unsigned integer.
/// Stored as a decimal string in the API (e.g., "85070591730234615865843651857942052863")
/// but internally as u128 for efficient comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HashKey(pub u128);

impl HashKey {
    /// The minimum hash key (0).
    pub const MIN: Self = Self(0);

    /// The maximum hash key (2^128 - 1).
    pub const MAX: Self = Self(u128::MAX);

    /// Compute the MD5 hash key from a partition key string.
    pub fn from_partition_key(partition_key: &str) -> Self {
        let digest = Md5::digest(partition_key.as_bytes());
        Self(u128::from_be_bytes(digest.into()))
    }

    /// Parse from decimal string representation (API format).
    pub fn from_decimal_str(s: &str) -> Result<Self, KinesisServiceError> {
        s.parse::<u128>()
            .map(Self)
            .map_err(|_| KinesisServiceError::InvalidArgument {
                message: format!("Invalid hash key: {s}"),
            })
    }

    /// Convert to decimal string representation (API format).
    pub fn to_decimal_string(self) -> String {
        self.0.to_string()
    }
}

/// A contiguous range of hash keys owned by a shard.
#[derive(Debug, Clone)]
pub struct HashKeyRange {
    pub start: HashKey,
    pub end: HashKey,
}

impl HashKeyRange {
    /// Check if a hash key falls within this range (inclusive).
    pub fn contains(&self, key: HashKey) -> bool {
        key >= self.start && key <= self.end
    }

    /// Divide the full hash key space into N equal ranges.
    pub fn divide_evenly(n: u32) -> Vec<Self> {
        let n = n as u128;
        let range_size = u128::MAX / n;
        (0..n)
            .map(|i| {
                let start = HashKey(i * range_size + if i > 0 { 1 } else { 0 });
                let end = if i == n - 1 {
                    HashKey::MAX
                } else {
                    HashKey((i + 1) * range_size)
                };
                Self { start, end }
            })
            .collect()
    }
}
```

### 9.3 Shard Routing

When a record is written, the engine determines the target shard:

```rust
impl StreamState {
    /// Route a record to the appropriate shard based on partition key or explicit hash key.
    pub fn route_to_shard(
        &self,
        partition_key: &str,
        explicit_hash_key: Option<&str>,
    ) -> Result<&ShardHandle, KinesisServiceError> {
        let hash_key = match explicit_hash_key {
            Some(ehk) => HashKey::from_decimal_str(ehk)?,
            None => HashKey::from_partition_key(partition_key),
        };

        // Find the shard whose hash key range contains this hash.
        // Only route to open (non-closed) shards.
        self.active_shards
            .iter()
            .find(|shard| shard.hash_key_range.contains(hash_key))
            .ok_or_else(|| KinesisServiceError::InternalError {
                message: "No shard found for hash key".to_string(),
            })
    }
}
```

### 9.4 Sequence Number Generation

Sequence numbers are 128-bit integers represented as decimal strings. They must be monotonically increasing within a shard. AWS sequence numbers look like `"49543463076548007379340356408662344608690478424072724482"` (56 digits).

Our generation strategy uses a combination of shard epoch and per-shard counter to produce realistic-looking sequence numbers:

```rust
/// Sequence number generator for a shard.
///
/// Produces monotonically increasing 128-bit integers formatted as
/// zero-padded decimal strings. The high bits encode the shard's creation
/// epoch, the low bits are a per-shard counter. This ensures:
/// 1. Sequence numbers increase within a shard
/// 2. Sequence numbers are globally unique across shards
/// 3. They look realistic (large decimal strings like real AWS)
#[derive(Debug)]
pub struct SequenceNumberGenerator {
    /// High 64 bits: derived from shard creation time + shard index.
    /// Encodes shard identity for global uniqueness.
    prefix: u64,
    /// Low 64 bits: monotonically increasing counter within this shard.
    counter: AtomicU64,
}

impl SequenceNumberGenerator {
    /// Create a new generator for a shard.
    ///
    /// `shard_index` is the zero-based index of this shard within the stream.
    /// `creation_epoch_millis` is the stream creation time in epoch milliseconds.
    pub fn new(shard_index: u32, creation_epoch_millis: u64) -> Self {
        // Combine creation time and shard index into the prefix.
        // This produces sequence numbers that look like real AWS ones.
        let prefix = (creation_epoch_millis << 16) | (shard_index as u64);
        Self {
            prefix,
            counter: AtomicU64::new(0),
        }
    }

    /// Generate the next sequence number.
    pub fn next(&self) -> SequenceNumber {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        let value = ((self.prefix as u128) << 64) | (counter as u128);
        SequenceNumber(value)
    }

    /// Return the starting sequence number (the first one that will be generated).
    pub fn starting_sequence_number(&self) -> SequenceNumber {
        let value = (self.prefix as u128) << 64;
        SequenceNumber(value)
    }
}

/// A sequence number within a shard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SequenceNumber(pub u128);

impl SequenceNumber {
    /// Parse from decimal string.
    pub fn from_str(s: &str) -> Result<Self, KinesisServiceError> {
        s.parse::<u128>()
            .map(Self)
            .map_err(|_| KinesisServiceError::InvalidArgument {
                message: format!("Invalid sequence number: {s}"),
            })
    }

    /// Format as zero-padded decimal string (56 digits like AWS).
    pub fn to_string(&self) -> String {
        format!("{:056}", self.0)
    }
}
```

### 9.5 Shard Iterator State Machine

Five iterator types determine where in the shard's record log reading begins:

```rust
/// The type of shard iterator to create.
#[derive(Debug, Clone)]
pub enum ShardIteratorType {
    /// Start reading at the oldest available record (beginning of retention window).
    TrimHorizon,
    /// Start reading just after the most recent record (new records only).
    Latest,
    /// Start reading at the record with this exact sequence number.
    AtSequenceNumber(SequenceNumber),
    /// Start reading just after the record with this sequence number.
    AfterSequenceNumber(SequenceNumber),
    /// Start reading at the first record with an arrival timestamp >= this value.
    AtTimestamp(u64), // epoch milliseconds
}

/// Opaque shard iterator token.
///
/// Encodes enough state to resume reading from a specific position in a shard.
/// Format: base64(stream_name + ":" + shard_id + ":" + position + ":" + nonce)
///
/// The `position` is the index into the shard's record log at which reading
/// should continue. Each GetRecords call returns a new iterator token with
/// an updated position.
#[derive(Debug, Clone)]
pub struct ShardIteratorToken {
    pub stream_name: String,
    pub shard_id: String,
    /// Index into the shard's record Vec at which to start reading.
    pub position: usize,
    /// Random nonce to ensure each token is unique (GetRecords requires
    /// a new iterator for each call).
    pub nonce: String,
}

impl ShardIteratorToken {
    /// Encode as an opaque string.
    pub fn encode(&self) -> String {
        use base64::Engine;
        let raw = format!(
            "{}:{}:{}:{}",
            self.stream_name, self.shard_id, self.position, self.nonce
        );
        base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
    }

    /// Decode from an opaque string. Strips surrounding quotes if present
    /// (AWS SDKs sometimes add them).
    pub fn decode(token: &str) -> Result<Self, KinesisServiceError> {
        use base64::Engine;
        let token = token.trim_matches('"');
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(token)
            .map_err(|_| KinesisServiceError::InvalidArgument {
                message: "Invalid shard iterator".to_string(),
            })?;
        let raw = String::from_utf8(bytes)
            .map_err(|_| KinesisServiceError::InvalidArgument {
                message: "Invalid shard iterator encoding".to_string(),
            })?;
        let parts: Vec<&str> = raw.splitn(4, ':').collect();
        if parts.len() != 4 {
            return Err(KinesisServiceError::InvalidArgument {
                message: "Invalid shard iterator format".to_string(),
            });
        }
        Ok(Self {
            stream_name: parts[0].to_string(),
            shard_id: parts[1].to_string(),
            position: parts[2].parse().map_err(|_| KinesisServiceError::InvalidArgument {
                message: "Invalid shard iterator position".to_string(),
            })?,
            nonce: parts[3].to_string(),
        })
    }
}
```

### 9.6 Shard Splitting

Splitting a shard creates two child shards that together cover the parent's hash key range:

```rust
/// Split a shard at a given hash key.
///
/// The parent shard is closed (its SequenceNumberRange gets an EndingSequenceNumber).
/// Two child shards are created:
///   child-A: [parent.start, new_starting_hash_key - 1]
///   child-B: [new_starting_hash_key, parent.end]
pub fn split_shard(
    &mut self,
    shard_id: &str,
    new_starting_hash_key: HashKey,
) -> Result<(ShardInfo, ShardInfo), KinesisServiceError> {
    let parent = self.find_shard_mut(shard_id)?;

    // Validate: new_starting_hash_key must be within parent's range.
    if new_starting_hash_key <= parent.hash_key_range.start
        || new_starting_hash_key > parent.hash_key_range.end
    {
        return Err(KinesisServiceError::InvalidArgument {
            message: "NewStartingHashKey must be within the shard's hash key range".to_string(),
        });
    }

    // Close the parent shard.
    let parent_end_seq = parent.actor.close();
    let parent_range = parent.hash_key_range.clone();
    let parent_id = parent.shard_id.clone();

    // Create child shards.
    let child_a_range = HashKeyRange {
        start: parent_range.start,
        end: HashKey(new_starting_hash_key.0 - 1),
    };
    let child_b_range = HashKeyRange {
        start: new_starting_hash_key,
        end: parent_range.end,
    };

    let child_a = self.create_child_shard(child_a_range, Some(&parent_id), None);
    let child_b = self.create_child_shard(child_b_range, Some(&parent_id), None);

    Ok((child_a, child_b))
}
```

### 9.7 Shard Merging

Merging combines two adjacent shards into one:

```rust
/// Merge two adjacent shards.
///
/// Both parent shards are closed. A single child shard is created whose
/// hash key range spans both parents.
///
/// The shards must be adjacent (one's ending hash key + 1 == the other's
/// starting hash key).
pub fn merge_shards(
    &mut self,
    shard_id: &str,
    adjacent_shard_id: &str,
) -> Result<ShardInfo, KinesisServiceError> {
    let shard = self.find_shard(shard_id)?;
    let adjacent = self.find_shard(adjacent_shard_id)?;

    // Verify adjacency.
    let (first, second) = if shard.hash_key_range.end.0 + 1 == adjacent.hash_key_range.start.0 {
        (shard, adjacent)
    } else if adjacent.hash_key_range.end.0 + 1 == shard.hash_key_range.start.0 {
        (adjacent, shard)
    } else {
        return Err(KinesisServiceError::InvalidArgument {
            message: "Shards are not adjacent".to_string(),
        });
    };

    // Close both parents.
    let first_id = first.shard_id.clone();
    let second_id = second.shard_id.clone();
    first.actor.close();
    second.actor.close();

    // Create merged child shard.
    let merged_range = HashKeyRange {
        start: first.hash_key_range.start,
        end: second.hash_key_range.end,
    };

    let child = self.create_child_shard(merged_range, Some(&first_id), Some(&second_id));
    Ok(child)
}
```

---

## 10. Storage Engine Design

### 10.1 Overview

The storage engine implements a per-shard append-only record log with time-based retention. Unlike SQS's message lifecycle (available -> in-flight -> deleted), Kinesis records are immutable once written: they sit in the log until the retention period expires and can be read multiple times by multiple consumers at different positions.

### 10.2 Record Lifecycle

```
                  PutRecord / PutRecords
                      |
                      v
              +---------------+
              |   RECORD LOG  |  (Vec<StoredRecord>, append-only)
              |   indexed by  |
              |  sequence num |
              +-------+-------+
                     /|\
                    / | \
                   /  |  \
    GetRecords   /   |   \  GetRecords
    (consumer A)/    |    \(consumer B)
              v      |     v
     +--------+ +--------+ +--------+
     | iter-A | | iter-B | | iter-C |  (independent read positions)
     +--------+ +--------+ +--------+

              After retention period:
              +---------------+
              |   TRIMMED     |  (records removed from front of log)
              +---------------+
```

### 10.3 Core Data Structures

```rust
/// A stored record within a shard's log.
#[derive(Debug, Clone)]
pub struct StoredRecord {
    /// Monotonically increasing sequence number within this shard.
    pub sequence_number: SequenceNumber,
    /// Record data (blob).
    pub data: Bytes,
    /// Partition key used to route this record.
    pub partition_key: String,
    /// Explicit hash key, if provided by the producer.
    pub explicit_hash_key: Option<String>,
    /// Server-side timestamp when the record was received.
    pub approximate_arrival_timestamp: u64, // epoch milliseconds
    /// Encryption type (always NONE for local dev).
    pub encryption_type: EncryptionType,
}

/// Encryption type for records.
#[derive(Debug, Clone, Copy, Default)]
pub enum EncryptionType {
    #[default]
    None,
    Kms,
}

/// Per-shard record storage.
///
/// An append-only log with retention-based trimming.
/// Records are indexed by their position (Vec index) and can be
/// looked up by sequence number via binary search.
pub struct ShardRecordLog {
    /// The record log. Records are appended at the back and trimmed
    /// from the front when retention expires.
    records: Vec<StoredRecord>,
    /// The sequence number offset: records[0].sequence_number corresponds
    /// to the first un-trimmed record. When records are trimmed from the
    /// front, this offset increases.
    trim_offset: usize,
    /// Retention period for this shard's stream.
    retention_period: Duration,
}

impl ShardRecordLog {
    /// Append a record to the log. Returns the assigned sequence number.
    pub fn append(&mut self, record: StoredRecord) -> SequenceNumber {
        let seq = record.sequence_number;
        self.records.push(record);
        seq
    }

    /// Get records starting at a given position (Vec index relative to trim_offset).
    /// Returns up to `limit` records and the next position for the iterator.
    pub fn get_records(
        &self,
        position: usize,
        limit: usize,
    ) -> (Vec<&StoredRecord>, usize) {
        let effective_pos = if position < self.trim_offset {
            0 // Position was trimmed, start from beginning
        } else {
            position - self.trim_offset
        };

        let records: Vec<&StoredRecord> = self.records
            .iter()
            .skip(effective_pos)
            .take(limit)
            .collect();

        let next_position = self.trim_offset + effective_pos + records.len();
        (records, next_position)
    }

    /// Find the position of the first record with sequence_number >= target.
    pub fn position_at_sequence_number(&self, target: SequenceNumber) -> usize {
        let idx = self.records
            .binary_search_by_key(&target, |r| r.sequence_number)
            .unwrap_or_else(|i| i);
        self.trim_offset + idx
    }

    /// Find the position of the first record with sequence_number > target.
    pub fn position_after_sequence_number(&self, target: SequenceNumber) -> usize {
        let idx = self.records
            .partition_point(|r| r.sequence_number <= target);
        self.trim_offset + idx
    }

    /// Find the position of the first record with timestamp >= target.
    pub fn position_at_timestamp(&self, target_millis: u64) -> usize {
        let idx = self.records
            .partition_point(|r| r.approximate_arrival_timestamp < target_millis);
        self.trim_offset + idx
    }

    /// Trim horizon position (first available record).
    pub fn trim_horizon_position(&self) -> usize {
        self.trim_offset
    }

    /// Latest position (after the last record).
    pub fn latest_position(&self) -> usize {
        self.trim_offset + self.records.len()
    }

    /// Remove expired records from the front of the log.
    pub fn trim_expired(&mut self) {
        let now = now_epoch_millis();
        let retention_millis = self.retention_period.as_millis() as u64;
        let cutoff = now.saturating_sub(retention_millis);

        let trim_count = self.records
            .partition_point(|r| r.approximate_arrival_timestamp < cutoff);

        if trim_count > 0 {
            self.records.drain(..trim_count);
            self.trim_offset += trim_count;
        }
    }

    /// Total number of records currently in the log.
    pub fn len(&self) -> usize {
        self.records.len()
    }
}
```

### 10.4 Shard Actor

Each shard runs as an independent actor, following the actor model:

```rust
/// Commands sent to a shard actor via its channel.
pub enum ShardCommand {
    PutRecord {
        data: Bytes,
        partition_key: String,
        explicit_hash_key: Option<String>,
        reply: oneshot::Sender<Result<PutRecordResult, KinesisServiceError>>,
    },
    GetRecords {
        position: usize,
        limit: usize,
        reply: oneshot::Sender<Result<GetRecordsResult, KinesisServiceError>>,
    },
    GetShardIterator {
        iterator_type: ShardIteratorType,
        reply: oneshot::Sender<Result<usize, KinesisServiceError>>,
    },
    Close {
        reply: oneshot::Sender<SequenceNumber>,
    },
    Shutdown,
}

pub struct PutRecordResult {
    pub shard_id: String,
    pub sequence_number: SequenceNumber,
}

pub struct GetRecordsResult {
    pub records: Vec<StoredRecord>,
    pub next_position: usize,
    pub millis_behind_latest: u64,
}

/// Per-shard actor that owns the record log.
pub struct ShardActor {
    /// Shard identifier (e.g., "shardId-000000000000").
    shard_id: String,
    /// Hash key range owned by this shard.
    hash_key_range: HashKeyRange,
    /// Record log.
    record_log: ShardRecordLog,
    /// Sequence number generator.
    seq_gen: SequenceNumberGenerator,
    /// Command channel receiver.
    commands: mpsc::Receiver<ShardCommand>,
    /// Whether the shard is closed (no more writes accepted).
    closed: bool,
    /// Shutdown signal.
    shutdown: AtomicBool,
}

impl ShardActor {
    pub async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                Some(cmd) = self.commands.recv() => {
                    match cmd {
                        ShardCommand::Shutdown => break,
                        cmd => self.handle_command(cmd),
                    }
                }
                _ = cleanup_interval.tick() => {
                    self.record_log.trim_expired();
                }
            }
        }
    }

    fn handle_command(&mut self, cmd: ShardCommand) {
        match cmd {
            ShardCommand::PutRecord { data, partition_key, explicit_hash_key, reply } => {
                if self.closed {
                    let _ = reply.send(Err(KinesisServiceError::InvalidArgument {
                        message: "Cannot write to a closed shard".to_string(),
                    }));
                    return;
                }
                let seq = self.seq_gen.next();
                let record = StoredRecord {
                    sequence_number: seq,
                    data,
                    partition_key,
                    explicit_hash_key,
                    approximate_arrival_timestamp: now_epoch_millis(),
                    encryption_type: EncryptionType::None,
                };
                self.record_log.append(record);
                let _ = reply.send(Ok(PutRecordResult {
                    shard_id: self.shard_id.clone(),
                    sequence_number: seq,
                }));
            }
            ShardCommand::GetRecords { position, limit, reply } => {
                let (records, next_pos) = self.record_log.get_records(position, limit);
                let millis_behind = if records.is_empty() {
                    0
                } else {
                    now_epoch_millis().saturating_sub(
                        records.last().map_or(0, |r| r.approximate_arrival_timestamp)
                    )
                };
                let _ = reply.send(Ok(GetRecordsResult {
                    records: records.into_iter().cloned().collect(),
                    next_position: next_pos,
                    millis_behind_latest: millis_behind,
                }));
            }
            ShardCommand::GetShardIterator { iterator_type, reply } => {
                let position = match iterator_type {
                    ShardIteratorType::TrimHorizon => self.record_log.trim_horizon_position(),
                    ShardIteratorType::Latest => self.record_log.latest_position(),
                    ShardIteratorType::AtSequenceNumber(seq) => {
                        self.record_log.position_at_sequence_number(seq)
                    }
                    ShardIteratorType::AfterSequenceNumber(seq) => {
                        self.record_log.position_after_sequence_number(seq)
                    }
                    ShardIteratorType::AtTimestamp(ts) => {
                        self.record_log.position_at_timestamp(ts)
                    }
                };
                let _ = reply.send(Ok(position));
            }
            ShardCommand::Close { reply } => {
                self.closed = true;
                let last_seq = self.seq_gen.next(); // ending sequence number
                let _ = reply.send(last_seq);
            }
            ShardCommand::Shutdown => unreachable!("handled in run()"),
        }
    }
}
```

### 10.5 Stream State

```rust
/// Metadata and state for a Kinesis stream.
pub struct StreamState {
    /// Stream name.
    pub name: String,
    /// Stream ARN.
    pub arn: String,
    /// Stream status.
    pub status: StreamStatus,
    /// Stream mode (PROVISIONED or ON_DEMAND).
    pub mode: StreamMode,
    /// Data retention period.
    pub retention_period: Duration,
    /// Creation timestamp.
    pub creation_timestamp: u64,
    /// All shards (active + closed). Active shards accept writes; closed do not.
    pub shards: Vec<ShardInfo>,
    /// Active shard handles indexed for fast partition key routing.
    pub active_shards: Vec<ShardHandle>,
    /// Tags.
    pub tags: HashMap<String, String>,
    /// Encryption configuration.
    pub encryption_type: EncryptionType,
    pub key_id: Option<String>,
    /// Registered stream consumers (enhanced fan-out).
    pub consumers: HashMap<String, StreamConsumer>,
    /// Resource policy (stored, not enforced).
    pub resource_policy: Option<String>,
    /// Shard ID counter for generating new shard IDs.
    next_shard_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStatus {
    Creating,
    Active,
    Deleting,
    Updating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamMode {
    #[default]
    Provisioned,
    OnDemand,
}

/// Handle to a running shard actor.
pub struct ShardHandle {
    /// Shard metadata.
    pub info: ShardInfo,
    /// Channel to send commands to the shard actor.
    pub sender: mpsc::Sender<ShardCommand>,
    /// Actor task join handle.
    pub task: tokio::task::JoinHandle<()>,
}

/// Shard metadata (serializable for API responses).
#[derive(Debug, Clone)]
pub struct ShardInfo {
    pub shard_id: String,
    pub hash_key_range: HashKeyRange,
    pub starting_sequence_number: SequenceNumber,
    pub ending_sequence_number: Option<SequenceNumber>,
    pub parent_shard_id: Option<String>,
    pub adjacent_parent_shard_id: Option<String>,
}

/// Registered stream consumer for enhanced fan-out.
#[derive(Debug, Clone)]
pub struct StreamConsumer {
    pub consumer_name: String,
    pub consumer_arn: String,
    pub consumer_status: ConsumerStatus,
    pub consumer_creation_timestamp: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumerStatus {
    Creating,
    Active,
    Deleting,
}
```

### 10.6 Shard ID Format

Shard IDs follow the AWS format: `shardId-000000000000`, `shardId-000000000001`, etc.

```rust
fn format_shard_id(index: u32) -> String {
    format!("shardId-{index:012}")
}
```

---

## 11. Core Business Logic

### 11.1 Provider (StreamManager)

```rust
/// Main Kinesis provider. Manages all streams and their shards.
pub struct RustackKinesis {
    /// Stream registry: stream_name -> StreamState.
    streams: DashMap<String, StreamState>,
    /// Configuration.
    config: Arc<KinesisConfig>,
}

pub struct KinesisConfig {
    pub skip_signature_validation: bool,
    pub default_region: String,
    pub account_id: String,
    pub host: String,
    pub port: u16,
    pub default_shard_count: u32,
    pub default_retention_hours: u32,
}

impl KinesisConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("KINESIS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            host: env_str("GATEWAY_HOST", "localhost"),
            port: env_u16("GATEWAY_PORT", 4566),
            default_shard_count: 4, // AWS default for on-demand streams
            default_retention_hours: 24,
        }
    }
}
```

### 11.2 Operations Grouped by Category

#### Stream Management (7 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreateStream` | 0 | Medium | Create stream with N shards, divide hash key space evenly, spawn shard actors |
| `DeleteStream` | 0 | Low | Send Shutdown to all shard actors, remove from registry |
| `DescribeStream` | 0 | Medium | Return stream description with all shard info, hash key ranges, sequence numbers |
| `DescribeStreamSummary` | 0 | Low | Return summary without shard details |
| `ListStreams` | 0 | Low | Paginate streams by name with ExclusiveStartStreamName |
| `UpdateStreamMode` | 2 | Low | Store mode (PROVISIONED/ON_DEMAND), no behavioral difference |
| `DescribeLimits` | 3 | Low | Return static defaults (shard limit, open shard count) |

#### Record Operations (3 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `PutRecord` | 0 | Medium | MD5 hash partition key, route to shard, assign sequence number |
| `PutRecords` | 0 | High | Batch write up to 500 records, per-record shard routing, per-record success/failure |
| `GetRecords` | 0 | Medium | Resolve iterator token, read up to Limit records, return next iterator |

#### Shard Operations (5 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `GetShardIterator` | 0 | Medium | Create iterator token from 5 iterator types |
| `ListShards` | 0 | Medium | List shards with filtering by ShardFilter, pagination |
| `SplitShard` | 1 | High | Close parent, create two children, redistribute hash key ranges |
| `MergeShards` | 1 | High | Close two adjacent parents, create one merged child |
| `UpdateShardCount` | 2 | High | Resize stream by splitting/merging multiple shards |

#### Consumer Management (4 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `RegisterStreamConsumer` | 2 | Medium | Register enhanced fan-out consumer, store metadata |
| `DeregisterStreamConsumer` | 2 | Low | Remove consumer registration |
| `ListStreamConsumers` | 2 | Low | List consumers with pagination |
| `DescribeStreamConsumer` | 2 | Low | Return consumer metadata |

#### Tags (3 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `AddTagsToStream` | 1 | Low | Add/update tags (max 50) |
| `RemoveTagsFromStream` | 1 | Low | Remove specified tag keys |
| `ListTagsForStream` | 1 | Low | Return all tags with HasMoreTags pagination |

#### Retention (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `IncreaseStreamRetentionPeriod` | 1 | Low | Increase retention up to 8760 hours (365 days) |
| `DecreaseStreamRetentionPeriod` | 1 | Low | Decrease retention down to 24 hours |

#### Encryption (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `StartStreamEncryption` | 2 | Low | Store encryption metadata, no actual encryption |
| `StopStreamEncryption` | 2 | Low | Clear encryption metadata |

#### Monitoring (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `EnableEnhancedMonitoring` | 3 | Low | No-op, return requested shard-level metrics |
| `DisableEnhancedMonitoring` | 3 | Low | No-op, return empty metrics list |

#### Resource Policy (3 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `PutResourcePolicy` | 2 | Low | Store policy JSON, no enforcement |
| `GetResourcePolicy` | 2 | Low | Return stored policy |
| `DeleteResourcePolicy` | 2 | Low | Remove stored policy |

#### Enhanced Fan-out (1 operation)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `SubscribeToShard` | Deferred | Very High | HTTP/2 event streaming, requires major HTTP layer work |

#### Account Settings (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `DescribeAccountSettings` | 3 | Low | Return static defaults |
| `UpdateAccountSettings` | 3 | Low | No-op |

### 11.3 CreateStream Logic

```rust
impl RustackKinesis {
    pub async fn create_stream(
        &self,
        input: CreateStreamInput,
    ) -> Result<(), KinesisServiceError> {
        let stream_name = input.stream_name
            .ok_or_else(|| KinesisServiceError::InvalidArgument {
                message: "StreamName is required".to_string(),
            })?;

        // Validate stream name: 1-128 chars, [a-zA-Z0-9_.-]+
        validate_stream_name(&stream_name)?;

        // Check if stream already exists.
        if self.streams.contains_key(&stream_name) {
            return Err(KinesisServiceError::ResourceInUse {
                message: format!(
                    "Stream {stream_name} under account {} already exists.",
                    self.config.account_id
                ),
            });
        }

        let shard_count = input.shard_count.unwrap_or(self.config.default_shard_count as i32) as u32;
        let creation_time = now_epoch_millis();
        let arn = format!(
            "arn:aws:kinesis:{}:{}:stream/{}",
            self.config.default_region, self.config.account_id, stream_name
        );

        // Divide hash key space evenly among shards.
        let ranges = HashKeyRange::divide_evenly(shard_count);
        let mut shards = Vec::with_capacity(shard_count as usize);
        let mut active_shards = Vec::with_capacity(shard_count as usize);

        for (i, range) in ranges.into_iter().enumerate() {
            let shard_id = format_shard_id(i as u32);
            let seq_gen = SequenceNumberGenerator::new(i as u32, creation_time);
            let starting_seq = seq_gen.starting_sequence_number();

            let (sender, receiver) = mpsc::channel(256);
            let actor = ShardActor::new(
                shard_id.clone(),
                range.clone(),
                seq_gen,
                receiver,
                Duration::from_secs(self.config.default_retention_hours as u64 * 3600),
            );
            let task = tokio::spawn(actor.run());

            let info = ShardInfo {
                shard_id: shard_id.clone(),
                hash_key_range: range,
                starting_sequence_number: starting_seq,
                ending_sequence_number: None,
                parent_shard_id: None,
                adjacent_parent_shard_id: None,
            };

            shards.push(info.clone());
            active_shards.push(ShardHandle { info, sender, task });
        }

        let state = StreamState {
            name: stream_name.clone(),
            arn,
            status: StreamStatus::Active, // Skip CREATING for local dev
            mode: input.stream_mode_details
                .map_or(StreamMode::Provisioned, |m| {
                    if m.stream_mode.as_deref() == Some("ON_DEMAND") {
                        StreamMode::OnDemand
                    } else {
                        StreamMode::Provisioned
                    }
                }),
            retention_period: Duration::from_secs(
                self.config.default_retention_hours as u64 * 3600
            ),
            creation_timestamp: creation_time,
            shards,
            active_shards,
            tags: HashMap::new(),
            encryption_type: EncryptionType::None,
            key_id: None,
            consumers: HashMap::new(),
            resource_policy: None,
            next_shard_index: shard_count,
        };

        self.streams.insert(stream_name, state);
        Ok(())
    }
}
```

### 11.4 PutRecord Logic

```rust
impl RustackKinesis {
    pub async fn put_record(
        &self,
        input: PutRecordInput,
    ) -> Result<PutRecordOutput, KinesisServiceError> {
        let stream_name = self.resolve_stream_name(&input.stream_name, &input.stream_arn)?;
        let stream = self.get_active_stream(&stream_name)?;

        // Validate record data size (max 1MB).
        if input.data.len() > 1_048_576 {
            return Err(KinesisServiceError::InvalidArgument {
                message: "Record data exceeds 1MB limit".to_string(),
            });
        }

        // Validate partition key (1-256 characters).
        if input.partition_key.is_empty() || input.partition_key.len() > 256 {
            return Err(KinesisServiceError::InvalidArgument {
                message: "Partition key must be 1-256 characters".to_string(),
            });
        }

        // Route to appropriate shard.
        let shard = stream.route_to_shard(
            &input.partition_key,
            input.explicit_hash_key.as_deref(),
        )?;

        // Send record to shard actor.
        let (reply_tx, reply_rx) = oneshot::channel();
        shard.sender
            .send(ShardCommand::PutRecord {
                data: input.data,
                partition_key: input.partition_key,
                explicit_hash_key: input.explicit_hash_key,
                reply: reply_tx,
            })
            .await
            .map_err(|_| KinesisServiceError::InternalError {
                message: "Shard actor channel closed".to_string(),
            })?;

        let result = reply_rx.await.map_err(|_| KinesisServiceError::InternalError {
            message: "Shard actor did not respond".to_string(),
        })??;

        Ok(PutRecordOutput {
            shard_id: result.shard_id,
            sequence_number: result.sequence_number.to_string(),
            encryption_type: Some("NONE".to_string()),
        })
    }
}
```

### 11.5 GetShardIterator Logic

```rust
impl RustackKinesis {
    pub async fn get_shard_iterator(
        &self,
        input: GetShardIteratorInput,
    ) -> Result<GetShardIteratorOutput, KinesisServiceError> {
        let stream_name = self.resolve_stream_name(&input.stream_name, &input.stream_arn)?;
        let stream = self.get_stream(&stream_name)?;

        let shard = stream.find_shard_handle(&input.shard_id)?;

        let iterator_type = match input.shard_iterator_type.as_str() {
            "TRIM_HORIZON" => ShardIteratorType::TrimHorizon,
            "LATEST" => ShardIteratorType::Latest,
            "AT_SEQUENCE_NUMBER" => {
                let seq = input.starting_sequence_number
                    .ok_or_else(|| KinesisServiceError::InvalidArgument {
                        message: "StartingSequenceNumber required for AT_SEQUENCE_NUMBER".to_string(),
                    })?;
                ShardIteratorType::AtSequenceNumber(SequenceNumber::from_str(&seq)?)
            }
            "AFTER_SEQUENCE_NUMBER" => {
                let seq = input.starting_sequence_number
                    .ok_or_else(|| KinesisServiceError::InvalidArgument {
                        message: "StartingSequenceNumber required for AFTER_SEQUENCE_NUMBER".to_string(),
                    })?;
                ShardIteratorType::AfterSequenceNumber(SequenceNumber::from_str(&seq)?)
            }
            "AT_TIMESTAMP" => {
                let ts = input.timestamp
                    .ok_or_else(|| KinesisServiceError::InvalidArgument {
                        message: "Timestamp required for AT_TIMESTAMP".to_string(),
                    })?;
                ShardIteratorType::AtTimestamp((ts * 1000.0) as u64)
            }
            other => {
                return Err(KinesisServiceError::InvalidArgument {
                    message: format!("Invalid ShardIteratorType: {other}"),
                });
            }
        };

        // Ask shard actor for the position.
        let (reply_tx, reply_rx) = oneshot::channel();
        shard.sender
            .send(ShardCommand::GetShardIterator { iterator_type, reply: reply_tx })
            .await
            .map_err(|_| KinesisServiceError::InternalError {
                message: "Shard actor channel closed".to_string(),
            })?;

        let position = reply_rx.await.map_err(|_| KinesisServiceError::InternalError {
            message: "Shard actor did not respond".to_string(),
        })??;

        let token = ShardIteratorToken {
            stream_name,
            shard_id: input.shard_id,
            position,
            nonce: uuid::Uuid::new_v4().to_string(),
        };

        Ok(GetShardIteratorOutput {
            shard_iterator: Some(token.encode()),
        })
    }
}
```

### 11.6 GetRecords Logic

```rust
impl RustackKinesis {
    pub async fn get_records(
        &self,
        input: GetRecordsInput,
    ) -> Result<GetRecordsOutput, KinesisServiceError> {
        let token = ShardIteratorToken::decode(&input.shard_iterator)?;
        let stream = self.get_stream(&token.stream_name)?;
        let shard = stream.find_shard_handle(&token.shard_id)?;

        let limit = input.limit.unwrap_or(10_000).min(10_000) as usize;

        // Ask shard actor for records.
        let (reply_tx, reply_rx) = oneshot::channel();
        shard.sender
            .send(ShardCommand::GetRecords {
                position: token.position,
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|_| KinesisServiceError::InternalError {
                message: "Shard actor channel closed".to_string(),
            })?;

        let result = reply_rx.await.map_err(|_| KinesisServiceError::InternalError {
            message: "Shard actor did not respond".to_string(),
        })??;

        // Build next iterator token.
        let next_token = ShardIteratorToken {
            stream_name: token.stream_name,
            shard_id: token.shard_id,
            position: result.next_position,
            nonce: uuid::Uuid::new_v4().to_string(),
        };

        let records: Vec<Record> = result.records.into_iter().map(|r| Record {
            sequence_number: r.sequence_number.to_string(),
            approximate_arrival_timestamp: r.approximate_arrival_timestamp as f64 / 1000.0,
            data: r.data,
            partition_key: r.partition_key,
            encryption_type: Some("NONE".to_string()),
        }).collect();

        Ok(GetRecordsOutput {
            records,
            next_shard_iterator: Some(next_token.encode()),
            millis_behind_latest: Some(result.millis_behind_latest as i64),
            child_shards: Vec::new(),
        })
    }
}
```

### 11.7 Stream Name Resolution

Kinesis operations accept either `StreamName` or `StreamARN`. We must handle both:

```rust
impl RustackKinesis {
    /// Resolve a stream name from either StreamName or StreamARN input.
    fn resolve_stream_name(
        &self,
        stream_name: &Option<String>,
        stream_arn: &Option<String>,
    ) -> Result<String, KinesisServiceError> {
        if let Some(name) = stream_name {
            return Ok(name.clone());
        }
        if let Some(arn) = stream_arn {
            // ARN format: arn:aws:kinesis:<region>:<account>:stream/<name>
            return arn
                .rsplit_once('/')
                .map(|(_, name)| name.to_string())
                .ok_or_else(|| KinesisServiceError::InvalidArgument {
                    message: format!("Invalid stream ARN: {arn}"),
                });
        }
        Err(KinesisServiceError::InvalidArgument {
            message: "Either StreamName or StreamARN must be provided".to_string(),
        })
    }
}
```

---

## 12. Error Handling

### 12.1 Kinesis Error Codes

```rust
/// Domain-level errors for Kinesis business logic.
#[derive(Debug, thiserror::Error)]
pub enum KinesisServiceError {
    #[error("Stream {name} under account {account_id} not found.")]
    ResourceNotFound { name: String, account_id: String },

    #[error("Stream {name} under account {account_id} already exists.")]
    ResourceInUse { name: String, account_id: String },

    #[error("{message}")]
    InvalidArgument { message: String },

    #[error("Limit exceeded: {message}")]
    LimitExceeded { message: String },

    #[error("Provisioned throughput exceeded")]
    ProvisionedThroughputExceeded,

    #[error("Expired shard iterator")]
    ExpiredIterator,

    #[error("Access denied: {message}")]
    AccessDenied { message: String },

    #[error("Internal error: {message}")]
    InternalError { message: String },
}
```

### 12.2 Error Type Mapping

```rust
impl KinesisServiceError {
    /// JSON `__type` field value.
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::ResourceNotFound { .. } => "ResourceNotFoundException",
            Self::ResourceInUse { .. } => "ResourceInUseException",
            Self::InvalidArgument { .. } => "InvalidArgumentException",
            Self::LimitExceeded { .. } => "LimitExceededException",
            Self::ProvisionedThroughputExceeded => "ProvisionedThroughputExceededException",
            Self::ExpiredIterator => "ExpiredIteratorException",
            Self::AccessDenied { .. } => "AccessDeniedException",
            Self::InternalError { .. } => "InternalFailureException",
        }
    }

    /// HTTP status code.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::ResourceNotFound { .. } => 400,
            Self::ResourceInUse { .. } => 400,
            Self::InvalidArgument { .. } => 400,
            Self::LimitExceeded { .. } => 400,
            Self::ProvisionedThroughputExceeded => 400,
            Self::ExpiredIterator => 400,
            Self::AccessDenied { .. } => 400,
            Self::InternalError { .. } => 500,
        }
    }
}
```

### 12.3 Error Response Formatting

```rust
/// Format a Kinesis error response.
fn error_response(
    error: &KinesisServiceError,
    wire_format: KinesisWireFormat,
) -> http::Response<Bytes> {
    let body_map = serde_json::json!({
        "__type": error.error_type(),
        "message": error.to_string(),
    });

    let (body_bytes, content_type) = match wire_format {
        KinesisWireFormat::Json => (
            Bytes::from(serde_json::to_vec(&body_map).expect("JSON serialization")),
            "application/x-amz-json-1.1",
        ),
        KinesisWireFormat::Cbor => {
            let mut buf = Vec::new();
            ciborium::into_writer(&body_map, &mut buf).expect("CBOR serialization");
            (Bytes::from(buf), "application/x-amz-cbor-1.1")
        }
    };

    http::Response::builder()
        .status(error.status_code())
        .header("content-type", content_type)
        .body(body_bytes)
        .expect("valid error response")
}
```

---

## 13. Server Integration

### 13.1 Kinesis ServiceRouter

```rust
#[cfg(feature = "kinesis")]
mod kinesis_router {
    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the Kinesis service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `Kinesis_20131202.`.
    pub struct KinesisServiceRouter<H: KinesisHandler> {
        inner: KinesisHttpService<H>,
    }

    impl<H: KinesisHandler> KinesisServiceRouter<H> {
        pub fn new(inner: KinesisHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: KinesisHandler> ServiceRouter for KinesisServiceRouter<H> {
        fn name(&self) -> &'static str {
            "kinesis"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("Kinesis_20131202."))
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

#[cfg(feature = "kinesis")]
pub use kinesis_router::KinesisServiceRouter;
```

### 13.2 Feature Gate

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "kinesis"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http", "dep:rustack-s3-model"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
sqs = ["dep:rustack-sqs-core", "dep:rustack-sqs-http"]
ssm = ["dep:rustack-ssm-core", "dep:rustack-ssm-http"]
kinesis = ["dep:rustack-kinesis-core", "dep:rustack-kinesis-http"]
```

### 13.3 Gateway Registration Order

```rust
fn build_gateway(config: &ServerConfig) -> GatewayService {
    let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

    #[cfg(feature = "dynamodb")]
    services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_service)));

    #[cfg(feature = "sqs")]
    services.push(Box::new(SqsServiceRouter::new(sqs_service)));

    #[cfg(feature = "ssm")]
    services.push(Box::new(SsmServiceRouter::new(ssm_service)));

    #[cfg(feature = "kinesis")]
    services.push(Box::new(KinesisServiceRouter::new(kinesis_service)));

    #[cfg(feature = "s3")]
    services.push(Box::new(S3ServiceRouter::new(s3_service))); // catch-all, must be last

    GatewayService::new(services)
}
```

### 13.4 Configuration

```rust
impl KinesisConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("KINESIS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            host: env_str("GATEWAY_HOST", "localhost"),
            port: env_u16("GATEWAY_PORT", 4566),
            default_shard_count: env_u32("KINESIS_DEFAULT_SHARD_COUNT", 4),
            default_retention_hours: env_u32("KINESIS_DEFAULT_RETENTION_HOURS", 24),
        }
    }
}
```

### 13.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared with all services) |
| `KINESIS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification |
| `KINESIS_DEFAULT_SHARD_COUNT` | `4` | Default shard count for new streams |
| `KINESIS_DEFAULT_RETENTION_HOURS` | `24` | Default retention period in hours |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default AWS account ID |

### 13.6 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "running",
        "dynamodb": "running",
        "sqs": "running",
        "ssm": "running",
        "kinesis": "running"
    }
}
```

---

## 14. Testing Strategy

### 14.1 Unit Tests

Each module tested in isolation:

- **Shard routing**: Test MD5 partition key hashing, hash key range containment, even division of hash space
- **Sequence number generation**: Test monotonicity, uniqueness across shards, formatting
- **Shard iterator tokens**: Test encode/decode round-trip, all 5 iterator types resolve to correct positions
- **Record log**: Test append, get with position, trim expired, binary search by sequence number and timestamp
- **Shard splitting**: Test hash key range division, parent closing, child creation
- **Shard merging**: Test adjacency validation, merged range correctness
- **Wire format detection**: Test JSON vs CBOR content-type routing
- **CBOR timestamp encoding**: Test integer (not float) epoch seconds in CBOR responses
- **Stream name resolution**: Test StreamName vs StreamARN resolution, ARN parsing

### 14.2 Integration Tests with aws-sdk-kinesis

```rust
// tests/integration/kinesis_tests.rs
#[tokio::test]
#[ignore]
async fn test_kinesis_stream_lifecycle() {
    let client = aws_sdk_kinesis::Client::new(&config);

    // Create stream
    client.create_stream()
        .stream_name("test-stream")
        .shard_count(2)
        .send().await.unwrap();

    // Describe stream
    let desc = client.describe_stream()
        .stream_name("test-stream")
        .send().await.unwrap();
    let stream_desc = desc.stream_description().unwrap();
    assert_eq!(stream_desc.shards().len(), 2);
    assert_eq!(stream_desc.stream_status(), &StreamStatus::Active);

    // Put record
    let put = client.put_record()
        .stream_name("test-stream")
        .data(Blob::new("hello"))
        .partition_key("pk1")
        .send().await.unwrap();
    assert!(!put.sequence_number().unwrap().is_empty());

    // Get shard iterator (TRIM_HORIZON)
    let shard_id = stream_desc.shards()[0].shard_id();
    let iter = client.get_shard_iterator()
        .stream_name("test-stream")
        .shard_id(shard_id)
        .shard_iterator_type(ShardIteratorType::TrimHorizon)
        .send().await.unwrap();

    // Get records
    let records = client.get_records()
        .shard_iterator(iter.shard_iterator().unwrap())
        .send().await.unwrap();
    assert!(!records.records().is_empty());

    // Delete stream
    client.delete_stream()
        .stream_name("test-stream")
        .send().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_kinesis_partition_key_routing() {
    // Test that records with the same partition key always go to the same shard
}

#[tokio::test]
#[ignore]
async fn test_kinesis_all_iterator_types() {
    // Test TRIM_HORIZON, LATEST, AT_SEQUENCE_NUMBER, AFTER_SEQUENCE_NUMBER, AT_TIMESTAMP
}

#[tokio::test]
#[ignore]
async fn test_kinesis_put_records_batch() {
    // Test batch write with multiple records, verify per-record shard routing
}

#[tokio::test]
#[ignore]
async fn test_kinesis_shard_split_merge() {
    // Test splitting a shard, verifying hash key ranges, writing to children
    // Test merging two adjacent shards
}
```

### 14.3 Third-Party Test Suites

#### 14.3.1 LocalStack Test Suite (Primary)

The most comprehensive open-source Kinesis test suite. Already vendored at `vendors/localstack/tests/aws/services/kinesis/`.

- **`test_kinesis.py`** -- main test module covering:
  - Stream creation with/without shard count
  - Resource policy CRUD
  - Stream consumers (register, describe, list, deregister)
  - SubscribeToShard (with TRIM_HORIZON, AT_TIMESTAMP, AT_SEQUENCE_NUMBER)
  - GetRecords (JSON and CBOR encoding)
  - GetRecords with empty stream
  - Record lifecycle data integrity (Unicode, empty data, large data)
  - SubscribeToShard timeout
  - Tag operations (add, list)
  - Next shard iterator (verifying each call returns a new iterator)
  - Shard iterator with surrounding quotes
  - CBOR timestamp encoding (integer, not float)
  - CBOR blob handling
  - CBOR error responses
  - Java SDK v2 Lambda integration test

- **`conftest.py`** -- fixtures for stream consumer management
- **`helper_functions.py`** -- shard iterator helper functions

Adaptation strategy: same approach as DynamoDB/SQS -- run the Python test suite against Rustack's Kinesis endpoint, track pass/fail counts, progressively fix failures.

```makefile
test-kinesis-localstack:
    @cd vendors/localstack && python -m pytest tests/aws/services/kinesis/test_kinesis.py \
        --endpoint-url=http://localhost:4566 -v
```

#### 14.3.2 kinesalite Test Suite (Secondary Validation)

- **Repository**: https://github.com/mhart/kinesalite
- **Location**: `test/` directory
- **Language**: JavaScript (Node.js)
- **Coverage**: 19 test files covering:
  - `createStream.js` / `deleteStream.js` / `describeStream.js` / `describeStreamSummary.js`
  - `putRecord.js` / `putRecords.js` / `getRecords.js` / `getShardIterator.js`
  - `listShards.js` / `splitShard.js` / `mergeShards.js`
  - `increaseStreamRetentionPeriod.js` / `decreaseStreamRetentionPeriod.js`
  - `addTagsToStream.js` / `listTagsForStream.js` / `removeTagsFromStream.js`
  - `connection.js` / `helpers.js`

Adaptation strategy: run kinesalite's test suite against Rustack endpoint by configuring the test runner to use `http://localhost:4566`. Requires Node.js installed in CI.

```makefile
test-kinesis-kinesalite:
    @cd vendors/kinesalite && npm test -- --endpoint http://localhost:4566
```

#### 14.3.3 kinesis-mock Test Suite (Tertiary Validation)

- **Repository**: https://github.com/etspaceman/kinesis-mock
- **Language**: Scala (Http4s/Cats-effect)
- **Location**: `integration-tests/`, `unit-tests/`, `testkit/`
- **Coverage**: Core Kinesis operations with CBOR support
- **Notes**: This is the same engine LocalStack uses. Running its test suite validates behavioral parity.

#### 14.3.4 AWS SDK Integration Tests

Write targeted tests using `aws-sdk-kinesis` Rust crate:

```rust
// Test each operation against known AWS behavior
// Focus on edge cases: empty streams, max batch sizes, invalid parameters,
// shard iterator expiry, sequence number ordering, hash key routing
```

#### 14.3.5 AWS CLI Smoke Tests

Shell-based end-to-end tests for CI:

```bash
#!/bin/bash
# Basic Kinesis CLI smoke test
ENDPOINT="--endpoint-url http://localhost:4566"

# Create stream
aws kinesis create-stream $ENDPOINT --stream-name test-stream --shard-count 1

# Wait for ACTIVE (skip for Rustack -- immediate)
sleep 1

# Put record
aws kinesis put-record $ENDPOINT \
    --stream-name test-stream \
    --data "SGVsbG8=" \
    --partition-key "pk1"

# Get shard iterator
SHARD_ITERATOR=$(aws kinesis get-shard-iterator $ENDPOINT \
    --stream-name test-stream \
    --shard-id shardId-000000000000 \
    --shard-iterator-type TRIM_HORIZON \
    --query ShardIterator --output text)

# Get records
aws kinesis get-records $ENDPOINT --shard-iterator "$SHARD_ITERATOR"

# Delete stream
aws kinesis delete-stream $ENDPOINT --stream-name test-stream
```

### 14.4 Makefile Targets

```makefile
test-kinesis: test-kinesis-unit test-kinesis-integration

test-kinesis-unit:
    @cargo test -p rustack-kinesis-model -p rustack-kinesis-core -p rustack-kinesis-http

test-kinesis-integration:
    @cargo test -p integration-tests -- kinesis --ignored

test-kinesis-cli:
    @./tests/kinesis-cli-smoke.sh

test-kinesis-localstack:
    @cd vendors/localstack && python -m pytest tests/aws/services/kinesis/ -v
```

---

## 15. Phased Implementation Plan

### Phase 0: MVP (10 Operations -- Stream CRUD, Put/Get Records, Shard Iterators)

**Goal**: Cover the most common local development use case: create a stream, write records, read records with iterators.
**Estimated scope**: ~6,000-8,000 lines of Rust code across 3 new crates.

#### Step 0.1: Codegen Extension
- Extend codegen `main.rs` to accept Kinesis Smithy model path
- Add Kinesis namespace prefix to `shapes.rs`
- Download Kinesis Smithy model JSON from `aws/api-models-aws`
- Generate `rustack-kinesis-model` crate (operations enum, input/output structs, error codes)
- Generate serde derives with `#[serde(rename_all = "PascalCase")]`

#### Step 0.2: HTTP Layer (JSON Protocol Only)
- Implement Kinesis router (`Kinesis_20131202.*` dispatch)
- Implement `KinesisHttpService` (hyper Service)
- Implement JSON request deserialization
- Implement JSON response serialization
- Implement JSON error formatting with `__type` field

#### Step 0.3: Shard Engine Infrastructure
- Implement `HashKey` type with MD5 hashing and decimal string conversion
- Implement `HashKeyRange` with even division and containment checks
- Implement `SequenceNumberGenerator` with 128-bit monotonic sequence numbers
- Implement `ShardRecordLog` (append, get, position lookup by seq/timestamp)
- Implement `ShardActor` with command channel and event loop
- Implement `ShardIteratorToken` encode/decode

#### Step 0.4: Stream State Management
- Implement `StreamState` with shard creation and hash key space division
- Implement `RustackKinesis` provider with `DashMap<String, StreamState>`
- Implement partition key routing (MD5 hash -> shard lookup)

#### Step 0.5: Core Operations (10 ops)
- `CreateStream` / `DeleteStream` / `DescribeStream` / `DescribeStreamSummary` / `ListStreams`
- `PutRecord` / `PutRecords` / `GetRecords`
- `GetShardIterator` (all 5 iterator types)
- `ListShards`

#### Step 0.6: Server Integration
- Implement `KinesisServiceRouter` with gateway registration
- Add `kinesis` cargo feature gate
- Register Kinesis before S3 in gateway
- Update health endpoint

#### Step 0.7: Testing
- Unit tests for hash key, sequence number, record log, iterator tokens
- Integration tests with aws-sdk-kinesis
- CLI smoke tests
- Update Makefile with Kinesis test targets

### Phase 1: Tags, Retention, Shard Splitting/Merging (10 Operations + Features)

**Goal**: Support resharding and stream configuration changes.

- `AddTagsToStream` / `RemoveTagsFromStream` / `ListTagsForStream`
- `IncreaseStreamRetentionPeriod` / `DecreaseStreamRetentionPeriod`
- `SplitShard` / `MergeShards` (shard splitting and merging with hash key range redistribution)
- `UpdateShardCount` (automated resharding via split/merge)
- Background retention cleanup (periodic trimming of expired records)
- Closed shard handling (no writes, reads until retention expiry)

### Phase 2: Consumers, Encryption, Resource Policies, CBOR Protocol (12 Operations + Features)

**Goal**: Enhanced fan-out consumer registration and CBOR wire format support.

- `RegisterStreamConsumer` / `DeregisterStreamConsumer` / `ListStreamConsumers` / `DescribeStreamConsumer`
- `StartStreamEncryption` / `StopStreamEncryption` (metadata only)
- `PutResourcePolicy` / `GetResourcePolicy` / `DeleteResourcePolicy`
- `UpdateStreamMode`
- **CBOR wire format**: `application/x-amz-cbor-1.1` request/response serialization
  - CBOR request deserialization with `ciborium`
  - CBOR response serialization with integer timestamps (not float)
  - CBOR blob handling (byte strings, not base64)
  - CBOR error response formatting

### Phase 3: Remaining Operations, Polish

**Goal**: Feature completeness for all non-streaming operations.

- `EnableEnhancedMonitoring` / `DisableEnhancedMonitoring` (no-op, return metrics list)
- `DescribeLimits` / `DescribeAccountSettings` / `UpdateAccountSettings` (static defaults)
- `ListTagsForResource` / `TagResource` / `UntagResource` (newer tagging API)
- `UpdateMaxRecordSize` / `UpdateStreamWarmThroughput` (no-op, accept and store)
- Run full LocalStack test suite, fix remaining failures
- Run kinesalite test suite as secondary validation
- GitHub Action integration (add `kinesis` to `services` input)
- Docker image update

### Deferred: SubscribeToShard (Enhanced Fan-out)

**Goal**: HTTP/2 event streaming for enhanced fan-out consumers.

- `SubscribeToShard` requires HTTP/2 server push / chunked event streaming
- Significant HTTP layer complexity (event framing, keepalive, timeout management)
- Can be implemented as a follow-up when demand materializes
- Most local development uses polling consumers (GetRecords) rather than fan-out

---

## 16. Risk Analysis

### 16.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| CBOR timestamp encoding correctness | High | High | AWS Java SDK rejects float timestamps; test with actual Java SDK; verify CBOR byte-level encoding |
| Sequence number format incompatibility | Medium | High | Test against real AWS sequence numbers; ensure 56-digit decimal format; test KCL checkpoint compatibility |
| Hash key range division edge cases | Medium | Medium | Property-based tests with `proptest` to verify complete coverage of `[0, 2^128-1]` |
| Shard iterator token expiry (5-minute) | Medium | Low | For local dev, do not enforce expiry initially; add as config option |
| CBOR blob vs base64 confusion | High | High | CBOR sends raw bytes for Data field; JSON sends base64. Must handle correctly per wire format |
| Shard splitting/merging ordering | Medium | High | Complex state transitions; extensive unit tests for hash key range continuity |
| Record log memory growth | Medium | Medium | Periodic retention cleanup in shard actor; configurable retention period |
| Actor channel backpressure | Low | Medium | Bounded channels (256 capacity); return throttling errors if full |

### 16.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| SubscribeToShard demanded early | Medium | High | Defer; most local dev uses polling. KCL 2.x can be configured to use polling via `enhanced-fan-out=false` |
| KCL compatibility requires exact sequence number format | High | Medium | Study KCL source code for sequence number parsing; test with actual KCL |
| CBOR protocol more complex than expected | Medium | Medium | Defer to Phase 2; JSON protocol covers all SDKs except Java default |
| Users expect immediate stream ACTIVE status | Low | Low | Skip CREATING state for local dev (immediate ACTIVE) |

### 16.3 Behavioral Differences

| Behavior | kinesis-mock | kinesalite | Rustack | Justification |
|----------|-------------|------------|-----------|---------------|
| Stream creation time | Configurable delay | Configurable delay | Immediate (ACTIVE) | Faster local dev; no CREATING wait |
| Throughput limits | Not enforced | Not enforced | Not enforced | Not needed for local dev |
| Shard iterator expiry | 5 minutes | 5 minutes | Not enforced initially | Avoids annoying timeouts locally |
| Enhanced fan-out | Not supported | Not supported | Deferred | Complex HTTP/2 streaming |
| CBOR support | Yes | No | Phase 2 | JSON covers most SDKs |
| Shard split/merge | Yes | Yes | Phase 1 | Important for resharding tests |
| Error messages | Custom | Custom | Match AWS messages | Better SDK compatibility |

---

## Appendix A: Kinesis vs SQS Implementation Effort Comparison

| Component | SQS Lines (est.) | Kinesis Est. | Ratio | Notes |
|-----------|-----------------|-------------|-------|-------|
| Model (codegen output) | ~2,500 | ~3,000 | 1.2x | More operations, more types (Shard, HashKeyRange, etc.) |
| JSON serde | ~200 | ~200 | 1.0x | Same approach, serde derives |
| CBOR serde | N/A | ~800 | New | CBOR serialization/deserialization with timestamp handling |
| HTTP routing | ~300 | ~200 | 0.7x | No dual protocol (JSON+Query); just JSON+CBOR |
| Auth integration | ~100 | ~100 | 1.0x | SigV4 only, identical |
| Core business logic | ~4,000 | ~4,500 | 1.1x | More operations, shard routing logic |
| Storage engine | ~2,000 | ~2,500 | 1.3x | Record log + retention vs message lifecycle |
| Actor infrastructure | ~1,000 | ~1,200 | 1.2x | Actor per shard (similar to actor per queue) |
| Shard engine | N/A | ~1,500 | New | Hash key routing, splitting, merging, iterators |
| **Total** | **~12,100** | **~14,000** | **1.16x** | |

Kinesis is moderately more complex than SQS primarily due to the shard engine (hash key routing, sequence numbers, iterators, split/merge) and CBOR protocol support. There is no expression language or dual protocol (awsQuery) complexity.

## Appendix B: Kinesis Error Codes and HTTP Status Codes

| Error Code | HTTP Status | When |
|-----------|------------|------|
| `ResourceNotFoundException` | 400 | Stream or consumer not found |
| `ResourceInUseException` | 400 | Stream already exists or is in CREATING/DELETING state |
| `InvalidArgumentException` | 400 | Invalid parameter value |
| `LimitExceededException` | 400 | Too many shards or streams |
| `ProvisionedThroughputExceededException` | 400 | Write/read limit exceeded (not enforced) |
| `ExpiredIteratorException` | 400 | Shard iterator expired (5-minute TTL) |
| `KMSDisabledException` | 400 | KMS key disabled |
| `KMSInvalidStateException` | 400 | KMS key in invalid state |
| `KMSAccessDeniedException` | 400 | No access to KMS key |
| `KMSNotFoundException` | 400 | KMS key not found |
| `KMSOptInRequired` | 400 | KMS opt-in required |
| `KMSThrottlingException` | 400 | KMS throttled |
| `AccessDeniedException` | 400 | Access denied |
| `InternalFailureException` | 500 | Internal server error |
| `ServiceUnavailableException` | 503 | Service unavailable |

## Appendix C: Kinesis Constraints and Limits

| Resource | Limit | Enforced in Rustack? |
|----------|-------|----------------------|
| Max record data size | 1 MB | Yes |
| Max PutRecords batch | 500 records or 5 MB total | Yes (record count), Yes (size) |
| Max GetRecords limit | 10,000 records or 10 MB | Yes (record count), No (size) |
| Partition key length | 1-256 characters | Yes |
| Stream name length | 1-128 characters | Yes |
| Stream name characters | `[a-zA-Z0-9_.-]+` | Yes |
| Max shards per stream | 500 (default) | No (unbounded) |
| Max streams per account | 500 (default) | No (unbounded) |
| Max tags per stream | 50 | Yes |
| Retention period range | 24-8760 hours (1-365 days) | Yes |
| Default retention | 24 hours | Yes |
| Shard write throughput | 1 MB/s or 1,000 records/s | No (not enforced) |
| Shard read throughput | 2 MB/s or 10,000 records per GetRecords | No (not enforced) |
| Shard iterator TTL | 5 minutes | No (not enforced initially) |
| Max consumers per stream | 20 | No (unbounded) |

## Appendix D: Complete Operation List (40 Operations)

All Kinesis Data Streams API operations and their implementation status:

| # | Operation | Phase | Priority | Notes |
|---|-----------|-------|----------|-------|
| 1 | CreateStream | 0 | Critical | Stream creation with shard count |
| 2 | DeleteStream | 0 | Critical | Stream deletion |
| 3 | DescribeStream | 0 | Critical | Full stream description with shards |
| 4 | DescribeStreamSummary | 0 | Critical | Summary without shard details |
| 5 | ListStreams | 0 | Critical | Paginated stream listing |
| 6 | PutRecord | 0 | Critical | Single record write |
| 7 | PutRecords | 0 | Critical | Batch record write (up to 500) |
| 8 | GetRecords | 0 | Critical | Read records from shard |
| 9 | GetShardIterator | 0 | Critical | Create shard iterator (5 types) |
| 10 | ListShards | 0 | Critical | List shards with filtering |
| 11 | AddTagsToStream | 1 | High | Add/update stream tags |
| 12 | RemoveTagsFromStream | 1 | High | Remove stream tags |
| 13 | ListTagsForStream | 1 | High | List stream tags |
| 14 | IncreaseStreamRetentionPeriod | 1 | High | Increase retention up to 365d |
| 15 | DecreaseStreamRetentionPeriod | 1 | High | Decrease retention down to 24h |
| 16 | SplitShard | 1 | High | Split shard at hash key |
| 17 | MergeShards | 1 | High | Merge two adjacent shards |
| 18 | UpdateShardCount | 2 | Medium | Automated resharding |
| 19 | RegisterStreamConsumer | 2 | Medium | Register enhanced fan-out consumer |
| 20 | DeregisterStreamConsumer | 2 | Medium | Remove consumer |
| 21 | ListStreamConsumers | 2 | Medium | List consumers |
| 22 | DescribeStreamConsumer | 2 | Medium | Consumer details |
| 23 | StartStreamEncryption | 2 | Medium | Metadata only |
| 24 | StopStreamEncryption | 2 | Medium | Metadata only |
| 25 | PutResourcePolicy | 2 | Medium | Store policy, no enforcement |
| 26 | GetResourcePolicy | 2 | Medium | Return stored policy |
| 27 | DeleteResourcePolicy | 2 | Medium | Remove stored policy |
| 28 | UpdateStreamMode | 2 | Medium | Store mode, no behavioral difference |
| 29 | EnableEnhancedMonitoring | 3 | Low | No-op |
| 30 | DisableEnhancedMonitoring | 3 | Low | No-op |
| 31 | DescribeLimits | 3 | Low | Static defaults |
| 32 | DescribeAccountSettings | 3 | Low | Static defaults |
| 33 | UpdateAccountSettings | 3 | Low | No-op |
| 34 | ListTagsForResource | 3 | Low | Newer tagging API |
| 35 | TagResource | 3 | Low | Newer tagging API |
| 36 | UntagResource | 3 | Low | Newer tagging API |
| 37 | UpdateMaxRecordSize | 3 | Low | No-op |
| 38 | UpdateStreamWarmThroughput | 3 | Low | No-op |
| 39 | SubscribeToShard | Deferred | Low | HTTP/2 event streaming |
| 40 | DescribeAccountSettings | 3 | Low | Duplicate of #32 |

**Phase 0**: 10 operations (Critical path for local dev)
**Phase 1**: 7 operations (Tags, retention, resharding)
**Phase 2**: 10 operations (Consumers, encryption, CBOR)
**Phase 3**: 12 operations (Monitoring, limits, newer APIs)
**Deferred**: 1 operation (SubscribeToShard)
