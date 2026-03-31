# Rustack DynamoDB: Native Rust Implementation Design

**Date:** 2026-02-28
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [DynamoDB API Research](../docs/research/dynamodb-api-research.md)
**Scope:** Add native DynamoDB support to Rustack using the same Smithy-based codegen approach as S3, with a fully Rust-native storage engine (no DynamoDB Local Java dependency).

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Differences: S3 vs DynamoDB](#5-protocol-differences-s3-vs-dynamodb)
6. [Smithy Code Generation Strategy](#6-smithy-code-generation-strategy)
7. [Crate Structure](#7-crate-structure)
8. [HTTP Layer Design](#8-http-layer-design)
9. [Storage Engine Design](#9-storage-engine-design)
10. [Expression Parser & Evaluator](#10-expression-parser--evaluator)
11. [Core Business Logic](#11-core-business-logic)
12. [Error Handling](#12-error-handling)
13. [Server Integration](#13-server-integration)
14. [Testing Strategy](#14-testing-strategy)
15. [Phased Implementation Plan](#15-phased-implementation-plan)
16. [Risk Analysis](#16-risk-analysis)
17. [Open Questions](#17-open-questions)

---

## 1. Executive Summary

This spec proposes adding DynamoDB support to Rustack as a fully native Rust implementation, following the same architectural patterns established by the S3 implementation. Key design decisions:

- **Native Rust storage engine** -- unlike LocalStack which wraps AWS's DynamoDB Local (a 600MB Java application), we build a purpose-built in-memory storage engine with B-Tree indexing. This yields a ~10MB Docker image with millisecond startup.
- **Smithy codegen reuse** -- extend the existing `codegen/` system to generate DynamoDB model types from the official AWS DynamoDB Smithy JSON AST, producing a `rustack-dynamodb-model` crate.
- **JSON protocol** -- DynamoDB uses `awsJson1_0` (HTTP POST with `X-Amz-Target` header dispatch), which is dramatically simpler than S3's REST+XML protocol. No XML crate needed; serde_json handles all serialization.
- **Shared infrastructure** -- reuse `rustack-core` (multi-account/region state) and `rustack-auth` (SigV4 verification) unchanged.
- **Expression parser** -- the most complex component. DynamoDB's expression language (KeyCondition, Filter, Condition, Update, Projection) requires a proper parser and evaluator, implemented as a separate module within `rustack-dynamodb-core`.
- **Phased delivery** -- 4 phases from MVP (12 operations covering 90%+ of local dev use cases) to full feature parity.

---

## 2. Motivation

### 2.1 Why DynamoDB?

DynamoDB is the second most-used AWS service for local development after S3. Developers need a local DynamoDB for:

- **Unit/integration testing** -- test data access patterns without AWS credentials or costs
- **Local development** -- run full application stacks locally with `docker compose`
- **CI/CD pipelines** -- fast, deterministic DynamoDB in GitHub Actions / CI
- **Offline development** -- work without internet connectivity

### 2.2 Why Not Wrap DynamoDB Local?

LocalStack wraps AWS's DynamoDB Local (a Java/SQLite application). This approach has significant drawbacks:

| Issue | Impact |
|-------|--------|
| **Image size** | DynamoDB Local + JRE adds ~600MB to Docker image |
| **Startup time** | JVM startup takes 2-5 seconds |
| **Memory overhead** | JVM baseline memory is 100-200MB |
| **Java dependency** | Requires JRE 21+ installation and management |
| **Known bugs** | Case-insensitive table names, missing parallel scan support, incomplete transaction conflict detection |
| **Opaque behavior** | Cannot debug or fix issues in the closed-source Java binary |
| **Architecture complexity** | Must proxy HTTP between Rust and Java processes, manage process lifecycle |

### 2.3 Why Native Rust?

A native Rust implementation provides:

- **~10MB Docker image** (same as S3) vs ~600MB with DynamoDB Local
- **Millisecond startup** vs 2-5 seconds for JVM
- **~5MB memory baseline** vs 100-200MB for JVM
- **Full debuggability** -- we own every line of code
- **Correctness** -- fix known DynamoDB Local bugs (case-sensitive table names, proper parallel scan, transaction conflict detection)
- **Single binary** -- no process management, no inter-process communication

### 2.4 Existing Alternatives

| Implementation | Language | Storage | Image Size | Notes |
|---------------|----------|---------|------------|-------|
| DynamoDB Local | Java/SQLite | SQLite | ~600MB | Official AWS, closed-source core |
| LocalStack DDB | Python+Java | DynamoDB Local | ~1GB | Wraps DynamoDB Local, adds multi-account |
| Dynalite | Node.js/LevelDB | LevelDB | ~200MB | Abandoned, incomplete API |
| ScyllaDB Alternator | C++ | ScyllaDB | ~500MB | Full DB engine, overkill for local dev |
| **Rustack DDB** | **Rust** | **In-memory** | **~10MB** | **This proposal** |

No existing Rust-based DynamoDB emulator exists. This would be the first.

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Native Rust DynamoDB emulator** -- no Java, no external processes, no FFI
2. **Cover 90%+ of local development use cases** -- table CRUD, item CRUD, Query, Scan, Batch, Transactions
3. **Correct expression evaluation** -- parse and evaluate KeyConditionExpression, FilterExpression, ConditionExpression, UpdateExpression, and ProjectionExpression
4. **Smithy-generated types** -- all DynamoDB API types generated from official AWS Smithy model
5. **Shared infrastructure** -- reuse `rustack-core` and `rustack-auth` without modification
6. **Same Docker image** -- single binary serves both S3 and DynamoDB on the same port (4566)
7. **GitHub Action compatibility** -- extend the existing `tyrchen/rustack` GitHub Action
8. **Pass AWS SDK integration tests** -- validate against `aws-sdk-rust`, `aws-cli`, and `boto3`

### 3.2 Non-Goals

1. **DynamoDB Streams** -- not in MVP; may be added in Phase 3
2. **Global Tables** -- cross-region replication is unnecessary for local dev
3. **Backup/Restore** -- mocked or omitted; not needed for local dev
4. **Provisioned throughput enforcement** -- accept but ignore capacity settings
5. **Data persistence across restarts** -- in-memory only (same as S3)
6. **DynamoDB Accelerator (DAX)** -- caching layer not applicable to local dev
7. **Fine-grained access control (IAM policies)** -- accept but don't enforce

---

## 4. Architecture Overview

### 4.1 Layered Architecture (Mirrors S3)

```
                    AWS SDK / CLI / boto3
                         │
                         │ HTTP POST :4566
                         ▼
              ┌─────────────────────┐
              │   Gateway Router    │  ← Determines service (S3 vs DynamoDB)
              │   (by header/path)  │     based on X-Amz-Target or URL pattern
              └─────────┬──────────┘
                        │
          ┌─────────────┼──────────────┐
          ▼                            ▼
    ┌───────────┐              ┌──────────────┐
    │ S3 HTTP   │              │ DynamoDB HTTP │
    │ Service   │              │ Service       │
    │ (RestXml) │              │ (awsJson1_0)  │
    └─────┬─────┘              └──────┬───────┘
          │                           │
    ┌─────┴─────┐              ┌──────┴───────┐
    │ S3 Core   │              │ DynamoDB Core│
    │ (business │              │ (business    │
    │  logic)   │              │  logic)      │
    └─────┬─────┘              └──────┬───────┘
          │                           │
    ┌─────┴─────┐              ┌──────┴───────┐
    │ S3 State  │              │ DDB Storage  │
    │ & Storage │              │ Engine       │
    └───────────┘              └──────────────┘
          │                           │
          └─────────┬─────────────────┘
                    ▼
          ┌─────────────────┐
          │  rustack-core │  ← Shared: multi-account/region state
          │  rustack-auth │  ← Shared: SigV4 authentication
          └─────────────────┘
```

### 4.2 Gateway Service Routing

DynamoDB and S3 are distinguishable by their request signatures:

| Signal | S3 | DynamoDB |
|--------|----|---------:|
| HTTP Method | GET/PUT/DELETE/POST/HEAD | POST only |
| Content-Type | varies | `application/x-amz-json-1.0` |
| `X-Amz-Target` header | absent | `DynamoDB_20120810.<Op>` |
| URL path | `/{bucket}/{key}` | `/` |
| SigV4 service name | `s3` | `dynamodb` |

**Routing logic**: If `X-Amz-Target` starts with `DynamoDB_`, route to DynamoDB service. Otherwise, route to S3 (default). This is unambiguous and zero-cost.

### 4.3 Crate Dependency Graph

```
rustack (app) ← unified binary
├── rustack-core
├── rustack-auth
├── rustack-s3-core
├── rustack-s3-http
├── rustack-s3-model
├── rustack-dynamodb-core     ← NEW
├── rustack-dynamodb-http     ← NEW
└── rustack-dynamodb-model    ← NEW (auto-generated)

rustack-dynamodb-http
├── rustack-dynamodb-model
└── rustack-auth

rustack-dynamodb-core
├── rustack-core
├── rustack-dynamodb-model
└── rustack-auth

rustack-dynamodb-model (auto-generated, standalone)
```

---

## 5. Protocol Differences: S3 vs DynamoDB

Understanding protocol differences is critical for reusing vs building new infrastructure.

### 5.1 Protocol Comparison

| Aspect | S3 (RestXml) | DynamoDB (awsJson1_0) |
|--------|-------------|----------------------|
| HTTP Methods | GET, PUT, DELETE, POST, HEAD | POST only |
| URL Routing | Path + query params + virtual host | Always `POST /` |
| Operation Dispatch | Method + path + query + headers | `X-Amz-Target` header |
| Request Body | XML or binary blob | JSON always |
| Response Body | XML or binary blob | JSON always |
| Error Format | XML `<Error>` element | JSON `{"__type": "...", "Message": "..."}` |
| Authentication | SigV2 + SigV4 + presigned URLs | SigV4 only |
| Content-Type | varies | `application/x-amz-json-1.0` |
| Streaming | Yes (large objects) | No (400KB item limit) |
| Request size | Up to 5GB (multipart) | Up to 400KB per item |

### 5.2 What We Reuse

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| `rustack-core` | Yes | Multi-account/region state management |
| `rustack-auth` (SigV4) | Yes | SigV4 verification is service-agnostic |
| `rustack-auth` (SigV2) | No | DynamoDB doesn't use SigV2 |
| `rustack-auth` (presigned) | No | DynamoDB doesn't use presigned URLs |
| Codegen model parser | Partially | Smithy AST parsing reusable; shape resolution needs DynamoDB namespace |
| Codegen code generator | Partially | Struct/enum generation reusable; HTTP bindings differ, error format differs |
| `rustack-s3-xml` | No | DynamoDB uses JSON, not XML |
| `rustack-s3-http` router | No | Completely different routing model |
| `rustack-s3-core` | No | Different domain logic entirely |

### 5.3 What We Build New

| Component | Complexity | Notes |
|-----------|-----------|-------|
| `rustack-dynamodb-model` | Low | Auto-generated from Smithy; JSON serde is trivial |
| `rustack-dynamodb-http` | Low | POST-only routing via `X-Amz-Target` is much simpler than S3 REST |
| `rustack-dynamodb-core` | **High** | Storage engine + expression parser + business logic |
| Expression parser | **High** | Full grammar: comparisons, functions, logical ops, update clauses |
| Storage engine | Medium | B-Tree indexed tables with partition/sort key support |
| Gateway router | Low | Simple header-based service dispatch |

---

## 6. Smithy Code Generation Strategy

### 6.1 Approach: Extend Existing Codegen

Rather than creating a separate codegen binary, we extend the existing `codegen/` tool to support multiple AWS services. The Smithy AST parser (`model.rs`) and code generator (`codegen.rs`) are largely service-agnostic; only `shapes.rs` contains S3-specific logic (operation lists, categories, namespace).

### 6.2 Changes to Codegen

#### 6.2.1 Multi-Service Support

```
codegen/
├── src/
│   ├── main.rs              ← Add CLI arg: --service s3|dynamodb
│   ├── model.rs             ← Unchanged (generic Smithy AST)
│   ├── shapes.rs            ← Extract service-agnostic logic; add ServiceConfig trait
│   ├── codegen.rs           ← Parameterize: service name, error codes, protocol
│   ├── services/
│   │   ├── mod.rs           ← ServiceConfig trait definition
│   │   ├── s3.rs            ← S3-specific: operations, categories, errors, namespace
│   │   └── dynamodb.rs      ← DynamoDB-specific: operations, categories, errors, namespace
│   └── ...
├── smithy-model/
│   ├── s3.json              ← Existing S3 model
│   └── dynamodb.json        ← NEW: DynamoDB Smithy JSON AST
└── Cargo.toml
```

#### 6.2.2 ServiceConfig Trait

```rust
/// Service-specific configuration for code generation.
pub trait ServiceConfig {
    /// Smithy namespace prefix (e.g., "com.amazonaws.s3#" or "com.amazonaws.dynamodb#").
    fn namespace(&self) -> &str;

    /// Service name for generated code (e.g., "S3" or "DynamoDB").
    fn service_name(&self) -> &str;

    /// Target operations to generate code for.
    fn target_operations(&self) -> &[&str];

    /// Operation categories for file organization.
    fn categorize_operations(&self) -> (OperationCategories, OperationCategories);

    /// Error codes specific to this service.
    fn error_codes(&self) -> &[(&str, &str, u16)];

    /// Protocol-specific adjustments (e.g., HTTP bindings for REST vs JSON headers).
    fn protocol(&self) -> Protocol;
}

pub enum Protocol {
    RestXml,     // S3: HTTP method + path + query + XML body
    AwsJson1_0,  // DynamoDB: POST + X-Amz-Target + JSON body
}
```

#### 6.2.3 Key Differences in Code Generation

| Aspect | S3 Codegen | DynamoDB Codegen |
|--------|-----------|-----------------|
| Namespace | `com.amazonaws.s3#` | `com.amazonaws.dynamodb#` |
| HTTP Bindings | Label, Query, Header, Payload, PrefixHeaders | None (all fields in JSON body) |
| Serde derives | No Serialize/Deserialize on I/O structs | `#[derive(Serialize, Deserialize)]` on all I/O structs |
| Field wrapping | HTTP binding comments | `#[serde(rename = "PascalCase")]` attributes |
| Error format | XML-based S3ErrorCode | JSON-based DynamoDBErrorCode with `__type` |
| StreamingBlob | Used for object bodies | Not needed (no streaming) |
| Rename convention | None (fields snake_case internally) | `#[serde(rename_all = "PascalCase")]` |

#### 6.2.4 DynamoDB-Specific Struct Generation

Since DynamoDB uses JSON protocol, all input/output structs need serde derives with PascalCase renaming:

```rust
/// DynamoDB CreateTableInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateTableInput {
    pub table_name: String,
    pub key_schema: Vec<KeySchemaElement>,
    pub attribute_definitions: Vec<AttributeDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<BillingMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioned_throughput: Option<ProvisionedThroughput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub global_secondary_indexes: Vec<GlobalSecondaryIndex>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub local_secondary_indexes: Vec<LocalSecondaryIndex>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_specification: Option<StreamSpecification>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_specification: Option<SSESpecification>,
    pub tags: Vec<Tag>,
}
```

#### 6.2.5 AttributeValue: Special Handling

DynamoDB's `AttributeValue` is a tagged union with single-key JSON objects:

```json
{"S": "hello"}          // String
{"N": "42"}             // Number (always string-encoded)
{"B": "dGVzdA=="}       // Binary (base64)
{"BOOL": true}          // Boolean
{"NULL": true}          // Null
{"L": [{"S": "a"}]}    // List
{"M": {"k": {"S":"v"}}} // Map
{"SS": ["a", "b"]}     // String Set
{"NS": ["1", "2"]}     // Number Set
{"BS": ["dA==", "dQ=="]} // Binary Set
```

This requires a custom Rust enum with manual serde implementation:

```rust
/// DynamoDB attribute value.
///
/// Represented as a tagged union where exactly one variant is present.
/// Numbers are always string-encoded to preserve precision.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeValue {
    /// String value.
    S(String),
    /// Number value (string-encoded for arbitrary precision).
    N(String),
    /// Binary value (base64-encoded bytes).
    B(Bytes),
    /// String Set.
    Ss(Vec<String>),
    /// Number Set (string-encoded).
    Ns(Vec<String>),
    /// Binary Set.
    Bs(Vec<Bytes>),
    /// Boolean value.
    Bool(bool),
    /// Null value.
    Null(bool),
    /// List of attribute values.
    L(Vec<AttributeValue>),
    /// Map of attribute values.
    M(HashMap<String, AttributeValue>),
}
```

This type is too complex for auto-generation and should be hand-written in `rustack-dynamodb-model` as a non-generated file alongside the generated code.

### 6.3 Smithy Model Acquisition

The DynamoDB Smithy model is available from:

1. **aws-sdk-rust**: `aws/sdk/dynamodb/model/` in the `aws-sdk-rust` repo
2. **aws-models**: `https://github.com/aws/aws-models` or `https://github.com/aws/api-models-aws`
3. **smithy-rs**: Bundled in the smithy-rs codegen

We download the DynamoDB Smithy JSON AST and place it at `codegen/smithy-model/dynamodb.json`.

### 6.4 Target Operations (Phased)

```rust
/// Phase 0 (MVP): 12 operations covering 90%+ of local dev use cases.
const P0_OPERATIONS: &[&str] = &[
    // Table management
    "CreateTable",
    "DeleteTable",
    "DescribeTable",
    "ListTables",
    // Item CRUD
    "PutItem",
    "GetItem",
    "UpdateItem",
    "DeleteItem",
    // Query & Scan
    "Query",
    "Scan",
    // Batch
    "BatchGetItem",
    "BatchWriteItem",
];

/// Phase 1: Transactions + advanced features (6 operations).
const P1_OPERATIONS: &[&str] = &[
    "UpdateTable",
    "TransactGetItems",
    "TransactWriteItems",
    "DescribeTimeToLive",
    "UpdateTimeToLive",
    "ExecuteStatement",
];

/// Phase 2: Tags, limits, batch PartiQL (6 operations).
const P2_OPERATIONS: &[&str] = &[
    "TagResource",
    "UntagResource",
    "ListTagsOfResource",
    "DescribeLimits",
    "BatchExecuteStatement",
    "ExecuteTransaction",
];

/// Phase 3: Streams, backups, global tables (~40 operations).
/// Deferred until demand materializes.
```

### 6.5 Makefile Integration

```makefile
codegen-s3:
    @cd codegen && cargo run -- --service s3
    @cargo +nightly fmt -p rustack-s3-model

codegen-dynamodb:
    @cd codegen && cargo run -- --service dynamodb
    @cargo +nightly fmt -p rustack-dynamodb-model

codegen: codegen-s3 codegen-dynamodb
```

---

## 7. Crate Structure

### 7.1 New Crates

#### `rustack-dynamodb-model` (auto-generated)

```
crates/rustack-dynamodb-model/
├── Cargo.toml
└── src/
    ├── lib.rs                    # Module re-exports
    ├── attribute_value.rs        # Hand-written: AttributeValue enum + serde
    ├── types.rs                  # Auto-generated: enums + shared structs
    ├── operations.rs             # Auto-generated: DynamoDBOperation enum
    ├── error.rs                  # Auto-generated: DynamoDBError + error codes
    ├── request.rs                # Auto-generated: DynamoDBRequest<T>
    ├── input/
    │   ├── mod.rs
    │   ├── table.rs              # CreateTableInput, DeleteTableInput, etc.
    │   ├── item.rs               # PutItemInput, GetItemInput, etc.
    │   ├── query.rs              # QueryInput, ScanInput
    │   ├── batch.rs              # BatchGetItemInput, BatchWriteItemInput
    │   ├── transaction.rs        # TransactGetItemsInput, TransactWriteItemsInput
    │   └── config.rs             # TTL, tags, limits
    └── output/
        ├── mod.rs
        ├── table.rs
        ├── item.rs
        ├── query.rs
        ├── batch.rs
        ├── transaction.rs
        └── config.rs
```

**Dependencies**: `serde`, `serde_json`, `bytes`, `chrono`, `http`

#### `rustack-dynamodb-http`

```
crates/rustack-dynamodb-http/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── router.rs                 # X-Amz-Target → DynamoDBOperation dispatch
    ├── dispatch.rs               # DynamoDBHandler trait + dispatch logic
    ├── service.rs                # Hyper Service impl for DynamoDB
    ├── request.rs                # FromDynamoDBRequest trait (JSON deserialization)
    ├── response.rs               # IntoDynamoDBResponse trait (JSON serialization)
    ├── error.rs                  # JSON error response formatting
    └── body.rs                   # Response body type
```

**Dependencies**: `rustack-dynamodb-model`, `rustack-auth`, `hyper`, `serde_json`, `bytes`

#### `rustack-dynamodb-core`

```
crates/rustack-dynamodb-core/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── config.rs                 # DynamoDBConfig
    ├── provider.rs               # RustackDynamoDB (main provider)
    ├── error.rs                  # DynamoDBServiceError
    ├── state/
    │   ├── mod.rs
    │   ├── service.rs            # DynamoDBServiceState (table registry)
    │   └── table.rs              # DynamoDBTable (metadata + storage)
    ├── storage/
    │   ├── mod.rs
    │   ├── engine.rs             # TableStorage: B-Tree based item storage
    │   ├── key.rs                # PrimaryKey: partition + sort key handling
    │   ├── index.rs              # GSI/LSI secondary index management
    │   └── item.rs               # Item: attribute map wrapper with validation
    ├── expression/
    │   ├── mod.rs
    │   ├── lexer.rs              # Token lexer for expression strings
    │   ├── parser.rs             # Recursive descent parser → AST
    │   ├── ast.rs                # Expression AST node types
    │   ├── evaluator.rs          # Evaluate expressions against items
    │   ├── projection.rs         # Projection expression evaluation
    │   ├── update.rs             # Update expression evaluation (SET/REMOVE/ADD/DELETE)
    │   ├── condition.rs          # Condition + filter expression evaluation
    │   ├── key_condition.rs      # Key condition expression (query planning)
    │   └── names_values.rs       # ExpressionAttributeNames/Values substitution
    └── ops/
        ├── mod.rs
        ├── table.rs              # create_table, delete_table, describe_table, list_tables
        ├── item.rs               # put_item, get_item, update_item, delete_item
        ├── query.rs              # query, scan
        ├── batch.rs              # batch_get_item, batch_write_item
        ├── transaction.rs        # transact_get_items, transact_write_items
        └── config.rs             # TTL, tags, limits
```

**Dependencies**: `rustack-core`, `rustack-dynamodb-model`, `rustack-auth`, `dashmap`, `parking_lot`, `serde_json`, `bytes`, `chrono`, `uuid`, `tracing`

### 7.2 Workspace Changes

```toml
# Root Cargo.toml
[workspace]
members = [
    "crates/*",
    "apps/*",
    "tests/integration",
]

# New workspace dependencies
[workspace.dependencies]
# ... existing deps unchanged ...
rustack-dynamodb-model = { path = "crates/rustack-dynamodb-model" }
rustack-dynamodb-http = { path = "crates/rustack-dynamodb-http" }
rustack-dynamodb-core = { path = "crates/rustack-dynamodb-core" }
```

### 7.3 Server Binary Changes

The server binary (`apps/rustack-s3-server/`) should be renamed or extended to serve both S3 and DynamoDB. Two approaches:

**Option A: Unified Binary** (recommended)
Rename to `apps/rustack/`. Single binary, single port (4566), gateway router dispatches by protocol.

**Option B: Separate Binaries**
Keep `apps/rustack-s3-server/` and add `apps/rustack-dynamodb-server/`. Simpler but requires running two processes.

We recommend **Option A** for Docker simplicity and LocalStack API compatibility (LocalStack serves all services on port 4566).

---

## 8. HTTP Layer Design

### 8.1 awsJson1_0 Protocol

DynamoDB uses the AWS JSON 1.0 protocol, which is dramatically simpler than S3's REST protocol.

**Request format:**
```
POST / HTTP/1.1
Host: dynamodb.us-east-1.amazonaws.com
Content-Type: application/x-amz-json-1.0
X-Amz-Target: DynamoDB_20120810.CreateTable
Authorization: AWS4-HMAC-SHA256 Credential=.../dynamodb/aws4_request, ...

{"TableName": "MyTable", "KeySchema": [...], ...}
```

**Response format (success):**
```
HTTP/1.1 200 OK
Content-Type: application/x-amz-json-1.0
x-amzn-RequestId: <uuid>

{"TableDescription": {"TableName": "MyTable", ...}}
```

**Response format (error):**
```
HTTP/1.1 400 Bad Request
Content-Type: application/x-amz-json-1.0
x-amzn-RequestId: <uuid>

{"__type": "com.amazonaws.dynamodb.v20120810#ResourceNotFoundException",
 "Message": "Requested resource not found: Table: MyTable not found"}
```

### 8.2 Router Design

```rust
/// DynamoDB operation router.
///
/// Parses the `X-Amz-Target` header to determine the operation.
/// All DynamoDB requests are POST to `/` with JSON bodies.
pub struct DynamoDBRouter;

impl DynamoDBRouter {
    /// Resolve an HTTP request to a DynamoDB operation.
    pub fn resolve(req: &http::Request<()>) -> Result<DynamoDBOperation, DynamoDBError> {
        // 1. Verify method is POST
        if req.method() != http::Method::POST {
            return Err(DynamoDBError::missing_action());
        }

        // 2. Extract X-Amz-Target header
        let target = req.headers()
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(DynamoDBError::missing_action)?;

        // 3. Parse "DynamoDB_20120810.<OperationName>"
        let op_name = target
            .strip_prefix("DynamoDB_20120810.")
            .ok_or_else(|| DynamoDBError::unknown_operation(target))?;

        // 4. Map to DynamoDBOperation enum
        DynamoDBOperation::from_name(op_name)
            .ok_or_else(|| DynamoDBError::unknown_operation(op_name))
    }
}
```

This is ~20 lines of routing logic vs S3's ~400 lines. No path parsing, no query parameters, no virtual hosting.

### 8.3 Request Deserialization

Since all DynamoDB I/O structs have `#[derive(Deserialize)]`, request deserialization is trivial:

```rust
/// Deserialize a DynamoDB request from JSON body.
pub trait FromDynamoDBRequest: Sized + DeserializeOwned {
    fn from_dynamodb_request(body: &[u8]) -> Result<Self, DynamoDBError> {
        serde_json::from_slice(body)
            .map_err(|e| DynamoDBError::serialization_exception(e.to_string()))
    }
}

// Blanket implementation for all DeserializeOwned types
impl<T: DeserializeOwned> FromDynamoDBRequest for T {}
```

Compare with S3 where each operation requires manual extraction of HTTP labels, query params, headers, and XML body parsing.

### 8.4 Response Serialization

```rust
/// Serialize a DynamoDB response to JSON.
pub trait IntoDynamoDBResponse: Sized + Serialize {
    fn into_dynamodb_response(self) -> Result<http::Response<Bytes>, DynamoDBError> {
        let body = serde_json::to_vec(&self)
            .map_err(|e| DynamoDBError::internal_error(e.to_string()))?;

        Ok(http::Response::builder()
            .status(200)
            .header("content-type", "application/x-amz-json-1.0")
            .body(Bytes::from(body))
            .expect("valid response"))
    }
}

impl<T: Serialize> IntoDynamoDBResponse for T {}
```

### 8.5 Error Response Format

```rust
/// Format a DynamoDB error as JSON.
pub fn error_to_json(error: &DynamoDBError) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "__type": error.error_type(),
        "Message": error.message,
    })).expect("JSON serialization cannot fail")
}
```

### 8.6 DynamoDBHandler Trait

```rust
/// The boundary between HTTP and business logic.
///
/// Analogous to S3Handler but for DynamoDB operations.
pub trait DynamoDBHandler: Send + Sync + 'static {
    /// Handle a DynamoDB operation.
    fn handle_operation(
        &self,
        op: DynamoDBOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<Bytes>, DynamoDBError>> + Send>>;
}
```

### 8.7 Service Integration

```rust
/// Hyper Service implementation for DynamoDB.
pub struct DynamoDBHttpService<H> {
    handler: Arc<H>,
    config: DynamoDBHttpConfig,
}

pub struct DynamoDBHttpConfig {
    pub skip_signature_validation: bool,
    pub region: String,
    pub credential_provider: Option<Arc<dyn CredentialProvider>>,
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

The storage engine provides an in-memory, indexed data structure that supports DynamoDB's data model: tables with partition keys (and optional sort keys), global secondary indexes (GSI), and local secondary indexes (LSI).

### 9.2 Core Data Structures

```rust
/// A single DynamoDB item (row).
#[derive(Debug, Clone)]
pub struct Item {
    /// The attribute map.
    pub attributes: HashMap<String, AttributeValue>,
    /// Size in bytes (cached for limit checking).
    pub size_bytes: u64,
}

/// Primary key extracted from an item.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrimaryKey {
    /// Partition key value.
    pub partition: AttributeValue,
    /// Sort key value (None if table has no sort key).
    pub sort: Option<AttributeValue>,
}

/// Table storage engine with primary and secondary indexes.
pub struct TableStorage {
    /// Table key schema.
    key_schema: KeySchema,
    /// Primary data: partition_key -> BTreeMap<sort_key, Item>
    /// Using BTreeMap for sort key ordering (required for Query range operations).
    data: DashMap<AttributeValue, BTreeMap<SortableAttributeValue, Item>>,
    /// Global Secondary Indexes.
    gsi: HashMap<String, SecondaryIndex>,
    /// Local Secondary Indexes.
    lsi: HashMap<String, SecondaryIndex>,
    /// Total item count.
    item_count: AtomicU64,
    /// Total size in bytes.
    total_size: AtomicU64,
}

/// A secondary index (GSI or LSI).
pub struct SecondaryIndex {
    /// Index name.
    pub name: String,
    /// Index key schema.
    pub key_schema: KeySchema,
    /// Projection specification.
    pub projection: Projection,
    /// Index data: partition_key -> BTreeMap<sort_key, IndexEntry>
    pub data: DashMap<AttributeValue, BTreeMap<SortableAttributeValue, IndexEntry>>,
}

/// An entry in a secondary index.
/// Contains projected attributes and a reference back to the primary key.
pub struct IndexEntry {
    /// Primary key for fetching full item.
    pub primary_key: PrimaryKey,
    /// Projected attributes (depends on projection type).
    pub attributes: HashMap<String, AttributeValue>,
}
```

### 9.3 Key Schema

```rust
/// Parsed key schema for a table or index.
pub struct KeySchema {
    /// Partition key attribute name.
    pub partition_key: String,
    /// Partition key type (S, N, or B).
    pub partition_key_type: ScalarAttributeType,
    /// Sort key attribute name (None if partition-only).
    pub sort_key: Option<String>,
    /// Sort key type (S, N, or B).
    pub sort_key_type: Option<ScalarAttributeType>,
}
```

### 9.4 Attribute Value Ordering

DynamoDB defines ordering for sort keys. `AttributeValue` needs `Ord` for B-Tree storage:

```rust
/// Wrapper that implements Ord for AttributeValue (sort key ordering).
///
/// DynamoDB ordering rules:
/// - Strings: UTF-8 byte ordering
/// - Numbers: numeric ordering (arbitrary precision)
/// - Binary: byte-by-byte unsigned ordering
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortableAttributeValue(pub AttributeValue);

impl Ord for SortableAttributeValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (&self.0, &other.0) {
            (AttributeValue::S(a), AttributeValue::S(b)) => a.cmp(b),
            (AttributeValue::N(a), AttributeValue::N(b)) => {
                compare_dynamodb_numbers(a, b)
            }
            (AttributeValue::B(a), AttributeValue::B(b)) => a.cmp(b),
            _ => std::cmp::Ordering::Equal,
        }
    }
}

/// Compare DynamoDB number strings with arbitrary precision.
///
/// DynamoDB numbers can be up to 38 digits with 38 decimal places.
fn compare_dynamodb_numbers(a: &str, b: &str) -> std::cmp::Ordering {
    // Parse as decimal for comparison
    // Consider using `rust_decimal` crate for 128-bit decimal arithmetic
    todo!()
}
```

### 9.5 Core Operations

```rust
impl TableStorage {
    /// Insert or replace an item. Returns the old item if it existed.
    pub fn put_item(&self, item: Item) -> Result<Option<Item>, StorageError>;

    /// Get an item by primary key.
    pub fn get_item(&self, key: &PrimaryKey) -> Option<Item>;

    /// Delete an item by primary key. Returns the deleted item.
    pub fn delete_item(&self, key: &PrimaryKey) -> Option<Item>;

    /// Query items by partition key and optional sort key condition.
    /// Returns items in sort key order (ascending or descending).
    pub fn query(
        &self,
        partition_key: &AttributeValue,
        sort_condition: Option<&SortKeyCondition>,
        scan_forward: bool,
        limit: Option<i32>,
        exclusive_start_key: Option<&PrimaryKey>,
    ) -> QueryResult;

    /// Scan all items with optional filter.
    /// Supports parallel scan via segment/total_segments.
    pub fn scan(
        &self,
        segment: Option<i32>,
        total_segments: Option<i32>,
        limit: Option<i32>,
        exclusive_start_key: Option<&PrimaryKey>,
    ) -> ScanResult;

    /// Update GSI/LSI entries after a put/delete.
    fn update_secondary_indexes(&self, old: Option<&Item>, new: Option<&Item>);
}

/// Sort key conditions for Query operations.
pub enum SortKeyCondition {
    Eq(AttributeValue),
    Lt(AttributeValue),
    Le(AttributeValue),
    Gt(AttributeValue),
    Ge(AttributeValue),
    Between(AttributeValue, AttributeValue),
    BeginsWith(String),
}
```

### 9.6 Secondary Index Maintenance

When an item is inserted, updated, or deleted, all secondary indexes must be updated:

1. **For each GSI/LSI**: Extract the index key attributes from the new item
2. **Check projection**: If KEYS_ONLY, store only key attributes. If INCLUDE, store specified attributes. If ALL, store all attributes.
3. **Update index B-Tree**: Insert/remove entries maintaining sort order
4. **Handle sparse indexes**: If the index key attribute is missing from the item, don't create an index entry (GSI only)

### 9.7 Item Size Calculation

DynamoDB items are limited to 400KB. Size calculation follows AWS rules:

```rust
impl Item {
    /// Calculate item size in bytes following DynamoDB rules.
    pub fn calculate_size(&self) -> u64 {
        self.attributes.iter().map(|(k, v)| {
            k.len() as u64 + attribute_value_size(v)
        }).sum()
    }
}

fn attribute_value_size(val: &AttributeValue) -> u64 {
    match val {
        AttributeValue::S(s) => s.len() as u64,
        AttributeValue::N(n) => {
            // Numbers: 1 byte per 2 digits + 1 byte
            (n.len() as u64 + 1) / 2 + 1
        }
        AttributeValue::B(b) => b.len() as u64,
        AttributeValue::Bool(_) => 1,
        AttributeValue::Null(_) => 1,
        AttributeValue::Ss(set) => set.iter().map(|s| s.len() as u64).sum(),
        AttributeValue::Ns(set) => set.iter().map(|n| (n.len() as u64 + 1) / 2 + 1).sum(),
        AttributeValue::Bs(set) => set.iter().map(|b| b.len() as u64).sum(),
        AttributeValue::L(list) => 3 + list.iter().map(|v| 1 + attribute_value_size(v)).sum::<u64>(),
        AttributeValue::M(map) => 3 + map.iter().map(|(k, v)| {
            k.len() as u64 + 1 + attribute_value_size(v)
        }).sum::<u64>(),
    }
}
```

### 9.8 Number Handling

DynamoDB numbers require arbitrary precision arithmetic. Key considerations:

- Numbers can have up to 38 significant digits
- Range: 1E-130 to 9.9999999999999999999999999999999999999E+125
- Numbers are always string-encoded in the JSON wire format
- Comparison, arithmetic (for SET path = path + :val), and sorting must be precise

**Recommended crate**: `rust_decimal` (128-bit decimal) or `bigdecimal` (arbitrary precision). Since DynamoDB's maximum precision is 38 digits, `rust_decimal` (28-29 digits) may not suffice; `bigdecimal` is safer.

---

## 10. Expression Parser & Evaluator

### 10.1 Overview

DynamoDB's expression language is the most complex component. There are 5 expression types:

| Expression Type | Used By | Purpose |
|----------------|---------|---------|
| `KeyConditionExpression` | Query | Filter by partition/sort key |
| `FilterExpression` | Query, Scan | Post-query filter on non-key attributes |
| `ConditionExpression` | PutItem, UpdateItem, DeleteItem | Conditional writes |
| `UpdateExpression` | UpdateItem | Modify item attributes |
| `ProjectionExpression` | GetItem, Query, Scan | Select specific attributes to return |

### 10.2 Expression Grammar

#### 10.2.1 Condition/Filter/KeyCondition Expression Grammar

```
condition     = operand comparator operand
              | operand "BETWEEN" operand "AND" operand
              | operand "IN" "(" operand ("," operand)* ")"
              | "(" condition ")"
              | "NOT" condition
              | condition "AND" condition
              | condition "OR" condition
              | function

comparator    = "=" | "<>" | "<" | "<=" | ">" | ">="

operand       = path | literal

path          = attribute_name
              | path "." attribute_name
              | path "[" number "]"
              | expression_attribute_name    (e.g., #name)

literal       = expression_attribute_value   (e.g., :value)

function      = "attribute_exists" "(" path ")"
              | "attribute_not_exists" "(" path ")"
              | "attribute_type" "(" path "," operand ")"
              | "begins_with" "(" path "," operand ")"
              | "contains" "(" path "," operand ")"
              | "size" "(" path ")"
```

#### 10.2.2 Update Expression Grammar

```
update_expr   = set_clause
              | remove_clause
              | add_clause
              | delete_clause
              | update_expr update_expr    (multiple clauses)

set_clause    = "SET" set_action ("," set_action)*
set_action    = path "=" operand
              | path "=" operand "+" operand
              | path "=" operand "-" operand
              | path "=" "if_not_exists" "(" path "," operand ")"
              | path "=" "list_append" "(" operand "," operand ")"

remove_clause = "REMOVE" path ("," path)*

add_clause    = "ADD" path operand ("," path operand)*

delete_clause = "DELETE" path operand ("," path operand)*
```

#### 10.2.3 Projection Expression Grammar

```
projection    = path ("," path)*
```

### 10.3 Lexer

```rust
/// Token types for DynamoDB expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Identifiers and references
    Identifier(String),           // attribute name
    ExprAttrName(String),         // #name
    ExprAttrValue(String),        // :value

    // Operators
    Eq,                           // =
    Ne,                           // <>
    Lt,                           // <
    Le,                           // <=
    Gt,                           // >
    Ge,                           // >=
    Plus,                         // +
    Minus,                        // -
    Dot,                          // .
    Comma,                        // ,
    LParen,                       // (
    RParen,                       // )
    LBracket,                     // [
    RBracket,                     // ]

    // Keywords
    And,
    Or,
    Not,
    Between,
    In,
    Set,
    Remove,
    Add,
    Delete,

    // Functions
    AttributeExists,
    AttributeNotExists,
    AttributeType,
    BeginsWith,
    Contains,
    Size,
    IfNotExists,
    ListAppend,

    // Literals
    Number(i64),                  // for array index [0]

    Eof,
}
```

### 10.4 AST

```rust
/// Expression AST node.
#[derive(Debug, Clone)]
pub enum Expr {
    /// Comparison: operand <op> operand
    Compare {
        left: Box<Operand>,
        op: CompareOp,
        right: Box<Operand>,
    },
    /// BETWEEN: operand BETWEEN operand AND operand
    Between {
        value: Box<Operand>,
        low: Box<Operand>,
        high: Box<Operand>,
    },
    /// IN: operand IN (operand, operand, ...)
    In {
        value: Box<Operand>,
        list: Vec<Operand>,
    },
    /// AND / OR
    Logical {
        op: LogicalOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// NOT
    Not(Box<Expr>),
    /// Function call
    Function {
        name: FunctionName,
        args: Vec<Operand>,
    },
}

/// An operand in an expression.
#[derive(Debug, Clone)]
pub enum Operand {
    /// Attribute path (e.g., info.rating, #name, list[0])
    Path(AttributePath),
    /// Value reference (e.g., :value)
    Value(String),
    /// size(path) function returning numeric value
    Size(AttributePath),
}

/// A dot/bracket-separated attribute path.
#[derive(Debug, Clone)]
pub struct AttributePath {
    pub elements: Vec<PathElement>,
}

#[derive(Debug, Clone)]
pub enum PathElement {
    /// Named attribute (e.g., "info", "#name")
    Attribute(String),
    /// Array index (e.g., [0])
    Index(usize),
}

/// Update expression AST.
#[derive(Debug, Clone)]
pub struct UpdateExpr {
    pub set_actions: Vec<SetAction>,
    pub remove_paths: Vec<AttributePath>,
    pub add_actions: Vec<AddAction>,
    pub delete_actions: Vec<DeleteAction>,
}

#[derive(Debug, Clone)]
pub struct SetAction {
    pub path: AttributePath,
    pub value: SetValue,
}

#[derive(Debug, Clone)]
pub enum SetValue {
    /// Simple value: path = :value
    Operand(Operand),
    /// Arithmetic: path = operand + operand
    Plus(Operand, Operand),
    /// Arithmetic: path = operand - operand
    Minus(Operand, Operand),
    /// if_not_exists(path, operand)
    IfNotExists(AttributePath, Operand),
    /// list_append(operand, operand)
    ListAppend(Operand, Operand),
}
```

### 10.5 Evaluator

```rust
/// Context for expression evaluation.
pub struct EvalContext<'a> {
    /// The item being evaluated.
    pub item: &'a Item,
    /// Expression attribute names (#name -> actual_name).
    pub names: &'a HashMap<String, String>,
    /// Expression attribute values (:value -> AttributeValue).
    pub values: &'a HashMap<String, AttributeValue>,
}

impl<'a> EvalContext<'a> {
    /// Evaluate a condition/filter expression against an item.
    pub fn evaluate(&self, expr: &Expr) -> Result<bool, ExpressionError>;

    /// Resolve an operand to an AttributeValue.
    pub fn resolve_operand(&self, operand: &Operand) -> Result<Option<&AttributeValue>, ExpressionError>;

    /// Navigate an attribute path to find the value in the item.
    pub fn resolve_path(&self, path: &AttributePath) -> Option<&AttributeValue>;

    /// Apply an update expression to an item, returning the modified item.
    pub fn apply_update(&self, update: &UpdateExpr) -> Result<Item, ExpressionError>;

    /// Apply a projection expression, returning only selected attributes.
    pub fn apply_projection(&self, paths: &[AttributePath]) -> Item;
}
```

### 10.6 ExpressionAttributeNames and ExpressionAttributeValues

DynamoDB uses substitution placeholders to avoid conflicts with reserved words:

- `#name` in expression text → maps to real attribute name via `ExpressionAttributeNames`
- `:value` in expression text → maps to `AttributeValue` via `ExpressionAttributeValues`

The evaluator resolves these before evaluation:

```rust
/// Resolve expression attribute name placeholders.
fn resolve_name<'a>(
    name: &str,
    names: &'a HashMap<String, String>,
) -> Result<&'a str, ExpressionError> {
    if name.starts_with('#') {
        names.get(name)
            .map(String::as_str)
            .ok_or_else(|| ExpressionError::unresolved_name(name))
    } else {
        Ok(name)
    }
}
```

---

## 11. Core Business Logic

### 11.1 Provider

```rust
/// Main DynamoDB provider implementing all operations.
pub struct RustackDynamoDB {
    /// Table registry and metadata.
    pub(crate) state: Arc<DynamoDBServiceState>,
    /// Configuration.
    pub(crate) config: Arc<DynamoDBConfig>,
}

impl RustackDynamoDB {
    pub fn new(config: DynamoDBConfig) -> Self;
    pub fn state(&self) -> &DynamoDBServiceState;
    pub fn config(&self) -> &DynamoDBConfig;
    pub fn reset(&self);
}
```

### 11.2 State Management

```rust
/// Top-level state owning all tables.
pub struct DynamoDBServiceState {
    /// All tables keyed by name.
    tables: DashMap<String, DynamoDBTable>,
}

/// A single DynamoDB table with metadata and storage.
pub struct DynamoDBTable {
    /// Table name.
    pub name: String,
    /// Table status.
    pub status: TableStatus,
    /// Key schema.
    pub key_schema: KeySchema,
    /// Attribute definitions.
    pub attribute_definitions: Vec<AttributeDefinition>,
    /// Billing mode.
    pub billing_mode: BillingMode,
    /// Provisioned throughput (accepted but not enforced).
    pub provisioned_throughput: Option<ProvisionedThroughput>,
    /// Stream specification.
    pub stream_specification: Option<StreamSpecification>,
    /// SSE specification.
    pub sse_specification: Option<SSESpecification>,
    /// TTL specification.
    pub ttl: parking_lot::RwLock<Option<TimeToLiveSpecification>>,
    /// Tags.
    pub tags: parking_lot::RwLock<HashMap<String, String>>,
    /// Table ARN.
    pub arn: String,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Item storage engine.
    pub storage: TableStorage,
}
```

### 11.3 Operation Handlers

Each operation handler follows the same pattern as S3:

```rust
impl RustackDynamoDB {
    /// Handle CreateTable.
    pub fn handle_create_table(
        &self,
        input: CreateTableInput,
    ) -> Result<CreateTableOutput, DynamoDBServiceError> {
        // 1. Validate input (key schema, attribute definitions, indexes)
        // 2. Check table doesn't already exist
        // 3. Create DynamoDBTable with TableStorage
        // 4. Insert into state
        // 5. Return CreateTableOutput with TableDescription
    }

    /// Handle PutItem.
    pub fn handle_put_item(
        &self,
        input: PutItemInput,
    ) -> Result<PutItemOutput, DynamoDBServiceError> {
        // 1. Validate table exists
        // 2. Validate item has required key attributes
        // 3. Validate item size <= 400KB
        // 4. If ConditionExpression, parse and evaluate against existing item
        // 5. Put item into storage (updates secondary indexes)
        // 6. Return old item if ReturnValues specified
    }

    /// Handle Query.
    pub fn handle_query(
        &self,
        input: QueryInput,
    ) -> Result<QueryOutput, DynamoDBServiceError> {
        // 1. Validate table (or index) exists
        // 2. Parse KeyConditionExpression → extract partition key + sort condition
        // 3. Query storage with partition key and sort condition
        // 4. If FilterExpression, parse and apply filter
        // 5. If ProjectionExpression, apply projection
        // 6. If Limit, truncate and set LastEvaluatedKey
        // 7. Return items + count + scanned count
    }

    /// Handle UpdateItem.
    pub fn handle_update_item(
        &self,
        input: UpdateItemInput,
    ) -> Result<UpdateItemOutput, DynamoDBServiceError> {
        // 1. Validate table exists
        // 2. If ConditionExpression, parse and evaluate against existing item
        // 3. Parse UpdateExpression
        // 4. Apply update to item (or create new item if not exists)
        // 5. Validate updated item size <= 400KB
        // 6. Store updated item
        // 7. Return old/new values based on ReturnValues
    }
}
```

### 11.4 Conditional Writes

DynamoDB supports conditional writes on PutItem, UpdateItem, and DeleteItem. The flow:

1. Parse `ConditionExpression` string into AST
2. Resolve `ExpressionAttributeNames` and `ExpressionAttributeValues`
3. Fetch existing item from storage
4. Evaluate condition against existing item
5. If condition fails, return `ConditionalCheckFailedException`
6. If condition passes (or no condition), proceed with write

### 11.5 Batch Operations

Batch operations have specific constraints:

- **BatchGetItem**: Up to 100 items across multiple tables
- **BatchWriteItem**: Up to 25 items (puts or deletes) across multiple tables
- Individual items that fail are returned in `UnprocessedItems` / `UnprocessedKeys`
- For our in-memory implementation, all items should succeed (no throttling)

### 11.6 Transaction Operations

Transactions require atomicity:

- **TransactWriteItems**: Up to 100 items, all-or-nothing
- **TransactGetItems**: Up to 100 items, consistent snapshot
- Must detect conflicts between items in the same transaction
- Must validate all conditions before applying any writes

```rust
impl RustackDynamoDB {
    pub fn handle_transact_write_items(
        &self,
        input: TransactWriteItemsInput,
    ) -> Result<TransactWriteItemsOutput, DynamoDBServiceError> {
        // 1. Validate all items (tables exist, key schemas match)
        // 2. Detect conflicts (same item referenced multiple times)
        // 3. Acquire locks on all affected items (prevent concurrent modification)
        // 4. Evaluate all conditions
        // 5. If any condition fails, return TransactionCanceledException
        //    with per-item cancellation reasons
        // 6. Apply all writes atomically
        // 7. Release locks
    }
}
```

---

## 12. Error Handling

### 12.1 DynamoDB Error Codes

```rust
/// DynamoDB error codes.
pub enum DynamoDBErrorCode {
    /// Table already exists.
    ResourceInUseException,
    /// Table not found.
    ResourceNotFoundException,
    /// Condition check failed.
    ConditionalCheckFailedException,
    /// Transaction canceled.
    TransactionCanceledException,
    /// Transaction conflict.
    TransactionConflictException,
    /// Transaction in progress.
    TransactionInProgressException,
    /// Idempotent parameter mismatch.
    IdempotentParameterMismatchException,
    /// Item collection size limit exceeded (10GB per partition).
    ItemCollectionSizeLimitExceededException,
    /// Provisioned throughput exceeded (we may never actually return this).
    ProvisionedThroughputExceededException,
    /// Request limit exceeded.
    RequestLimitExceeded,
    /// Validation error.
    ValidationException,
    /// Serialization error.
    SerializationException,
    /// Internal server error.
    InternalServerError,
    /// Missing action.
    MissingAction,
    /// Access denied.
    AccessDeniedException,
}
```

### 12.2 Error Format

DynamoDB errors use JSON with `__type` field containing the fully-qualified error type:

```json
{
    "__type": "com.amazonaws.dynamodb.v20120810#ResourceNotFoundException",
    "Message": "Requested resource not found: Table: MyTable not found"
}
```

The `__type` prefix varies:
- Most errors: `com.amazonaws.dynamodb.v20120810#ErrorName`
- Some errors: `com.amazon.coral.validate#ValidationException`
- Auth errors: handled by `rustack-auth` before reaching DynamoDB handler

### 12.3 Service Error Enum

```rust
/// Domain-level errors for DynamoDB business logic.
pub enum DynamoDBServiceError {
    TableAlreadyExists { table_name: String },
    TableNotFound { table_name: String },
    ConditionalCheckFailed { message: String },
    TransactionCanceled { reasons: Vec<CancellationReason> },
    ValidationError { message: String },
    ItemTooLarge { size: u64, max: u64 },
    InvalidKeySchema { message: String },
    InvalidExpression { message: String },
    // ...
}
```

---

## 13. Server Integration

### 13.1 Gateway Router

The server binary needs a top-level gateway that routes requests to the correct service:

```rust
/// Gateway that routes requests to S3 or DynamoDB based on protocol signals.
pub struct GatewayService<S3, DDB> {
    s3: Arc<S3>,
    dynamodb: Arc<DDB>,
}

impl<S3, DDB> Service<Request<Incoming>> for GatewayService<S3, DDB>
where
    S3: S3Handler,
    DDB: DynamoDBHandler,
{
    type Response = Response<GatewayBody>;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        // Route based on X-Amz-Target header
        if let Some(target) = req.headers().get("x-amz-target") {
            if let Ok(s) = target.to_str() {
                if s.starts_with("DynamoDB_") {
                    return self.dynamodb.handle(req);
                }
            }
        }

        // Check health endpoint
        if req.uri().path() == "/_localstack/health" {
            return self.handle_health(req);
        }

        // Default: S3
        self.s3.handle(req)
    }
}
```

### 13.2 Unified Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "available",
        "dynamodb": "available"
    },
    "version": "0.1.0"
}
```

### 13.3 Configuration

```rust
/// DynamoDB configuration.
pub struct DynamoDBConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
}

impl DynamoDBConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("DYNAMODB_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
        }
    }
}
```

### 13.4 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared with S3) |
| `DYNAMODB_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `S3_SKIP_SIGNATURE_VALIDATION` | `true` | Existing S3 config |
| `ACCESS_KEY` / `SECRET_KEY` | *(unset)* | Existing auth config |

---

## 14. Testing Strategy

### 14.1 Unit Tests

Each module tested in isolation:

- **Expression parser**: Test all grammar productions with valid and invalid inputs
- **Expression evaluator**: Test all comparison operators, functions, logical operators
- **Storage engine**: Test put/get/delete, query with sort conditions, scan, secondary indexes
- **Number comparison**: Test ordering with edge cases (negative, decimal, large numbers)
- **Item size calculation**: Test all attribute types including nested structures
- **Condition evaluation**: Test all condition expression patterns

### 14.2 Integration Tests

```rust
// tests/integration/dynamodb_tests.rs

#[tokio::test]
#[ignore] // Run with: cargo test -- --ignored
async fn test_dynamodb_create_and_query_table() {
    let client = aws_sdk_dynamodb::Client::new(&config);
    // Create table, put items, query, scan, delete
}
```

Test against:
- **aws-sdk-rust**: Official AWS SDK for Rust
- **aws-cli**: `aws dynamodb` CLI commands
- **boto3**: Python AWS SDK (most popular DynamoDB client)

### 14.3 Compliance Tests

Research existing DynamoDB test suites:

- **NoSQL Workbench**: AWS's DynamoDB modeling tool has a test mode
- **Dynalite tests**: Dynalite has a comprehensive test suite we can adapt
- **Custom test suite**: Write our own based on the AWS DynamoDB API Reference

### 14.4 Expression Parser Tests

The expression parser needs extensive testing. Use `rstest` for parameterized tests:

```rust
#[rstest]
#[case("attribute_exists(#name)", true)]
#[case("attribute_not_exists(missing)", true)]
#[case("#name = :value", true)]
#[case("size(#list) > :zero", true)]
#[case("#a BETWEEN :low AND :high", true)]
#[case("#x IN (:a, :b, :c)", true)]
#[case("#a = :v1 AND #b = :v2", true)]
#[case("NOT #deleted = :true", true)]
fn test_condition_expression_parsing(#[case] expr: &str, #[case] should_parse: bool) {
    let result = parse_condition_expression(expr);
    assert_eq!(result.is_ok(), should_parse, "expr: {expr}, result: {result:?}");
}
```

---

## 15. Phased Implementation Plan

### Phase 0: MVP (12 Operations)

**Goal**: Cover 90%+ of local development use cases.
**Estimated scope**: ~8,000-10,000 lines of Rust code across 3 new crates.

#### Step 0.1: Codegen Extension
- Add `--service` CLI arg to codegen
- Extract `ServiceConfig` trait
- Implement `DynamoDBServiceConfig`
- Download DynamoDB Smithy model JSON
- Generate `rustack-dynamodb-model` crate
- Hand-write `AttributeValue` with serde

#### Step 0.2: HTTP Layer
- Implement `DynamoDBRouter` (X-Amz-Target dispatch)
- Implement `DynamoDBHttpService` (hyper Service)
- Implement JSON request/response serialization
- Implement JSON error formatting

#### Step 0.3: Storage Engine
- Implement `TableStorage` with B-Tree indexing
- Implement `PrimaryKey` extraction and ordering
- Implement `SortableAttributeValue` with proper comparison
- Implement item size calculation
- Implement GSI/LSI data structures (without population yet)

#### Step 0.4: Expression Parser (Core)
- Implement lexer
- Implement recursive descent parser for condition expressions
- Implement update expression parser
- Implement projection expression parser
- Implement evaluator for all expression types

#### Step 0.5: Operations
- `CreateTable` / `DeleteTable` / `DescribeTable` / `ListTables`
- `PutItem` / `GetItem` / `DeleteItem` (with conditional expressions)
- `UpdateItem` (with update expressions)
- `Query` (with key condition + filter + projection)
- `Scan` (with filter + projection)
- `BatchGetItem` / `BatchWriteItem`

#### Step 0.6: Server Integration
- Implement `GatewayService` for S3+DynamoDB routing
- Rename server binary to `rustack`
- Update Docker image, GitHub Action

#### Step 0.7: Testing
- Unit tests for all components
- Integration tests with aws-sdk-rust
- Integration tests with aws-cli
- Update Makefile with DynamoDB test targets

### Phase 1: Transactions + Advanced (6 Operations)

- `UpdateTable` (add/remove GSI, change billing mode)
- `TransactGetItems` / `TransactWriteItems` (atomicity, conflict detection)
- `DescribeTimeToLive` / `UpdateTimeToLive` (TTL metadata, no background deletion)
- `ExecuteStatement` (PartiQL support)

### Phase 2: Tags, Limits, PartiQL (6 Operations)

- `TagResource` / `UntagResource` / `ListTagsOfResource`
- `DescribeLimits`
- `BatchExecuteStatement` / `ExecuteTransaction`

### Phase 3: Streams, Backups, Global Tables (~40 Operations)

Deferred until user demand materializes. These features are rarely needed for local development.

---

## 16. Risk Analysis

### 16.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Expression parser correctness | High | High | Extensive test suite, reference AWS docs, test against DynamoDB Local |
| Number precision edge cases | Medium | Medium | Use `bigdecimal` crate, test with AWS SDK |
| AttributeValue serde correctness | Medium | High | Test with real AWS SDK requests, match wire format exactly |
| Secondary index consistency | Medium | High | Comprehensive unit tests, fuzzing with random operations |
| Transaction atomicity | Medium | High | Use locking strategy, test concurrent access |
| Smithy model compatibility | Low | Medium | Use official AWS model, regenerate regularly |
| Gateway routing conflicts | Low | Low | X-Amz-Target header is unambiguous |

### 16.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Expression language more complex than expected | Medium | Medium | Phase expression features, start with subset |
| PartiQL parser complexity | High | Low | Defer to Phase 2, focus on expression API first |
| Feature creep (users want more operations) | High | Medium | Strict phasing, return NotImplemented for unsupported ops |

### 16.3 Behavioral Differences

Our implementation will intentionally differ from DynamoDB Local in some areas:

| Behavior | DynamoDB Local | Rustack | Justification |
|----------|---------------|-----------|---------------|
| Table name case | Case-insensitive | Case-sensitive (correct) | AWS DynamoDB is case-sensitive |
| Parallel scan | Ignored (returns all) | Implemented properly | Correct behavior |
| Transaction conflicts | Incomplete detection | Full detection | Correct behavior |
| Provisioned throughput | Ignored | Ignored | Not needed for local dev |
| Table status transitions | Instant | Instant | Not needed for local dev |
| Item collection size | Not enforced | Not enforced initially | Low priority |

---

## 17. Open Questions

### 17.1 Storage Persistence

Should we support optional persistence (write-ahead log or periodic snapshots)? S3 currently does not persist data across restarts. Consistency is simpler without persistence, but some users want it.

**Recommendation**: Defer to a future phase. In-memory only for now, matching S3 behavior.

### 17.2 Number Precision Library

`rust_decimal` (28-29 digits) vs `bigdecimal` (arbitrary precision)? DynamoDB supports up to 38 significant digits.

**Recommendation**: Use `bigdecimal` for correctness. The performance difference is negligible for local development workloads.

### 17.3 PartiQL Support

PartiQL (SQL-compatible query language) is used by some newer SDKs. How much effort should we invest?

**Recommendation**: Defer to Phase 2. PartiQL requires a separate parser (SQL-like grammar). Most SDK users use the expression API, not PartiQL.

### 17.4 Unified vs Separate Server Binary

Should we rename `rustack-s3-server` to `rustack` now, or keep separate binaries?

**Recommendation**: Rename to `rustack` with a gateway router. This matches LocalStack's single-port architecture and simplifies Docker/CI configuration.

### 17.5 DynamoDB Streams

Should we implement DynamoDB Streams for event-driven architectures?

**Recommendation**: Defer to Phase 3. Streams require a separate API endpoint (`DynamoDBStreams_20120810`) and are complex to implement correctly. Focus on core CRUD first.

---

## Appendix A: DynamoDB Attribute Types

| Type | JSON Key | Rust Type | Example JSON |
|------|----------|-----------|-------------|
| String | `S` | `String` | `{"S": "hello"}` |
| Number | `N` | `String` | `{"N": "42.5"}` |
| Binary | `B` | `Bytes` (base64) | `{"B": "dGVzdA=="}` |
| String Set | `SS` | `Vec<String>` | `{"SS": ["a", "b"]}` |
| Number Set | `NS` | `Vec<String>` | `{"NS": ["1", "2"]}` |
| Binary Set | `BS` | `Vec<Bytes>` | `{"BS": ["dA=="]}` |
| Boolean | `BOOL` | `bool` | `{"BOOL": true}` |
| Null | `NULL` | `bool` | `{"NULL": true}` |
| List | `L` | `Vec<AttributeValue>` | `{"L": [{"S": "a"}]}` |
| Map | `M` | `HashMap<String, AttributeValue>` | `{"M": {"k": {"S": "v"}}}` |

## Appendix B: DynamoDB vs S3 Implementation Effort Comparison

| Component | S3 Lines | DynamoDB Est. | Ratio | Notes |
|-----------|----------|--------------|-------|-------|
| Model (codegen output) | ~5,000 | ~4,000 | 0.8x | Fewer operations, no HTTP bindings |
| XML/JSON serde | ~1,200 | ~200 | 0.2x | JSON serde is trivial with derives |
| HTTP routing | ~800 | ~100 | 0.1x | POST-only, header dispatch |
| Auth integration | ~200 | ~100 | 0.5x | SigV4 only, no SigV2/presigned |
| Core business logic | ~4,000 | ~6,000 | 1.5x | More complex (expressions, transactions) |
| Storage engine | ~1,500 | ~2,500 | 1.7x | B-Tree indexing, secondary indexes |
| Expression parser | N/A | ~2,500 | New | Entirely new component |
| **Total** | **~12,700** | **~15,400** | **1.2x** | |

The expression parser is the main source of additional complexity. Without it, DynamoDB would be simpler than S3.

## Appendix C: DynamoDB Error Codes and HTTP Status Codes

| Error Code | HTTP Status | When |
|-----------|------------|------|
| `ResourceInUseException` | 400 | Table already exists |
| `ResourceNotFoundException` | 400 | Table not found |
| `ValidationException` | 400 | Invalid parameters |
| `ConditionalCheckFailedException` | 400 | Condition expression false |
| `TransactionCanceledException` | 400 | Transaction condition failed |
| `TransactionConflictException` | 400 | Concurrent transaction conflict |
| `TransactionInProgressException` | 400 | Item in another transaction |
| `ItemCollectionSizeLimitExceededException` | 400 | Partition > 10GB |
| `ProvisionedThroughputExceededException` | 400 | Throttled |
| `RequestLimitExceeded` | 400 | Too many requests |
| `SerializationException` | 400 | Malformed request |
| `AccessDeniedException` | 400 | Auth failure |
| `InternalServerError` | 500 | Server error |
