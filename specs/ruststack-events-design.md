# Rustack EventBridge: Native Rust Implementation Design

**Date:** 2026-03-06
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-sqs-design.md](./rustack-sqs-design.md), [rustack-ssm-design.md](./rustack-ssm-design.md)
**Scope:** Add native EventBridge support to Rustack -- event bus management, rule/target CRUD, event pattern matching, and event delivery to local targets (SQS, CloudWatch Logs). Uses the same Smithy-based codegen and gateway routing patterns established by DynamoDB and SSM.

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
9. [Event Pattern Matching Engine Design](#9-event-pattern-matching-engine-design)
10. [Storage Engine Design](#10-storage-engine-design)
11. [Core Business Logic](#11-core-business-logic)
12. [Error Handling](#12-error-handling)
13. [Server Integration](#13-server-integration)
14. [Testing Strategy](#14-testing-strategy)
15. [Phased Implementation Plan](#15-phased-implementation-plan)
16. [Risk Analysis](#16-risk-analysis)

---

## 1. Executive Summary

This spec proposes adding EventBridge support to Rustack as a fully native Rust implementation. Key design decisions:

- **Native Rust event pattern matching engine** -- the core complexity of EventBridge is the event pattern matching language (prefix, suffix, anything-but, numeric, exists, wildcard, cidr, equals-ignore-case, $or). We implement a purpose-built pattern matcher inspired by AWS's open-source Event Ruler (Java) and quamina-rs (Rust). No JVM, no external processes.
- **awsJson1.1 protocol** -- EventBridge (service name: `events`) uses the same `awsJson1.1` protocol as SSM. The `X-Amz-Target` prefix is `AWSEvents`. All JSON serialization, routing, and error formatting infrastructure from SSM is directly reusable.
- **Smithy codegen reuse** -- extend the existing codegen to generate an `rustack-events-model` crate from the official AWS EventBridge Smithy JSON AST (`aws/api-models-aws`, path `models/events/service/2015-10-07/events-2015-10-07.json`).
- **Local target delivery** -- when `PutEvents` matches a rule, deliver the event to configured targets. For MVP, support SQS queue targets (integrate with Rustack's existing SQS service via internal channel). CloudWatch Logs targets in a later phase.
- **Actor-based event bus** -- each event bus runs as an independent actor owning its rules, targets, and pattern matcher state. Communicates via `tokio::sync::mpsc` channels, following the actor model mandated by CLAUDE.md.
- **Phased delivery** -- 4 phases from MVP (event bus CRUD, rules, targets, PutEvents with SQS delivery, TestEventPattern) to full feature parity including input transformers, scheduled rules, and archive/replay.

---

## 2. Motivation

### 2.1 Why EventBridge?

EventBridge is the central event routing service in AWS. It is the backbone of event-driven architectures and is used by virtually every modern AWS application:

- **Event-driven architectures** -- microservices emit domain events, EventBridge routes them to consumers based on content-based filtering rules
- **CDK / CloudFormation** -- infrastructure-as-code tools create EventBridge rules as part of stack definitions; local deployment requires a working EventBridge
- **Serverless Framework** -- `serverless.yml` defines EventBridge event sources for Lambda functions
- **Step Functions** -- EventBridge rules trigger Step Function state machines
- **CI/CD pipelines** -- CodePipeline, CodeBuild, and custom CI emit events to EventBridge; integration tests need a local bus
- **Cross-service integration** -- S3 event notifications, DynamoDB Streams, and other AWS services route through EventBridge
- **SAM (Serverless Application Model)** -- `sam local` workflows rely on EventBridge for event routing

Without a local EventBridge, developers must either mock the event bus, skip integration tests, or make real AWS API calls during development.

### 2.2 Why Native Rust?

LocalStack implements EventBridge in Python. A native Rust implementation provides:

- **~10MB Docker image** (same as S3/DynamoDB/SQS/SSM) vs ~1GB for LocalStack
- **Millisecond startup** vs seconds for Python
- **~5MB memory baseline** vs 100MB+ for Python
- **Full debuggability** -- we own every line of code, including the pattern matching engine
- **Single binary** -- no process management, no Python runtime
- **Type-safe pattern matching** -- compile-time guarantees on the pattern matching engine
- **Tokio-native concurrency** -- event delivery to multiple targets runs concurrently on the async runtime

### 2.3 Tool Coverage

With EventBridge implemented, the following tools and frameworks work locally:

| Tool | Operations Used | Phase Available |
|------|----------------|-----------------|
| AWS CLI (`aws events`) | All CRUD ops + PutEvents | Phase 0 |
| AWS CDK | CreateEventBus, PutRule, PutTargets | Phase 0 |
| Serverless Framework | PutRule, PutTargets (event sources) | Phase 0 |
| SAM CLI | PutEvents, PutRule | Phase 0 |
| Terraform (`aws_cloudwatch_event_*`) | Full rule/target CRUD + tags | Phase 1 |
| EventBridge Scheduler (basic) | Scheduled rules (cron/rate) | Phase 3 |
| evb-cli | Pattern testing, debugging | Phase 0 |
| sls-test-tools | SQS target verification | Phase 0 |

### 2.4 Existing Alternatives

| Implementation | Language | Image Size | Pattern Matching | Target Delivery | Notes |
|---------------|----------|------------|-----------------|-----------------|-------|
| LocalStack Events | Python | ~1GB | Full (uses event-ruler via Java) | Full | Most mature, complex Python+Java hybrid |
| Moto | Python | N/A (library) | Partial | Mock only | Test mock, no actual delivery |
| **Rustack Events** | **Rust** | **~10MB** | **Full** | **SQS, Logs** | **This proposal** |

No existing Rust-based EventBridge emulator exists. This would be the first.

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Native Rust EventBridge emulator** -- no JVM, no Python, no external processes
2. **Full event pattern matching** -- all 10 comparison operators: exact, prefix, suffix, anything-but (all variants), numeric, exists, equals-ignore-case, wildcard, cidr, $or
3. **Event bus management** -- default bus + custom buses, CRUD operations
4. **Rule management** -- PutRule, DeleteRule, DescribeRule, ListRules, EnableRule, DisableRule
5. **Target management** -- PutTargets, RemoveTargets, ListTargetsByRule, ListRuleNamesByTarget
6. **Event delivery** -- PutEvents routes through pattern matching to targets; deliver to SQS queues (in-process)
7. **TestEventPattern** -- validate patterns against events without delivering (critical for development/debugging)
8. **Tag support** -- TagResource, UntagResource, ListTagsForResource for buses and rules
9. **Permission management** -- PutPermission, RemovePermission (store, do not enforce)
10. **Smithy-generated types** -- all EventBridge API types generated from official AWS Smithy model
11. **Same Docker image** -- single binary serves S3, DynamoDB, SQS, SSM, and EventBridge on port 4566
12. **Pass LocalStack events test suite** -- validate against vendored `test_events.py` and `test_events_patterns.py`

### 3.2 Non-Goals

1. **Archive and Replay** -- accept CreateArchive/StartReplay API calls but do not implement storage or replay
2. **Schema Registry** -- EventBridge Schemas service is a separate API; out of scope
3. **Pipes** -- EventBridge Pipes is a separate service; out of scope
4. **API Destinations and Connections** -- accept CreateApiDestination/CreateConnection but do not make outbound HTTP calls
5. **Endpoints (global endpoints)** -- accept CreateEndpoint but do not implement cross-region failover
6. **Partner event sources** -- accept ActivateEventSource/CreatePartnerEventSource but no real partner integration
7. **Cross-account event delivery** -- all buses exist within a single account context
8. **Lambda target invocation** -- Lambda is not implemented in Rustack; Lambda targets are accepted but not invoked
9. **IAM policy enforcement** -- accept PutPermission/RemovePermission, do not evaluate policies
10. **Data persistence across restarts** -- in-memory only, matching all other Rustack services
11. **CloudWatch metrics** -- no metrics emission
12. **Scheduled rule execution** -- accept cron/rate ScheduleExpression in PutRule, but defer actual timer-based triggering to Phase 3

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                    AWS SDK / CLI / CDK / Terraform
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  X-Amz-Target dispatch
              +--------+------------+
                       |
         +-------+-----+------+------+------+
         |       |            |      |      |
         v       v            v      v      v
   +-------+ +--------+ +------+ +-----+ +--------+
   |  S3   | |  DDB   | |  SQS | | SSM | | Events |
   |(Xml)  | |(Json10)| |(Json)| |(J11)| | (J11)  |
   +---+---+ +---+----+ +--+---+ +--+--+ +---+----+
       |         |          |        |        |
   +---+---+ +--+----+ +---+--+ +--+---+ +---+----+
   |S3 Core| |DDB    | |SQS   | |SSM   | |Events  |
   |       | |Core   | |Core   | |Core  | |Core    |
   +---+---+ +--+----+ +--+---+ +--+---+ +---+----+
       |         |          |        |        |
       +---------+-----+----+--------+--------+
                       |
                +------+------+
                | rustack-  |
                | core + auth |
                +-----------  +
```

### 4.2 Internal Service Integration

A critical design consideration is that EventBridge delivers events to targets. For SQS targets, rather than making HTTP calls to ourselves, we use direct in-process channels:

```
PutEvents -> EventBus Actor -> Pattern Match -> Target Delivery
                                                     |
                                         +-----------+-----------+
                                         |                       |
                                    SQS Target              Log Target
                                    (channel to               (append to
                                     SQS actor)              log group)
```

The Events Core crate takes a `TargetDelivery` trait that abstracts delivery:

```rust
/// Abstraction for delivering events to targets.
///
/// Implemented by the server binary which has access to all service cores.
/// The events-core crate does not depend on sqs-core directly.
#[async_trait]
pub trait TargetDelivery: Send + Sync + 'static {
    /// Deliver an event to a target identified by ARN.
    async fn deliver(
        &self,
        target_arn: &str,
        event_json: &str,
        target_config: &TargetConfig,
    ) -> Result<(), DeliveryError>;
}
```

### 4.3 Gateway Service Routing

EventBridge requests are distinguished by the `X-Amz-Target` header prefix:

| Service | X-Amz-Target Prefix | Content-Type |
|---------|---------------------|--------------|
| DynamoDB | `DynamoDB_20120810.` | `application/x-amz-json-1.0` |
| SQS | `AmazonSQS.` | `application/x-amz-json-1.0` |
| SSM | `AmazonSSM.` | `application/x-amz-json-1.1` |
| **Events** | **`AWSEvents.`** | **`application/x-amz-json-1.1`** |
| S3 | *(absent)* | varies |

Routing logic: check `X-Amz-Target` header. If prefix is `AWSEvents.`, route to EventBridge. This is unambiguous and does not conflict with any other service.

### 4.4 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-events-model       <-- NEW (auto-generated)
+-- rustack-events-core        <-- NEW
+-- rustack-events-http        <-- NEW

rustack-events-http
+-- rustack-events-model
+-- rustack-auth

rustack-events-core
+-- rustack-core
+-- rustack-events-model
+-- tokio (channels, timers)
+-- dashmap
+-- serde_json (for pattern matching against event JSON)

rustack-events-model (auto-generated, standalone)
```

**Important**: `rustack-events-core` does NOT depend on `rustack-sqs-core`. Target delivery is abstracted via the `TargetDelivery` trait, and the server binary wires them together. This keeps crate dependencies clean and avoids circular dependencies.

---

## 5. Protocol Design: awsJson1.1

### 5.1 Protocol Details

EventBridge uses `awsJson1.1`, identical to SSM. The entire protocol infrastructure from SSM is reusable.

| Aspect | SSM (awsJson1.1) | Events (awsJson1.1) |
|--------|-------------------|---------------------|
| HTTP Method | POST only | POST only |
| URL Path | `/` always | `/` always |
| Content-Type | `application/x-amz-json-1.1` | `application/x-amz-json-1.1` |
| X-Amz-Target | `AmazonSSM.<Op>` | `AWSEvents.<Op>` |
| Request body | JSON | JSON |
| Response body | JSON | JSON |
| Error `__type` | Short name | Short name |
| Timestamp format | Epoch seconds (double) | ISO 8601 strings |
| Auth | SigV4, service=`ssm` | SigV4, service=`events` |

### 5.2 Request/Response Example

Request:
```http
POST / HTTP/1.1
Content-Type: application/x-amz-json-1.1
X-Amz-Target: AWSEvents.PutEvents

{
  "Entries": [
    {
      "Source": "my.application",
      "DetailType": "MyEvent",
      "Detail": "{\"key\": \"value\"}",
      "EventBusName": "default"
    }
  ]
}
```

Success response:
```http
HTTP/1.1 200 OK
Content-Type: application/x-amz-json-1.1

{
  "Entries": [
    {
      "EventId": "uuid-here"
    }
  ],
  "FailedEntryCount": 0
}
```

Error response:
```http
HTTP/1.1 400 Bad Request
Content-Type: application/x-amz-json-1.1

{
  "__type": "ResourceNotFoundException",
  "message": "Event bus my-bus does not exist."
}
```

### 5.3 No Legacy Protocol

Unlike SQS (which has awsQuery backward compatibility), EventBridge was introduced in 2019 with `awsJson1.1` from the start. There is no legacy protocol to support.

---

## 6. Smithy Code Generation Strategy

### 6.1 Approach: Extract EventBridge Subset

The full EventBridge Smithy model defines 57 operations. We generate types for the operations we plan to implement, plus stub types for the remaining operations. The codegen resolves transitive type dependencies.

### 6.2 Smithy Model Acquisition

The EventBridge Smithy model is available at:
- **Repository:** `https://github.com/aws/api-models-aws`
- **Path:** `models/events/service/2015-10-07/events-2015-10-07.json`

Download and place at `codegen/smithy-model/events.json`.

### 6.3 Events Service Configuration

```rust
const EVENTS_OPERATIONS: &[&str] = &[
    // Event bus management
    "CreateEventBus",
    "DeleteEventBus",
    "DescribeEventBus",
    "ListEventBuses",
    "UpdateEventBus",
    // Rule management
    "PutRule",
    "DeleteRule",
    "DescribeRule",
    "ListRules",
    "EnableRule",
    "DisableRule",
    // Target management
    "PutTargets",
    "RemoveTargets",
    "ListTargetsByRule",
    "ListRuleNamesByTarget",
    // Event operations
    "PutEvents",
    "TestEventPattern",
    // Permission management
    "PutPermission",
    "RemovePermission",
    // Tagging
    "TagResource",
    "UntagResource",
    "ListTagsForResource",
    // Archive (stub)
    "CreateArchive",
    "DeleteArchive",
    "DescribeArchive",
    "ListArchives",
    "UpdateArchive",
    // Replay (stub)
    "StartReplay",
    "CancelReplay",
    "DescribeReplay",
    "ListReplays",
    // API Destinations (stub)
    "CreateApiDestination",
    "DeleteApiDestination",
    "DescribeApiDestination",
    "ListApiDestinations",
    "UpdateApiDestination",
    // Connections (stub)
    "CreateConnection",
    "DeleteConnection",
    "DescribeConnection",
    "ListConnections",
    "UpdateConnection",
    "DeauthorizeConnection",
    // Endpoints (stub)
    "CreateEndpoint",
    "DeleteEndpoint",
    "DescribeEndpoint",
    "ListEndpoints",
    "UpdateEndpoint",
    // Partner event sources (stub)
    "ActivateEventSource",
    "CreatePartnerEventSource",
    "DeactivateEventSource",
    "DeletePartnerEventSource",
    "DescribeEventSource",
    "DescribePartnerEventSource",
    "ListEventSources",
    "ListPartnerEventSourceAccounts",
    "ListPartnerEventSources",
    "PutPartnerEvents",
];
```

### 6.4 Generated Types Estimate

From the 57 operations, the codegen will produce roughly:

- 57 input structs
- 57 output structs
- ~40 shared types (`Rule`, `Target`, `EventBus`, `PutEventsRequestEntry`, `PutEventsResultEntry`, `Tag`, `Condition`, `InputTransformer`, etc.)
- 1 operation enum (`EventsOperation` with 57 variants)
- ~15 error types
- Total: roughly 3,000-4,000 lines of generated code

### 6.5 Key Type Differences from SSM

| Aspect | SSM | Events |
|--------|-----|--------|
| Namespace | `com.amazonaws.ssm#` | `com.amazonaws.cloudwatchevents#` |
| Target prefix | `AmazonSSM` | `AWSEvents` |
| Operations | 13 | 57 |
| Timestamp serialization | Epoch seconds | ISO 8601 strings |
| Complex nested types | Few | `Target` with `InputTransformer`, `RunCommandParameters`, etc. |

### 6.6 Makefile Integration

```makefile
codegen-events:
	@cd codegen && cargo run -- smithy-model/events.json ../crates/rustack-events-model/src
	@cargo +nightly fmt -p rustack-events-model

codegen: codegen-s3 codegen-dynamodb codegen-sqs codegen-ssm codegen-events
```

---

## 7. Crate Structure

### 7.1 `rustack-events-model` (auto-generated)

```
crates/rustack-events-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs                    # Module re-exports
    +-- types.rs                  # Auto-generated: Rule, Target, EventBus, Tag, etc.
    +-- operations.rs             # Auto-generated: EventsOperation enum (57 variants)
    +-- error.rs                  # Auto-generated: EventsError + error codes
    +-- input.rs                  # Auto-generated: all input structs
    +-- output.rs                 # Auto-generated: all output structs
```

**Dependencies**: `serde`, `serde_json`, `bytes`, `http`

### 7.2 `rustack-events-http`

```
crates/rustack-events-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs                 # X-Amz-Target: AWSEvents.* dispatch -> EventsOperation
    +-- dispatch.rs               # EventsHandler trait + dispatch logic
    +-- service.rs                # Hyper Service impl for Events
    +-- error.rs                  # Error response formatting
    +-- body.rs                   # Response body type
```

**Dependencies**: `rustack-events-model`, `rustack-auth`, `hyper`, `serde_json`, `bytes`

### 7.3 `rustack-events-core`

```
crates/rustack-events-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs                 # EventsConfig
    +-- provider.rs               # RustackEvents (main provider, EventBusManager actor)
    +-- error.rs                  # EventsServiceError
    +-- bus/
    |   +-- mod.rs
    |   +-- actor.rs              # EventBusActor: per-bus rule/target management and event routing
    |   +-- default.rs            # Default event bus initialization
    +-- rule/
    |   +-- mod.rs
    |   +-- storage.rs            # Rule storage and state management
    |   +-- schedule.rs           # ScheduleExpression parsing (cron/rate), deferred execution
    +-- target/
    |   +-- mod.rs
    |   +-- storage.rs            # Target storage per rule
    |   +-- delivery.rs           # TargetDelivery trait and DeliveryError
    |   +-- transform.rs          # InputPath, InputTransformer processing
    +-- pattern/
    |   +-- mod.rs
    |   +-- engine.rs             # PatternMatcher: the core pattern matching engine
    |   +-- parser.rs             # Parse JSON event pattern into PatternNode tree
    |   +-- operators.rs          # Individual operator implementations
    |   +-- value.rs              # EventValue: normalized value type for matching
    +-- ops/
        +-- mod.rs
        +-- bus.rs                # CreateEventBus, DeleteEventBus, DescribeEventBus, ListEventBuses
        +-- rule.rs               # PutRule, DeleteRule, DescribeRule, ListRules, EnableRule, DisableRule
        +-- target.rs             # PutTargets, RemoveTargets, ListTargetsByRule, ListRuleNamesByTarget
        +-- events.rs             # PutEvents (routing + delivery)
        +-- test_pattern.rs       # TestEventPattern
        +-- tags.rs               # TagResource, UntagResource, ListTagsForResource
        +-- permissions.rs        # PutPermission, RemovePermission
        +-- stubs.rs              # Stub implementations for Archive, Replay, etc.
```

**Dependencies**: `rustack-core`, `rustack-events-model`, `tokio` (mpsc, sync), `dashmap`, `uuid`, `tracing`, `chrono`, `serde_json`, `ipnet` (for CIDR matching)

---

## 8. HTTP Layer Design

### 8.1 Events Router

EventBridge uses the same POST-to-root dispatch as SSM, with a different target prefix:

```rust
/// Events operation router.
///
/// Dispatches based on `X-Amz-Target: AWSEvents.<OperationName>`.
pub struct EventsRouter;

impl EventsRouter {
    /// Resolve an HTTP request to an Events operation.
    pub fn resolve(req: &http::Request<()>) -> Result<EventsOperation, EventsError> {
        let target = req
            .headers()
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| EventsError::missing_action())?;

        let op_name = target
            .strip_prefix("AWSEvents.")
            .ok_or_else(|| EventsError::unknown_operation(target))?;

        EventsOperation::from_name(op_name)
            .ok_or_else(|| EventsError::unknown_operation(op_name))
    }
}
```

### 8.2 EventsHandler Trait

```rust
/// The boundary between HTTP and business logic.
///
/// Protocol-agnostic: receives typed operation + raw JSON body,
/// returns JSON response bytes.
pub trait EventsHandler: Send + Sync + 'static {
    fn handle_operation(
        &self,
        op: EventsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<Bytes>, EventsError>> + Send>>;
}
```

### 8.3 Hyper Service

```rust
/// Hyper Service implementation for EventBridge.
pub struct EventsHttpService<H> {
    handler: Arc<H>,
    config: EventsHttpConfig,
}

pub struct EventsHttpConfig {
    pub skip_signature_validation: bool,
    pub region: String,
    pub account_id: String,
}
```

---

## 9. Event Pattern Matching Engine Design

This is the core complexity of EventBridge. The pattern matching engine must evaluate JSON event patterns against JSON events, supporting 10 comparison operators plus logical combinators.

### 9.1 Pattern Language Overview

An event pattern is a JSON object where:
- **Top-level keys** match event fields (`source`, `detail-type`, `detail`, `account`, `region`, etc.)
- **Values are arrays** of match conditions (implicit OR within the array)
- **Nested objects** recurse into the event structure
- **`$or`** provides explicit OR across different fields
- Multiple top-level keys are AND-ed together

Example pattern:
```json
{
  "source": ["my.app"],
  "detail-type": ["OrderPlaced", "OrderUpdated"],
  "detail": {
    "amount": [{"numeric": [">", 100]}],
    "status": [{"anything-but": "cancelled"}],
    "region": [{"prefix": "us-"}]
  }
}
```

### 9.2 Comparison Operators

| Operator | JSON Syntax | Matches |
|----------|-------------|---------|
| Exact match | `["value"]` | String/number/null equality |
| Prefix | `[{"prefix": "val"}]` | String starts with "val" |
| Prefix (ignore case) | `[{"prefix": {"equals-ignore-case": "val"}}]` | Case-insensitive prefix |
| Suffix | `[{"suffix": ".png"}]` | String ends with ".png" |
| Suffix (ignore case) | `[{"suffix": {"equals-ignore-case": ".PNG"}}]` | Case-insensitive suffix |
| Equals-ignore-case | `[{"equals-ignore-case": "alice"}]` | Case-insensitive equality |
| Wildcard | `[{"wildcard": "dir/*.png"}]` | Shell-style glob (`*` matches any) |
| Anything-but | `[{"anything-but": "val"}]` | NOT equal to "val" |
| Anything-but (list) | `[{"anything-but": ["a", "b"]}]` | NOT equal to any in list |
| Anything-but (prefix) | `[{"anything-but": {"prefix": "init"}}]` | Does NOT start with "init" |
| Anything-but (suffix) | `[{"anything-but": {"suffix": ".tmp"}}]` | Does NOT end with ".tmp" |
| Anything-but (ignorecase) | `[{"anything-but": {"equals-ignore-case": "val"}}]` | Case-insensitive NOT equal |
| Anything-but (wildcard) | `[{"anything-but": {"wildcard": "*/lib/*"}}]` | Does NOT match glob |
| Numeric (comparison) | `[{"numeric": [">", 100]}]` | Numeric comparison |
| Numeric (range) | `[{"numeric": [">", 10, "<=", 20]}]` | Numeric range |
| Exists (true) | `[{"exists": true}]` | Field is present (any value) |
| Exists (false) | `[{"exists": false}]` | Field is absent |
| CIDR | `[{"cidr": "10.0.0.0/24"}]` | IP in CIDR block |
| Null | `[null]` | Field value is JSON null |
| Empty string | `[""]` | Field value is empty string |

### 9.3 Data Model

```rust
/// A parsed event pattern, ready for matching.
#[derive(Debug, Clone)]
pub struct EventPattern {
    /// Top-level field matchers (AND-ed together).
    pub fields: Vec<FieldMatcher>,
    /// Explicit $or conditions.
    pub or_conditions: Vec<Vec<FieldMatcher>>,
}

/// A matcher for a single field path.
#[derive(Debug, Clone)]
pub struct FieldMatcher {
    /// Dot-separated field path from root (e.g., "detail.status").
    pub path: Vec<String>,
    /// Match node: either a leaf with conditions or a nested object.
    pub node: PatternNode,
}

/// A node in the pattern tree.
#[derive(Debug, Clone)]
pub enum PatternNode {
    /// Leaf: array of conditions (OR-ed together).
    Leaf(Vec<MatchCondition>),
    /// Nested object: recurse into sub-fields (AND-ed).
    Object {
        fields: Vec<FieldMatcher>,
        or_conditions: Vec<Vec<FieldMatcher>>,
    },
}

/// A single match condition within a leaf array.
#[derive(Debug, Clone)]
pub enum MatchCondition {
    /// Exact string match.
    ExactString(String),
    /// Exact numeric match (stored as f64 for IEEE 754 comparison).
    ExactNumeric(f64),
    /// Exact null match.
    ExactNull,
    /// Prefix match.
    Prefix(String),
    /// Prefix match (case-insensitive).
    PrefixIgnoreCase(String),
    /// Suffix match.
    Suffix(String),
    /// Suffix match (case-insensitive).
    SuffixIgnoreCase(String),
    /// Equals-ignore-case.
    EqualsIgnoreCase(String),
    /// Wildcard (shell-style glob, `*` matches any sequence).
    Wildcard(String),
    /// Anything-but: inverted match.
    AnythingBut(AnythingButCondition),
    /// Numeric comparison.
    Numeric(NumericCondition),
    /// Field existence check.
    Exists(bool),
    /// CIDR block match.
    Cidr(IpNet),
}

/// The inner condition for anything-but matching.
#[derive(Debug, Clone)]
pub enum AnythingButCondition {
    /// Not equal to any of these strings.
    Strings(Vec<String>),
    /// Not equal to any of these numbers.
    Numbers(Vec<f64>),
    /// Does not match prefix.
    Prefix(String),
    /// Does not match suffix.
    Suffix(String),
    /// Does not match (case-insensitive).
    EqualsIgnoreCase(String),
    /// Does not match (case-insensitive list).
    EqualsIgnoreCaseList(Vec<String>),
    /// Does not match wildcard pattern.
    Wildcard(String),
}

/// Numeric comparison condition.
#[derive(Debug, Clone)]
pub struct NumericCondition {
    /// Lower bound (inclusive or exclusive).
    pub lower: Option<NumericBound>,
    /// Upper bound (inclusive or exclusive).
    pub upper: Option<NumericBound>,
    /// Exact equality.
    pub equals: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct NumericBound {
    pub value: f64,
    pub inclusive: bool,
}
```

### 9.4 Pattern Parser

The pattern parser converts a JSON event pattern (a `serde_json::Value`) into the typed `EventPattern` structure, validating syntax along the way.

```rust
/// Parse a JSON event pattern string into an EventPattern.
///
/// # Errors
///
/// Returns `PatternParseError` if the pattern JSON is invalid or uses
/// unsupported syntax.
pub fn parse_event_pattern(pattern_json: &str) -> Result<EventPattern, PatternParseError> {
    let value: serde_json::Value = serde_json::from_str(pattern_json)
        .map_err(|e| PatternParseError::InvalidJson(e.to_string()))?;

    let obj = value
        .as_object()
        .ok_or(PatternParseError::NotAnObject)?;

    parse_object(obj)
}

fn parse_object(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<EventPattern, PatternParseError> {
    let mut fields = Vec::new();
    let mut or_conditions = Vec::new();

    for (key, value) in obj {
        if key == "$or" {
            // Parse $or: array of objects, each object is a set of field matchers.
            let or_array = value
                .as_array()
                .ok_or(PatternParseError::OrNotArray)?;
            for or_item in or_array {
                let or_obj = or_item
                    .as_object()
                    .ok_or(PatternParseError::OrItemNotObject)?;
                let sub_pattern = parse_object(or_obj)?;
                or_conditions.push(sub_pattern.fields);
            }
        } else {
            fields.push(parse_field(key, value)?);
        }
    }

    Ok(EventPattern { fields, or_conditions })
}

fn parse_field(
    key: &str,
    value: &serde_json::Value,
) -> Result<FieldMatcher, PatternParseError> {
    match value {
        // Array -> leaf node with conditions
        serde_json::Value::Array(arr) => {
            let conditions = arr.iter()
                .map(parse_condition)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(FieldMatcher {
                path: vec![key.to_string()],
                node: PatternNode::Leaf(conditions),
            })
        }
        // Object -> nested field matchers
        serde_json::Value::Object(obj) => {
            let sub_pattern = parse_object(obj)?;
            Ok(FieldMatcher {
                path: vec![key.to_string()],
                node: PatternNode::Object {
                    fields: sub_pattern.fields,
                    or_conditions: sub_pattern.or_conditions,
                },
            })
        }
        _ => Err(PatternParseError::InvalidFieldValue(key.to_string())),
    }
}
```

### 9.5 Condition Parser

```rust
fn parse_condition(value: &serde_json::Value) -> Result<MatchCondition, PatternParseError> {
    match value {
        // String -> exact string match
        serde_json::Value::String(s) => Ok(MatchCondition::ExactString(s.clone())),

        // Number -> exact numeric match
        serde_json::Value::Number(n) => {
            let f = n.as_f64()
                .ok_or(PatternParseError::InvalidNumeric)?;
            Ok(MatchCondition::ExactNumeric(f))
        }

        // Null -> exact null match
        serde_json::Value::Null => Ok(MatchCondition::ExactNull),

        // Object -> operator condition
        serde_json::Value::Object(obj) => parse_operator_condition(obj),

        _ => Err(PatternParseError::InvalidCondition),
    }
}

fn parse_operator_condition(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<MatchCondition, PatternParseError> {
    // Exactly one key expected.
    if obj.len() != 1 {
        return Err(PatternParseError::MultipleOperators);
    }

    let (op, val) = obj.iter().next().unwrap();

    match op.as_str() {
        "prefix" => parse_prefix_or_suffix(val, true),
        "suffix" => parse_prefix_or_suffix(val, false),
        "equals-ignore-case" => {
            let s = val.as_str()
                .ok_or(PatternParseError::OperatorRequiresString("equals-ignore-case"))?;
            Ok(MatchCondition::EqualsIgnoreCase(s.to_lowercase()))
        }
        "wildcard" => {
            let s = val.as_str()
                .ok_or(PatternParseError::OperatorRequiresString("wildcard"))?;
            validate_wildcard(s)?;
            Ok(MatchCondition::Wildcard(s.to_string()))
        }
        "anything-but" => parse_anything_but(val),
        "numeric" => parse_numeric(val),
        "exists" => {
            let b = val.as_bool()
                .ok_or(PatternParseError::ExistsRequiresBool)?;
            Ok(MatchCondition::Exists(b))
        }
        "cidr" => {
            let s = val.as_str()
                .ok_or(PatternParseError::OperatorRequiresString("cidr"))?;
            let net: IpNet = s.parse()
                .map_err(|_| PatternParseError::InvalidCidr(s.to_string()))?;
            Ok(MatchCondition::Cidr(net))
        }
        unknown => Err(PatternParseError::UnknownOperator(unknown.to_string())),
    }
}
```

### 9.6 Matching Engine

The matching engine evaluates a parsed `EventPattern` against a JSON event. The algorithm is a recursive tree walk:

```rust
/// Match an event against a parsed pattern.
///
/// Returns `true` if the event matches all conditions in the pattern.
pub fn matches(pattern: &EventPattern, event: &serde_json::Value) -> bool {
    // All field matchers must match (AND).
    let fields_match = pattern.fields.iter().all(|fm| match_field(fm, event));
    if !fields_match {
        return false;
    }

    // If there are $or conditions, at least one must match.
    if pattern.or_conditions.is_empty() {
        return true;
    }

    pattern.or_conditions.iter().any(|or_group| {
        or_group.iter().all(|fm| match_field(fm, event))
    })
}

fn match_field(matcher: &FieldMatcher, event: &serde_json::Value) -> bool {
    // Navigate to the target field in the event.
    let field_value = navigate_path(event, &matcher.path);

    match &matcher.node {
        PatternNode::Leaf(conditions) => {
            match_leaf(conditions, field_value)
        }
        PatternNode::Object { fields, or_conditions } => {
            // Field must exist and be an object to recurse into.
            match field_value {
                Some(v) => {
                    let sub_pattern = EventPattern {
                        fields: fields.clone(),
                        or_conditions: or_conditions.clone(),
                    };
                    matches(&sub_pattern, v)
                }
                None => {
                    // Check if any condition is exists:false.
                    false
                }
            }
        }
    }
}

/// Navigate a dot-path through JSON to find the target value.
fn navigate_path<'a>(
    event: &'a serde_json::Value,
    path: &[String],
) -> Option<&'a serde_json::Value> {
    let mut current = event;
    for segment in path {
        match current.get(segment) {
            Some(v) => current = v,
            None => return None,
        }
    }
    Some(current)
}

/// Match leaf conditions (OR-ed) against a field value.
///
/// The field value may be absent (None), a scalar, or an array.
/// When the event field is an array, any element matching any condition
/// constitutes a match.
fn match_leaf(conditions: &[MatchCondition], field_value: Option<&serde_json::Value>) -> bool {
    // Special handling for exists conditions.
    for cond in conditions {
        if let MatchCondition::Exists(should_exist) = cond {
            let exists = field_value.is_some();
            if exists == *should_exist {
                return true;
            }
            continue;
        }
    }

    let Some(value) = field_value else {
        // Field is absent. Only exists:false would match, handled above.
        return false;
    };

    // If the event field is an array, check if any element matches.
    if let serde_json::Value::Array(arr) = value {
        return arr.iter().any(|elem| {
            conditions.iter().any(|cond| match_single_value(cond, elem))
        });
    }

    // Scalar value: check if any condition matches.
    conditions.iter().any(|cond| match_single_value(cond, value))
}
```

### 9.7 Individual Operator Matching

```rust
/// Match a single condition against a single JSON value.
fn match_single_value(condition: &MatchCondition, value: &serde_json::Value) -> bool {
    match condition {
        MatchCondition::ExactString(expected) => {
            value.as_str().is_some_and(|s| s == expected)
        }

        MatchCondition::ExactNumeric(expected) => {
            value.as_f64().is_some_and(|n| (n - expected).abs() < f64::EPSILON)
        }

        MatchCondition::ExactNull => value.is_null(),

        MatchCondition::Prefix(prefix) => {
            value.as_str().is_some_and(|s| s.starts_with(prefix.as_str()))
        }

        MatchCondition::PrefixIgnoreCase(prefix) => {
            value.as_str().is_some_and(|s| {
                s.to_lowercase().starts_with(&prefix.to_lowercase())
            })
        }

        MatchCondition::Suffix(suffix) => {
            value.as_str().is_some_and(|s| s.ends_with(suffix.as_str()))
        }

        MatchCondition::SuffixIgnoreCase(suffix) => {
            value.as_str().is_some_and(|s| {
                s.to_lowercase().ends_with(&suffix.to_lowercase())
            })
        }

        MatchCondition::EqualsIgnoreCase(expected) => {
            value.as_str().is_some_and(|s| s.to_lowercase() == *expected)
        }

        MatchCondition::Wildcard(pattern) => {
            value.as_str().is_some_and(|s| wildcard_match(pattern, s))
        }

        MatchCondition::AnythingBut(ab) => match_anything_but(ab, value),

        MatchCondition::Numeric(nc) => {
            value.as_f64().is_some_and(|n| match_numeric(nc, n))
        }

        MatchCondition::Exists(_) => {
            // Exists is handled at the leaf level, not per-value.
            true
        }

        MatchCondition::Cidr(net) => {
            value.as_str().is_some_and(|s| {
                s.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| net.contains(&ip))
            })
        }
    }
}

/// Wildcard matching: `*` matches any sequence of characters (including empty).
/// Escape: `\*` matches literal `*`, `\\` matches literal `\`.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    // Split pattern by unescaped `*` and match segments.
    let segments = split_wildcard_pattern(pattern);
    wildcard_match_segments(&segments, text)
}

fn wildcard_match_segments(segments: &[WildcardSegment], text: &str) -> bool {
    match segments {
        [] => text.is_empty(),
        [WildcardSegment::Literal(lit)] => text == lit,
        [WildcardSegment::Star] => true,
        [WildcardSegment::Literal(lit), rest @ ..] => {
            if let Some(remainder) = text.strip_prefix(lit.as_str()) {
                wildcard_match_segments(rest, remainder)
            } else {
                false
            }
        }
        [WildcardSegment::Star, rest @ ..] => {
            // Star matches zero or more characters. Try each position.
            (0..=text.len()).any(|i| {
                wildcard_match_segments(rest, &text[i..])
            })
        }
    }
}

#[derive(Debug)]
enum WildcardSegment {
    Literal(String),
    Star,
}

/// Anything-but matching: returns true if the value does NOT match the condition.
fn match_anything_but(ab: &AnythingButCondition, value: &serde_json::Value) -> bool {
    match ab {
        AnythingButCondition::Strings(strings) => {
            // null values match anything-but (they are not equal to any string)
            match value.as_str() {
                Some(s) => !strings.iter().any(|expected| s == expected),
                None => !value.is_null() || true, // null matches anything-but string
            }
        }
        AnythingButCondition::Numbers(numbers) => {
            match value.as_f64() {
                Some(n) => !numbers.iter().any(|expected| (n - expected).abs() < f64::EPSILON),
                None => true,
            }
        }
        AnythingButCondition::Prefix(prefix) => {
            value.as_str().map_or(true, |s| !s.starts_with(prefix.as_str()))
        }
        AnythingButCondition::Suffix(suffix) => {
            value.as_str().map_or(true, |s| !s.ends_with(suffix.as_str()))
        }
        AnythingButCondition::EqualsIgnoreCase(expected) => {
            value.as_str().map_or(true, |s| s.to_lowercase() != *expected)
        }
        AnythingButCondition::EqualsIgnoreCaseList(list) => {
            value.as_str().map_or(true, |s| {
                let lower = s.to_lowercase();
                !list.iter().any(|expected| lower == *expected)
            })
        }
        AnythingButCondition::Wildcard(pattern) => {
            value.as_str().map_or(true, |s| !wildcard_match(pattern, s))
        }
    }
}

/// Numeric comparison.
fn match_numeric(nc: &NumericCondition, value: f64) -> bool {
    if let Some(eq) = nc.equals {
        return (value - eq).abs() < f64::EPSILON;
    }

    let lower_ok = match &nc.lower {
        Some(bound) if bound.inclusive => value >= bound.value,
        Some(bound) => value > bound.value,
        None => true,
    };

    let upper_ok = match &nc.upper {
        Some(bound) if bound.inclusive => value <= bound.value,
        Some(bound) => value < bound.value,
        None => true,
    };

    lower_ok && upper_ok
}
```

### 9.8 Pattern Validation

Pattern validation occurs at `PutRule` time and `TestEventPattern` time. Invalid patterns are rejected with `InvalidEventPatternException`.

Key validation rules:
1. Pattern must be a JSON object (not array, not string, not null)
2. Field values must be arrays or nested objects
3. Operators must be recognized (case-sensitive: `prefix` not `Prefix`)
4. Numeric arrays must have valid operator strings (`<`, `<=`, `>`, `>=`, `=`) alternating with numbers
5. Wildcard patterns must not have consecutive `*` characters (after accounting for escapes)
6. CIDR must be a valid IPv4 or IPv6 CIDR notation
7. `exists` must have a boolean value
8. Anything-but variants must have valid inner values

### 9.9 Performance Considerations

For MVP, we use a straightforward recursive tree-walk matcher. This is O(P * E) where P is pattern complexity and E is event size. For local development with dozens of rules and small events, this is more than sufficient.

If performance becomes a concern in the future (thousands of rules), we can adopt the automaton-based approach from Event Ruler / quamina-rs, which compiles all patterns into a single finite state machine for O(E) matching regardless of pattern count. The `EventPattern` data model is designed to support this evolution -- the parsed pattern tree can be compiled into an automaton without changing the public API.

---

## 10. Storage Engine Design

### 10.1 Overview

The storage engine manages event buses, rules, and targets. Each event bus is an independent actor that owns its rules, targets, and handles event routing.

### 10.2 Core Data Structures

```rust
/// An event bus with its rules and targets.
#[derive(Debug)]
pub struct EventBusState {
    /// Bus metadata.
    pub name: String,
    pub arn: String,
    pub description: Option<String>,
    /// Permissions policy (stored, not enforced).
    pub policy: Option<String>,
    /// Tags.
    pub tags: HashMap<String, String>,
    /// Rules on this bus, keyed by rule name.
    pub rules: HashMap<String, RuleState>,
    /// Creation time.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last modified time.
    pub last_modified_at: chrono::DateTime<chrono::Utc>,
}

/// A rule on an event bus.
#[derive(Debug)]
pub struct RuleState {
    /// Rule metadata.
    pub name: String,
    pub arn: String,
    pub description: Option<String>,
    /// The parsed event pattern (None for scheduled rules).
    pub event_pattern: Option<EventPattern>,
    /// The raw event pattern JSON (stored for DescribeRule).
    pub event_pattern_json: Option<String>,
    /// Schedule expression (e.g., "rate(5 minutes)" or "cron(0 12 * * ? *)").
    pub schedule_expression: Option<String>,
    /// Current state.
    pub state: RuleStateValue,
    /// Role ARN for target invocation (stored, not used for auth).
    pub role_arn: Option<String>,
    /// Managed-by field (e.g., "aws.partner/xyz").
    pub managed_by: Option<String>,
    /// Event bus name this rule belongs to.
    pub event_bus_name: String,
    /// Tags on this rule.
    pub tags: HashMap<String, String>,
    /// Targets for this rule, keyed by target ID.
    pub targets: HashMap<String, TargetState>,
    /// Creation time.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleStateValue {
    Enabled,
    Disabled,
}

/// A target attached to a rule.
#[derive(Debug, Clone)]
pub struct TargetState {
    /// Target ID (user-provided, unique within a rule).
    pub id: String,
    /// Target ARN (SQS queue, SNS topic, Lambda function, etc.).
    pub arn: String,
    /// Role ARN for target invocation.
    pub role_arn: Option<String>,
    /// Input transformation: take a JSONPath subset of the event.
    pub input_path: Option<String>,
    /// Input transformation: provide a static JSON string.
    pub input: Option<String>,
    /// Input transformation: template with path variables.
    pub input_transformer: Option<InputTransformerState>,
    /// Retry policy.
    pub retry_policy: Option<RetryPolicyState>,
    /// Dead-letter config.
    pub dead_letter_config: Option<DeadLetterConfigState>,
}

#[derive(Debug, Clone)]
pub struct InputTransformerState {
    /// Map of variable names to JSONPath expressions.
    pub input_paths_map: HashMap<String, String>,
    /// Template string with <variable> placeholders.
    pub input_template: String,
}

#[derive(Debug, Clone)]
pub struct RetryPolicyState {
    pub maximum_retry_attempts: i32,
    pub maximum_event_age_in_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct DeadLetterConfigState {
    pub arn: Option<String>,
}

/// Configuration for target delivery, passed to the TargetDelivery trait.
#[derive(Debug, Clone)]
pub struct TargetConfig {
    pub input_path: Option<String>,
    pub input: Option<String>,
    pub input_transformer: Option<InputTransformerState>,
}
```

### 10.3 Event Bus Actor

Each event bus runs as an independent actor:

```rust
/// Commands sent to an event bus actor.
pub enum EventBusCommand {
    PutRule {
        input: PutRuleInput,
        reply: oneshot::Sender<Result<PutRuleOutput, EventsServiceError>>,
    },
    DeleteRule {
        name: String,
        reply: oneshot::Sender<Result<(), EventsServiceError>>,
    },
    DescribeRule {
        name: String,
        reply: oneshot::Sender<Result<DescribeRuleOutput, EventsServiceError>>,
    },
    ListRules {
        prefix: Option<String>,
        next_token: Option<String>,
        limit: Option<i32>,
        reply: oneshot::Sender<Result<ListRulesOutput, EventsServiceError>>,
    },
    EnableRule {
        name: String,
        reply: oneshot::Sender<Result<(), EventsServiceError>>,
    },
    DisableRule {
        name: String,
        reply: oneshot::Sender<Result<(), EventsServiceError>>,
    },
    PutTargets {
        rule_name: String,
        targets: Vec<Target>,
        reply: oneshot::Sender<Result<PutTargetsOutput, EventsServiceError>>,
    },
    RemoveTargets {
        rule_name: String,
        target_ids: Vec<String>,
        reply: oneshot::Sender<Result<RemoveTargetsOutput, EventsServiceError>>,
    },
    ListTargetsByRule {
        rule_name: String,
        next_token: Option<String>,
        limit: Option<i32>,
        reply: oneshot::Sender<Result<ListTargetsByRuleOutput, EventsServiceError>>,
    },
    PutEvents {
        entries: Vec<PutEventsRequestEntry>,
        reply: oneshot::Sender<Result<Vec<PutEventsResultEntry>, EventsServiceError>>,
    },
    GetTags {
        arn: String,
        reply: oneshot::Sender<Result<Vec<Tag>, EventsServiceError>>,
    },
    SetTags {
        arn: String,
        tags: Vec<Tag>,
        reply: oneshot::Sender<Result<(), EventsServiceError>>,
    },
    RemoveTags {
        arn: String,
        tag_keys: Vec<String>,
        reply: oneshot::Sender<Result<(), EventsServiceError>>,
    },
    Shutdown,
}

/// Per-event-bus actor.
pub struct EventBusActor {
    /// Bus state.
    state: EventBusState,
    /// Command channel receiver.
    commands: mpsc::Receiver<EventBusCommand>,
    /// Target delivery abstraction.
    delivery: Arc<dyn TargetDelivery>,
    /// Shutdown signal.
    shutdown: AtomicBool,
}

impl EventBusActor {
    pub async fn run(mut self) {
        loop {
            match self.commands.recv().await {
                Some(EventBusCommand::Shutdown) | None => break,
                Some(cmd) => self.handle_command(cmd).await,
            }
        }
    }

    async fn handle_command(&mut self, cmd: EventBusCommand) {
        match cmd {
            EventBusCommand::PutEvents { entries, reply } => {
                let results = self.route_events(entries).await;
                let _ = reply.send(results);
            }
            EventBusCommand::PutRule { input, reply } => {
                let result = self.put_rule(input);
                let _ = reply.send(result);
            }
            // ... other commands
            EventBusCommand::Shutdown => unreachable!(),
        }
    }
}
```

### 10.4 Event Routing (PutEvents)

When events arrive via `PutEvents`, the bus actor evaluates each event against all enabled rules:

```rust
impl EventBusActor {
    async fn route_events(
        &self,
        entries: Vec<PutEventsRequestEntry>,
    ) -> Result<Vec<PutEventsResultEntry>, EventsServiceError> {
        let mut results = Vec::with_capacity(entries.len());

        for entry in entries {
            let event_id = uuid::Uuid::new_v4().to_string();

            // Build the full event envelope.
            let event_envelope = build_event_envelope(&entry, &event_id, &self.state);

            // Parse the event envelope as JSON for pattern matching.
            let event_json: serde_json::Value = serde_json::from_str(&event_envelope)
                .unwrap_or(serde_json::Value::Null);

            // Evaluate against all enabled rules.
            let mut delivery_futures = Vec::new();

            for rule in self.state.rules.values() {
                if rule.state != RuleStateValue::Enabled {
                    continue;
                }

                // Check if the event matches the rule's pattern.
                let matched = match &rule.event_pattern {
                    Some(pattern) => crate::pattern::matches(pattern, &event_json),
                    None => false, // Scheduled rules do not match events
                };

                if matched {
                    // Deliver to all targets of this rule.
                    for target in rule.targets.values() {
                        let target_config = TargetConfig {
                            input_path: target.input_path.clone(),
                            input: target.input.clone(),
                            input_transformer: target.input_transformer.clone(),
                        };

                        let delivery = Arc::clone(&self.delivery);
                        let target_arn = target.arn.clone();
                        let event_str = apply_input_transform(
                            &event_envelope,
                            &target_config,
                        );

                        delivery_futures.push(async move {
                            if let Err(e) = delivery
                                .deliver(&target_arn, &event_str, &target_config)
                                .await
                            {
                                tracing::warn!(
                                    target_arn = %target_arn,
                                    error = %e,
                                    "Failed to deliver event to target"
                                );
                            }
                        });
                    }
                }
            }

            // Fire all deliveries concurrently.
            futures::future::join_all(delivery_futures).await;

            results.push(PutEventsResultEntry {
                event_id: Some(event_id),
                error_code: None,
                error_message: None,
            });
        }

        Ok(results)
    }
}

/// Build the canonical event envelope that EventBridge delivers.
fn build_event_envelope(
    entry: &PutEventsRequestEntry,
    event_id: &str,
    bus: &EventBusState,
) -> String {
    let time = entry.time.clone().unwrap_or_else(|| {
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
    });

    let envelope = serde_json::json!({
        "version": "0",
        "id": event_id,
        "source": entry.source.as_deref().unwrap_or(""),
        "account": bus.arn.split(':').nth(4).unwrap_or("000000000000"),
        "time": time,
        "region": bus.arn.split(':').nth(3).unwrap_or("us-east-1"),
        "resources": entry.resources.as_deref().unwrap_or(&[]),
        "detail-type": entry.detail_type.as_deref().unwrap_or(""),
        "detail": entry.detail.as_deref()
            .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
            .unwrap_or(serde_json::Value::Object(Default::default())),
    });

    serde_json::to_string(&envelope).unwrap_or_default()
}
```

### 10.5 Input Path and Input Transformer

Targets can transform the event before delivery:

```rust
/// Apply input transformation to an event before delivery.
///
/// Priority order (mutually exclusive):
/// 1. `Input` -- static string replaces the entire event
/// 2. `InputPath` -- JSONPath selects a subset of the event
/// 3. `InputTransformer` -- template with variable substitution
/// 4. None -- deliver the full event envelope
fn apply_input_transform(event_json: &str, config: &TargetConfig) -> String {
    // Static input overrides everything.
    if let Some(ref input) = config.input {
        return input.clone();
    }

    // InputPath selects a JSON subset.
    if let Some(ref path) = config.input_path {
        return apply_json_path(event_json, path);
    }

    // InputTransformer: resolve variables and substitute into template.
    if let Some(ref transformer) = config.input_transformer {
        return apply_input_transformer(event_json, transformer);
    }

    // Default: full event envelope.
    event_json.to_string()
}

/// Simple JSONPath resolution ($.detail, $.source, etc.)
///
/// EventBridge supports a limited JSONPath subset:
/// - `$` = root
/// - `.key` = object member access
fn apply_json_path(event_json: &str, path: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(event_json) {
        Ok(v) => v,
        Err(_) => return event_json.to_string(),
    };

    let segments: Vec<&str> = path
        .strip_prefix("$.")
        .unwrap_or(path.strip_prefix('$').unwrap_or(path))
        .split('.')
        .filter(|s| !s.is_empty())
        .collect();

    let mut current = &parsed;
    for seg in &segments {
        match current.get(*seg) {
            Some(v) => current = v,
            None => return "null".to_string(),
        }
    }

    serde_json::to_string(current).unwrap_or_else(|_| "null".to_string())
}
```

---

## 11. Core Business Logic

### 11.1 Provider (EventBusManager)

```rust
/// Main EventBridge provider. Manages all event bus actors.
pub struct RustackEvents {
    /// Event bus registry: bus_name -> EventBusHandle.
    buses: DashMap<String, EventBusHandle>,
    /// Reverse index: rule_arn -> (bus_name, rule_name).
    rule_index: DashMap<String, (String, String)>,
    /// Reverse index: target_arn -> Vec<(bus_name, rule_name)>.
    target_to_rules: DashMap<String, Vec<(String, String)>>,
    /// Configuration.
    config: Arc<EventsConfig>,
    /// Target delivery implementation.
    delivery: Arc<dyn TargetDelivery>,
}

/// Handle to a running event bus actor.
pub struct EventBusHandle {
    /// Channel to send commands to the event bus actor.
    sender: mpsc::Sender<EventBusCommand>,
    /// Bus metadata (read-only after creation).
    metadata: EventBusMetadata,
    /// Actor task join handle.
    task: tokio::task::JoinHandle<()>,
}

pub struct EventBusMetadata {
    pub name: String,
    pub arn: String,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

### 11.2 Operations Grouped by Category

#### Event Bus Management (5 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreateEventBus` | 0 | Medium | Spawn bus actor, validate name, handle default bus |
| `DeleteEventBus` | 0 | Low | Send Shutdown to actor, remove from registry. Cannot delete default bus. |
| `DescribeEventBus` | 0 | Low | Return bus metadata, policy, rule count |
| `ListEventBuses` | 0 | Low | Filter by name prefix, paginate |
| `UpdateEventBus` | 2 | Low | Update description, KMS key (metadata only) |

#### Rule Management (6 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `PutRule` | 0 | High | Parse and validate event pattern, create/update rule, return RuleArn |
| `DeleteRule` | 0 | Low | Must have no targets. Remove rule from bus. |
| `DescribeRule` | 0 | Low | Return rule metadata, pattern, schedule, state |
| `ListRules` | 0 | Low | Filter by name prefix, paginate |
| `EnableRule` | 0 | Low | Set state to ENABLED |
| `DisableRule` | 0 | Low | Set state to DISABLED |

#### Target Management (4 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `PutTargets` | 0 | Medium | Add/update targets on a rule (max 5 per rule). Validate target config. |
| `RemoveTargets` | 0 | Low | Remove specified target IDs from a rule |
| `ListTargetsByRule` | 0 | Low | Return targets for a rule, paginate |
| `ListRuleNamesByTarget` | 1 | Medium | Reverse lookup: find all rules targeting a given ARN |

#### Event Operations (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `PutEvents` | 0 | High | Core operation. Route events through pattern matcher to targets. Max 10 entries per call. |
| `TestEventPattern` | 0 | High | Validate pattern against event. No delivery. Critical for dev tooling. |

#### Permission Management (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `PutPermission` | 1 | Low | Store permission policy on bus, do not enforce |
| `RemovePermission` | 1 | Low | Remove permission statement |

#### Tagging (3 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `TagResource` | 1 | Low | Add/update tags on bus or rule |
| `UntagResource` | 1 | Low | Remove tag keys |
| `ListTagsForResource` | 1 | Low | Return tags for a resource ARN |

#### Archive/Replay (7 operations -- stubs)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreateArchive` | 3 | Low | Stub: store metadata, return ArchiveArn |
| `DeleteArchive` | 3 | Low | Stub: remove metadata |
| `DescribeArchive` | 3 | Low | Stub: return metadata |
| `ListArchives` | 3 | Low | Stub: list stored metadata |
| `UpdateArchive` | 3 | Low | Stub: update metadata |
| `StartReplay` | 3 | Low | Stub: return ReplayArn, state=STARTING |
| `CancelReplay` | 3 | Low | Stub: update state |

#### API Destinations/Connections (11 operations -- stubs)

| Operation | Phase | Notes |
|-----------|-------|-------|
| `CreateApiDestination` | 3 | Store metadata only |
| `DeleteApiDestination` | 3 | Remove metadata |
| `DescribeApiDestination` | 3 | Return metadata |
| `ListApiDestinations` | 3 | List metadata |
| `UpdateApiDestination` | 3 | Update metadata |
| `CreateConnection` | 3 | Store metadata only |
| `DeleteConnection` | 3 | Remove metadata |
| `DescribeConnection` | 3 | Return metadata |
| `ListConnections` | 3 | List metadata |
| `UpdateConnection` | 3 | Update metadata |
| `DeauthorizeConnection` | 3 | Update state |

#### Partner/Endpoints (11 operations -- stubs)

All partner event source and endpoint operations are stubs that store metadata but perform no real cross-account or cross-region actions.

### 11.3 CreateEventBus Logic

```rust
impl RustackEvents {
    pub async fn create_event_bus(
        &self,
        input: CreateEventBusInput,
    ) -> Result<CreateEventBusOutput, EventsServiceError> {
        let bus_name = input.name.as_str();

        // Validate bus name: 1-256 chars, [/.\-_A-Za-z0-9]
        validate_event_bus_name(bus_name)?;

        // Cannot create "default" explicitly.
        if bus_name == "default" {
            return Err(EventsServiceError::ResourceAlreadyExists {
                message: "Event bus default already exists.".to_string(),
            });
        }

        // Check for duplicate.
        if self.buses.contains_key(bus_name) {
            return Err(EventsServiceError::ResourceAlreadyExists {
                message: format!("Event bus {bus_name} already exists."),
            });
        }

        let arn = format!(
            "arn:aws:events:{}:{}:event-bus/{}",
            self.config.region, self.config.account_id, bus_name
        );

        // Spawn bus actor.
        let (sender, receiver) = mpsc::channel(256);
        let actor = EventBusActor::new(
            EventBusState {
                name: bus_name.to_string(),
                arn: arn.clone(),
                description: input.description,
                policy: None,
                tags: input.tags.as_ref()
                    .map(|tags| tags.iter()
                        .map(|t| (t.key.clone(), t.value.clone()))
                        .collect())
                    .unwrap_or_default(),
                rules: HashMap::new(),
                created_at: chrono::Utc::now(),
                last_modified_at: chrono::Utc::now(),
            },
            receiver,
            Arc::clone(&self.delivery),
        );
        let task = tokio::spawn(actor.run());

        let handle = EventBusHandle {
            sender,
            metadata: EventBusMetadata {
                name: bus_name.to_string(),
                arn: arn.clone(),
                description: input.description.clone(),
                created_at: chrono::Utc::now(),
            },
            task,
        };
        self.buses.insert(bus_name.to_string(), handle);

        Ok(CreateEventBusOutput {
            event_bus_arn: Some(arn),
            ..Default::default()
        })
    }
}
```

### 11.4 PutRule Logic

```rust
impl EventBusActor {
    fn put_rule(
        &mut self,
        input: PutRuleInput,
    ) -> Result<PutRuleOutput, EventsServiceError> {
        let rule_name = &input.name;

        // Validate rule name.
        validate_rule_name(rule_name)?;

        // Must have at least one of EventPattern or ScheduleExpression.
        if input.event_pattern.is_none() && input.schedule_expression.is_none() {
            return Err(EventsServiceError::ValidationError {
                message: "Either EventPattern or ScheduleExpression must be provided.".to_string(),
            });
        }

        // Parse and validate event pattern.
        let parsed_pattern = if let Some(ref pattern_json) = input.event_pattern {
            Some(crate::pattern::parse_event_pattern(pattern_json)
                .map_err(|e| EventsServiceError::InvalidEventPattern {
                    message: e.to_string(),
                })?)
        } else {
            None
        };

        // Validate schedule expression (syntax only, do not execute).
        if let Some(ref expr) = input.schedule_expression {
            validate_schedule_expression(expr)?;
        }

        let arn = format!(
            "arn:aws:events:{}:{}:rule/{}/{}",
            self.state.arn.split(':').nth(3).unwrap_or("us-east-1"),
            self.state.arn.split(':').nth(4).unwrap_or("000000000000"),
            self.state.name,
            rule_name,
        );

        let rule = RuleState {
            name: rule_name.clone(),
            arn: arn.clone(),
            description: input.description,
            event_pattern: parsed_pattern,
            event_pattern_json: input.event_pattern,
            schedule_expression: input.schedule_expression,
            state: input.state
                .as_deref()
                .map(|s| match s {
                    "DISABLED" => RuleStateValue::Disabled,
                    _ => RuleStateValue::Enabled,
                })
                .unwrap_or(RuleStateValue::Enabled),
            role_arn: input.role_arn,
            managed_by: None,
            event_bus_name: self.state.name.clone(),
            tags: input.tags.as_ref()
                .map(|tags| tags.iter()
                    .map(|t| (t.key.clone(), t.value.clone()))
                    .collect())
                .unwrap_or_default(),
            targets: self.state.rules
                .get(rule_name)
                .map(|existing| existing.targets.clone())
                .unwrap_or_default(),
            created_at: self.state.rules
                .get(rule_name)
                .map(|existing| existing.created_at)
                .unwrap_or_else(chrono::Utc::now),
        };

        self.state.rules.insert(rule_name.clone(), rule);
        self.state.last_modified_at = chrono::Utc::now();

        Ok(PutRuleOutput {
            rule_arn: Some(arn),
        })
    }
}
```

### 11.5 TestEventPattern Logic

```rust
impl RustackEvents {
    pub fn test_event_pattern(
        &self,
        input: TestEventPatternInput,
    ) -> Result<TestEventPatternOutput, EventsServiceError> {
        let event_pattern_json = &input.event_pattern;
        let event_json = &input.event;

        // Parse the event pattern.
        let pattern = crate::pattern::parse_event_pattern(event_pattern_json)
            .map_err(|e| EventsServiceError::InvalidEventPattern {
                message: e.to_string(),
            })?;

        // Parse the event.
        let event: serde_json::Value = serde_json::from_str(event_json)
            .map_err(|e| EventsServiceError::InvalidEventPattern {
                message: format!("Event is not valid JSON: {e}"),
            })?;

        // Event must be an object.
        if !event.is_object() {
            return Err(EventsServiceError::InvalidEventPattern {
                message: "Event must be a JSON object.".to_string(),
            });
        }

        // Match the pattern against the event.
        let result = crate::pattern::matches(&pattern, &event);

        Ok(TestEventPatternOutput {
            result: Some(result),
        })
    }
}
```

### 11.6 Default Event Bus

On startup, the provider creates a "default" event bus automatically:

```rust
impl RustackEvents {
    pub fn new(config: EventsConfig, delivery: Arc<dyn TargetDelivery>) -> Self {
        let provider = Self {
            buses: DashMap::new(),
            rule_index: DashMap::new(),
            target_to_rules: DashMap::new(),
            config: Arc::new(config),
            delivery,
        };

        // Create the default event bus.
        provider.create_default_bus();

        provider
    }

    fn create_default_bus(&self) {
        let arn = format!(
            "arn:aws:events:{}:{}:event-bus/default",
            self.config.region, self.config.account_id
        );

        let (sender, receiver) = mpsc::channel(256);
        let actor = EventBusActor::new(
            EventBusState {
                name: "default".to_string(),
                arn: arn.clone(),
                description: None,
                policy: None,
                tags: HashMap::new(),
                rules: HashMap::new(),
                created_at: chrono::Utc::now(),
                last_modified_at: chrono::Utc::now(),
            },
            receiver,
            Arc::clone(&self.delivery),
        );
        let task = tokio::spawn(actor.run());

        self.buses.insert("default".to_string(), EventBusHandle {
            sender,
            metadata: EventBusMetadata {
                name: "default".to_string(),
                arn,
                description: None,
                created_at: chrono::Utc::now(),
            },
            task,
        });
    }
}
```

---

## 12. Error Handling

### 12.1 EventBridge Error Codes

```rust
/// Domain-level errors for EventBridge business logic.
#[derive(Debug, thiserror::Error)]
pub enum EventsServiceError {
    #[error("Event bus {name} does not exist")]
    ResourceNotFound { name: String },

    #[error("{message}")]
    ResourceAlreadyExists { message: String },

    #[error("Invalid event pattern: {message}")]
    InvalidEventPattern { message: String },

    #[error("Validation error: {message}")]
    ValidationError { message: String },

    #[error("Rule {name} has targets. Remove targets before deleting the rule.")]
    RuleHasTargets { name: String },

    #[error("Cannot delete the default event bus")]
    CannotDeleteDefaultBus,

    #[error("Too many targets for rule {rule_name}. Maximum is 5.")]
    LimitExceeded { rule_name: String },

    #[error("Concurrent modification: {message}")]
    ConcurrentModification { message: String },

    #[error("Internal error: {message}")]
    Internal { message: String },
}
```

### 12.2 Error Type Mapping

```rust
impl EventsServiceError {
    /// JSON `__type` field value.
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::ResourceNotFound { .. } => "ResourceNotFoundException",
            Self::ResourceAlreadyExists { .. } => "ResourceAlreadyExistsException",
            Self::InvalidEventPattern { .. } => "InvalidEventPatternException",
            Self::ValidationError { .. } => "ValidationException",
            Self::RuleHasTargets { .. } => "ValidationException",
            Self::CannotDeleteDefaultBus => "ValidationException",
            Self::LimitExceeded { .. } => "LimitExceededException",
            Self::ConcurrentModification { .. } => "ConcurrentModificationException",
            Self::Internal { .. } => "InternalException",
        }
    }

    /// HTTP status code.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::ResourceNotFound { .. } => 400,
            Self::ResourceAlreadyExists { .. } => 400,
            Self::InvalidEventPattern { .. } => 400,
            Self::ValidationError { .. } => 400,
            Self::RuleHasTargets { .. } => 400,
            Self::CannotDeleteDefaultBus => 400,
            Self::LimitExceeded { .. } => 400,
            Self::ConcurrentModification { .. } => 400,
            Self::Internal { .. } => 500,
        }
    }
}
```

### 12.3 Error Response Formatting

```rust
/// Format an EventBridge error response (awsJson1.1).
fn error_response(error: &EventsServiceError) -> http::Response<Bytes> {
    let body = serde_json::json!({
        "__type": error.error_type(),
        "message": error.to_string(),
    });

    http::Response::builder()
        .status(error.status_code())
        .header("content-type", "application/x-amz-json-1.1")
        .body(Bytes::from(serde_json::to_vec(&body).unwrap_or_default()))
        .unwrap_or_else(|_| {
            http::Response::builder()
                .status(500)
                .body(Bytes::new())
                .unwrap()
        })
}
```

---

## 13. Server Integration

### 13.1 Events ServiceRouter

```rust
#[cfg(feature = "events")]
mod events_router {
    use super::{GatewayBody, ServiceRouter};

    /// Matches requests whose `X-Amz-Target` header starts with `AWSEvents.`.
    pub struct EventsServiceRouter<H: EventsHandler> {
        inner: EventsHttpService<H>,
    }

    impl<H: EventsHandler> EventsServiceRouter<H> {
        pub fn new(inner: EventsHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: EventsHandler> ServiceRouter for EventsServiceRouter<H> {
        fn name(&self) -> &'static str { "events" }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("AWSEvents."))
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

### 13.2 Feature Gate

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "events"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http", "dep:rustack-s3-model"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
sqs = ["dep:rustack-sqs-core", "dep:rustack-sqs-http"]
ssm = ["dep:rustack-ssm-core", "dep:rustack-ssm-http"]
events = ["dep:rustack-events-core", "dep:rustack-events-http"]
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

    #[cfg(feature = "events")]
    services.push(Box::new(EventsServiceRouter::new(events_service)));

    #[cfg(feature = "s3")]
    services.push(Box::new(S3ServiceRouter::new(s3_service))); // catch-all, must be last

    GatewayService::new(services)
}
```

### 13.4 Target Delivery Wiring

The server binary wires EventBridge's `TargetDelivery` trait to the SQS service:

```rust
/// Server-level target delivery that routes to in-process services.
pub struct LocalTargetDelivery {
    /// SQS provider for queue targets.
    sqs: Arc<RustackSqs>,
}

#[async_trait]
impl TargetDelivery for LocalTargetDelivery {
    async fn deliver(
        &self,
        target_arn: &str,
        event_json: &str,
        _target_config: &TargetConfig,
    ) -> Result<(), DeliveryError> {
        if target_arn.contains(":sqs:") {
            // Extract queue name from ARN.
            let queue_name = target_arn
                .rsplit(':')
                .next()
                .ok_or_else(|| DeliveryError::InvalidArn(target_arn.to_string()))?;

            // Send message to SQS queue via internal channel.
            self.sqs.send_message_internal(queue_name, event_json).await
                .map_err(|e| DeliveryError::TargetError(e.to_string()))?;

            Ok(())
        } else {
            // Unsupported target type -- log and ignore.
            tracing::debug!(
                target_arn = %target_arn,
                "Unsupported target type, event not delivered"
            );
            Ok(())
        }
    }
}
```

### 13.5 Configuration

```rust
/// EventBridge configuration.
pub struct EventsConfig {
    pub skip_signature_validation: bool,
    pub default_region: String,
    pub account_id: String,
    pub host: String,
    pub port: u16,
}

impl EventsConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("EVENTS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            host: env_str("GATEWAY_HOST", "localhost"),
            port: env_u16("GATEWAY_PORT", 4566),
        }
    }
}
```

### 13.6 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "running",
        "dynamodb": "running",
        "sqs": "running",
        "ssm": "running",
        "events": "running"
    }
}
```

---

## 14. Testing Strategy

### 14.1 Unit Tests

Each module tested in isolation:

- **Pattern parser**: Test parsing all 10 operator types, nested patterns, $or, validation errors
- **Pattern matcher**: Test each operator against matching and non-matching values
- **Wildcard matching**: Test `*` matching, escape sequences (`\*`, `\\`), edge cases (empty, consecutive stars)
- **Numeric matching**: Test all 5 comparison operators, ranges, float precision, integer-float comparison
- **Anything-but**: Test all variants (strings, numbers, prefix, suffix, ignorecase, wildcard)
- **CIDR matching**: Test IPv4 and IPv6 CIDR blocks, boundary IPs
- **Event routing**: Test event-to-rule matching with multiple rules, enabled/disabled rules
- **Input transformation**: Test InputPath, Input, InputTransformer separately
- **Event envelope**: Test canonical event format with all fields

### 14.2 Integration Tests with aws-sdk-eventbridge

```rust
// tests/integration/events_tests.rs
#[tokio::test]
#[ignore]
async fn test_events_bus_lifecycle() {
    let client = aws_sdk_eventbridge::Client::new(&config);

    // Create custom bus
    let create = client.create_event_bus()
        .name("test-bus")
        .send().await.unwrap();
    assert!(create.event_bus_arn().is_some());

    // List buses (should include default + test-bus)
    let list = client.list_event_buses().send().await.unwrap();
    assert!(list.event_buses().len() >= 2);

    // Delete custom bus
    client.delete_event_bus().name("test-bus").send().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_events_rule_with_sqs_target() {
    let events = aws_sdk_eventbridge::Client::new(&config);
    let sqs = aws_sdk_sqs::Client::new(&config);

    // Create SQS queue
    let queue_url = sqs.create_queue()
        .queue_name("events-target")
        .send().await.unwrap()
        .queue_url().unwrap().to_string();

    // Create rule with pattern
    events.put_rule()
        .name("test-rule")
        .event_pattern(r#"{"source": ["my.app"]}"#)
        .send().await.unwrap();

    // Add SQS target
    events.put_targets()
        .rule("test-rule")
        .targets(Target::builder()
            .id("sqs-target")
            .arn("arn:aws:sqs:us-east-1:000000000000:events-target")
            .build())
        .send().await.unwrap();

    // Put event
    events.put_events()
        .entries(PutEventsRequestEntry::builder()
            .source("my.app")
            .detail_type("TestEvent")
            .detail(r#"{"key": "value"}"#)
            .build())
        .send().await.unwrap();

    // Verify SQS received the event
    let recv = sqs.receive_message()
        .queue_url(&queue_url)
        .wait_time_seconds(5)
        .send().await.unwrap();
    assert_eq!(recv.messages().len(), 1);
}

#[tokio::test]
#[ignore]
async fn test_event_pattern_matching() {
    let client = aws_sdk_eventbridge::Client::new(&config);

    // Test various patterns
    let result = client.test_event_pattern()
        .event(r#"{"source":"my.app","detail-type":"test","detail":{"status":"active"}}"#)
        .event_pattern(r#"{"detail":{"status":["active"]}}"#)
        .send().await.unwrap();
    assert!(result.result());

    // Test negative match
    let result = client.test_event_pattern()
        .event(r#"{"source":"my.app","detail-type":"test","detail":{"status":"inactive"}}"#)
        .event_pattern(r#"{"detail":{"status":["active"]}}"#)
        .send().await.unwrap();
    assert!(!result.result());
}
```

### 14.3 Third-Party Test Suites

#### 14.3.1 LocalStack Events Test Suite (Primary)

The most comprehensive open-source EventBridge test suite. Already vendored at `vendors/localstack/tests/aws/services/events/`.

**Test files:**

| File | Focus | Test Count (approx) |
|------|-------|---------------------|
| `test_events.py` | Core: PutEvents, event bus CRUD, event delivery | ~30 |
| `test_events_patterns.py` | Pattern matching: 157 parameterized templates + TestEventPattern | ~170 |
| `test_events_targets.py` | Target delivery: SQS, SNS, Lambda, Kinesis, API destinations | ~20 |
| `test_events_inputs.py` | InputPath and InputTransformer | ~15 |
| `test_events_tags.py` | TagResource, UntagResource, ListTagsForResource | ~10 |
| `test_events_schedule.py` | Scheduled rules (cron/rate) | ~10 |
| `test_archive_and_replay.py` | Archive and Replay operations | ~15 |
| `test_events_cross_account_region.py` | Cross-account/region (non-goal) | ~5 |
| `test_api_destinations_and_connection.py` | API destinations and connections | ~10 |

The **event pattern template directory** contains **157 test cases** in JSON5 format covering:
- Exact match, boolean, null, empty string, arrays
- Prefix, suffix (with/without ignore-case)
- Anything-but (string, number, prefix, suffix, ignorecase, list variants)
- Numeric (comparisons, ranges, float/int, null, string type mismatch)
- Wildcard (simple, repeating, non-repeating, complex, escape)
- Exists (true/false, nested, DynamoDB-style)
- $or (with exists, numeric, anything-but)
- Dot-joining in event and pattern keys
- Key case sensitivity
- Operator case sensitivity (case-sensitive operators)
- Complex multi-key patterns

Adaptation strategy: same approach as SQS -- run the Python test suite against Rustack's EventBridge endpoint, track pass/fail counts, progressively fix failures.

```makefile
test-events-localstack-patterns:
	@cd vendors/localstack && python -m pytest \
		tests/aws/services/events/test_events_patterns.py \
		--endpoint-url=http://localhost:4566 -v

test-events-localstack:
	@cd vendors/localstack && python -m pytest \
		tests/aws/services/events/ \
		--endpoint-url=http://localhost:4566 -v
```

#### 14.3.2 AWS Event Ruler Test Suite (Pattern Matching Validation)

- **Repository**: https://github.com/aws/event-ruler
- **Language**: Java
- **Coverage**: Exhaustive pattern matching tests for all operators
- **Usage**: Reference implementation. Extract test vectors (input event + pattern + expected result) and translate to Rust unit tests for the pattern matching engine. Event Ruler's test suite is the authoritative source for pattern matching semantics.

Key test files to extract vectors from:
- `RulerTest.java` -- core matching tests
- `GenericMachineTest.java` -- edge cases in finite state machine

#### 14.3.3 quamina-rs Benchmark Suite

- **Repository**: https://github.com/baldawarishi/quamina-rs
- **Language**: Rust
- **Usage**: If we adopt quamina-rs as a dependency or port its approach, its benchmark suite provides performance baselines. The library supports exact, prefix, suffix, wildcard, numeric, anything-but, cidr, equals-ignore-case, exists matching.
- **Status**: Not published to crates.io. Could vendor or use as reference for our own implementation.

#### 14.3.4 evb-cli Pattern Tests

- **Repository**: https://github.com/ljacobsson/evb-cli (also https://github.com/mhlabs/evb-cli)
- **Language**: Node.js
- **Coverage**: Pattern generation and debugging tool for EventBridge
- **Usage**: Can be pointed at Rustack endpoint for manual testing and pattern debugging

#### 14.3.5 sls-test-tools

- **Repository**: https://github.com/aleios-cloud/sls-test-tools
- **Language**: TypeScript/Jest
- **Coverage**: Custom Jest assertions for Serverless integration testing, including EventBridge event verification via SQS targets
- **Usage**: Run sls-test-tools test suites against Rustack to validate end-to-end event flow from PutEvents through pattern matching to SQS delivery

#### 14.3.6 AWS CLI Smoke Tests

```bash
#!/bin/bash
# EventBridge CLI smoke test
ENDPOINT="--endpoint-url http://localhost:4566"

# Create custom bus
aws events create-event-bus $ENDPOINT --name test-bus

# Put rule
aws events put-rule $ENDPOINT \
    --name test-rule \
    --event-bus-name test-bus \
    --event-pattern '{"source":["my.app"]}'

# Create SQS queue for target
QUEUE_URL=$(aws sqs create-queue $ENDPOINT --queue-name events-target \
    --query QueueUrl --output text)

# Put target
aws events put-targets $ENDPOINT \
    --rule test-rule \
    --event-bus-name test-bus \
    --targets "Id=sqs-1,Arn=arn:aws:sqs:us-east-1:000000000000:events-target"

# Put event
aws events put-events $ENDPOINT --entries \
    '[{"Source":"my.app","DetailType":"Test","Detail":"{\"key\":\"value\"}","EventBusName":"test-bus"}]'

# Verify delivery
aws sqs receive-message $ENDPOINT --queue-url "$QUEUE_URL" --wait-time-seconds 5

# Test event pattern
aws events test-event-pattern $ENDPOINT \
    --event '{"source":"my.app","detail-type":"test","detail":{"status":"active"}}' \
    --event-pattern '{"detail":{"status":["active"]}}'

# Cleanup
aws events remove-targets $ENDPOINT --rule test-rule --event-bus-name test-bus --ids sqs-1
aws events delete-rule $ENDPOINT --name test-rule --event-bus-name test-bus
aws events delete-event-bus $ENDPOINT --name test-bus
aws sqs delete-queue $ENDPOINT --queue-url "$QUEUE_URL"
```

### 14.4 Makefile Targets

```makefile
test-events: test-events-unit test-events-integration

test-events-unit:
	@cargo test -p rustack-events-model -p rustack-events-core -p rustack-events-http

test-events-integration:
	@cargo test -p integration-tests -- events --ignored

test-events-patterns:
	@cargo test -p rustack-events-core -- pattern

test-events-cli:
	@./tests/events-cli-smoke.sh

test-events-localstack:
	@cd vendors/localstack && python -m pytest tests/aws/services/events/ -v
```

---

## 15. Phased Implementation Plan

### Phase 0: MVP (22 Operations -- Event Bus, Rules, Targets, PutEvents, TestEventPattern)

**Goal**: A working EventBridge that can create buses, define rules with patterns, add SQS targets, route events, and test patterns. This covers the core CDK/Serverless Framework/Terraform use case.
**Estimated scope**: ~6,000-8,000 lines of Rust code across 3 new crates.

#### Step 0.1: Codegen Extension
- Download EventBridge Smithy model JSON from `aws/api-models-aws`
- Generate `rustack-events-model` crate (operations enum, input/output structs, error codes)
- Generate serde derives with appropriate field naming

#### Step 0.2: Pattern Matching Engine
- Implement `PatternParser` (JSON -> `EventPattern` tree)
- Implement `PatternMatcher` (evaluate `EventPattern` against `serde_json::Value`)
- Implement all 10 comparison operators: exact, prefix, suffix, equals-ignore-case, wildcard, anything-but (all variants), numeric, exists, cidr
- Implement $or logical combinator
- Implement array field matching (any element in array matches)
- Write exhaustive unit tests (~200+ test cases extracted from LocalStack templates)

#### Step 0.3: HTTP Layer
- Implement `EventsRouter` (`X-Amz-Target: AWSEvents.*` dispatch)
- Implement `EventsHttpService` (hyper Service)
- Implement JSON request deserialization / response serialization
- Implement error formatting (awsJson1.1 style)

#### Step 0.4: Storage and Bus Actor
- Implement `EventBusState`, `RuleState`, `TargetState` data structures
- Implement `EventBusActor` with command channel and event loop
- Implement `EventBusHandle` and `EventBusMetadata`
- Create default event bus on startup

#### Step 0.5: Core Operations (22 ops)
- Event bus: `CreateEventBus`, `DeleteEventBus`, `DescribeEventBus`, `ListEventBuses` (4)
- Rules: `PutRule`, `DeleteRule`, `DescribeRule`, `ListRules`, `EnableRule`, `DisableRule` (6)
- Targets: `PutTargets`, `RemoveTargets`, `ListTargetsByRule` (3)
- Events: `PutEvents`, `TestEventPattern` (2)
- Stubs for remaining 7 operations that CDK/Terraform may call

#### Step 0.6: Target Delivery (SQS)
- Implement `TargetDelivery` trait
- Implement `LocalTargetDelivery` for SQS targets in the server binary
- Implement event envelope construction (version, id, source, account, time, region, detail-type, detail)
- Wire EventBridge -> SQS delivery via internal channel

#### Step 0.7: Server Integration
- Implement `EventsServiceRouter` with gateway registration
- Add `events` cargo feature gate
- Register Events before S3 in gateway
- Update health endpoint

#### Step 0.8: Testing
- Unit tests for pattern matching engine (all operators, 157 LocalStack templates)
- Integration tests with `aws-sdk-eventbridge`
- End-to-end test: PutRule -> PutTargets (SQS) -> PutEvents -> ReceiveMessage
- CLI smoke tests
- Update Makefile

### Phase 1: Tags, Permissions, ListRuleNamesByTarget, InputPath

**Goal**: Terraform/CDK full compatibility and basic input transformation.

- `TagResource` / `UntagResource` / `ListTagsForResource` (3)
- `PutPermission` / `RemovePermission` (2)
- `ListRuleNamesByTarget` (reverse lookup)
- `InputPath` -- select a JSON subset of the event for target delivery
- `Input` -- static input string for target delivery

### Phase 2: Input Transformer, UpdateEventBus, Advanced Targets

**Goal**: Full input transformation and expanded target support.

- `InputTransformer` -- template with variable substitution from InputPathsMap
- `UpdateEventBus` -- update description and metadata
- CloudWatch Logs target delivery (if implemented as a Rustack service)
- Dead-letter config on targets (store, do not enforce for unsupported targets)
- Retry policy on targets (store configuration)

### Phase 3: Archive/Replay Stubs, API Destinations Stubs, Scheduled Rules

**Goal**: Feature completeness for the full API surface. Stubs for rarely-used operations.

- Archive operations: `CreateArchive`, `DeleteArchive`, `DescribeArchive`, `ListArchives`, `UpdateArchive` (metadata storage only)
- Replay operations: `StartReplay`, `CancelReplay`, `DescribeReplay`, `ListReplays` (metadata storage only)
- API Destination operations: full CRUD stubs (11 operations)
- Partner event source operations: full stubs (9 operations)
- Endpoint operations: full stubs (5 operations)
- **Scheduled rules**: Parse cron/rate expressions and execute rules on timers. This requires a background timer task in the bus actor that periodically evaluates scheduled rules and delivers synthetic events to targets.

---

## 16. Risk Analysis

### 16.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Pattern matching semantics diverge from AWS | High | High | Use Event Ruler test vectors as reference; run 157 LocalStack pattern templates; iteratively fix discrepancies |
| Anything-but with null values edge cases | High | Medium | AWS treats null specially in anything-but (null matches anything-but-string). Test exhaustively against AWS behavior. |
| Wildcard escape sequences (`\*`, `\\`) | Medium | Medium | Implement escape parsing carefully; test against LocalStack wildcard templates |
| Numeric precision (IEEE 754 float vs integer) | Medium | Medium | Use f64 throughout; test integer-float comparisons (e.g., `100` vs `100.0`); match Event Ruler behavior |
| Event envelope format differences | Medium | High | Test canonical event format against AWS; ensure `version`, `id`, `source`, `account`, `time`, `region`, `resources`, `detail-type`, `detail` all present |
| SQS internal delivery reliability | Medium | Medium | Use bounded channels with backpressure; log delivery failures; test concurrent event delivery |
| Concurrent PutRule and PutEvents race conditions | Medium | Medium | Actor model ensures serialized access within a bus; no data races possible |
| InputTransformer template parsing | Low | Medium | Defer to Phase 2; template syntax is simple (`<variable>` substitution) |
| Pattern matching performance with many rules | Low | Low | Sufficient for local dev with dozens of rules; can upgrade to automaton-based matching if needed |

### 16.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Users expect Lambda target invocation | High | Medium | Document as non-goal; Lambda targets accepted but not invoked; log when events match but target is unsupported |
| CDK/Terraform creates archive/replay resources | Medium | Medium | Implement stubs in Phase 3 that accept API calls without real functionality |
| Scheduled rule execution required for CI | Medium | Medium | Defer to Phase 3; most local dev uses event-driven rules not scheduled |
| Event pattern language evolves with new operators | Low | Low | Design parser to reject unknown operators gracefully; add new operators incrementally |

### 16.3 Behavioral Differences

Our implementation will intentionally differ from LocalStack in some areas:

| Behavior | LocalStack | Rustack | Justification |
|----------|------------|-----------|---------------|
| Pattern matching backend | Java Event Ruler (via subprocess or in-process) | Native Rust engine | Same semantics, different implementation |
| Event ID format | UUID v4 | UUID v4 | Match AWS behavior |
| Default bus creation | Created on first access | Created on startup | Simplifies implementation; always available |
| Unsupported target types | Various error behaviors | Accept target, log warning on delivery | Graceful degradation for local dev |
| Event ordering | Best-effort | Best-effort within a PutEvents batch | Match AWS behavior |
| PutEvents batch limit | 10 entries | 10 entries | Match AWS limit |
| Event size limit | 256 KB per entry | 256 KB per entry | Match AWS limit |
| Target limit per rule | 5 targets | 5 targets | Match AWS limit |
| Rule limit per bus | 300 rules | No enforced limit | Simplification for local dev |

---
