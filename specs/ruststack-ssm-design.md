# Rustack SSM Parameter Store: Native Rust Implementation Design

**Date:** 2026-03-02
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-dynamodb-design.md](./rustack-dynamodb-design.md), [SSM Parameter Store Research](../docs/research/ssm-parameter-store-research.md)
**Scope:** Add SSM Parameter Store support to Rustack -- 13 operations covering the full Parameter Store API surface, using the same Smithy-based codegen and gateway routing patterns established by S3 and DynamoDB.

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

This spec proposes adding SSM Parameter Store support to Rustack. Key points:

- **Trivially small scope** -- 13 operations total (compared to DynamoDB's 66 or S3's 90+). Parameter Store is the simplest AWS service we will implement.
- **High value** -- SSM Parameter Store is the standard configuration/secrets management primitive in AWS. Tools like Chamber, Terraform, CDK, Spring Cloud AWS, and ssm-env all depend on it. Adding it unlocks local development for any application that reads config from SSM.
- **Near-zero protocol work** -- SSM uses `awsJson1.1`, which differs from DynamoDB's `awsJson1.0` only in the `Content-Type` version string and `X-Amz-Target` prefix. The entire JSON serialization, routing, and error formatting infrastructure from DynamoDB can be reused.
- **Simple storage model** -- a `DashMap<String, ParameterRecord>` with `BTreeMap<u64, ParameterVersion>` per parameter for version history. No B-Trees, no secondary indexes, no expression parser.
- **Smithy codegen reuse** -- extract the 13 Parameter Store operations from the full SSM Smithy model (146 operations) and generate a `rustack-ssm-model` crate using the same codegen infrastructure as DynamoDB.
- **Estimated effort** -- 2-3 days for MVP (6 operations), 4-5 days for full Parameter Store (13 operations).

---

## 2. Motivation

### 2.1 Why SSM Parameter Store?

SSM Parameter Store is the de facto standard for storing application configuration and secrets in AWS. Every non-trivial AWS application reads configuration from it:

- **Application config** -- database URLs, feature flags, API endpoints stored as `/myapp/prod/db-host`
- **Secrets** -- API keys and passwords stored as SecureString parameters
- **CI/CD** -- Terraform reads/writes SSM for infrastructure config; CDK synthesizes from SSM
- **12-factor config** -- tools like Chamber and ssm-env inject SSM values as environment variables

Without a local SSM, developers must either hard-code config, maintain .env files that diverge from production, or make real AWS API calls during development.

### 2.2 Why It Is Trivially Small

| Dimension | SSM Parameter Store | DynamoDB | S3 |
|-----------|-------------------|----------|-----|
| Total operations | 13 | 66 | 90+ |
| Complex parsers needed | 0 | 1 (expression language) | 1 (XML) |
| Storage engine complexity | HashMap + BTreeMap | B-Tree + GSI/LSI | Object store + multipart |
| Concurrency model | Request/response | Transactions, batch | Multipart upload, streaming |
| Protocol | awsJson1.1 (reuse) | awsJson1.0 (exists) | RestXml (custom) |
| Estimated lines of code | ~3,000 | ~15,000 | ~12,700 |

Parameter Store has no expression language, no secondary indexes, no transactions, no streaming, and no complex state machines. The path hierarchy is implemented with `starts_with()` string matching. The version history is a capped `BTreeMap`. This is as simple as AWS services get.

### 2.3 Tool Coverage

With all 13 operations implemented, the following tools work out of the box:

| Tool | Operations Used | Phase Available |
|------|----------------|-----------------|
| AWS CLI (`aws ssm`) | All CRUD ops | Phase 0 |
| ssm-env (Rust) | GetParametersByPath | Phase 0 |
| confd | GetParametersByPath | Phase 0 |
| Spring Cloud AWS | GetParametersByPath | Phase 0 |
| Terraform | Put/Get/Delete + tags | Phase 1 |
| AWS CDK | GetParameter, PutParameter + tags | Phase 1 |
| Chamber (full) | All 10 core ops | Phase 1 |
| Doppler | PutParameter, GetParametersByPath | Phase 0 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Full Parameter Store API** -- implement all 13 Parameter Store operations
2. **Correct version semantics** -- version auto-increment, 100-version cap, label management
3. **Path hierarchy** -- `GetParametersByPath` with recursive and non-recursive modes
4. **Version and label selectors** -- `name:3` and `name:label` selector syntax in GetParameter/GetParameters
5. **Tag support** -- AddTagsToResource, RemoveTagsFromResource, ListTagsForResource for Parameter resources
6. **DescribeParameters filtering** -- Name, Type, KeyId, Path, Tier, tag, DataType filters
7. **Smithy-generated types** -- all SSM types generated from official AWS Smithy model
8. **Shared infrastructure** -- reuse `rustack-core`, `rustack-auth`, and the awsJson protocol layer
9. **Same Docker image** -- single binary serves S3 + DynamoDB + SSM on port 4566
10. **Pass moto test suite** -- validate against the most comprehensive SSM mock test suite

### 3.2 Non-Goals

1. **Real KMS encryption** -- SecureString parameters stored as plaintext. No KMS integration.
2. **Parameter policies enforcement** -- accept policy JSON but do not enforce expiration/notification
3. **Public parameters** -- `/aws/service/*` paths (AMI IDs, etc.) are not populated
4. **IAM policy enforcement** -- accept but do not enforce resource-based policies
5. **CloudWatch metrics/events** -- no EventBridge integration for parameter changes
6. **Cross-account sharing** -- no AWS RAM integration
7. **DataType validation** -- accept `aws:ec2:image` and `aws:ssm:integration` but do not validate values
8. **Throughput limiting** -- accept throughput settings but do not enforce rate limits
9. **Non-Parameter-Store SSM operations** -- Run Command, Automation, Patch Manager, Session Manager, etc.
10. **Data persistence across restarts** -- in-memory only, matching S3 and DynamoDB behavior

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                    AWS SDK / CLI / Chamber
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  X-Amz-Target dispatch
              +--------+------------+
                       |
         +-------------+-------------+
         |             |             |
         v             v             v
   +-----------+ +----------+ +-----------+
   | S3 HTTP   | | DDB HTTP | | SSM HTTP  |
   | (RestXml) | | (Json10) | | (Json11)  |
   +-----+-----+ +----+-----+ +-----+----+
         |             |             |
   +-----+-----+ +----+-----+ +-----+----+
   | S3 Core   | | DDB Core | | SSM Core |
   +-----+-----+ +----+-----+ +-----+----+
         |             |             |
         +------+------+------+------+
                |
         +------+------+
         | rustack-  |
         | core + auth |
         +-------------+
```

### 4.2 Gateway Routing

SSM requests are distinguished by the `X-Amz-Target` header prefix:

| Service | X-Amz-Target Prefix | Content-Type |
|---------|---------------------|--------------|
| DynamoDB | `DynamoDB_20120810.` | `application/x-amz-json-1.0` |
| SSM | `AmazonSSM.` | `application/x-amz-json-1.1` |
| S3 | *(absent)* | varies |

Routing logic: check `X-Amz-Target` header. If prefix is `AmazonSSM.`, route to SSM. If prefix is `DynamoDB_`, route to DynamoDB. Otherwise, default to S3. This is unambiguous.

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-ssm-model        <-- NEW (auto-generated)
+-- rustack-ssm-core         <-- NEW
+-- rustack-ssm-http         <-- NEW

rustack-ssm-http
+-- rustack-ssm-model
+-- rustack-auth

rustack-ssm-core
+-- rustack-core
+-- rustack-ssm-model

rustack-ssm-model (auto-generated, standalone)
```

---

## 5. Protocol Design: awsJson1.1

### 5.1 Protocol Comparison

SSM uses `awsJson1.1`, which is identical to DynamoDB's `awsJson1.0` in all aspects except the Content-Type version string.

| Aspect | DynamoDB (awsJson1.0) | SSM (awsJson1.1) |
|--------|----------------------|-------------------|
| HTTP Method | POST only | POST only |
| URL Path | `/` always | `/` always |
| Content-Type | `application/x-amz-json-1.0` | `application/x-amz-json-1.1` |
| X-Amz-Target | `DynamoDB_20120810.<Op>` | `AmazonSSM.<Op>` |
| Request body | JSON | JSON |
| Response body | JSON | JSON |
| Error `__type` | `com.amazonaws.dynamodb.v20120810#Err` | `Err` (short name) |
| Timestamp format | Epoch seconds (double) | Epoch seconds (double) |
| Auth | SigV4, service=`dynamodb` | SigV4, service=`ssm` |

The only functional difference is the Content-Type version and the target prefix. JSON serialization, request dispatch, and error formatting are identical.

### 5.2 What We Reuse from DynamoDB

The DynamoDB implementation provides all the infrastructure SSM needs:

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| JSON request deserialization | Yes | `serde_json::from_slice` with `Deserialize` derives |
| JSON response serialization | Yes | `serde_json::to_vec` with `Serialize` derives |
| `X-Amz-Target` header parsing | Yes | Same pattern, different prefix |
| JSON error formatting | Yes | Same `{"__type": "...", "message": "..."}` format |
| SigV4 auth | Yes | `rustack-auth` is service-agnostic |
| Multi-account/region state | Yes | `rustack-core` unchanged |

### 5.3 No Legacy Compatibility Needed

Unlike DynamoDB (which has some awsQuery-era behaviors) or S3 (which has SigV2 and virtual hosting), SSM was introduced in 2014 with `awsJson1.1` from the start. There is no legacy protocol to support.

---

## 6. Smithy Code Generation Strategy

### 6.1 Approach: Extract Parameter Store Subset

The full SSM Smithy model (`ssm-2014-11-06.json` from `aws/api-models-aws`) defines 146 operations. We generate types for only the 13 Parameter Store operations plus their transitive type dependencies.

### 6.2 SSM Service Config

```rust
const SSM_PARAMETER_STORE_OPERATIONS: &[&str] = &[
    "PutParameter",
    "GetParameter",
    "GetParameters",
    "GetParametersByPath",
    "GetParameterHistory",
    "DescribeParameters",
    "DeleteParameter",
    "DeleteParameters",
    "LabelParameterVersion",
    "UnlabelParameterVersion",
    "AddTagsToResource",
    "RemoveTagsFromResource",
    "ListTagsForResource",
];
```

The codegen `ServiceConfig` trait (from the DynamoDB design) is extended with an SSM implementation:

```rust
pub struct SsmServiceConfig;

impl ServiceConfig for SsmServiceConfig {
    fn namespace(&self) -> &str { "com.amazonaws.ssm#" }
    fn service_name(&self) -> &str { "SSM" }
    fn target_operations(&self) -> &[&str] { SSM_PARAMETER_STORE_OPERATIONS }
    fn protocol(&self) -> Protocol { Protocol::AwsJson1_1 }
    // ...
}
```

### 6.3 Smithy Model Acquisition

The SSM Smithy model is available at:
- **Repository:** `https://github.com/aws/api-models-aws`
- **Path:** `models/ssm/service/2014-11-06/ssm-2014-11-06.json`

Download and place at `codegen/smithy-model/ssm.json`. The codegen tool resolves transitive type dependencies from the 13 target operations, generating only the types actually needed.

### 6.4 Generated Types Estimate

From the 13 operations, the codegen will produce roughly:

- 13 input structs (e.g., `PutParameterInput`, `GetParameterInput`)
- 13 output structs (e.g., `PutParameterOutput`, `GetParameterOutput`)
- ~15 shared types (`Parameter`, `ParameterMetadata`, `ParameterHistory`, `Tag`, `ParameterType`, `ParameterTier`, `ParameterStringFilter`, etc.)
- 1 operation enum (`SSMOperation` with 13 variants)
- ~20 error types

Total: roughly 1,000-1,500 lines of generated code, compared to DynamoDB's ~4,000 lines.

### 6.5 Makefile Integration

```makefile
codegen-ssm:
	@cd codegen && cargo run -- --service ssm
	@cargo +nightly fmt -p rustack-ssm-model

codegen: codegen-s3 codegen-dynamodb codegen-ssm
```

---

## 7. Crate Structure

### 7.1 `rustack-ssm-model` (auto-generated)

```
crates/rustack-ssm-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports
    +-- types.rs            # Auto-generated: enums + shared structs
    +-- operations.rs       # Auto-generated: SSMOperation enum
    +-- error.rs            # Auto-generated: SSMError + error codes
    +-- input.rs            # Auto-generated: all 13 input structs
    +-- output.rs           # Auto-generated: all 13 output structs
```

**Dependencies:** `serde`, `serde_json`

No hand-written types needed. SSM has no equivalent of DynamoDB's `AttributeValue` -- all types are straightforward structs and enums that serde handles natively.

### 7.2 `rustack-ssm-core`

```
crates/rustack-ssm-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # SSMConfig
    +-- provider.rs         # RustackSSM (main provider, all 13 handlers)
    +-- error.rs            # SSMServiceError
    +-- state.rs            # ParameterStore (DashMap<String, ParameterRecord>)
    +-- storage.rs          # ParameterRecord, ParameterVersion, path matching
    +-- filter.rs           # DescribeParameters filter evaluation
    +-- selector.rs         # Version/label selector parsing (name:3, name:label)
    +-- validation.rs       # Parameter name, value, label, tag validation
```

**Dependencies:** `rustack-core`, `rustack-ssm-model`, `dashmap`, `serde_json`, `chrono`, `tracing`

### 7.3 `rustack-ssm-http`

```
crates/rustack-ssm-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # AmazonSSM.* target dispatch
    +-- service.rs          # SSMHttpService (hyper Service impl)
    +-- dispatch.rs         # SSMHandler trait + operation dispatch
```

**Dependencies:** `rustack-ssm-model`, `rustack-auth`, `hyper`, `serde_json`, `bytes`

This crate is structurally identical to `rustack-dynamodb-http`. The router parses `AmazonSSM.<Op>` instead of `DynamoDB_20120810.<Op>`. Request deserialization and response serialization use the same `serde_json` machinery.

### 7.4 Workspace Changes

```toml
[workspace.dependencies]
rustack-ssm-model = { path = "crates/rustack-ssm-model" }
rustack-ssm-http = { path = "crates/rustack-ssm-http" }
rustack-ssm-core = { path = "crates/rustack-ssm-core" }
```

---

## 8. HTTP Layer Design

### 8.1 Router

```rust
/// SSM operation router.
///
/// Parses the `X-Amz-Target: AmazonSSM.<Op>` header to determine the operation.
pub struct SSMRouter;

impl SSMRouter {
    pub fn resolve(target: &str) -> Result<SSMOperation, SSMError> {
        let op_name = target
            .strip_prefix("AmazonSSM.")
            .ok_or_else(|| SSMError::unknown_operation(target))?;

        SSMOperation::from_name(op_name)
            .ok_or_else(|| SSMError::unknown_operation(op_name))
    }
}
```

### 8.2 ServiceRouter Trait Implementation

```rust
/// SSM service router for the gateway.
pub struct SSMServiceRouter {
    handler: Arc<RustackSSM>,
    config: SSMHttpConfig,
}

impl ServiceRouter for SSMServiceRouter {
    fn matches(&self, req: &Request<()>) -> bool {
        req.headers()
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.starts_with("AmazonSSM."))
    }

    async fn handle(&self, req: Request<Incoming>) -> Response<Body> {
        // 1. Extract X-Amz-Target, resolve to SSMOperation
        // 2. Read body bytes
        // 3. Deserialize JSON input
        // 4. Dispatch to handler
        // 5. Serialize JSON output or error
    }
}
```

### 8.3 Non-Parameter-Store Operations

When SSM receives a target like `AmazonSSM.SendCommand` (a Run Command operation), we return a structured error rather than silently failing:

```json
{
    "__type": "InvalidAction",
    "message": "Operation SendCommand is not supported. Only Parameter Store operations are implemented."
}
```

This is a deliberate choice: match all `AmazonSSM.*` targets at the gateway level, but return an explicit error for the 133 non-Parameter-Store operations. This prevents confusing "connection refused" or S3-format errors when an SDK accidentally sends SSM requests.

---

## 9. Storage Engine Design

### 9.1 Overview

The storage model is a flat hashmap of parameters with versioned history. There is no tree structure for paths -- hierarchy is implemented as string prefix matching.

### 9.2 Core Data Structures

```rust
/// Top-level parameter store.
/// Keyed by (account_id, region) via rustack-core, then by parameter name.
pub struct ParameterStore {
    /// All parameters keyed by name.
    parameters: DashMap<String, ParameterRecord>,
}

/// A single parameter with its version history and metadata.
pub struct ParameterRecord {
    /// Parameter name (e.g., "/myapp/prod/db-host").
    pub name: String,
    /// Current (latest) version number.
    pub current_version: u64,
    /// Version history. BTreeMap for ordered iteration.
    /// Capped at 100 entries.
    pub versions: BTreeMap<u64, ParameterVersion>,
    /// Tags on the parameter resource (not per-version).
    pub tags: HashMap<String, String>,
    /// Parameter type (String, StringList, SecureString).
    /// Immutable after creation -- cannot change type on overwrite.
    pub parameter_type: ParameterType,
    /// KMS key ID for SecureString (stored but not used for encryption).
    pub key_id: Option<String>,
}

/// A single version of a parameter.
pub struct ParameterVersion {
    /// Version number (1-indexed, auto-incremented).
    pub version: u64,
    /// Parameter value.
    pub value: String,
    /// Description (optional, up to 1024 chars).
    pub description: Option<String>,
    /// Allowed pattern regex (optional).
    pub allowed_pattern: Option<String>,
    /// Data type ("text" for our purposes).
    pub data_type: String,
    /// Parameter tier (Standard, Advanced, Intelligent-Tiering).
    pub tier: ParameterTier,
    /// Labels attached to this version (max 10).
    pub labels: HashSet<String>,
    /// Parameter policies (stored as JSON, not enforced).
    pub policies: Vec<String>,
    /// Last modified timestamp (epoch seconds as f64).
    pub last_modified_date: f64,
    /// Last modified user ARN.
    pub last_modified_user: String,
}
```

### 9.3 Path Hierarchy

Path matching for `GetParametersByPath` is pure string matching:

```rust
impl ParameterStore {
    /// Get parameters under a path, with optional recursion.
    fn get_by_path(
        &self,
        path: &str,
        recursive: bool,
    ) -> Vec<(&String, &ParameterRecord)> {
        let normalized = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };

        self.parameters.iter()
            .filter(|entry| {
                let name = entry.key();
                if !name.starts_with(&normalized) {
                    return false;
                }
                if !recursive {
                    // Non-recursive: only direct children (no further `/` after prefix)
                    let remainder = &name[normalized.len()..];
                    !remainder.contains('/')
                } else {
                    true
                }
            })
            .collect()
    }
}
```

There is no tree data structure. With typical local dev parameter counts (tens to hundreds), linear scanning with `starts_with` is more than adequate.

### 9.4 Version Management

Each `PutParameter` with `Overwrite: true` increments the version counter and adds a new `ParameterVersion` entry.

```rust
impl ParameterRecord {
    /// Add a new version. Enforces the 100-version cap.
    fn add_version(&mut self, version: ParameterVersion) -> Result<u64, SSMServiceError> {
        if self.versions.len() >= 100 {
            // Check if oldest version has labels
            let oldest_version = *self.versions.keys().next().unwrap();
            let oldest = &self.versions[&oldest_version];
            if !oldest.labels.is_empty() {
                return Err(SSMServiceError::ParameterMaxVersionLimitExceeded {
                    name: self.name.clone(),
                });
            }
            // Remove oldest version
            self.versions.remove(&oldest_version);
        }
        self.current_version += 1;
        let v = self.current_version;
        self.versions.insert(v, version);
        Ok(v)
    }
}
```

### 9.5 Label Management

Labels are unique per parameter across all versions. When a label is attached to a new version, it must be detached from any previous version first.

```rust
impl ParameterRecord {
    /// Attach labels to a specific version.
    fn label_version(
        &mut self,
        version: u64,
        labels: &[String],
    ) -> Result<Vec<String>, SSMServiceError> {
        let mut invalid = Vec::new();
        for label in labels {
            // Validate label format
            if !is_valid_label(label) {
                invalid.push(label.clone());
                continue;
            }
            // Remove label from any other version
            for (_, v) in &mut self.versions {
                v.labels.remove(label);
            }
            // Check version-level limit (max 10 labels per version)
            let target = self.versions.get_mut(&version)
                .ok_or(SSMServiceError::ParameterVersionNotFound {
                    name: self.name.clone(),
                    version,
                })?;
            if target.labels.len() >= 10 {
                return Err(SSMServiceError::ParameterVersionLabelLimitExceeded {
                    name: self.name.clone(),
                    version,
                });
            }
            target.labels.insert(label.clone());
        }
        Ok(invalid)
    }
}
```

### 9.6 Version and Label Selectors

`GetParameter` and `GetParameters` support selector syntax in the name:

```rust
/// Parse a parameter name with optional version/label selector.
///
/// Formats:
///   "name"       -> (name, None)
///   "name:3"     -> (name, Some(Version(3)))
///   "name:label" -> (name, Some(Label("label")))
pub fn parse_selector(input: &str) -> (String, Option<Selector>) {
    match input.rsplit_once(':') {
        Some((name, suffix)) => {
            if let Ok(v) = suffix.parse::<u64>() {
                (name.to_string(), Some(Selector::Version(v)))
            } else {
                (name.to_string(), Some(Selector::Label(suffix.to_string())))
            }
        }
        None => (input.to_string(), None),
    }
}

pub enum Selector {
    Version(u64),
    Label(String),
}
```

### 9.7 SecureString Handling

For local development, SecureString parameters are stored as plaintext. The `Type: SecureString` field is preserved in responses. When `WithDecryption: false` is specified for a SecureString, the value is still returned as-is (AWS returns a KMS ciphertext blob, but simulating that adds no value for local dev). The `KeyId` field is stored but not used for encryption; it defaults to `alias/aws/ssm`.

### 9.8 Concurrency Model

Parameter Store has no real-time constraints, no streaming, and no background processing. A `DashMap` provides sufficient concurrent access:

- **Reads** (Get, GetByPath, Describe, History, ListTags): lock-free concurrent reads
- **Writes** (Put, Delete, Label, Unlabel, AddTags, RemoveTags): per-entry write locks via DashMap

No actors, no channels, no background tasks. This is straightforward request/response processing.

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main SSM Parameter Store provider implementing all 13 operations.
pub struct RustackSSM {
    pub(crate) state: Arc<ParameterStore>,
    pub(crate) config: Arc<SSMConfig>,
}

impl RustackSSM {
    pub fn new(config: SSMConfig) -> Self;
}
```

### 10.2 Operations

#### Phase 0: Core CRUD (6 operations)

**PutParameter** -- Create or update a parameter.

1. Validate name (format, length, reserved prefixes)
2. If parameter exists and `Overwrite` is false, return `ParameterAlreadyExists`
3. If parameter exists and `Overwrite` is true, validate type has not changed
4. If `AllowedPattern` is set, validate value matches regex
5. Create new `ParameterVersion`, add to record (enforcing 100-version cap)
6. If creating (not overwriting) and `Tags` is set, store tags
7. Return `{ Version, Tier }`

**GetParameter** -- Get a single parameter.

1. Parse selector from name (`name:3` or `name:label`)
2. Look up parameter record
3. If selector, resolve to specific version; otherwise use latest
4. Build `Parameter` response with ARN, value, metadata
5. Return `{ Parameter }`

**GetParameters** -- Batch get up to 10 parameters.

1. For each name, parse selector and attempt lookup
2. Found parameters go in `Parameters` list
3. Missing parameters go in `InvalidParameters` list
4. Deduplicate: if the same name appears multiple times, return it once

**GetParametersByPath** -- Get parameters under a hierarchy path.

1. Normalize path (ensure trailing `/`)
2. Scan parameters matching prefix
3. If `Recursive: false`, filter to direct children only
4. Apply `ParameterFilters` if present (Type, KeyId, Label)
5. Paginate with `MaxResults` (1-10) and `NextToken`
6. Return `{ Parameters, NextToken }`

**DeleteParameter** -- Delete a single parameter.

1. Look up parameter; return `ParameterNotFound` if missing
2. Remove from store (all versions, labels, tags)
3. Return `{}`

**DeleteParameters** -- Batch delete up to 10 parameters.

1. For each name, attempt deletion
2. Deleted names go in `DeletedParameters`
3. Missing names go in `InvalidParameters`
4. Return `{ DeletedParameters, InvalidParameters }`

#### Phase 1: Metadata and Tags (5 operations)

**DescribeParameters** -- List/search parameter metadata.

1. Apply `ParameterFilters` as predicate chain:
   - `Name` with option `BeginsWith` or `Equals`
   - `Type` with option `Equals`
   - `KeyId` with option `Equals`
   - `Path` with option `Recursive` or `OneLevel`
   - `Tier` with option `Equals`
   - `tag:<key>` with option `Equals`
   - `DataType` with option `Equals`
2. Paginate with `MaxResults` (1-50) and `NextToken`
3. Return `ParameterMetadata` objects (no values)

**GetParameterHistory** -- Get version history.

1. Look up parameter; return `ParameterNotFound` if missing
2. Iterate versions in order, building `ParameterHistory` objects
3. Include labels, description, policies for each version
4. Paginate with `MaxResults` (1-50) and `NextToken`

**AddTagsToResource** -- Add tags to a parameter.

1. Validate `ResourceType` is `"Parameter"`
2. Look up parameter by `ResourceId`; return `InvalidResourceId` if missing
3. Merge new tags into existing tags (overwrite existing keys)
4. Enforce 50-tag limit
5. Return `{}`

**RemoveTagsFromResource** -- Remove tags from a parameter.

1. Validate `ResourceType` is `"Parameter"`
2. Look up parameter; return `InvalidResourceId` if missing
3. Remove specified tag keys
4. Return `{}`

**ListTagsForResource** -- List tags on a parameter.

1. Validate `ResourceType` is `"Parameter"`
2. Look up parameter; return `InvalidResourceId` if missing
3. Return `{ TagList }`

#### Phase 2: Labels (2 operations)

**LabelParameterVersion** -- Attach labels to a version.

1. Look up parameter; return `ParameterNotFound` if missing
2. If `ParameterVersion` omitted, use latest version
3. Validate each label (format, length, reserved prefixes)
4. Move labels from any existing version to target version
5. Enforce 10-label-per-version limit
6. Return `{ InvalidLabels, ParameterVersion }`

**UnlabelParameterVersion** -- Remove labels from a version.

1. Look up parameter; return `ParameterNotFound` if missing
2. Remove specified labels from the specified version
3. Labels not found on that version go in `InvalidLabels`
4. Return `{ InvalidLabels, RemovedLabels }`

### 10.3 ARN Construction

```rust
fn parameter_arn(region: &str, account_id: &str, name: &str) -> String {
    // Names starting with "/" get a single "/" separator
    // Names not starting with "/" get "/" prepended
    if name.starts_with('/') {
        format!("arn:aws:ssm:{region}:{account_id}:parameter{name}")
    } else {
        format!("arn:aws:ssm:{region}:{account_id}:parameter/{name}")
    }
}
```

For local dev: `account_id` defaults to `"000000000000"`, `region` from config.

### 10.4 Validation Rules

| Field | Rule |
|-------|------|
| Parameter name | 1-2048 chars, `[a-zA-Z0-9_.\-/]`, no spaces, cannot prefix with `aws` or `ssm` (case-insensitive) |
| Parameter value | Max 4 KB (Standard) / 8 KB (Advanced) |
| Description | Max 1024 chars |
| AllowedPattern | Valid regex, max 1024 chars |
| Hierarchy depth | Max 15 levels (count `/` separators) |
| Label | 1-100 chars, `[a-zA-Z0-9_.\-]`, cannot start with `aws`, `ssm`, or a digit |
| Tag key | 1-128 chars |
| Tag value | 0-256 chars |
| Tags per parameter | Max 50 |
| Labels per version | Max 10 |
| Versions per parameter | Max 100 (oldest auto-deleted if unlabeled) |

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// SSM service errors mapped to API error types.
pub enum SSMServiceError {
    /// Parameter does not exist.
    ParameterNotFound { name: String },
    /// Parameter already exists and Overwrite=false.
    ParameterAlreadyExists { name: String },
    /// 100-version limit reached, oldest has labels.
    ParameterMaxVersionLimitExceeded { name: String },
    /// Specified version does not exist.
    ParameterVersionNotFound { name: String, version: u64 },
    /// 10-label-per-version limit reached.
    ParameterVersionLabelLimitExceeded { name: String, version: u64 },
    /// More than 15 hierarchy levels.
    HierarchyLevelLimitExceeded { name: String },
    /// Cannot change parameter type on overwrite.
    HierarchyTypeMismatch { name: String },
    /// Value does not match AllowedPattern.
    InvalidAllowedPattern { name: String, pattern: String },
    /// Invalid parameter name format.
    ParameterPatternMismatch { name: String, message: String },
    /// Invalid filter key/option/value.
    InvalidFilterKey { message: String },
    InvalidFilterOption { message: String },
    InvalidFilterValue { message: String },
    /// Invalid pagination token.
    InvalidNextToken,
    /// Invalid resource ID for tag operations.
    InvalidResourceId { resource_id: String },
    /// Invalid resource type for tag operations.
    InvalidResourceType { resource_type: String },
    /// Unsupported parameter type.
    UnsupportedParameterType { type_name: String },
    /// Too many tags (> 50).
    TooManyTagsError,
    /// Internal error.
    InternalServerError { message: String },
}
```

### 11.2 Error Mapping

```rust
impl SSMServiceError {
    /// Map to HTTP status code and __type string.
    pub fn to_error_response(&self) -> (u16, &'static str, String) {
        match self {
            Self::ParameterNotFound { name } =>
                (400, "ParameterNotFound",
                 format!("Parameter {name} not found.")),
            Self::ParameterAlreadyExists { name } =>
                (400, "ParameterAlreadyExists",
                 format!("The parameter already exists. To overwrite, set Overwrite: true.")),
            Self::ParameterMaxVersionLimitExceeded { .. } =>
                (400, "ParameterMaxVersionLimitExceeded",
                 "The parameter has reached the maximum number of versions.".into()),
            Self::InternalServerError { message } =>
                (500, "InternalServerError", message.clone()),
            // ... etc
        }
    }
}
```

### 11.3 Error Response Format

```json
{
    "__type": "ParameterNotFound",
    "message": "Parameter /myapp/prod/missing not found."
}
```

SSM uses short error type names (no namespace prefix), unlike DynamoDB which uses fully-qualified names. The `__type` field contains just the error shape name.

---

## 12. Server Integration

### 12.1 Feature Gate

SSM support is gated behind a cargo feature:

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "ssm"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
ssm = ["dep:rustack-ssm-core", "dep:rustack-ssm-http"]
```

### 12.2 Gateway Registration

SSM is registered in the gateway before S3 (S3 is the catch-all):

```rust
// In gateway setup
let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

#[cfg(feature = "dynamodb")]
services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_handler)));

#[cfg(feature = "ssm")]
services.push(Box::new(SSMServiceRouter::new(ssm_handler)));

// S3 is always last (catch-all for requests without X-Amz-Target)
#[cfg(feature = "s3")]
services.push(Box::new(S3ServiceRouter::new(s3_handler)));
```

### 12.3 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "available",
        "dynamodb": "available",
        "ssm": "available"
    },
    "version": "0.2.0"
}
```

### 12.4 Configuration

```rust
pub struct SSMConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub default_account_id: String,
}

impl SSMConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("SSM_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            default_account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
        }
    }
}
```

### 12.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared) |
| `SSM_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 for SSM |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default account for ARNs |

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **Selector parsing**: test `name:3`, `name:label`, `name`, edge cases with colons in names
- **Path matching**: recursive vs non-recursive, trailing slash normalization, depth counting
- **Version management**: 100-version cap, oldest deletion, label-blocks-deletion
- **Label management**: uniqueness across versions, label moving, format validation
- **Filter evaluation**: all DescribeParameters filter keys and options
- **Validation**: parameter names, labels, tags, hierarchy depth

### 13.2 Integration Tests with aws-sdk-ssm

```rust
// tests/integration/ssm_tests.rs
#[tokio::test]
#[ignore]
async fn test_ssm_put_get_delete_parameter() {
    let client = aws_sdk_ssm::Client::new(&config);
    // PutParameter, GetParameter, DeleteParameter round-trip
}

#[tokio::test]
#[ignore]
async fn test_ssm_parameter_versioning() {
    // Put same parameter 5 times with Overwrite
    // GetParameterHistory, verify 5 versions
    // GetParameter with :version selector
}

#[tokio::test]
#[ignore]
async fn test_ssm_get_parameters_by_path() {
    // Create /app/prod/a, /app/prod/b, /app/staging/c
    // GetParametersByPath("/app/prod/") -> [a, b]
    // GetParametersByPath("/app/", recursive=true) -> [a, b, c]
}
```

### 13.3 moto Test Suite (Primary Compatibility Reference)

The moto project (`getmoto/moto`, `tests/test_ssm/test_ssm.py`) contains 66+ test functions covering SSM Parameter Store. This is the most comprehensive SSM mock test suite available.

**Porting strategy:** Convert the relevant Parameter Store tests from moto's Python/boto3 format to Rust integration tests using `aws-sdk-ssm`. Key test areas to port:

| Test Area | moto Test Count | Priority |
|-----------|----------------|----------|
| Put/Get/Delete basics | ~10 | Phase 0 |
| Overwrite and versioning | ~5 | Phase 0 |
| GetParametersByPath | ~3 | Phase 0 |
| GetParameters batch (including invalid) | ~5 | Phase 0 |
| DescribeParameters filters | ~12 | Phase 1 |
| GetParameterHistory | ~6 | Phase 1 |
| Tags (add/remove/list) | ~5 | Phase 1 |
| Label management | ~10 | Phase 2 |
| Version limit edge cases | ~3 | Phase 2 |
| Error conditions (invalid names, types) | ~7 | All phases |

### 13.4 LocalStack Test Suite

The vendored LocalStack tests (`vendors/localstack/tests/aws/services/ssm/test_ssm.py`) contain 11 tests covering core happy paths. These serve as quick smoke tests and are already available in the repository.

### 13.5 Chamber End-to-End Validation

Chamber (`segmentio/chamber`) uses all 10 core Parameter Store operations and is the most popular SSM management CLI. Testing with Chamber validates the entire API surface in a realistic workflow:

```makefile
test-ssm-chamber:
	@echo "Starting Rustack..."
	@./target/release/rustack &
	@sleep 1
	@AWS_REGION=us-east-1 \
	 AWS_ACCESS_KEY_ID=test \
	 AWS_SECRET_ACCESS_KEY=test \
	 AWS_SSM_ENDPOINT=http://localhost:4566 \
	 chamber write myapp/prod db-host db.example.com
	@chamber read myapp/prod db-host
	@chamber list myapp/prod
	@chamber history myapp/prod db-host
	@chamber delete myapp/prod db-host
```

### 13.6 AWS CLI Smoke Tests

Shell-based end-to-end tests using the AWS CLI:

```bash
# Put parameter
aws ssm put-parameter --name "/test/param1" --value "hello" --type String \
    --endpoint-url http://localhost:4566

# Get parameter
aws ssm get-parameter --name "/test/param1" \
    --endpoint-url http://localhost:4566

# Get parameters by path
aws ssm get-parameters-by-path --path "/test/" --recursive \
    --endpoint-url http://localhost:4566

# Delete parameter
aws ssm delete-parameter --name "/test/param1" \
    --endpoint-url http://localhost:4566
```

---

## 14. Phased Implementation Plan

### Phase 0: MVP (6 Operations)

**Goal:** Core CRUD + path queries. Covers AWS CLI, ssm-env, confd, basic Terraform/CDK.
**Estimated effort:** 2-3 days.

#### Step 0.1: Codegen
- Add SSM service config to codegen
- Download SSM Smithy model, extract Parameter Store subset
- Generate `rustack-ssm-model` crate
- Verify generated types compile and serde round-trip

#### Step 0.2: HTTP Layer
- Implement `SSMRouter` (AmazonSSM.* dispatch)
- Implement `SSMHttpService` (reuse DynamoDB's JSON protocol pattern)
- Implement `SSMServiceRouter` for gateway integration
- Wire into gateway with feature gate

#### Step 0.3: Storage Engine
- Implement `ParameterStore` with `DashMap`
- Implement `ParameterRecord` with `BTreeMap<u64, ParameterVersion>`
- Implement path matching for `GetParametersByPath`
- Implement selector parsing (`name:3`, `name:label`)
- Implement parameter name and value validation

#### Step 0.4: Core Operations
- `PutParameter` (create, overwrite, version increment, validation)
- `GetParameter` (with version/label selector)
- `GetParameters` (batch, with InvalidParameters)
- `GetParametersByPath` (recursive/non-recursive, pagination)
- `DeleteParameter`
- `DeleteParameters` (batch)

#### Step 0.5: Testing
- Unit tests for storage, selectors, path matching, validation
- Integration tests with `aws-sdk-ssm`
- AWS CLI smoke tests

### Phase 1: Metadata, History, Tags (5 Operations)

**Goal:** Full Chamber support, Terraform lifecycle.
**Estimated effort:** 1-2 days.

- `DescribeParameters` (with all filter types)
- `GetParameterHistory` (with pagination)
- `AddTagsToResource` (Parameter resource type only)
- `RemoveTagsFromResource`
- `ListTagsForResource`
- Port moto tests for filters, history, tags
- Chamber end-to-end validation

### Phase 2: Labels (2 Operations)

**Goal:** Full Parameter Store API.
**Estimated effort:** 1 day.

- `LabelParameterVersion`
- `UnlabelParameterVersion`
- Port moto label management tests
- Version limit edge case tests (100-version cap with labeled oldest)

---

## 15. Risk Analysis

### 15.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Smithy codegen SSM model parsing | Low | Medium | Same codegen used for DynamoDB; SSM model is simpler |
| DescribeParameters filter edge cases | Medium | Low | Port moto filter tests; they cover all filter combinations |
| Pagination token format | Low | Low | Use opaque base64-encoded offset; same pattern as DynamoDB |
| GetParametersByPath depth counting | Low | Low | Simple `/` counting; well-tested in moto |
| SecureString behavior differences | Medium | Low | Document plaintext storage; accept WithDecryption but ignore |

### 15.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Users want non-Parameter-Store SSM ops | Low | Low | Return structured error; add operations on demand |
| Users expect real KMS encryption | Low | Low | Document as non-goal; local dev does not need encryption |
| Chamber expects specific error formats | Medium | Medium | Test with Chamber early; fix error format mismatches |

### 15.3 Behavioral Differences from AWS

| Behavior | AWS | Rustack | Justification |
|----------|-----|-----------|---------------|
| SecureString encryption | KMS-encrypted | Plaintext | No KMS service; local dev does not need encryption |
| Parameter policies | Enforced (expiration, notification) | Stored but not enforced | No background enforcement engine |
| Public parameters | Available under `/aws/service/*` | Not populated | Would require replicating AWS's public parameter data |
| DataType validation | `aws:ec2:image` validates AMI IDs | Accepted but not validated | No EC2 service to validate against |
| Throughput limiting | 40-1000 TPS depending on settings | Unlimited | Not meaningful for local dev |
| Cross-account sharing | Supported via AWS RAM | Not supported | Single-account local dev scenario |

### 15.4 Implementation Effort Comparison

| Component | SSM Estimate | DynamoDB | Ratio |
|-----------|-------------|----------|-------|
| Model (codegen) | ~1,200 | ~4,000 | 0.3x |
| HTTP routing | ~100 | ~100 | 1.0x |
| Storage engine | ~500 | ~2,500 | 0.2x |
| Business logic | ~1,200 | ~6,000 | 0.2x |
| Expression parser | 0 | ~2,500 | 0.0x |
| **Total** | **~3,000** | **~15,400** | **0.2x** |

SSM Parameter Store is approximately one-fifth the implementation effort of DynamoDB. The absence of an expression language, secondary indexes, and transaction semantics makes this the simplest AWS service in the Rustack roadmap.
