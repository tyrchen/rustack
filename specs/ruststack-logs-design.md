# Rustack CloudWatch Logs: Native Rust Implementation Design

**Date:** 2026-03-06
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-ssm-design.md](./rustack-ssm-design.md)
**Scope:** Add CloudWatch Logs support to Rustack -- targeting ~35 core operations covering log group/stream management, event ingestion/retrieval, filtering, metric filters, subscription filters, resource policies, tags, and basic Insights queries, using the same Smithy-based codegen and `awsJson1.1` gateway routing patterns established by SSM.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Design: awsJson1.1](#5-protocol-design-awsjson11)
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

This spec proposes adding CloudWatch Logs support to Rustack as a fully native Rust implementation. Key design decisions:

- **Medium-scope service** -- ~35 operations for the MVP surface area (out of 152+ total CW Logs operations). CloudWatch Logs is larger than SSM Parameter Store (13 ops) but smaller than DynamoDB (66 ops) or S3 (90+ ops). We target the operations that matter for local development: log group/stream CRUD, event ingestion/retrieval, filtering, metric filters, subscription filters, resource policies, tagging, and basic Insights queries.
- **Same protocol as SSM** -- CloudWatch Logs uses `awsJson1.1` with `X-Amz-Target: Logs_20140328.<Op>`. This is identical to SSM's protocol pattern. We reuse the same codegen path, JSON request/response serialization, and error formatting.
- **Append-only log storage engine** -- log events are stored in time-ordered sequences per log stream. Unlike SQS (message lifecycle state machine) or DynamoDB (indexed tables), the storage model is simple: append events, query by time range. Memory management is handled via configurable per-stream event limits and retention policy enforcement.
- **No background processing for MVP** -- metric filters are stored but do not emit CloudWatch metrics. Subscription filters are stored but do not forward to Lambda/Kinesis/Firehose. These are metadata-only for MVP.
- **Filter pattern evaluation** -- `FilterLogEvents` with simple text patterns (space-separated terms, quoted phrases) is implemented in Phase 1. JSON filter patterns (`{$.field = value}`) are a Phase 2 stretch goal.
- **Shared infrastructure** -- reuse `rustack-core` (multi-account/region state), `rustack-auth` (SigV4 verification), and the SSM-established `awsJson1.1` HTTP layer patterns unchanged.
- **Phased delivery** -- 4 phases from MVP (log group/stream CRUD, event put/get) to full feature parity including filter patterns, Insights queries, and metric/subscription filter CRUD.

---

## 2. Motivation

### 2.1 Why CloudWatch Logs?

CloudWatch Logs is the universal logging destination for AWS workloads. Developers need a local CW Logs emulator for:

- **Lambda function log testing** -- every Lambda invocation writes to `/aws/lambda/<function-name>`. Testing Lambda locally requires a CW Logs endpoint to receive and query those logs.
- **Container logging** -- ECS and EKS use the `awslogs` log driver to ship container stdout/stderr to CW Logs. Local `docker compose` workflows need a CW Logs endpoint for log verification.
- **fluent-bit / Fluentd / Vector.dev** -- the most popular log shippers all have native CW Logs output plugins. Local testing of observability pipelines requires a CW Logs-compatible endpoint.
- **Application logging** -- AWS SDK-based applications use the CW Logs API directly (e.g., `PutLogEvents`) for structured logging. Local development needs a destination that accepts and stores these events.
- **CI/CD pipelines** -- integration tests that verify log output (e.g., "assert this Lambda wrote this log message") need a CW Logs API to query against.
- **CloudWatch Agent testing** -- the AWS CloudWatch Agent ships logs from EC2 instances; testing agent configurations locally requires an endpoint.
- **Terraform/CDK infrastructure testing** -- IaC tools create log groups with retention policies, metric filters, and subscription filters. Local plan/apply cycles need the CW Logs API to succeed.

### 2.2 Why Native Rust?

A native Rust implementation provides:

- **~10MB Docker image** (same as S3/DynamoDB/SQS/SSM) -- no additional runtime dependencies
- **Millisecond startup** -- CW Logs is available immediately when the container starts
- **~5MB memory baseline** -- memory grows only with stored log events
- **Full debuggability** -- we own every line of code
- **Single binary** -- CW Logs, S3, DynamoDB, SQS, and SSM all served from one process on port 4566

### 2.3 Existing Alternatives

| Implementation | Language | Image Size | Operations | Notes |
|---------------|----------|------------|------------|-------|
| LocalStack CW Logs | Python | ~1GB | ~40 (Pro: full) | Built-in, mature, but heavy |
| moto (getmoto/moto) | Python | N/A (lib) | ~30 | In-process mock, no standalone server |
| fake-cloudwatch-logs | Node.js | ~100MB | ~5 | Minimal, abandoned |
| Mockoon | N/A | N/A | Static mocks | No business logic |
| **Rustack CW Logs** | **Rust** | **~10MB** | **~35** | **This proposal** |

No existing Rust-based CloudWatch Logs emulator exists. This would be the first.

### 2.4 Tools and Integrations Unlocked

| Tool | CW Logs Operations Used | Priority |
|------|------------------------|----------|
| AWS CLI (`aws logs`) | All core operations | P0 |
| AWS SDK (Rust, Python, Java, Go) | All core operations | P0 |
| fluent-bit `cloudwatch_logs` output | `CreateLogGroup`, `CreateLogStream`, `PutLogEvents`, `DescribeLogStreams` | P0 |
| Vector.dev `aws_cloudwatch_logs` sink | Same as fluent-bit | P0 |
| Fluentd `fluent-plugin-cloudwatch-logs` | Same as fluent-bit | P0 |
| CloudWatch Agent | `CreateLogGroup`, `CreateLogStream`, `PutLogEvents` | P0 |
| Terraform `aws_cloudwatch_log_group` | `CreateLogGroup`, `DeleteLogGroup`, `DescribeLogGroups`, `PutRetentionPolicy`, tags | P1 |
| Terraform `aws_cloudwatch_metric_alarm` | `PutMetricFilter`, `DescribeMetricFilters` | P2 |
| Docker `awslogs` log driver | `CreateLogGroup`, `CreateLogStream`, `PutLogEvents` | P0 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Native Rust CW Logs emulator** -- no external processes, no FFI, no JVM
2. **Cover 95%+ of local development use cases** -- log group/stream CRUD, event ingestion/retrieval, filtering, metric filter CRUD, subscription filter CRUD, resource policies, tagging
3. **awsJson1.1 protocol** -- identical to SSM; `X-Amz-Target: Logs_20140328.<Op>`
4. **Smithy-generated types** -- all CW Logs API types generated from official AWS Smithy model
5. **Append-only log storage** -- time-ordered event storage with configurable memory limits
6. **Retention policy enforcement** -- background task that evicts events older than the configured retention period
7. **Simple text filter patterns** -- `FilterLogEvents` with space-separated term matching and quoted phrases
8. **Same Docker image** -- single binary serves S3, DynamoDB, SQS, SSM, and CW Logs on port 4566
9. **GitHub Action compatibility** -- extend the existing `tyrchen/rustack` GitHub Action
10. **Pass LocalStack CW Logs test suite** -- validate against vendored `vendors/localstack/tests/aws/services/logs/`

### 3.2 Non-Goals

1. **Log Insights query language** -- `StartQuery`/`GetQueryResults` will return stub results for MVP; full query parsing is a future enhancement
2. **Subscription filter forwarding** -- accept `PutSubscriptionFilter` and store the filter metadata, but do not actually forward matching events to Lambda/Kinesis/Firehose destinations
3. **Metric filter evaluation** -- accept `PutMetricFilter` and store the filter metadata, but do not actually publish CloudWatch metrics
4. **Export tasks** -- `CreateExportTask` to S3 is out of scope
5. **Live tail** -- `StartLiveTail` streaming is out of scope
6. **Anomaly detection** -- `CreateLogAnomalyDetector` and related operations are out of scope
7. **Data protection policies** -- accept but do not enforce data masking
8. **Cross-account log delivery** -- `PutDestination`/`PutDestinationPolicy` are stored but cross-account routing is not implemented
9. **KMS encryption** -- accept `kmsKeyId` on log groups but do not perform actual encryption
10. **Data persistence across restarts** -- in-memory only, matching all other Rustack services
11. **JSON filter patterns** -- `{$.field = value}` syntax in `FilterLogEvents` is a Phase 2 stretch goal
12. **Delivery/integration operations** -- `CreateDelivery`, `PutDeliverySource`, `PutDeliveryDestination`, S3 table integrations, transformers, and other advanced features added post-2023

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                    AWS SDK / CLI / fluent-bit / Vector
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  X-Amz-Target dispatch
              +--------+------------+
                       |
         +------+------+------+------+------+
         |      |      |      |      |
         v      v      v      v      v
   +------+ +------+ +------+ +------+ +------+
   |S3 HTTP| |DDB   | |SQS   | |SSM   | |Logs  |
   |RestXml| |Json10| |Json10 | |Json11| |Json11|
   +--+---+ +--+---+ |+Query | +--+---+ +--+---+
      |        |      +--+---+    |         |
   +--+---+ +--+---+ +--+---+ +--+---+ +--+---+
   |S3    | |DDB   | |SQS   | |SSM   | |Logs  |
   |Core  | |Core  | |Core  | |Core  | |Core  |
   +--+---+ +--+---+ +--+---+ +--+---+ +--+---+
      |        |        |        |         |
      +--------+--------+--------+---------+
                       |
              +--------+--------+
              | rustack-core  |
              | rustack-auth  |
              +-----------------+
```

### 4.2 Gateway Routing

CW Logs requests are distinguished by the `X-Amz-Target` header prefix:

| Service | X-Amz-Target Prefix | Content-Type |
|---------|---------------------|--------------|
| DynamoDB | `DynamoDB_20120810.` | `application/x-amz-json-1.0` |
| SQS | `AmazonSQS.` | `application/x-amz-json-1.0` |
| SSM | `AmazonSSM.` | `application/x-amz-json-1.1` |
| **CW Logs** | **`Logs_20140328.`** | **`application/x-amz-json-1.1`** |
| S3 | *(absent)* | varies |

Routing logic: check `X-Amz-Target` header. If prefix is `Logs_20140328.`, route to CW Logs. This is unambiguous and does not conflict with any existing service prefix.

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-logs-model        <-- NEW (auto-generated)
+-- rustack-logs-core         <-- NEW
+-- rustack-logs-http         <-- NEW

rustack-logs-http
+-- rustack-logs-model
+-- rustack-auth

rustack-logs-core
+-- rustack-core
+-- rustack-logs-model
+-- tokio (time for retention cleanup)
+-- dashmap

rustack-logs-model (auto-generated, standalone)
```

---

## 5. Protocol Design: awsJson1.1

### 5.1 Protocol Details

CloudWatch Logs uses `awsJson1.1`, identical to SSM. The only differences from SSM are the target prefix and service name for SigV4.

| Aspect | SSM (awsJson1.1) | CW Logs (awsJson1.1) |
|--------|-------------------|----------------------|
| HTTP Method | POST only | POST only |
| URL Path | `/` always | `/` always |
| Content-Type | `application/x-amz-json-1.1` | `application/x-amz-json-1.1` |
| X-Amz-Target | `AmazonSSM.<Op>` | `Logs_20140328.<Op>` |
| Request body | JSON | JSON |
| Response body | JSON | JSON |
| Error `__type` | Short name (e.g., `ParameterNotFound`) | Short name (e.g., `ResourceNotFoundException`) |
| Auth | SigV4, service=`ssm` | SigV4, service=`logs` |

### 5.2 What We Reuse from SSM

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| JSON request deserialization | Yes | `serde_json::from_slice` with `Deserialize` derives |
| JSON response serialization | Yes | `serde_json::to_vec` with `Serialize` derives |
| `X-Amz-Target` header parsing | Yes | Same pattern, different prefix (`Logs_20140328.`) |
| JSON error formatting | Yes | Same `{"__type": "...", "message": "..."}` format |
| SigV4 auth | Yes | `rustack-auth` is service-agnostic, service name = `logs` |
| Multi-account/region state | Yes | `rustack-core` unchanged |

### 5.3 Example Request/Response

Request:
```http
POST / HTTP/1.1
Content-Type: application/x-amz-json-1.1
X-Amz-Target: Logs_20140328.CreateLogGroup

{"logGroupName":"/test/my-app","tags":{"env":"dev"}}
```

Success response:
```http
HTTP/1.1 200 OK
Content-Type: application/x-amz-json-1.1

{}
```

Error response:
```http
HTTP/1.1 400 Bad Request
Content-Type: application/x-amz-json-1.1

{"__type":"ResourceAlreadyExistsException","message":"The specified log group already exists"}
```

### 5.4 No Legacy Compatibility Needed

CloudWatch Logs was introduced in 2014 with `awsJson1.1` from the start. There is no legacy protocol (no awsQuery, no XML responses) to support. All AWS SDKs use the JSON protocol.

---

## 6. Smithy Code Generation Strategy

### 6.1 Universal Codegen

The `rustack-logs-model` crate is generated from the official AWS Smithy JSON AST using the universal codegen tool at `codegen/`. The codegen reads a TOML service configuration and the Smithy model to produce all model types with correct serde attributes.

**Smithy model:** `codegen/smithy-model/logs.json` (780KB, namespace `com.amazonaws.cloudwatchlogs`, 43 operations)
**Service config:** `codegen/services/logs.toml`
**Generate:** `make codegen-logs`

### 6.2 Generated Output

The codegen produces 6 files in `crates/rustack-logs-model/src/`:

| File | Contents |
|------|----------|
| `lib.rs` | Module declarations and re-exports |
| `types.rs` | Shared types (enums and structs) with serde derives |
| `operations.rs` | `LogsOperation` enum with `as_str()`, `from_name()`, phase methods |
| `error.rs` | `LogsErrorCode` enum + `LogsError` struct + `logs_error!` macro |
| `input.rs` | All input structs with `#[serde(rename_all = "camelCase")]` |
| `output.rs` | All output structs with serde derives |

### 6.3 Service-Specific Notes

CloudWatch Logs uses `camelCase` JSON field naming, which the codegen handles via `serde_rename = "camelCase"` in the service TOML config. This differs from most other services which use `PascalCase`.

See [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md) for full codegen architecture details.

---

## 7. Crate Structure

### 7.1 `rustack-logs-model` (auto-generated)

```
crates/rustack-logs-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports
    +-- types.rs            # Auto-generated: enums + shared structs
    +-- operations.rs       # Auto-generated: LogsOperation enum
    +-- error.rs            # Auto-generated: LogsError + error codes
    +-- input.rs            # Auto-generated: all input structs
    +-- output.rs           # Auto-generated: all output structs
```

**Dependencies:** `serde`, `serde_json`

No hand-written types needed. CW Logs has no equivalent of DynamoDB's `AttributeValue` -- all types are straightforward structs and enums.

### 7.2 `rustack-logs-core`

```
crates/rustack-logs-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # LogsConfig
    +-- provider.rs         # RustackLogs (main provider, handler dispatch)
    +-- error.rs            # LogsServiceError
    +-- state.rs            # LogsState (top-level: DashMap of log groups)
    +-- group.rs            # LogGroupRecord (metadata, retention, tags, streams)
    +-- stream.rs           # LogStreamRecord (events, sequence token)
    +-- event.rs            # StoredLogEvent, event validation, timestamp checks
    +-- filter.rs           # FilterLogEvents pattern matching
    +-- retention.rs        # Retention policy enforcement (background timer)
    +-- metric_filter.rs    # MetricFilterRecord (stored, not evaluated)
    +-- subscription.rs     # SubscriptionFilterRecord (stored, not forwarded)
    +-- resource_policy.rs  # ResourcePolicyRecord
    +-- destination.rs      # DestinationRecord
    +-- query.rs            # Insights query stubs (StartQuery/GetQueryResults)
    +-- query_definition.rs # QueryDefinition CRUD
    +-- validation.rs       # Name validation, timestamp validation
    +-- pagination.rs       # Pagination token encoding/decoding
    +-- ops/
        +-- mod.rs
        +-- group.rs        # CreateLogGroup, DeleteLogGroup, DescribeLogGroups, ListLogGroups
        +-- stream.rs       # CreateLogStream, DeleteLogStream, DescribeLogStreams
        +-- events.rs       # PutLogEvents, GetLogEvents
        +-- filter_events.rs # FilterLogEvents
        +-- retention.rs    # PutRetentionPolicy, DeleteRetentionPolicy
        +-- metric_filter.rs # PutMetricFilter, DeleteMetricFilter, DescribeMetricFilters, TestMetricFilter
        +-- subscription.rs # PutSubscriptionFilter, DeleteSubscriptionFilter, DescribeSubscriptionFilters
        +-- resource_policy.rs # PutResourcePolicy, DeleteResourcePolicy, DescribeResourcePolicies
        +-- destination.rs  # PutDestination, PutDestinationPolicy, DeleteDestination, DescribeDestinations
        +-- tags.rs         # TagLogGroup, UntagLogGroup, ListTagsLogGroup, TagResource, UntagResource, ListTagsForResource
        +-- query.rs        # StartQuery, StopQuery, GetQueryResults, DescribeQueries
        +-- query_definition.rs # PutQueryDefinition, DeleteQueryDefinition, DescribeQueryDefinitions
        +-- kms.rs          # AssociateKmsKey, DisassociateKmsKey
```

**Dependencies:** `rustack-core`, `rustack-logs-model`, `dashmap`, `serde_json`, `chrono`, `tracing`, `tokio` (for retention timer), `uuid`

### 7.3 `rustack-logs-http`

```
crates/rustack-logs-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # Logs_20140328.* target dispatch
    +-- service.rs          # LogsHttpService (hyper Service impl)
    +-- dispatch.rs         # LogsHandler trait + operation dispatch
```

**Dependencies:** `rustack-logs-model`, `rustack-auth`, `hyper`, `serde_json`, `bytes`

This crate is structurally identical to `rustack-ssm-http`. The router parses `Logs_20140328.<Op>` instead of `AmazonSSM.<Op>`.

### 7.4 Workspace Changes

```toml
[workspace.dependencies]
rustack-logs-model = { path = "crates/rustack-logs-model" }
rustack-logs-http = { path = "crates/rustack-logs-http" }
rustack-logs-core = { path = "crates/rustack-logs-core" }
```

---

## 8. HTTP Layer Design

### 8.1 Router

```rust
/// CW Logs operation router.
///
/// Parses the `X-Amz-Target: Logs_20140328.<Op>` header to determine the operation.
pub struct LogsRouter;

impl LogsRouter {
    pub fn resolve(target: &str) -> Result<LogsOperation, LogsError> {
        let op_name = target
            .strip_prefix("Logs_20140328.")
            .ok_or_else(|| LogsError::unknown_operation(target))?;

        LogsOperation::from_name(op_name)
            .ok_or_else(|| LogsError::unknown_operation(op_name))
    }
}
```

### 8.2 ServiceRouter Trait Implementation

```rust
/// CW Logs service router for the gateway.
pub struct LogsServiceRouter<H: LogsHandler> {
    inner: LogsHttpService<H>,
}

impl<H: LogsHandler> ServiceRouter for LogsServiceRouter<H> {
    fn name(&self) -> &'static str {
        "logs"
    }

    fn matches(&self, req: &Request<()>) -> bool {
        req.headers()
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.starts_with("Logs_20140328."))
    }

    // ... call() delegates to inner.handle()
}
```

### 8.3 Handler Trait

```rust
/// The boundary between HTTP and business logic.
///
/// Analogous to SsmHandler but for CW Logs operations.
/// The handler is protocol-agnostic -- it receives typed inputs and returns typed outputs.
pub trait LogsHandler: Send + Sync + 'static {
    fn handle_operation(
        &self,
        op: LogsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<Bytes, LogsError>> + Send>>;
}
```

### 8.4 Unimplemented Operations

When CW Logs receives a target for an operation not in our supported set (e.g., `Logs_20140328.CreateExportTask`), we return a structured error:

```json
{
    "__type": "InvalidParameterException",
    "message": "Operation CreateExportTask is not supported."
}
```

This prevents confusing S3-format errors when an SDK sends an unsupported CW Logs operation.

---

## 9. Storage Engine Design

### 9.1 Overview

The storage engine implements an append-only log with time-ordered events per stream, organized into log groups. Unlike SQS (message lifecycle state machine) or DynamoDB (indexed tables), the CW Logs storage is conceptually simple: log groups contain log streams, and log streams contain time-ordered events.

### 9.2 Data Model Hierarchy

```
Account + Region
  +-- LogGroupRecord (keyed by log group name)
        +-- metadata (creation time, retention, KMS, tags)
        +-- LogStreamRecord (keyed by stream name within group)
        |     +-- metadata (creation time, sequence token)
        |     +-- Vec<StoredLogEvent> (time-ordered events)
        +-- MetricFilterRecord (keyed by filter name)
        +-- SubscriptionFilterRecord (keyed by filter name)
        +-- ResourcePolicyRecord (keyed by policy name, global scope)
        +-- DestinationRecord (keyed by destination name, global scope)
```

### 9.3 Core Data Structures

```rust
/// Top-level CW Logs state.
/// Keyed by (account_id, region) via rustack-core.
pub struct LogsState {
    /// All log groups keyed by name.
    groups: DashMap<String, LogGroupRecord>,
    /// Resource policies (global, not per-group). Max 10 per region.
    resource_policies: DashMap<String, ResourcePolicyRecord>,
    /// Destinations (global, not per-group).
    destinations: DashMap<String, DestinationRecord>,
    /// Query definitions (global).
    query_definitions: DashMap<String, QueryDefinitionRecord>,
    /// Active and completed queries.
    queries: DashMap<String, QueryRecord>,
    /// Global configuration.
    config: Arc<LogsConfig>,
}

/// A single log group with all its state.
pub struct LogGroupRecord {
    /// Log group name (e.g., "/aws/lambda/my-function").
    pub name: String,
    /// ARN: arn:aws:logs:{region}:{account}:log-group:{name}:*
    pub arn: String,
    /// Creation time (epoch millis).
    pub creation_time: i64,
    /// Retention in days (None = never expire). Valid values:
    /// 1, 3, 5, 7, 14, 30, 60, 90, 120, 150, 180, 365, 400, 545, 731,
    /// 1096, 1827, 2192, 2557, 2922, 3288, 3653.
    pub retention_in_days: Option<i32>,
    /// KMS key ID (stored but not used for encryption).
    pub kms_key_id: Option<String>,
    /// Log group class (STANDARD or INFREQUENT_ACCESS).
    pub log_group_class: LogGroupClass,
    /// Tags on the log group resource.
    pub tags: HashMap<String, String>,
    /// Log streams keyed by stream name.
    pub streams: HashMap<String, LogStreamRecord>,
    /// Metric filters keyed by filter name. Max 100 per group.
    pub metric_filters: HashMap<String, MetricFilterRecord>,
    /// Subscription filters keyed by filter name. Max 2 per group.
    pub subscription_filters: HashMap<String, SubscriptionFilterRecord>,
    /// Accumulated bytes stored (approximate).
    pub stored_bytes: i64,
}

/// A single log stream with its events.
pub struct LogStreamRecord {
    /// Log stream name.
    pub name: String,
    /// ARN: arn:aws:logs:{region}:{account}:log-group:{group}:log-stream:{stream}
    pub arn: String,
    /// Creation time (epoch millis).
    pub creation_time: i64,
    /// Timestamp of the first event in the stream.
    pub first_event_timestamp: Option<i64>,
    /// Timestamp of the last event in the stream.
    pub last_event_timestamp: Option<i64>,
    /// Ingestion time of the last PutLogEvents call.
    pub last_ingestion_time: Option<i64>,
    /// Sequence token for the next PutLogEvents call.
    /// Monotonically increasing counter encoded as zero-padded string.
    pub sequence_token: u64,
    /// Time-ordered log events.
    pub events: Vec<StoredLogEvent>,
    /// Approximate stored bytes.
    pub stored_bytes: i64,
}

/// A single stored log event.
#[derive(Debug, Clone)]
pub struct StoredLogEvent {
    /// Event timestamp (epoch millis, from the producer).
    pub timestamp: i64,
    /// Ingestion time (epoch millis, when Rustack received it).
    pub ingestion_time: i64,
    /// Log message content.
    pub message: String,
    /// Unique event ID (hex-encoded: 18 digits derived from timestamp + counter).
    pub event_id: String,
}

/// Metric filter record (stored, not evaluated).
pub struct MetricFilterRecord {
    pub filter_name: String,
    pub filter_pattern: String,
    pub metric_transformations: Vec<MetricTransformationRecord>,
    pub creation_time: i64,
    pub log_group_name: String,
}

/// Metric transformation record.
pub struct MetricTransformationRecord {
    pub metric_namespace: String,
    pub metric_name: String,
    pub metric_value: String,
    pub default_value: Option<f64>,
    pub dimensions: HashMap<String, String>,
    pub unit: Option<String>,
}

/// Subscription filter record (stored, not forwarded).
pub struct SubscriptionFilterRecord {
    pub filter_name: String,
    pub log_group_name: String,
    pub filter_pattern: String,
    pub destination_arn: String,
    pub role_arn: Option<String>,
    pub distribution: Option<String>,
    pub creation_time: i64,
}

/// Resource policy record.
pub struct ResourcePolicyRecord {
    pub policy_name: String,
    pub policy_document: String,
    pub last_updated_time: i64,
}

/// Destination record (cross-account log delivery).
pub struct DestinationRecord {
    pub destination_name: String,
    pub target_arn: String,
    pub role_arn: String,
    pub access_policy: Option<String>,
    pub arn: String,
    pub creation_time: i64,
    pub tags: HashMap<String, String>,
}

/// Query definition record.
pub struct QueryDefinitionRecord {
    pub query_definition_id: String,
    pub name: String,
    pub query_string: String,
    pub log_group_names: Vec<String>,
    pub last_modified: i64,
}

/// Active query record (stub implementation).
pub struct QueryRecord {
    pub query_id: String,
    pub query_string: String,
    pub log_group_name: Option<String>,
    pub log_group_names: Vec<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub create_time: i64,
    pub status: QueryStatus,
}

#[derive(Debug, Clone, Copy)]
pub enum QueryStatus {
    Scheduled,
    Running,
    Complete,
    Failed,
    Cancelled,
    Timeout,
    Unknown,
}
```

### 9.4 Sequence Token Management

AWS CW Logs historically required a sequence token for `PutLogEvents` to enforce ordering. Modern behavior (since late 2022) no longer requires the token -- the API accepts `PutLogEvents` without a sequence token and ignores invalid tokens. Our implementation follows the modern behavior:

```rust
impl LogStreamRecord {
    /// Accept log events. Sequence token is accepted but not enforced.
    /// Returns the next sequence token for backward compatibility.
    fn put_events(
        &mut self,
        events: Vec<InputLogEvent>,
    ) -> Result<PutLogEventsResult, LogsServiceError> {
        let now = current_time_millis();
        let mut rejected_info = RejectedLogEventsInfo::default();
        let mut accepted = Vec::new();

        for (idx, event) in events.iter().enumerate() {
            // Reject events too far in the past (>14 days)
            if event.timestamp < now - 14 * 24 * 60 * 60 * 1000 {
                rejected_info.too_old_log_event_end_index =
                    Some(idx as i32);
                continue;
            }
            // Reject events too far in the future (>2 hours)
            if event.timestamp > now + 2 * 60 * 60 * 1000 {
                if rejected_info.too_new_log_event_start_index.is_none() {
                    rejected_info.too_new_log_event_start_index =
                        Some(idx as i32);
                }
                continue;
            }
            accepted.push(StoredLogEvent {
                timestamp: event.timestamp,
                ingestion_time: now,
                message: event.message.clone(),
                event_id: generate_event_id(event.timestamp, &mut self.event_counter),
            });
        }

        // Events must be in chronological order within a batch
        // (already validated by the caller, or we sort here)
        // Insert into the stream's event list maintaining time order
        self.events.extend(accepted);
        self.events.sort_by_key(|e| e.timestamp);

        // Update stream metadata
        if let Some(first) = self.events.first() {
            self.first_event_timestamp = Some(first.timestamp);
        }
        if let Some(last) = self.events.last() {
            self.last_event_timestamp = Some(last.timestamp);
        }
        self.last_ingestion_time = Some(now);
        self.sequence_token += 1;

        Ok(PutLogEventsResult {
            next_sequence_token: format!("{:056}", self.sequence_token),
            rejected_info: if rejected_info.has_rejections() {
                Some(rejected_info)
            } else {
                None
            },
        })
    }
}
```

### 9.5 Event ID Generation

AWS CW Logs generates event IDs as hex-encoded strings. We generate deterministic IDs based on timestamp and a per-stream counter:

```rust
/// Generate a unique event ID.
/// Format: 18-digit zero-padded counter (matching AWS format).
fn generate_event_id(timestamp: i64, counter: &mut u64) -> String {
    *counter += 1;
    format!("{:018}", *counter)
}
```

### 9.6 Memory Management

Log events accumulate in memory. We provide configurable limits to prevent unbounded growth:

```rust
pub struct LogsConfig {
    /// Maximum events per stream before oldest are evicted.
    /// Default: 100_000. Set to 0 for unlimited.
    pub max_events_per_stream: usize,
    /// Maximum total events across all streams.
    /// Default: 1_000_000. Set to 0 for unlimited.
    pub max_total_events: usize,
    /// Retention cleanup interval in seconds.
    /// Default: 60.
    pub retention_cleanup_interval_secs: u64,
    // ... other config fields
}
```

When `max_events_per_stream` is reached, the oldest events are evicted (FIFO eviction). This is independent of retention policy -- retention is time-based, this limit is count-based.

### 9.7 Retention Policy Enforcement

A background task periodically scans log groups with retention policies and evicts expired events:

```rust
/// Start the retention cleanup background task.
pub fn start_retention_cleanup(
    state: Arc<LogsState>,
    interval: Duration,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            cleanup_expired_events(&state);
        }
    })
}

fn cleanup_expired_events(state: &LogsState) {
    let now = current_time_millis();
    for mut group_entry in state.groups.iter_mut() {
        let group = group_entry.value_mut();
        if let Some(retention_days) = group.retention_in_days {
            let cutoff = now - (retention_days as i64) * 24 * 60 * 60 * 1000;
            for stream in group.streams.values_mut() {
                stream.events.retain(|e| e.timestamp >= cutoff);
            }
        }
    }
}
```

### 9.8 Concurrency Model

CW Logs has no real-time constraints like SQS long polling. A `DashMap` provides sufficient concurrent access:

- **Reads** (Describe, Get, Filter, List): lock-free concurrent reads via DashMap
- **Writes** (Create, Delete, Put): per-entry write locks via DashMap
- **Retention cleanup**: periodic background task that acquires write locks briefly

No actors needed. No channels needed. This is straightforward request/response processing with one background timer for retention.

### 9.9 GetLogEvents Pagination

`GetLogEvents` supports forward and backward pagination using opaque tokens. Our tokens encode the current position:

```rust
/// Pagination token for GetLogEvents.
/// Encoded as: "f/<offset>" for forward, "b/<offset>" for backward.
fn encode_log_events_token(direction: Direction, offset: usize) -> String {
    match direction {
        Direction::Forward => format!("f/{offset:012}"),
        Direction::Backward => format!("b/{offset:012}"),
    }
}

fn decode_log_events_token(token: &str) -> Result<(Direction, usize), LogsServiceError> {
    let (dir_str, offset_str) = token
        .split_once('/')
        .ok_or(LogsServiceError::InvalidParameterException {
            message: "The specified nextToken is invalid.".into(),
        })?;
    let direction = match dir_str {
        "f" => Direction::Forward,
        "b" => Direction::Backward,
        _ => return Err(LogsServiceError::InvalidParameterException {
            message: "The specified nextToken is invalid.".into(),
        }),
    };
    let offset = offset_str.parse::<usize>().map_err(|_| {
        LogsServiceError::InvalidParameterException {
            message: "The specified nextToken is invalid.".into(),
        }
    })?;
    Ok((direction, offset))
}
```

### 9.10 FilterLogEvents Pattern Matching

Simple text pattern matching for `FilterLogEvents`:

```rust
/// Evaluate a filter pattern against a log message.
///
/// Pattern syntax (Phase 1):
/// - Empty or whitespace-only: match all
/// - Space-separated terms: ALL terms must appear (AND logic)
/// - Quoted phrase: exact substring match
///
/// Phase 2 (future):
/// - JSON patterns: `{$.field = value}`
pub fn matches_filter_pattern(pattern: &str, message: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return true;
    }

    // Parse quoted phrases and bare terms
    let terms = parse_filter_terms(pattern);

    // ALL terms must match (AND logic)
    terms.iter().all(|term| match term {
        FilterTerm::Exact(phrase) => message.contains(phrase.as_str()),
        FilterTerm::Word(word) => message.contains(word.as_str()),
    })
}

enum FilterTerm {
    /// Quoted phrase: must appear as exact substring.
    Exact(String),
    /// Bare word: must appear as substring (case-sensitive).
    Word(String),
}

fn parse_filter_terms(pattern: &str) -> Vec<FilterTerm> {
    let mut terms = Vec::new();
    let mut chars = pattern.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c == '"' {
            chars.next(); // consume opening quote
            let phrase: String = chars.by_ref().take_while(|&ch| ch != '"').collect();
            if !phrase.is_empty() {
                terms.push(FilterTerm::Exact(phrase));
            }
        } else if c.is_whitespace() {
            chars.next();
        } else {
            let word: String = chars
                .by_ref()
                .take_while(|&ch| !ch.is_whitespace() && ch != '"')
                .collect();
            if !word.is_empty() {
                terms.push(FilterTerm::Word(word));
            }
        }
    }

    terms
}
```

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main CW Logs provider implementing all operations.
pub struct RustackLogs {
    pub(crate) state: Arc<LogsState>,
    pub(crate) config: Arc<LogsConfig>,
}

impl RustackLogs {
    pub fn new(config: LogsConfig) -> Self {
        let state = Arc::new(LogsState::new(Arc::new(config.clone())));
        Self {
            state,
            config: Arc::new(config),
        }
    }

    /// Start background tasks (retention cleanup).
    pub fn start(&self, shutdown: Arc<AtomicBool>) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();
        handles.push(start_retention_cleanup(
            Arc::clone(&self.state),
            Duration::from_secs(self.config.retention_cleanup_interval_secs),
            shutdown,
        ));
        handles
    }
}
```

### 10.2 Operations

#### Phase 0: Log Group/Stream CRUD + Event Ingestion/Retrieval (10 operations)

**CreateLogGroup** -- Create a log group.

1. Validate log group name (1-512 chars, pattern `[\.\-_/#A-Za-z0-9]+`)
2. If group exists, return `ResourceAlreadyExistsException`
3. Create `LogGroupRecord` with creation time, optional KMS key, optional tags
4. Enforce limit: 1,000,000 log groups per account/region (practical limit for local dev: configurable)
5. Return `{}`

**DeleteLogGroup** -- Delete a log group and all its streams/events.

1. If group does not exist, return `ResourceNotFoundException`
2. Remove group and all contained streams, events, metric filters, subscription filters
3. Return `{}`

**DescribeLogGroups** -- List log groups with optional prefix or pattern filter.

1. If both `logGroupNamePrefix` and `logGroupNamePattern` are specified, return `InvalidParameterException`
2. Apply prefix filter (starts-with) or pattern filter (contains)
3. Sort by log group name (lexicographic)
4. Paginate with `limit` (1-50, default 50) and `nextToken`
5. Return `{ logGroups, nextToken }`

**ListLogGroups** -- Newer API for listing log groups (same semantics as DescribeLogGroups).

1. Same logic as `DescribeLogGroups` with slightly different response shape
2. Return `{ logGroups, nextToken }`

**CreateLogStream** -- Create a log stream within a group.

1. Validate log group exists; return `ResourceNotFoundException` if not
2. Validate stream name (1-512 chars, pattern `[^:*]*`)
3. If stream exists in the group, return `ResourceAlreadyExistsException`
4. Create `LogStreamRecord` with creation time and initial sequence token
5. Return `{}`

**DeleteLogStream** -- Delete a log stream and its events.

1. Validate log group exists; return `ResourceNotFoundException` if not
2. Validate stream exists; return `ResourceNotFoundException` if not
3. Remove stream and all its events
4. Return `{}`

**DescribeLogStreams** -- List streams in a group.

1. Validate log group or log group identifier; return `ResourceNotFoundException` if not found
2. If both `logGroupName` and `logGroupIdentifier` specified, return `InvalidParameterException`
3. Apply `logStreamNamePrefix` filter if present
4. Sort by `orderBy` (`LogStreamName` or `LastEventTime`, default `LogStreamName`)
5. Apply `descending` flag
6. Paginate with `limit` (1-50, default 50) and `nextToken`
7. Return `{ logStreams, nextToken }`

**PutLogEvents** -- Ingest log events into a stream.

1. Validate log group exists; return `ResourceNotFoundException` if not
2. Validate log stream exists; return `ResourceNotFoundException` if not
3. Validate events are in chronological order within the batch; return `InvalidParameterException` if not
4. Validate individual event sizes (max 262,144 bytes per event including overhead)
5. Validate batch size (max 10,000 events, max 1,048,576 bytes total)
6. Identify events with timestamps too old (>14 days) or too new (>2 hours ahead)
7. Accept valid events, store with ingestion timestamp
8. Sequence token is accepted but not enforced (modern behavior)
9. Return `{ nextSequenceToken, rejectedLogEventsInfo }`

**GetLogEvents** -- Retrieve events from a single stream.

1. Validate log group and stream exist
2. Support `logGroupIdentifier` (name or ARN)
3. Apply `startTime` and `endTime` filters (epoch millis, inclusive)
4. Apply `startFromHead` (default false: return newest events)
5. Paginate with `limit` (1-10,000, default 10,000) and `nextToken`
6. Return `{ events, nextForwardToken, nextBackwardToken }`

**FilterLogEvents** -- Search events across streams in a group.

1. Validate log group exists
2. Apply `logStreamNames` filter (specific streams) or `logStreamNamePrefix` (prefix match)
3. Apply `startTime` and `endTime` filters
4. Apply `filterPattern` (text pattern matching; empty = match all)
5. Merge and sort events from all matching streams by timestamp
6. Apply `interleaved` flag (deprecated but still accepted)
7. Paginate with `limit` (1-10,000, default 10,000) and `nextToken`
8. Return `{ events, searchedLogStreams, nextToken }`

#### Phase 1: Retention + Tags + Resource Policies (10 operations)

**PutRetentionPolicy** -- Set retention on a log group.

1. Validate log group exists
2. Validate `retentionInDays` is a valid value (1, 3, 5, 7, 14, 30, 60, 90, 120, 150, 180, 365, 400, 545, 731, 1096, 1827, 2192, 2557, 2922, 3288, 3653)
3. Update `retention_in_days` on the log group record
4. Return `{}`

**DeleteRetentionPolicy** -- Remove retention from a log group.

1. Validate log group exists
2. Set `retention_in_days` to `None`
3. Return `{}`

**TagLogGroup** -- Add tags to a log group (legacy API).

1. Validate log group exists
2. Merge new tags into existing tags
3. Return `{}`

**UntagLogGroup** -- Remove tags from a log group (legacy API).

1. Validate log group exists
2. Remove specified tag keys
3. Return `{}`

**ListTagsLogGroup** -- List tags on a log group (legacy API).

1. Validate log group exists
2. Return `{ tags }`

**TagResource** -- Add tags to a resource (new API, supports log groups and destinations).

1. Parse resource ARN to determine resource type and name
2. Validate resource exists
3. Merge new tags
4. Return `{}`

**UntagResource** -- Remove tags from a resource (new API).

1. Parse resource ARN
2. Validate resource exists
3. Remove specified tag keys
4. Return `{}`

**ListTagsForResource** -- List tags on a resource (new API).

1. Parse resource ARN
2. Validate resource exists
3. Return `{ tags }`

**PutResourcePolicy** -- Create or update a resource policy.

1. Validate policy name (1-256 chars)
2. Validate policy document (1-5120 chars, valid JSON)
3. If policy exists, update it; otherwise create (enforce max 10 policies per region)
4. Return `{ resourcePolicy }`

**DeleteResourcePolicy** -- Delete a resource policy.

1. Validate policy exists; return `ResourceNotFoundException` if not
2. Remove policy
3. Return `{}`

**DescribeResourcePolicies** -- List resource policies.

1. Paginate with `limit` and `nextToken`
2. Return `{ resourcePolicies, nextToken }`

#### Phase 2: Metric Filters + Subscription Filters (7 operations)

**PutMetricFilter** -- Create or update a metric filter on a log group.

1. Validate log group exists
2. Validate filter name (1-512 chars)
3. Validate filter pattern (1-1024 chars)
4. Validate metric transformations (exactly 1 transformation)
5. If filter exists (same name), update it; otherwise create (max 100 per group)
6. Return `{}`

**DeleteMetricFilter** -- Delete a metric filter.

1. Validate log group exists
2. Validate filter exists; return `ResourceNotFoundException` if not
3. Remove filter
4. Return `{}`

**DescribeMetricFilters** -- List metric filters.

1. Support filtering by `logGroupName`, `filterNamePrefix`, or `metricName`+`metricNamespace`
2. Paginate with `limit` and `nextToken`
3. Return `{ metricFilters, nextToken }`

**TestMetricFilter** -- Test a filter pattern against log event messages.

1. Apply filter pattern to provided log event messages
2. Return `{ matches }` -- list of matched events with extracted values

**PutSubscriptionFilter** -- Create or update a subscription filter.

1. Validate log group exists
2. Validate filter name
3. Validate destination ARN format (Lambda, Kinesis, or Firehose ARN)
4. If filter exists (same name), update it; otherwise create (max 2 per group)
5. Store filter metadata (do not actually set up forwarding)
6. Return `{}`

**DeleteSubscriptionFilter** -- Delete a subscription filter.

1. Validate log group exists
2. Validate filter exists; return `ResourceNotFoundException` if not
3. Remove filter
4. Return `{}`

**DescribeSubscriptionFilters** -- List subscription filters.

1. Validate log group exists
2. Apply `filterNamePrefix` filter
3. Paginate with `limit` and `nextToken`
4. Return `{ subscriptionFilters, nextToken }`

#### Phase 3: Destinations + Insights Queries + KMS (12 operations)

**PutDestination** -- Create or update a cross-account destination.

1. Validate destination name
2. Create or update destination record with target ARN, role ARN, tags
3. Return `{ destination }`

**PutDestinationPolicy** -- Set access policy on a destination.

1. Validate destination exists
2. Store access policy document
3. Return `{}`

**DeleteDestination** -- Delete a destination.

1. Validate destination exists
2. Remove destination
3. Return `{}`

**DescribeDestinations** -- List destinations.

1. Apply `DestinationNamePrefix` filter
2. Paginate with `limit` and `nextToken`
3. Return `{ destinations, nextToken }`

**StartQuery** -- Start a Logs Insights query (stub).

1. Validate log group exists
2. Generate query ID (UUID)
3. Store query record with status `Complete` (immediate completion for stub)
4. Return `{ queryId }`

**StopQuery** -- Stop a running query (stub).

1. Validate query exists
2. Set status to `Cancelled` (if not already complete)
3. Return `{ success: true }`

**GetQueryResults** -- Get query results (stub).

1. Validate query exists
2. Return `{ status: "Complete", results: [], statistics: { ... } }`

**DescribeQueries** -- List queries.

1. Filter by `logGroupName` and `status` if provided
2. Return `{ queries }`

**PutQueryDefinition** / **DeleteQueryDefinition** / **DescribeQueryDefinitions** -- CRUD for saved query definitions.

Standard CRUD operations for query definition records.

**AssociateKmsKey** / **DisassociateKmsKey** -- KMS key association.

1. Validate log group exists
2. Store/remove KMS key ID on the log group record
3. Return `{}`

### 10.3 ARN Construction

```rust
fn log_group_arn(region: &str, account_id: &str, name: &str) -> String {
    format!("arn:aws:logs:{region}:{account_id}:log-group:{name}:*")
}

fn log_stream_arn(
    region: &str,
    account_id: &str,
    group_name: &str,
    stream_name: &str,
) -> String {
    format!(
        "arn:aws:logs:{region}:{account_id}:log-group:{group_name}:log-stream:{stream_name}"
    )
}

fn destination_arn(region: &str, account_id: &str, destination_name: &str) -> String {
    format!("arn:aws:logs:{region}:{account_id}:destination:{destination_name}")
}
```

### 10.4 Validation Rules

| Field | Rule |
|-------|------|
| Log group name | 1-512 chars, pattern `[\.\-_/#A-Za-z0-9]+` |
| Log stream name | 1-512 chars, pattern `[^:*]*` (no `:` or `*`) |
| Log event message | Max 262,144 bytes (256 KiB, including 26 bytes overhead per event) |
| Log event timestamp | Within 14 days in past to 2 hours in future |
| PutLogEvents batch | Max 10,000 events, max 1,048,576 bytes total |
| PutLogEvents ordering | Events must be in chronological order within batch |
| Retention days | One of: 1, 3, 5, 7, 14, 30, 60, 90, 120, 150, 180, 365, 400, 545, 731, 1096, 1827, 2192, 2557, 2922, 3288, 3653 |
| Metric filter name | 1-512 chars, pattern `[^:*]*` |
| Filter pattern | Max 1024 chars |
| Metric transformations | Exactly 1 per metric filter |
| Subscription filters per group | Max 2 |
| Metric filters per group | Max 100 |
| Resource policies per region | Max 10 |
| Policy document | Max 5120 chars |
| Tags per resource | Max 50 |
| Tag key | 1-128 chars |
| Tag value | 0-256 chars |

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// CW Logs service errors mapped to API error types.
pub enum LogsServiceError {
    /// Log group or stream does not exist.
    ResourceNotFoundException { message: String },
    /// Log group or stream already exists.
    ResourceAlreadyExistsException { message: String },
    /// Invalid parameter value.
    InvalidParameterException { message: String },
    /// Operation parameter combination is invalid.
    InvalidOperationException { message: String },
    /// Service quota exceeded (log groups, metric filters, subscription filters, etc.).
    LimitExceededException { message: String },
    /// Resource policy limit exceeded (max 10).
    ServiceUnavailableException { message: String },
    /// The sequence token is not valid (deprecated, accepted but not enforced).
    InvalidSequenceTokenException {
        message: String,
        expected_sequence_token: String,
    },
    /// Data already accepted (idempotent PutLogEvents).
    DataAlreadyAcceptedException {
        message: String,
        expected_sequence_token: String,
    },
    /// Missing required parameter.
    MissingParameterException { message: String },
    /// Unrecognized client exception.
    UnrecognizedClientException { message: String },
    /// Operation not supported.
    OperationNotSupportedException { message: String },
    /// Internal server error.
    InternalServerError { message: String },
}
```

### 11.2 Error Mapping

```rust
impl LogsServiceError {
    /// Map to HTTP status code and __type string.
    pub fn to_error_response(&self) -> (u16, &'static str, String) {
        match self {
            Self::ResourceNotFoundException { message } =>
                (400, "ResourceNotFoundException", message.clone()),
            Self::ResourceAlreadyExistsException { message } =>
                (400, "ResourceAlreadyExistsException", message.clone()),
            Self::InvalidParameterException { message } =>
                (400, "InvalidParameterException", message.clone()),
            Self::InvalidOperationException { message } =>
                (400, "InvalidOperationException", message.clone()),
            Self::LimitExceededException { message } =>
                (400, "LimitExceededException", message.clone()),
            Self::InvalidSequenceTokenException { message, .. } =>
                (400, "InvalidSequenceTokenException", message.clone()),
            Self::DataAlreadyAcceptedException { message, .. } =>
                (400, "DataAlreadyAcceptedException", message.clone()),
            Self::MissingParameterException { message } =>
                (400, "MissingParameterException", message.clone()),
            Self::UnrecognizedClientException { message } =>
                (400, "UnrecognizedClientException", message.clone()),
            Self::OperationNotSupportedException { message } =>
                (400, "OperationNotSupportedException", message.clone()),
            Self::ServiceUnavailableException { message } =>
                (503, "ServiceUnavailableException", message.clone()),
            Self::InternalServerError { message } =>
                (500, "InternalServerError", message.clone()),
        }
    }
}
```

### 11.3 Error Response Format

```json
{
    "__type": "ResourceNotFoundException",
    "message": "The specified log group does not exist."
}
```

CW Logs uses short error type names (no namespace prefix), same as SSM.

---

## 12. Server Integration

### 12.1 Feature Gate

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "logs"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
sqs = ["dep:rustack-sqs-core", "dep:rustack-sqs-http"]
ssm = ["dep:rustack-ssm-core", "dep:rustack-ssm-http"]
logs = ["dep:rustack-logs-core", "dep:rustack-logs-http"]
```

### 12.2 Gateway Registration

CW Logs is registered in the gateway before S3 (S3 is the catch-all):

```rust
// In gateway setup
let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

#[cfg(feature = "dynamodb")]
services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_service)));

#[cfg(feature = "sqs")]
services.push(Box::new(SqsServiceRouter::new(sqs_service)));

#[cfg(feature = "ssm")]
services.push(Box::new(SsmServiceRouter::new(ssm_service)));

#[cfg(feature = "logs")]
services.push(Box::new(LogsServiceRouter::new(logs_service)));

// S3 is always last (catch-all)
#[cfg(feature = "s3")]
services.push(Box::new(S3ServiceRouter::new(s3_service)));
```

### 12.3 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "available",
        "dynamodb": "available",
        "sqs": "available",
        "ssm": "available",
        "logs": "available"
    },
    "version": "0.3.0"
}
```

### 12.4 Configuration

```rust
pub struct LogsConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub default_account_id: String,
    /// Maximum events per log stream (default: 100,000).
    pub max_events_per_stream: usize,
    /// Maximum total events across all streams (default: 1,000,000).
    pub max_total_events: usize,
    /// Retention cleanup interval in seconds (default: 60).
    pub retention_cleanup_interval_secs: u64,
}

impl LogsConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("LOGS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            default_account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            max_events_per_stream: env_usize("LOGS_MAX_EVENTS_PER_STREAM", 100_000),
            max_total_events: env_usize("LOGS_MAX_TOTAL_EVENTS", 1_000_000),
            retention_cleanup_interval_secs: env_u64("LOGS_RETENTION_CLEANUP_INTERVAL", 60),
        }
    }
}
```

### 12.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared) |
| `LOGS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 for CW Logs |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default account for ARNs |
| `LOGS_MAX_EVENTS_PER_STREAM` | `100000` | Max events per stream before eviction |
| `LOGS_MAX_TOTAL_EVENTS` | `1000000` | Max total events |
| `LOGS_RETENTION_CLEANUP_INTERVAL` | `60` | Retention cleanup interval (seconds) |

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **Name validation**: log group names, log stream names, filter names, policy names
- **Event validation**: timestamp range checks, batch size limits, ordering
- **Filter pattern matching**: empty patterns, single words, multiple words, quoted phrases, edge cases
- **Pagination token encoding/decoding**: forward/backward tokens, invalid tokens
- **Retention enforcement**: eviction of expired events, no-retention groups unaffected
- **Memory limits**: stream event cap, total event cap, FIFO eviction
- **ARN construction**: log group ARNs, log stream ARNs, destination ARNs

### 13.2 Integration Tests with aws-sdk-cloudwatchlogs

```rust
// tests/integration/logs_tests.rs
#[tokio::test]
#[ignore]
async fn test_logs_create_delete_log_group() {
    let client = aws_sdk_cloudwatchlogs::Client::new(&config);
    // CreateLogGroup, DescribeLogGroups, DeleteLogGroup round-trip
}

#[tokio::test]
#[ignore]
async fn test_logs_put_get_events() {
    let client = aws_sdk_cloudwatchlogs::Client::new(&config);
    // CreateLogGroup, CreateLogStream, PutLogEvents, GetLogEvents
}

#[tokio::test]
#[ignore]
async fn test_logs_filter_events_basic() {
    let client = aws_sdk_cloudwatchlogs::Client::new(&config);
    // PutLogEvents, FilterLogEvents with pattern matching
}

#[tokio::test]
#[ignore]
async fn test_logs_retention_policy() {
    let client = aws_sdk_cloudwatchlogs::Client::new(&config);
    // PutRetentionPolicy, verify in DescribeLogGroups, DeleteRetentionPolicy
}
```

### 13.3 LocalStack Test Suite (Primary Compatibility Reference)

The vendored LocalStack tests (`vendors/localstack/tests/aws/services/logs/`) contain comprehensive test coverage organized by feature area:

| Test File | Test Count | Coverage Area | Phase |
|-----------|-----------|---------------|-------|
| `test_logs_groups.py` | ~12 | Create, delete, describe, prefix/pattern filter, retention, tags | 0, 1 |
| `test_logs_streams.py` | ~5 | Create, delete, describe, prefix filter, ARN format, logGroupIdentifier | 0 |
| `test_logs_events.py` | ~12 | PutLogEvents (basic, unicode, errors, ordering, too old/new), GetLogEvents (basic, pagination, limit, logGroupIdentifier) | 0 |
| `test_logs_filter_events.py` | ~10 | FilterLogEvents (basic, stream filter, interleaved, pagination, limit, patterns) | 0, 1 |
| `test_logs_metric_filters.py` | ~12 | PutMetricFilter, DeleteMetricFilter, DescribeMetricFilters (by prefix, log group, metric name), validation | 2 |
| `test_logs_subscription_filters.py` | ~10 | PutSubscriptionFilter (Lambda, Kinesis, Firehose), describe, update, delete, limit exceeded, errors | 2 |
| `test_logs_resource_policies.py` | ~8 | PutResourcePolicy, DescribeResourcePolicies, DeleteResourcePolicy, update, limit exceeded | 1 |
| `test_logs_destinations.py` | ~7 | PutDestination, DescribeDestinations, DeleteDestination, PutDestinationPolicy, tags | 3 |
| `test_logs_queries.py` | ~8 | StartQuery, DescribeQueries, GetQueryResults, status filters, query strings | 3 |

**Total: ~84 tests** covering the full API surface we plan to implement.

These tests use boto3 and can be run directly against Rustack by pointing `AWS_ENDPOINT_URL` at `http://localhost:4566`.

### 13.4 moto Test Suite

The moto project (`getmoto/moto`, `tests/test_logs/`) contains additional CloudWatch Logs tests. Key test files:

- `test_logs.py` -- core log group/stream/event operations
- `test_logs_filter.py` -- FilterLogEvents with various patterns
- `test_logs_query.py` -- Insights query operations

These serve as supplementary reference for expected behavior and edge cases.

### 13.5 Third-Party Test Suites and Tools

| Tool | Type | URL | Notes |
|------|------|-----|-------|
| **fluent-bit** | Log shipper | https://github.com/aws/amazon-cloudwatch-logs-for-fluent-bit | CW Logs output plugin; tests create log groups/streams and put events. Can be pointed at custom endpoint. |
| **Vector.dev** | Log pipeline | https://vector.dev/docs/reference/configuration/sinks/aws_cloudwatch_logs/ | `aws_cloudwatch_logs` sink supports `endpoint` parameter for custom endpoints. Tests create groups/streams and put events. |
| **fake-cloudwatch-logs** | Node.js mock | https://github.com/Raynos/fake-cloudwatch-logs | Minimal CW Logs mock; useful as behavioral reference. |
| **Mockoon** | Mock generator | https://mockoon.com/mock-samples/amazonawscom-logs/ | Static mock samples for CW Logs API. |
| **CloudWatch Agent** | Log agent | https://github.com/aws/amazon-cloudwatch-agent | Can be configured to use custom endpoint; tests the PutLogEvents path. |

### 13.6 fluent-bit End-to-End Validation

fluent-bit is the most important log shipper to validate against. It uses `CreateLogGroup`, `CreateLogStream`, `PutLogEvents`, and `DescribeLogStreams`:

```makefile
test-logs-fluentbit:
	@echo "Starting Rustack..."
	@./target/release/rustack &
	@sleep 1
	@echo '{"log": "test message from fluent-bit"}' | fluent-bit \
	    -i stdin \
	    -o cloudwatch_logs \
	    -p "region=us-east-1" \
	    -p "log_group_name=/test/fluent-bit" \
	    -p "log_stream_name=test-stream" \
	    -p "auto_create_group=true" \
	    -p "endpoint=http://localhost:4566" \
	    -p "tls=off"
	@aws logs get-log-events \
	    --log-group-name /test/fluent-bit \
	    --log-stream-name test-stream \
	    --endpoint-url http://localhost:4566
```

### 13.7 AWS CLI Smoke Tests

```bash
# Create log group
aws logs create-log-group --log-group-name /test/my-app \
    --endpoint-url http://localhost:4566

# Create log stream
aws logs create-log-stream --log-group-name /test/my-app \
    --log-stream-name stream-1 \
    --endpoint-url http://localhost:4566

# Put log events
aws logs put-log-events --log-group-name /test/my-app \
    --log-stream-name stream-1 \
    --log-events '[{"timestamp":'$(date +%s000)',"message":"hello world"}]' \
    --endpoint-url http://localhost:4566

# Get log events
aws logs get-log-events --log-group-name /test/my-app \
    --log-stream-name stream-1 \
    --endpoint-url http://localhost:4566

# Filter log events
aws logs filter-log-events --log-group-name /test/my-app \
    --filter-pattern "hello" \
    --endpoint-url http://localhost:4566

# Describe log groups
aws logs describe-log-groups --log-group-name-prefix /test/ \
    --endpoint-url http://localhost:4566

# Set retention policy
aws logs put-retention-policy --log-group-name /test/my-app \
    --retention-in-days 7 \
    --endpoint-url http://localhost:4566

# Delete log group
aws logs delete-log-group --log-group-name /test/my-app \
    --endpoint-url http://localhost:4566
```

### 13.8 Docker awslogs Driver Test

```bash
# Start Rustack
docker run -d -p 4566:4566 tyrchen/rustack

# Run a container with awslogs driver pointed at Rustack
docker run --rm \
    --log-driver=awslogs \
    --log-opt awslogs-region=us-east-1 \
    --log-opt awslogs-group=/docker/test \
    --log-opt awslogs-stream=container-1 \
    --log-opt awslogs-endpoint=http://host.docker.internal:4566 \
    --log-opt awslogs-credentials-endpoint=/dummy \
    alpine echo "hello from container"

# Verify logs arrived
aws logs get-log-events \
    --log-group-name /docker/test \
    --log-stream-name container-1 \
    --endpoint-url http://localhost:4566
```

---

## 14. Phased Implementation Plan

### Phase 0: MVP (10 Operations)

**Goal:** Core log group/stream CRUD + event ingestion/retrieval. Covers fluent-bit, Vector, CloudWatch Agent, Docker awslogs driver, and basic AWS CLI/SDK usage.
**Estimated effort:** 3-4 days.

#### Step 0.1: Codegen
- Add CW Logs service config to codegen
- Download CW Logs Smithy model from `aws/api-models-aws`
- Generate `rustack-logs-model` crate
- Verify generated types compile and serde round-trip

#### Step 0.2: HTTP Layer
- Implement `LogsRouter` (`Logs_20140328.*` dispatch)
- Implement `LogsHttpService` (reuse SSM's JSON protocol pattern)
- Implement `LogsServiceRouter` for gateway integration
- Wire into gateway with feature gate

#### Step 0.3: Storage Engine
- Implement `LogsState` with `DashMap<String, LogGroupRecord>`
- Implement `LogGroupRecord` with `HashMap<String, LogStreamRecord>`
- Implement `LogStreamRecord` with `Vec<StoredLogEvent>`
- Implement event validation (timestamps, batch size)
- Implement sequence token generation
- Implement `GetLogEvents` pagination (forward/backward tokens)
- Implement `FilterLogEvents` without pattern matching (empty pattern = match all)
- Implement name validation

#### Step 0.4: Core Operations
- `CreateLogGroup`, `DeleteLogGroup`, `DescribeLogGroups`, `ListLogGroups`
- `CreateLogStream`, `DeleteLogStream`, `DescribeLogStreams`
- `PutLogEvents`, `GetLogEvents`, `FilterLogEvents`

#### Step 0.5: Testing
- Unit tests for storage, validation, pagination
- Integration tests with `aws-sdk-cloudwatchlogs`
- AWS CLI smoke tests
- fluent-bit end-to-end test

### Phase 1: Retention + Tags + Resource Policies + Filter Patterns (11 operations)

**Goal:** Full Terraform lifecycle, tag management, resource policies, text filter patterns.
**Estimated effort:** 2-3 days.

- `PutRetentionPolicy`, `DeleteRetentionPolicy`
- `TagLogGroup`, `UntagLogGroup`, `ListTagsLogGroup`
- `TagResource`, `UntagResource`, `ListTagsForResource`
- `PutResourcePolicy`, `DeleteResourcePolicy`, `DescribeResourcePolicies`
- Implement text filter pattern matching for `FilterLogEvents`
- Start retention cleanup background task
- Port LocalStack test suite: `test_logs_groups.py`, `test_logs_streams.py`, `test_logs_events.py`, `test_logs_filter_events.py`, `test_logs_resource_policies.py`

### Phase 2: Metric Filters + Subscription Filters (7 operations)

**Goal:** Terraform metric filter and subscription filter resource support.
**Estimated effort:** 1-2 days.

- `PutMetricFilter`, `DeleteMetricFilter`, `DescribeMetricFilters`, `TestMetricFilter`
- `PutSubscriptionFilter`, `DeleteSubscriptionFilter`, `DescribeSubscriptionFilters`
- Port LocalStack test suite: `test_logs_metric_filters.py`, `test_logs_subscription_filters.py`

### Phase 3: Destinations + Insights Queries + KMS (12 operations)

**Goal:** Cross-account destinations, basic Insights query stubs, KMS key association.
**Estimated effort:** 2 days.

- `PutDestination`, `PutDestinationPolicy`, `DeleteDestination`, `DescribeDestinations`
- `StartQuery`, `StopQuery`, `GetQueryResults`, `DescribeQueries`
- `PutQueryDefinition`, `DeleteQueryDefinition`, `DescribeQueryDefinitions`
- `AssociateKmsKey`, `DisassociateKmsKey`
- Port LocalStack test suite: `test_logs_destinations.py`, `test_logs_queries.py`

---

## 15. Risk Analysis

### 15.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Smithy codegen CW Logs model parsing | Low | Medium | Same codegen used for SSM; CW Logs model uses same `awsJson1.1` protocol |
| Memory growth from unbounded log events | Medium | High | Configurable per-stream and total event limits; retention enforcement |
| FilterLogEvents pattern matching edge cases | Medium | Medium | Start with simple text matching; JSON patterns deferred to Phase 2 stretch |
| Pagination token format | Low | Low | Use opaque encoded offset; same pattern as SSM/DynamoDB |
| PutLogEvents timestamp validation | Low | Low | Well-documented: reject >14 days old, >2 hours future |
| Sequence token behavior changes | Low | Low | Modern AWS behavior ignores sequence tokens; we follow modern behavior |
| Retention cleanup under concurrent writes | Low | Low | DashMap per-entry locks; cleanup only touches event vectors |

### 15.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Users need real subscription filter forwarding | Medium | Medium | Document as non-goal; add forwarding in future version |
| Users need real metric filter evaluation | Low | Low | Document as non-goal; store metadata correctly |
| Users need Insights query language | Low | Medium | Return stub results; document as non-goal |
| Users need JSON filter patterns | Medium | Medium | Defer to Phase 2 stretch; simple text patterns cover most cases |
| fluent-bit has specific behavioral expectations | Medium | Medium | Test with fluent-bit early; fix mismatches |

### 15.3 Behavioral Differences from AWS

| Behavior | AWS | Rustack | Justification |
|----------|-----|-----------|---------------|
| KMS encryption | Encrypts with KMS key | Stores key ID, no encryption | No KMS service; local dev does not need encryption |
| Subscription filter forwarding | Forwards matching events | Stores filter metadata only | No Lambda/Kinesis/Firehose service dependencies |
| Metric filter evaluation | Publishes CloudWatch metrics | Stores filter metadata only | No CloudWatch metrics service |
| Insights query engine | Full query language | Stub results | Complex query engine; not needed for local dev |
| Data protection | Masks sensitive data | Accepts policy, does not mask | No data classification engine |
| Sequence token enforcement | No longer enforced (modern) | Not enforced | Matches modern AWS behavior |
| Event ingestion latency | Near-real-time (seconds) | Immediate | Local dev is faster |
| Cross-account delivery | Functional | Stores metadata only | Single-account local dev scenario |
| Log group limits | 1,000,000 per account | Configurable | Practical limits for local dev |
| Event retention enforcement | Exact to the second | Periodic cleanup (configurable interval) | Close enough for local dev |

### 15.4 Implementation Effort Comparison

| Component | CW Logs Estimate | SSM | DynamoDB | Ratio (vs SSM) |
|-----------|-----------------|-----|----------|----------------|
| Model (codegen) | ~2,500 LOC | ~1,200 | ~4,000 | 2.1x |
| HTTP routing | ~100 LOC | ~100 | ~100 | 1.0x |
| Storage engine | ~800 LOC | ~500 | ~2,500 | 1.6x |
| Business logic | ~2,500 LOC | ~1,200 | ~6,000 | 2.1x |
| Filter pattern engine | ~300 LOC | 0 | 0 | N/A |
| Retention task | ~100 LOC | 0 | 0 | N/A |
| **Total** | **~6,300 LOC** | **~3,000** | **~15,400** | **2.1x** |

CloudWatch Logs is roughly twice the implementation effort of SSM Parameter Store, but less than half of DynamoDB. The storage model is simpler than both SQS (lifecycle state machine) and DynamoDB (indexed tables with expressions). The main complexity comes from the breadth of operations (~40) rather than depth of any single operation.
