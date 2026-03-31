# Rustack Secrets Manager: Native Rust Implementation Design

**Date:** 2026-03-06
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-ssm-design.md](./rustack-ssm-design.md)
**Scope:** Add AWS Secrets Manager support to Rustack -- 23 operations covering the full Secrets Manager API surface, using the same Smithy-based codegen and gateway routing patterns established by DynamoDB and SSM.

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

This spec proposes adding AWS Secrets Manager support to Rustack. Key points:

- **Moderate scope** -- 23 operations total, comparable in complexity to SSM Parameter Store (13 operations) but with richer versioning semantics. Secrets Manager introduces version staging labels (AWSCURRENT, AWSPREVIOUS, AWSPENDING), secret rotation lifecycle, and scheduled deletion -- concepts absent from Parameter Store.
- **High value** -- Secrets Manager is the primary secrets management service in AWS. Tools like Terraform, CDK, external-secrets-operator (Kubernetes), Spring Cloud AWS, Chamber, and virtually every AWS SDK-based application depend on it. Adding it unlocks local development and CI testing for any application that reads secrets from Secrets Manager.
- **Near-zero protocol work** -- Secrets Manager uses `awsJson1.1`, identical to SSM. The `X-Amz-Target` prefix is `secretsmanager.` and Content-Type is `application/x-amz-json-1.1`. The entire JSON serialization, routing, and error formatting infrastructure from SSM can be reused.
- **Version staging labels are the core complexity** -- unlike SSM Parameter Store's simple auto-incrementing version numbers, Secrets Manager tracks versions by UUID and assigns staging labels (AWSCURRENT, AWSPREVIOUS, AWSPENDING, plus custom labels) that can be moved between versions. This requires a more sophisticated version management model.
- **Smithy codegen reuse** -- generate a `rustack-secretsmanager-model` crate from the official Smithy model (`secretsmanager-2017-10-17.json`) using the same codegen infrastructure as DynamoDB and SSM.
- **Estimated effort** -- 3-4 days for Phase 0 (10 core operations), 5-7 days for full implementation (20 operations), plus 1 day for CI integration.

---

## 2. Motivation

### 2.1 Why Secrets Manager?

AWS Secrets Manager is the standard for storing, retrieving, and rotating application secrets. It is distinct from SSM Parameter Store in purpose and API:

- **Application secrets** -- database credentials, API keys, OAuth tokens, TLS certificates
- **Secret rotation** -- automated credential rotation via Lambda (or state machine in local dev)
- **Kubernetes integration** -- external-secrets-operator reads from Secrets Manager and syncs to K8s Secrets
- **Terraform/CDK** -- `aws_secretsmanager_secret` is one of the most commonly used Terraform resources
- **Spring Cloud AWS** -- `spring-cloud-aws-starter-secrets-manager` reads secrets at startup
- **CI/CD** -- GitHub Actions, CircleCI, and other CI systems retrieve secrets from Secrets Manager during builds

Without a local Secrets Manager, developers must either hard-code secrets, use .env files that diverge from production patterns, or make real AWS API calls during development and testing.

### 2.2 Complexity Assessment

| Dimension | Secrets Manager | SSM Parameter Store | DynamoDB |
|-----------|----------------|-------------------|----------|
| Total operations | 23 | 13 | 66 |
| Complex state machines | 1 (rotation lifecycle) | 0 | 1 (transactions) |
| Version model | UUID + staging labels | Auto-increment + labels | N/A |
| Storage complexity | HashMap + version map | HashMap + BTreeMap | B-Tree + GSI/LSI |
| Concurrency model | Request/response + deletion scheduler | Request/response | Transactions, batch |
| Protocol | awsJson1.1 (reuse SSM) | awsJson1.1 (exists) | awsJson1.0 |
| Estimated lines of code | ~4,500 | ~3,000 | ~15,000 |

Secrets Manager is moderately more complex than SSM Parameter Store due to:
1. Version staging labels (AWSCURRENT/AWSPREVIOUS/AWSPENDING) with label-move semantics
2. Scheduled deletion with recovery window (7-30 days)
3. Secret rotation lifecycle (create/set/test/finish steps)
4. Resource policies (accept and store, not enforce)
5. Secret name uniqueness enforcement even during deletion recovery window

### 2.3 Tool Coverage

With all 20 MVP operations implemented (excluding replication), the following tools work out of the box:

| Tool | Operations Used | Phase Available |
|------|----------------|-----------------|
| AWS CLI (`aws secretsmanager`) | All CRUD ops | Phase 0 |
| Terraform (`aws_secretsmanager_secret`) | Create/Get/Put/Delete/Describe + tags | Phase 0 + Phase 1 |
| AWS CDK | CreateSecret, GetSecretValue, PutSecretValue | Phase 0 |
| external-secrets-operator (K8s) | GetSecretValue, DescribeSecret, BatchGetSecretValue | Phase 0 + Phase 2 |
| Spring Cloud AWS | GetSecretValue, ListSecrets | Phase 0 |
| Chamber (secretsmanager backend) | CreateSecret, GetSecretValue, ListSecrets, PutSecretValue, DeleteSecret | Phase 0 |
| AWS Secrets Manager Agent | GetSecretValue | Phase 0 |
| Doppler | GetSecretValue, PutSecretValue | Phase 0 |
| HashiCorp Vault (AWS backend) | GetSecretValue, PutSecretValue | Phase 0 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Full core API** -- implement 20 of 23 Secrets Manager operations (excluding 3 replication operations)
2. **Correct version staging semantics** -- AWSCURRENT, AWSPREVIOUS, AWSPENDING labels with proper move/promotion behavior
3. **Scheduled deletion** -- `DeleteSecret` with configurable recovery window (7-30 days), `RestoreSecret` to cancel, `ForceDeleteWithoutRecovery` for immediate deletion
4. **Secret rotation lifecycle** -- support the create/set/test/finish rotation state machine without requiring a real Lambda invocation
5. **Resource policies** -- accept, store, and return resource policy JSON
6. **Tag support** -- TagResource, UntagResource with tags reflected in ListSecrets and DescribeSecret
7. **ListSecrets filtering** -- name, description, tag-key, tag-value, owning-service, primary-region, and all filter types
8. **ListSecretVersionIds** -- enumerate versions with staging labels, including deprecated versions
9. **GetRandomPassword** -- generate cryptographically random passwords with configurable constraints
10. **BatchGetSecretValue** -- bulk retrieval by secret ID list or filter
11. **Smithy-generated types** -- all types generated from official AWS Smithy model
12. **Shared infrastructure** -- reuse `rustack-core`, `rustack-auth`, and the awsJson1.1 protocol layer
13. **Same Docker image** -- single binary serves S3 + DynamoDB + SQS + SSM + Secrets Manager on port 4566
14. **Pass LocalStack and moto test suites** -- validate against the most comprehensive Secrets Manager mock test suites

### 3.2 Non-Goals

1. **Real KMS encryption** -- secrets stored as plaintext; KmsKeyId field accepted and stored but not used for encryption
2. **Real Lambda invocation for rotation** -- rotation lifecycle state machine tracked locally; RotationLambdaARN stored but Lambda not invoked
3. **Secret replication** -- `ReplicateSecretToRegions`, `RemoveRegionsFromReplication`, `StopReplicationToReplica` return success but do not actually replicate data across regions
4. **IAM policy enforcement** -- resource policies accepted and stored but not evaluated for authorization decisions
5. **Automatic rotation scheduling** -- `RotationRules` stored but no background scheduler; rotation triggered only by explicit `RotateSecret` calls
6. **CloudWatch metrics/events** -- no EventBridge integration for secret changes
7. **Cross-account access** -- no AWS RAM or cross-account policy evaluation
8. **Data persistence across restarts** -- in-memory only, matching all other Rustack services
9. **ValidateResourcePolicy enforcement** -- accept the call but always return valid (no actual policy validation engine)
10. **Real deletion scheduling** -- `ForceDeleteWithoutRecovery` always available; scheduled deletion window honored by marking the secret but deletion occurs immediately on the internal timer (simplified)

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                AWS SDK / CLI / Terraform / external-secrets
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
   +-------+ +-----+ +-----+ +-----+ +----------+
   | S3    | | DDB | | SQS | | SSM | | SecrMgr  |
   |(Xml)  | |(J10)| |(Qry)| |(J11)| | (Json11) |
   +---+---+ +--+--+ +--+--+ +--+--+ +----+-----+
       |        |        |       |         |
   +---+---+ +--+--+ +--+--+ +--+--+ +----+-----+
   |S3 Core| |DDB  | |SQS  | |SSM  | |SecretsMgr|
   |       | |Core | |Core | |Core | |Core      |
   +---+---+ +--+--+ +--+--+ +--+--+ +----+-----+
       |        |        |       |         |
       +--------+--------+-------+---------+
                         |
                  +------+------+
                  | rustack-  |
                  | core + auth |
                  +-------------+
```

### 4.2 Gateway Routing

Secrets Manager requests are distinguished by the `X-Amz-Target` header prefix:

| Service | X-Amz-Target Prefix | Content-Type |
|---------|---------------------|--------------|
| DynamoDB | `DynamoDB_20120810.` | `application/x-amz-json-1.0` |
| SSM | `AmazonSSM.` | `application/x-amz-json-1.1` |
| SQS | *(absent, uses query string `Action=`)* | `application/x-www-form-urlencoded` |
| Secrets Manager | `secretsmanager.` | `application/x-amz-json-1.1` |
| S3 | *(absent)* | varies |

Routing logic: check `X-Amz-Target` header. If prefix is `secretsmanager.`, route to Secrets Manager. If `AmazonSSM.`, route to SSM. If `DynamoDB_`, route to DynamoDB. Then check for SQS query-string pattern. Otherwise default to S3. This is unambiguous -- Secrets Manager's target prefix (`secretsmanager.`) is distinct from all other services.

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-secretsmanager-model        <-- NEW (auto-generated)
+-- rustack-secretsmanager-core         <-- NEW
+-- rustack-secretsmanager-http         <-- NEW

rustack-secretsmanager-http
+-- rustack-secretsmanager-model
+-- rustack-auth

rustack-secretsmanager-core
+-- rustack-core
+-- rustack-secretsmanager-model

rustack-secretsmanager-model (auto-generated, standalone)
```

---

## 5. Protocol Design: awsJson1.1

### 5.1 Protocol Comparison

Secrets Manager uses `awsJson1.1`, which is identical to SSM's protocol. Both share the same serialization, Content-Type version, and error formatting conventions.

| Aspect | SSM (awsJson1.1) | Secrets Manager (awsJson1.1) |
|--------|-------------------|------------------------------|
| HTTP Method | POST only | POST only |
| URL Path | `/` always | `/` always |
| Content-Type | `application/x-amz-json-1.1` | `application/x-amz-json-1.1` |
| X-Amz-Target | `AmazonSSM.<Op>` | `secretsmanager.<Op>` |
| Request body | JSON | JSON |
| Response body | JSON | JSON |
| Error `__type` | Short name (e.g., `ParameterNotFound`) | Short name (e.g., `ResourceNotFoundException`) |
| Timestamp format | Epoch seconds (double) | Epoch seconds (double) |
| Auth | SigV4, service=`ssm` | SigV4, service=`secretsmanager` |

The only differences are the target prefix (`secretsmanager.` vs `AmazonSSM.`) and the SigV4 service name. JSON serialization, request dispatch, and error formatting are identical.

### 5.2 What We Reuse from SSM

The SSM implementation provides all the infrastructure Secrets Manager needs:

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| JSON request deserialization | Yes | `serde_json::from_slice` with `Deserialize` derives |
| JSON response serialization | Yes | `serde_json::to_vec` with `Serialize` derives |
| `X-Amz-Target` header parsing | Yes | Same pattern, different prefix |
| JSON error formatting | Yes | Same `{"__type": "...", "message": "..."}` format |
| SigV4 auth | Yes | `rustack-auth` is service-agnostic |
| Multi-account/region state | Yes | `rustack-core` unchanged |

### 5.3 Wire Format Examples

**CreateSecret request:**

```
POST / HTTP/1.1
Content-Type: application/x-amz-json-1.1
X-Amz-Target: secretsmanager.CreateSecret

{
    "Name": "MyTestDatabaseSecret",
    "Description": "My test database secret",
    "SecretString": "{\"username\":\"david\",\"password\":\"EXAMPLE\"}",
    "ClientRequestToken": "EXAMPLE1-90ab-cdef-fedc-ba987SECRET1",
    "Tags": [
        {"Key": "Environment", "Value": "production"}
    ]
}
```

**CreateSecret response:**

```
HTTP/1.1 200 OK
Content-Type: application/x-amz-json-1.1

{
    "ARN": "arn:aws:secretsmanager:us-east-1:000000000000:secret:MyTestDatabaseSecret-a1b2c3",
    "Name": "MyTestDatabaseSecret",
    "VersionId": "EXAMPLE1-90ab-cdef-fedc-ba987SECRET1"
}
```

**Error response:**

```
HTTP/1.1 400 Bad Request
Content-Type: application/x-amz-json-1.1

{
    "__type": "ResourceNotFoundException",
    "Message": "Secrets Manager can't find the specified secret."
}
```

---

## 6. Smithy Code Generation Strategy

### 6.1 Universal Codegen

The `rustack-secretsmanager-model` crate is generated from the official AWS Smithy JSON AST using the universal codegen tool at `codegen/`. The codegen reads a TOML service configuration and the Smithy model to produce all model types with correct serde attributes.

**Smithy model:** `codegen/smithy-model/secretsmanager.json` (269KB, namespace `com.amazonaws.secretsmanager`, 23 operations)
**Service config:** `codegen/services/secretsmanager.toml`
**Generate:** `make codegen-secretsmanager`

### 6.2 Generated Output

The codegen produces 6 files in `crates/rustack-secretsmanager-model/src/`:

| File | Contents |
|------|----------|
| `lib.rs` | Module declarations and re-exports |
| `types.rs` | Shared types (enums and structs) with serde derives |
| `operations.rs` | `SecretsManagerOperation` enum with `as_str()`, `from_name()`, phase methods |
| `error.rs` | `SecretsManagerErrorCode` enum + `SecretsManagerError` struct + `secretsmanager_error!` macro |
| `input.rs` | All input structs with `#[serde(rename_all = "PascalCase")]` |
| `output.rs` | All output structs with serde derives |

### 6.3 Service-Specific Notes

No special considerations; Secrets Manager is a straightforward `awsJson1.1` service with `PascalCase` JSON field naming.

See [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md) for full codegen architecture details.

---

## 7. Crate Structure

### 7.1 `rustack-secretsmanager-model` (auto-generated)

```
crates/rustack-secretsmanager-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports
    +-- types.rs            # Auto-generated: enums + shared structs
    +-- operations.rs       # Auto-generated: SecretsManagerOperation enum
    +-- error.rs            # Auto-generated: error types + error codes
    +-- input.rs            # Auto-generated: all 23 input structs
    +-- output.rs           # Auto-generated: all 23 output structs
```

**Dependencies:** `serde`, `serde_json`

No hand-written types needed. Secrets Manager uses straightforward JSON types that serde handles natively.

### 7.2 `rustack-secretsmanager-core`

```
crates/rustack-secretsmanager-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # SecretsManagerConfig
    +-- handler.rs          # SecretsManagerHandler trait (all 23 operation dispatch)
    +-- provider.rs         # RustackSecretsManager (main provider, all operation handlers)
    +-- storage.rs          # SecretStore (DashMap<String, SecretRecord>), SecretRecord, SecretVersion
    +-- version.rs          # Version staging label management (AWSCURRENT/AWSPREVIOUS/AWSPENDING)
    +-- rotation.rs         # Rotation lifecycle state machine
    +-- deletion.rs         # Scheduled deletion and recovery logic
    +-- filter.rs           # ListSecrets filter evaluation
    +-- password.rs         # GetRandomPassword generator
    +-- validation.rs       # Secret name, ARN, version ID, tag validation
```

**Dependencies:** `rustack-core`, `rustack-secretsmanager-model`, `dashmap`, `serde_json`, `tracing`, `rand`, `uuid`, `base64`

### 7.3 `rustack-secretsmanager-http`

```
crates/rustack-secretsmanager-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # secretsmanager.* target dispatch
    +-- service.rs          # SecretsManagerHttpService (hyper Service impl)
    +-- dispatch.rs         # SecretsManagerHandler trait + operation dispatch
    +-- body.rs             # Response body type
    +-- response.rs         # HTTP response construction
```

**Dependencies:** `rustack-secretsmanager-model`, `rustack-auth`, `hyper`, `http`, `serde_json`, `bytes`

This crate is structurally identical to `rustack-ssm-http`. The router parses `secretsmanager.<Op>` instead of `AmazonSSM.<Op>`.

### 7.4 Workspace Changes

```toml
[workspace.dependencies]
rustack-secretsmanager-model = { path = "crates/rustack-secretsmanager-model" }
rustack-secretsmanager-http = { path = "crates/rustack-secretsmanager-http" }
rustack-secretsmanager-core = { path = "crates/rustack-secretsmanager-core" }
```

---

## 8. HTTP Layer Design

### 8.1 Router

```rust
/// Secrets Manager operation router.
///
/// Parses the `X-Amz-Target: secretsmanager.<Op>` header to determine the operation.
pub struct SecretsManagerRouter;

impl SecretsManagerRouter {
    pub fn resolve(target: &str) -> Result<SecretsManagerOperation, SecretsManagerError> {
        let op_name = target
            .strip_prefix("secretsmanager.")
            .ok_or_else(|| SecretsManagerError::unknown_operation(target))?;

        SecretsManagerOperation::from_name(op_name)
            .ok_or_else(|| SecretsManagerError::unknown_operation(op_name))
    }
}
```

### 8.2 ServiceRouter Trait Implementation

```rust
/// Secrets Manager service router for the gateway.
pub struct SecretsManagerServiceRouter {
    handler: Arc<RustackSecretsManager>,
    config: SecretsManagerHttpConfig,
}

impl ServiceRouter for SecretsManagerServiceRouter {
    fn matches(&self, req: &Request<()>) -> bool {
        req.headers()
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.starts_with("secretsmanager."))
    }

    async fn handle(&self, req: Request<Incoming>) -> Response<Body> {
        // 1. Extract X-Amz-Target, resolve to SecretsManagerOperation
        // 2. Read body bytes
        // 3. Deserialize JSON input
        // 4. Dispatch to handler
        // 5. Serialize JSON output or error
    }
}
```

### 8.3 Handler Trait

```rust
/// Trait that the Secrets Manager business logic provider must implement.
pub trait SecretsManagerHandler: Send + Sync + 'static {
    /// Handle a Secrets Manager operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: SecretsManagerOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<SecretsManagerResponseBody>, SecretsManagerError>> + Send>>;
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

The storage model is a hashmap of secrets, each containing a map of versions identified by UUID. Versions are linked to staging labels. The core innovation compared to SSM Parameter Store is the staging label system, where labels like AWSCURRENT and AWSPREVIOUS can be moved between versions.

### 9.2 Core Data Structures

```rust
/// Top-level secret store.
/// Keyed by (account_id, region) via rustack-core, then by secret name.
pub struct SecretStore {
    /// All secrets keyed by name.
    secrets: DashMap<String, SecretRecord>,
}

/// A single secret with its version history and metadata.
pub struct SecretRecord {
    /// Secret name (e.g., "MyDatabaseSecret" or "prod/myapp/db-password").
    pub name: String,
    /// Secret ARN (includes 6-character random suffix).
    pub arn: String,
    /// Description (optional, max 2048 chars).
    pub description: Option<String>,
    /// KMS key ID (stored but not used for encryption).
    pub kms_key_id: Option<String>,
    /// Tags on the secret resource.
    pub tags: Vec<Tag>,
    /// Resource policy JSON (stored but not enforced).
    pub resource_policy: Option<String>,

    // -- Versioning --
    /// All versions keyed by version ID (UUID string).
    pub versions: HashMap<String, SecretVersion>,
    /// Mapping from staging label to version ID.
    /// Each label points to exactly one version. A version can have multiple labels.
    pub staging_labels: HashMap<String, String>,

    // -- Rotation --
    /// Whether rotation is enabled.
    pub rotation_enabled: bool,
    /// Lambda ARN for rotation (stored, not invoked).
    pub rotation_lambda_arn: Option<String>,
    /// Rotation rules configuration.
    pub rotation_rules: Option<RotationRules>,
    /// Timestamp of last rotation.
    pub last_rotated_date: Option<f64>,

    // -- Deletion --
    /// If scheduled for deletion, the deletion date (epoch seconds).
    pub deleted_date: Option<f64>,
    /// Recovery window in days (7-30, default 30).
    pub recovery_window_in_days: Option<i64>,

    // -- Timestamps --
    /// When the secret was created (epoch seconds).
    pub created_date: f64,
    /// When the secret was last changed (epoch seconds).
    pub last_changed_date: f64,
    /// When the secret was last accessed (epoch seconds, date-only granularity).
    pub last_accessed_date: Option<f64>,

    // -- Type --
    /// Owning service (for managed secrets).
    pub owning_service: Option<String>,
    /// Primary region for replication.
    pub primary_region: Option<String>,
}

/// A single version of a secret.
pub struct SecretVersion {
    /// Version ID (UUID string).
    pub version_id: String,
    /// Secret string value (mutually exclusive with secret_binary).
    pub secret_string: Option<String>,
    /// Secret binary value as raw bytes (mutually exclusive with secret_string).
    pub secret_binary: Option<Vec<u8>>,
    /// When this version was created (epoch seconds).
    pub created_date: f64,
    /// Staging labels currently attached to this version.
    /// This is derived from SecretRecord.staging_labels but cached
    /// for efficient lookup.
    pub version_stages: Vec<String>,
}

/// Rotation rules configuration.
pub struct RotationRules {
    /// Rotate automatically after this many days.
    pub automatically_after_days: Option<i64>,
    /// Duration string (e.g., "2h").
    pub duration: Option<String>,
    /// Cron schedule expression.
    pub schedule_expression: Option<String>,
}
```

### 9.3 Version Staging Labels

The staging label system is the central complexity of Secrets Manager. Key invariants:

1. **Each staging label points to exactly one version.** Moving a label to a new version automatically removes it from the old version.
2. **AWSCURRENT is always present.** Every secret has exactly one version labeled AWSCURRENT (unless the secret has no value yet).
3. **AWSPREVIOUS is automatically managed.** When AWSCURRENT moves to a new version, the old AWSCURRENT version gets AWSPREVIOUS (and the old AWSPREVIOUS version loses it).
4. **AWSPENDING is used during rotation.** The rotation lifecycle creates a new version with AWSPENDING, then promotes it to AWSCURRENT.
5. **Custom labels are user-managed.** Users can create arbitrary staging labels with `PutSecretValue` or `UpdateSecretVersionStage`.
6. **Versions without labels are deprecated.** They are retained up to a limit (~100) and can be listed with `IncludeDeprecated`.

```rust
impl SecretRecord {
    /// Move a staging label from one version to another.
    /// If the label is AWSCURRENT, automatically move AWSPREVIOUS.
    pub fn move_staging_label(
        &mut self,
        label: &str,
        to_version_id: &str,
        from_version_id: Option<&str>,
    ) -> Result<(), SecretsManagerError> {
        // Validate that to_version exists
        if !self.versions.contains_key(to_version_id) {
            return Err(SecretsManagerError::resource_not_found(
                "Version not found",
            ));
        }

        // If from_version_id specified, validate it currently holds the label
        if let Some(from_id) = from_version_id {
            let current_holder = self.staging_labels.get(label);
            if current_holder.map(|s| s.as_str()) != Some(from_id) {
                return Err(SecretsManagerError::invalid_parameter(
                    "The staging label is not currently attached to the specified version.",
                ));
            }
        }

        // Special handling for AWSCURRENT: auto-move AWSPREVIOUS
        if label == "AWSCURRENT" {
            if let Some(old_current_id) = self.staging_labels.get("AWSCURRENT").cloned() {
                // Old AWSCURRENT becomes AWSPREVIOUS
                // Remove AWSPREVIOUS from whoever has it
                self.staging_labels.insert("AWSPREVIOUS".to_string(), old_current_id);
            }
        }

        // Move the label
        self.staging_labels.insert(label.to_string(), to_version_id.to_string());

        // Rebuild version_stages caches
        self.rebuild_version_stages();

        Ok(())
    }

    /// Rebuild the version_stages field on each SecretVersion from
    /// the authoritative staging_labels map.
    fn rebuild_version_stages(&mut self) {
        // Clear all version stages
        for version in self.versions.values_mut() {
            version.version_stages.clear();
        }
        // Rebuild from staging_labels
        for (label, version_id) in &self.staging_labels {
            if let Some(version) = self.versions.get_mut(version_id) {
                version.version_stages.push(label.clone());
            }
        }
        // Sort for deterministic output
        for version in self.versions.values_mut() {
            version.version_stages.sort();
        }
    }
}
```

### 9.4 Version Lifecycle

The version lifecycle during normal operations:

1. **CreateSecret with value** -- creates version V1, labels it AWSCURRENT
2. **PutSecretValue** (default) -- creates version V2, labels it AWSCURRENT, V1 gets AWSPREVIOUS
3. **PutSecretValue with AWSPENDING** -- creates version V3 with AWSPENDING only, AWSCURRENT stays on V2
4. **UpdateSecretVersionStage(AWSCURRENT, from=V2, to=V3)** -- V3 gets AWSCURRENT, V2 gets AWSPREVIOUS, V1 loses AWSPREVIOUS (becomes deprecated)
5. **UpdateSecret with new value** -- same as PutSecretValue: creates new version, promotes to AWSCURRENT

Version cleanup: deprecated versions (no staging labels) are retained up to 100 total. When the limit is exceeded, the oldest deprecated versions are removed.

```rust
impl SecretRecord {
    /// Add a new version and optionally assign staging labels.
    /// If version_stages includes AWSCURRENT, automatically handle AWSPREVIOUS.
    /// If version_stages is empty, default to ["AWSCURRENT"].
    pub fn add_version(
        &mut self,
        version_id: String,
        secret_string: Option<String>,
        secret_binary: Option<Vec<u8>>,
        version_stages: Vec<String>,
        created_date: f64,
    ) -> Result<(), SecretsManagerError> {
        let stages = if version_stages.is_empty() {
            vec!["AWSCURRENT".to_string()]
        } else {
            version_stages
        };

        // Check for idempotent request (same version_id with same content)
        if let Some(existing) = self.versions.get(&version_id) {
            if existing.secret_string == secret_string
                && existing.secret_binary == secret_binary
            {
                return Ok(()); // Idempotent: same content, no-op
            }
            return Err(SecretsManagerError::resource_exists(
                "A version with this ID already exists with different content.",
            ));
        }

        // Handle AWSCURRENT promotion
        if stages.contains(&"AWSCURRENT".to_string()) {
            if let Some(old_current_id) = self.staging_labels.get("AWSCURRENT").cloned() {
                // Move AWSPREVIOUS to old AWSCURRENT holder
                self.staging_labels.insert("AWSPREVIOUS".to_string(), old_current_id);
            }
        }

        // Move labels to new version
        for stage in &stages {
            self.staging_labels.insert(stage.clone(), version_id.clone());
        }

        // Insert version
        self.versions.insert(version_id.clone(), SecretVersion {
            version_id,
            secret_string,
            secret_binary,
            created_date,
            version_stages: Vec::new(), // rebuilt below
        });

        // Enforce version limit
        self.cleanup_deprecated_versions();

        // Rebuild cached stages
        self.rebuild_version_stages();

        // Update last_changed_date
        self.last_changed_date = created_date;

        Ok(())
    }

    /// Remove deprecated (label-less) versions exceeding the 100-version limit.
    fn cleanup_deprecated_versions(&mut self) {
        let labeled_version_ids: HashSet<&String> =
            self.staging_labels.values().collect();

        let mut deprecated: Vec<(String, f64)> = self.versions.iter()
            .filter(|(vid, _)| !labeled_version_ids.contains(vid))
            .map(|(vid, v)| (vid.clone(), v.created_date))
            .collect();

        // Sort by creation date ascending (oldest first)
        deprecated.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // Keep at most 100 total versions
        let total = self.versions.len();
        if total > 100 {
            let to_remove = total - 100;
            for (vid, _) in deprecated.iter().take(to_remove) {
                self.versions.remove(vid);
            }
        }
    }
}
```

### 9.5 Secret Resolution (SecretId Lookup)

Secrets Manager allows looking up secrets by:
1. **Secret name** (e.g., `"MyDatabaseSecret"`)
2. **Full ARN** (e.g., `"arn:aws:secretsmanager:us-east-1:000000000000:secret:MyDatabaseSecret-a1b2c3"`)
3. **Partial ARN** (ARN without the 6-character suffix)

```rust
impl SecretStore {
    /// Resolve a SecretId to a secret name.
    /// Handles name, full ARN, and partial ARN lookups.
    pub fn resolve_secret_id(&self, secret_id: &str) -> Result<String, SecretsManagerError> {
        // Direct name match
        if self.secrets.contains_key(secret_id) {
            return Ok(secret_id.to_string());
        }

        // ARN match (full or partial)
        if secret_id.starts_with("arn:") {
            for entry in self.secrets.iter() {
                let record = entry.value();
                if record.arn == secret_id || record.arn.starts_with(secret_id) {
                    return Ok(record.name.clone());
                }
            }
        }

        Err(SecretsManagerError::resource_not_found(
            "Secrets Manager can't find the specified secret.",
        ))
    }
}
```

### 9.6 Scheduled Deletion

When `DeleteSecret` is called without `ForceDeleteWithoutRecovery`:
1. The secret is marked with a `deleted_date` and `recovery_window_in_days`
2. The secret is hidden from `ListSecrets` (unless `IncludePlannedDeletion` is true)
3. `GetSecretValue` returns `InvalidRequestException` ("You can't perform this operation on secret that's marked for deletion")
4. `RestoreSecret` clears the deletion markers and makes the secret available again
5. After the recovery window expires, the secret is permanently deleted

For local development, we implement a simplified version:
- `ForceDeleteWithoutRecovery=true` immediately removes the secret
- Without force delete, the secret is marked as deleted but retained in memory
- A background task (or lazy cleanup on access) can purge expired secrets

```rust
impl SecretRecord {
    /// Returns true if this secret is scheduled for deletion.
    pub fn is_pending_deletion(&self) -> bool {
        self.deleted_date.is_some()
    }

    /// Returns true if the recovery window has expired.
    pub fn is_deletion_expired(&self, now: f64) -> bool {
        if let (Some(deleted), Some(window)) = (self.deleted_date, self.recovery_window_in_days) {
            let expiry = deleted + (window as f64 * 86400.0);
            now >= expiry
        } else {
            false
        }
    }

    /// Schedule this secret for deletion.
    pub fn schedule_deletion(&mut self, recovery_window_days: i64, now: f64) {
        self.deleted_date = Some(now);
        self.recovery_window_in_days = Some(recovery_window_days);
    }

    /// Restore a secret scheduled for deletion.
    pub fn restore(&mut self) -> Result<(), SecretsManagerError> {
        if self.deleted_date.is_none() {
            return Err(SecretsManagerError::invalid_request(
                "You can't perform this operation on a secret that's not scheduled for deletion.",
            ));
        }
        self.deleted_date = None;
        self.recovery_window_in_days = None;
        Ok(())
    }
}
```

### 9.7 Rotation Lifecycle

Real AWS Secrets Manager invokes a Lambda function with four steps: createSecret, setSecret, testSecret, finishSecret. For local development, we provide two approaches:

**Approach A (MVP): Store rotation metadata only.** `RotateSecret` stores the rotation configuration and creates a new version with AWSPENDING, but does not invoke any Lambda. The user must manually complete rotation by calling `UpdateSecretVersionStage` to promote AWSPENDING to AWSCURRENT.

**Approach B (Future): Simple rotation callback.** Support a configurable rotation hook (HTTP endpoint or in-process callback) that mimics the Lambda invocation steps. This is a non-goal for the initial implementation.

```rust
impl SecretRecord {
    /// Configure rotation for this secret.
    pub fn configure_rotation(
        &mut self,
        lambda_arn: Option<String>,
        rules: Option<RotationRules>,
    ) {
        if let Some(arn) = lambda_arn {
            self.rotation_lambda_arn = Some(arn);
        }
        if let Some(r) = rules {
            self.rotation_rules = Some(r);
        }
        self.rotation_enabled = true;
    }

    /// Start a rotation by creating a pending version.
    pub fn start_rotation(
        &mut self,
        version_id: String,
        now: f64,
    ) -> Result<(), SecretsManagerError> {
        if !self.rotation_enabled {
            return Err(SecretsManagerError::invalid_request(
                "Rotation is not configured for this secret.",
            ));
        }

        // Create a new version with AWSPENDING
        // The version has no value yet -- it will be populated by the rotation function
        // For local dev, we clone the current value into the pending version
        let current_value = self.get_current_version()
            .map(|v| (v.secret_string.clone(), v.secret_binary.clone()));

        if let Some((ss, sb)) = current_value {
            self.add_version(
                version_id,
                ss,
                sb,
                vec!["AWSPENDING".to_string()],
                now,
            )?;
        }

        self.last_rotated_date = Some(now);
        Ok(())
    }

    /// Get the version labeled AWSCURRENT.
    fn get_current_version(&self) -> Option<&SecretVersion> {
        self.staging_labels
            .get("AWSCURRENT")
            .and_then(|vid| self.versions.get(vid))
    }
}
```

### 9.8 ARN Construction

Secrets Manager ARNs include a 6-character random suffix that makes them unique:

```rust
fn secret_arn(region: &str, account_id: &str, name: &str) -> String {
    let suffix: String = (0..6)
        .map(|_| {
            let idx = rand::random::<usize>() % 36;
            if idx < 10 {
                (b'0' + idx as u8) as char
            } else {
                (b'a' + (idx - 10) as u8) as char
            }
        })
        .collect();
    format!("arn:aws:secretsmanager:{region}:{account_id}:secret:{name}-{suffix}")
}
```

### 9.9 VersionIdsToStages Map

The `DescribeSecret` and `ListSecrets` responses include a `VersionIdsToStages` map (called `SecretVersionsToStages` in ListSecrets). This maps version IDs to their staging labels:

```json
{
    "VersionIdsToStages": {
        "EXAMPLE1-90ab-cdef-fedc-ba987SECRET1": ["AWSCURRENT"],
        "EXAMPLE2-90ab-cdef-fedc-ba987SECRET2": ["AWSPREVIOUS"],
        "EXAMPLE3-90ab-cdef-fedc-ba987SECRET3": ["AWSPENDING"]
    }
}
```

```rust
impl SecretRecord {
    /// Build the VersionIdsToStages map for API responses.
    pub fn version_ids_to_stages(&self) -> HashMap<String, Vec<String>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (label, version_id) in &self.staging_labels {
            map.entry(version_id.clone())
                .or_default()
                .push(label.clone());
        }
        // Sort labels within each version for deterministic output
        for labels in map.values_mut() {
            labels.sort();
        }
        map
    }
}
```

### 9.10 Concurrency Model

Like SSM Parameter Store, Secrets Manager has no real-time constraints, no streaming, and minimal background processing. A `DashMap` provides sufficient concurrent access:

- **Reads** (GetSecretValue, DescribeSecret, ListSecrets, etc.): lock-free concurrent reads
- **Writes** (CreateSecret, PutSecretValue, DeleteSecret, etc.): per-entry write locks via DashMap

The only background processing needed is expired deletion cleanup, which can be handled lazily on access or via a periodic sweep.

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main Secrets Manager provider implementing all operations.
pub struct RustackSecretsManager {
    pub(crate) state: Arc<SecretStore>,
    pub(crate) config: Arc<SecretsManagerConfig>,
}

impl RustackSecretsManager {
    pub fn new(config: SecretsManagerConfig) -> Self;
}
```

### 10.2 Operations

#### Phase 0: Core CRUD (10 operations)

**CreateSecret** -- Create a new secret.

1. Validate name format (ASCII letters, numbers, `/_+=.@-`, 1-512 chars)
2. Check if secret with same name exists (including pending deletion -- name is reserved during recovery window)
3. Generate ARN with 6-character random suffix
4. If `SecretString` or `SecretBinary` provided, create version V1 with staging label AWSCURRENT
5. If `ClientRequestToken` provided, use as version ID; otherwise generate UUID
6. If `Tags` provided, store tags on secret
7. Store `Description`, `KmsKeyId` if provided
8. Return `{ ARN, Name, VersionId, ReplicationStatus }`

**GetSecretValue** -- Retrieve the decrypted value of a secret.

1. Resolve `SecretId` (name, full ARN, or partial ARN)
2. If secret is pending deletion, return `InvalidRequestException`
3. Determine version to return:
   - If `VersionId` specified, return that exact version
   - If `VersionStage` specified, return the version with that staging label
   - Otherwise, return the version with AWSCURRENT label
4. If no matching version found, return `ResourceNotFoundException`
5. Update `LastAccessedDate` (date-only granularity, truncated to midnight)
6. Return `{ ARN, Name, VersionId, SecretString|SecretBinary, VersionStages, CreatedDate }`

**PutSecretValue** -- Store a new version of a secret's value.

1. Resolve `SecretId`; secret must exist (unlike CreateSecret)
2. If secret is pending deletion, return `InvalidRequestException`
3. Validate `SecretString` or `SecretBinary` provided (exactly one)
4. Generate version ID from `ClientRequestToken` or new UUID
5. Determine staging labels: use `VersionStages` if provided, otherwise default to `["AWSCURRENT"]`
6. If labels include AWSCURRENT, auto-manage AWSPREVIOUS promotion
7. Create new `SecretVersion` and update staging labels
8. Return `{ ARN, Name, VersionId, VersionStages }`

**DescribeSecret** -- Get metadata about a secret (no value).

1. Resolve `SecretId`
2. Build response with all metadata fields: ARN, Name, Description, KmsKeyId, RotationEnabled, RotationLambdaARN, RotationRules, LastRotatedDate, LastChangedDate, LastAccessedDate, DeletedDate, Tags, VersionIdsToStages, CreatedDate, PrimaryRegion, ReplicationStatus, OwningService
3. Omit fields that are not set (e.g., no `DeletedDate` if not pending deletion, no `LastAccessedDate` if never accessed)
4. Return complete metadata

**DeleteSecret** -- Delete a secret.

1. Resolve `SecretId`
2. If `ForceDeleteWithoutRecovery` is true:
   - Immediately remove secret from store
3. If not:
   - If `RecoveryWindowInDays` provided, validate 7-30 range; default to 30
   - Cannot specify both `ForceDeleteWithoutRecovery` and `RecoveryWindowInDays`
   - Mark secret as pending deletion with deletion date
4. Return `{ ARN, Name, DeletionDate }`

**RestoreSecret** -- Cancel scheduled deletion.

1. Resolve `SecretId`
2. If secret is not pending deletion, return `InvalidRequestException`
3. Clear deletion markers
4. Return `{ ARN, Name }`

**UpdateSecret** -- Update secret metadata and/or value.

1. Resolve `SecretId`
2. If secret is pending deletion, return `InvalidRequestException`
3. If `Description` provided, update description
4. If `KmsKeyId` provided, update KMS key ID
5. If `SecretString` or `SecretBinary` provided, create new version (same as PutSecretValue with AWSCURRENT)
6. If only metadata changed (no new value), return without new VersionId
7. Return `{ ARN, Name, VersionId }`

**ListSecrets** -- List secrets with optional filtering and pagination.

1. Iterate all secrets in store
2. Apply filters if provided:
   - `name` -- prefix match on secret name (case-sensitive). `!` prefix for negation. Multiple space-separated values are all-must-match prefix filters on the name.
   - `description` -- prefix match on description. `!` prefix for negation. Multiple space-separated values are all-must-match prefix filters.
   - `tag-key` -- match by tag key
   - `tag-value` -- match by tag value
   - `owning-service` -- match by owning service
   - `primary-region` -- match by primary region
   - `all` -- match across name, description, and tag values
3. Exclude secrets pending deletion unless `IncludePlannedDeletion` is true
4. Sort by `SortBy` field (`name`, `created-date`, `last-accessed-date`, `last-changed-date`) and `SortOrder` (`asc`, `desc`)
5. Paginate with `MaxResults` (1-100) and `NextToken`
6. Return `{ SecretList, NextToken }`

**ListSecretVersionIds** -- List all version IDs for a secret.

1. Resolve `SecretId`
2. Iterate all versions
3. If `IncludeDeprecated` is false (default), only include versions with at least one staging label
4. Paginate with `MaxResults` and `NextToken`
5. Return `{ Versions: [{ VersionId, VersionStages, LastAccessedDate, CreatedDate, KmsKeyIds }], NextToken, Name, ARN }`

**GetRandomPassword** -- Generate a random password.

1. Apply constraints: `PasswordLength` (1-4096, default 32), `ExcludeCharacters`, `ExcludeLowercase`, `ExcludeNumbers`, `ExcludePunctuation`, `ExcludeUppercase`, `IncludeSpace`, `RequireEachIncludedType`
2. Build character pool based on constraints
3. Generate cryptographically random password
4. If `RequireEachIncludedType` is true, ensure at least one character from each included type
5. Return `{ RandomPassword }`

#### Phase 1: Tags and Resource Policies (6 operations)

**TagResource** -- Add tags to a secret.

1. Resolve `SecretId`
2. Merge new tags (overwrite existing keys, add new)
3. Enforce 50-tag limit
4. Return `{}`

**UntagResource** -- Remove tags from a secret.

1. Resolve `SecretId`
2. Remove tags by key
3. Return `{}`

**PutResourcePolicy** -- Attach a resource policy to a secret.

1. Resolve `SecretId`
2. Store the policy JSON string
3. Return `{ ARN, Name }`

**GetResourcePolicy** -- Get the resource policy for a secret.

1. Resolve `SecretId`
2. Return `{ ARN, Name, ResourcePolicy }`

**DeleteResourcePolicy** -- Remove the resource policy from a secret.

1. Resolve `SecretId`
2. Clear the stored policy
3. Return `{ ARN, Name }`

**ValidateResourcePolicy** -- Validate a resource policy (stub).

1. Accept the request
2. Always return `{ PolicyValidationPassed: true, ValidationErrors: [] }`

#### Phase 2: Rotation and Advanced Operations (4 operations)

**RotateSecret** -- Start secret rotation.

1. Resolve `SecretId`
2. If `RotationLambdaARN` provided, store it and enable rotation
3. If this is the first rotation and no Lambda ARN is configured, return error
4. Generate new version ID from `ClientRequestToken` or UUID
5. Create new version with AWSPENDING label
6. For local dev, clone current value to pending version (since we cannot invoke Lambda)
7. If `RotateImmediately` is not false, immediately promote AWSPENDING to AWSCURRENT
8. Return `{ ARN, Name, VersionId }`

**CancelRotateSecret** -- Cancel an in-progress rotation.

1. Resolve `SecretId`
2. If a version with AWSPENDING exists, remove the AWSPENDING label
3. Disable rotation flag
4. Return `{ ARN, Name, VersionId }`

**UpdateSecretVersionStage** -- Move a staging label between versions.

1. Resolve `SecretId`
2. Validate `VersionStage` label
3. If `RemoveFromVersionId` specified, validate the label is on that version
4. If `MoveToVersionId` specified, validate that version exists and move label there
5. If only `RemoveFromVersionId` specified, just remove the label
6. Handle AWSCURRENT/AWSPREVIOUS auto-promotion
7. Return `{ ARN, Name }`

**BatchGetSecretValue** -- Batch retrieve secret values.

1. If `SecretIdList` provided, resolve each and get current value
2. If `Filters` provided, list matching secrets and get values
3. Collect results and errors
4. Return `{ SecretValues: [...], Errors: [...] }`

#### Phase 3: Replication Stubs (3 operations)

**ReplicateSecretToRegions** -- Accept and return success (stub).

**RemoveRegionsFromReplication** -- Accept and return success (stub).

**StopReplicationToReplica** -- Accept and return success (stub).

### 10.3 Validation Rules

| Field | Rule |
|-------|------|
| Secret name | 1-512 chars, ASCII letters, numbers, `/_+=.@-` |
| Secret value (string) | Max 65,536 chars |
| Secret value (binary) | Max 65,536 bytes |
| Description | Max 2,048 chars |
| ClientRequestToken | 32-64 chars, pattern `[a-zA-Z0-9-]+` (UUID format) |
| Version stage label | 1-256 chars, alphanumeric plus `/_+=.@-` |
| Tag key | 1-128 chars |
| Tag value | 0-256 chars |
| Tags per secret | Max 50 |
| Versions per secret | Max ~100 (deprecated versions auto-cleaned) |
| Recovery window | 7-30 days |
| Password length | 1-4096 (default 32) |
| Filter values | 1-10 filters, each with 1-10 values |
| MaxResults (ListSecrets) | 1-100 |

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// Secrets Manager error codes matching the AWS API.
#[derive(Debug, Clone)]
pub enum SecretsManagerErrorCode {
    /// Secret does not exist.
    ResourceNotFoundException,
    /// Secret already exists with this name.
    ResourceExistsException,
    /// Invalid request (e.g., accessing deleted secret).
    InvalidRequestException,
    /// Invalid parameter value.
    InvalidParameterException,
    /// Client request token does not match.
    InvalidRequestTokenException,
    /// Operation not supported.
    InvalidAction,
    /// Limit exceeded (e.g., too many secrets).
    LimitExceededException,
    /// Encryption failure (stubbed).
    EncryptionFailure,
    /// Decryption failure (stubbed).
    DecryptionFailure,
    /// Internal server error.
    InternalServiceError,
    /// Cannot modify a deleted secret.
    InvalidNextTokenException,
    /// Missing required parameters.
    MissingRequiredParameterException,
    /// Malformed policy document.
    MalformedPolicyDocumentException,
    /// Precondition not met.
    PreconditionNotMetException,
    /// Public policy exception.
    PublicPolicyException,
}
```

### 11.2 Error Mapping

```rust
impl SecretsManagerError {
    /// Map to HTTP status code, __type string, and message.
    pub fn to_error_response(&self) -> (u16, &'static str, String) {
        match &self.code {
            SecretsManagerErrorCode::ResourceNotFoundException =>
                (400, "ResourceNotFoundException", self.message.clone()),
            SecretsManagerErrorCode::ResourceExistsException =>
                (400, "ResourceExistsException", self.message.clone()),
            SecretsManagerErrorCode::InvalidRequestException =>
                (400, "InvalidRequestException", self.message.clone()),
            SecretsManagerErrorCode::InvalidParameterException =>
                (400, "InvalidParameterException", self.message.clone()),
            SecretsManagerErrorCode::LimitExceededException =>
                (400, "LimitExceededException", self.message.clone()),
            SecretsManagerErrorCode::InternalServiceError =>
                (500, "InternalServiceError", self.message.clone()),
            // ... etc
        }
    }
}
```

### 11.3 Error Response Format

```json
{
    "__type": "ResourceNotFoundException",
    "Message": "Secrets Manager can't find the specified secret."
}
```

Note: Secrets Manager uses `"Message"` (capital M) in error responses, unlike SSM which uses `"message"` (lowercase m). The AWS SDKs check for both variants, but we should match the real service behavior.

---

## 12. Server Integration

### 12.1 Feature Gate

Secrets Manager support is gated behind a cargo feature:

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "secretsmanager"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
sqs = ["dep:rustack-sqs-core", "dep:rustack-sqs-http"]
ssm = ["dep:rustack-ssm-core", "dep:rustack-ssm-http"]
secretsmanager = ["dep:rustack-secretsmanager-core", "dep:rustack-secretsmanager-http"]
```

### 12.2 Gateway Registration

Secrets Manager is registered in the gateway alongside other services:

```rust
// In gateway setup
let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

#[cfg(feature = "dynamodb")]
services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_handler)));

#[cfg(feature = "ssm")]
services.push(Box::new(SSMServiceRouter::new(ssm_handler)));

#[cfg(feature = "secretsmanager")]
services.push(Box::new(SecretsManagerServiceRouter::new(secretsmanager_handler)));

#[cfg(feature = "sqs")]
services.push(Box::new(SQSServiceRouter::new(sqs_handler)));

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
        "sqs": "available",
        "ssm": "available",
        "secretsmanager": "available"
    },
    "version": "0.3.0"
}
```

### 12.4 Configuration

```rust
pub struct SecretsManagerConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub default_account_id: String,
}

impl SecretsManagerConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("SECRETSMANAGER_SKIP_SIGNATURE_VALIDATION", true),
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
| `SECRETSMANAGER_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 for Secrets Manager |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default account for ARNs |

### 12.6 Docker Image / GitHub Action

The existing Docker image and GitHub Action gain Secrets Manager support automatically when the feature is enabled. The same `rustack` binary serves all services. The GitHub Action `action.yml` should be updated to list `secretsmanager` as a supported service.

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **Version staging labels**: AWSCURRENT/AWSPREVIOUS auto-promotion, custom label management, label move between versions, rebuild_version_stages correctness
- **Secret resolution**: name lookup, full ARN lookup, partial ARN lookup, name collision handling
- **Scheduled deletion**: mark for deletion, recovery window validation, restore secret, force delete
- **Rotation lifecycle**: configure rotation, start rotation with AWSPENDING, cancel rotation
- **Password generation**: character set constraints, ExcludeCharacters, RequireEachIncludedType, length bounds
- **Filter evaluation**: all ListSecrets filter keys (name, description, tag-key, tag-value), negation with `!` prefix, multi-value AND semantics
- **Validation**: secret name format, version ID format, tag limits, value size limits
- **Version cleanup**: deprecated version eviction at 100-version limit

### 13.2 Integration Tests with aws-sdk-rust

```rust
// tests/integration/secretsmanager_tests.rs
#[tokio::test]
#[ignore]
async fn test_should_create_and_get_secret() {
    let client = aws_sdk_secretsmanager::Client::new(&config);
    // CreateSecret, GetSecretValue round-trip
}

#[tokio::test]
#[ignore]
async fn test_should_manage_version_staging_labels() {
    // CreateSecret, PutSecretValue with AWSPENDING,
    // UpdateSecretVersionStage to promote, verify AWSPREVIOUS
}

#[tokio::test]
#[ignore]
async fn test_should_delete_and_restore_secret() {
    // CreateSecret, DeleteSecret (with recovery window),
    // verify GetSecretValue fails, RestoreSecret, verify GetSecretValue works
}

#[tokio::test]
#[ignore]
async fn test_should_list_secrets_with_filters() {
    // Create multiple secrets with different names/tags
    // ListSecrets with name/tag filters
}

#[tokio::test]
#[ignore]
async fn test_should_generate_random_password() {
    // GetRandomPassword with constraints
    // Verify length, character set compliance
}

#[tokio::test]
#[ignore]
async fn test_should_manage_resource_policies() {
    // PutResourcePolicy, GetResourcePolicy, DeleteResourcePolicy
}

#[tokio::test]
#[ignore]
async fn test_should_manage_tags() {
    // TagResource, UntagResource, verify in DescribeSecret
}
```

### 13.3 AWS CLI Smoke Tests

```bash
# Create secret
aws secretsmanager create-secret --name test/db-password --secret-string "p@ssw0rd" --endpoint-url http://localhost:4566

# Get secret value
aws secretsmanager get-secret-value --secret-id test/db-password --endpoint-url http://localhost:4566

# Put new version
aws secretsmanager put-secret-value --secret-id test/db-password --secret-string "newp@ss" --endpoint-url http://localhost:4566

# List secrets
aws secretsmanager list-secrets --endpoint-url http://localhost:4566

# Describe secret (shows version info)
aws secretsmanager describe-secret --secret-id test/db-password --endpoint-url http://localhost:4566

# Delete secret
aws secretsmanager delete-secret --secret-id test/db-password --force-delete-without-recovery --endpoint-url http://localhost:4566

# Generate random password
aws secretsmanager get-random-password --password-length 32 --endpoint-url http://localhost:4566
```

### 13.4 Third-Party Test Suites

#### 13.4.1 LocalStack Secrets Manager Tests

**Location:** `vendors/localstack/tests/aws/services/secretsmanager/test_secretsmanager.py`
**Coverage:** Comprehensive -- 40+ test cases covering:
- CreateSecret, GetSecretValue, PutSecretValue, UpdateSecret, DeleteSecret
- Version staging labels (AWSCURRENT, AWSPREVIOUS, AWSPENDING, custom labels)
- Version stage cycling (create-pending-promote pattern)
- Deprecated versions and IncludeDeprecated
- Secret restoration after scheduled deletion
- Resource policies (put/get/delete)
- Rotation with Lambda (will not fully work without Lambda, but rotation metadata tests apply)
- ListSecrets filtering (name, description, negation)
- Invalid secret names (validation)
- LastAccessedDate and LastChangedDate timestamps
- Tag operations
- UpdateSecretVersionStage (multiple complex scenarios)
- GetRandomPassword
- BatchGetSecretValue
- HTTP-level JSON protocol tests (X-Amz-Target headers)

**How to run:** The LocalStack test suite is Python-based using pytest. We can adapt the non-Lambda-dependent tests to run against Rustack.

```makefile
test-secretsmanager-localstack:
	cd vendors/localstack && python -m pytest tests/aws/services/secretsmanager/ \
		-k "not rotate_secret_with_lambda" \
		--endpoint-url=http://localhost:4566
```

#### 13.4.2 Moto Secrets Manager Tests

**Source:** https://github.com/getmoto/moto/blob/master/tests/test_secretsmanager/test_secretsmanager.py
**Coverage:** 21 of 23 operations implemented. Moto's test suite includes:
- All CRUD operations
- Version management and staging labels
- Rotation lifecycle (mocked Lambda)
- Resource policies
- Secret replication
- Tag management
- GetRandomPassword
- Pagination
- Error handling

**How to run:** Moto tests are designed for the moto mock library but the `test_server.py` file tests against a standalone moto server. These tests can be adapted to run against Rustack by pointing the endpoint URL.

```makefile
test-secretsmanager-moto:
	cd vendors/moto && python -m pytest tests/test_secretsmanager/test_server.py \
		--endpoint-url=http://localhost:4566
```

#### 13.4.3 Terraform AWS Provider Acceptance Tests

**Source:** https://github.com/hashicorp/terraform-provider-aws (tests/resource_aws_secretsmanager_*.go)
**Coverage:** Tests `aws_secretsmanager_secret`, `aws_secretsmanager_secret_version`, `aws_secretsmanager_secret_policy`, `aws_secretsmanager_secret_rotation` resources.
**How to run:** Terraform acceptance tests require a running AWS-compatible endpoint. Use `tflocal` to point Terraform at Rustack:
```bash
pip install terraform-local
tflocal init
tflocal apply
```
**What this validates:** End-to-end Terraform workflow -- create secrets, update values, manage versions, delete and recreate. This is the most important external validation since Terraform is the primary IaC consumer of Secrets Manager.

#### 13.4.4 External-Secrets Operator (Kubernetes)

**Source:** https://github.com/external-secrets/external-secrets
**Coverage:** Tests `GetSecretValue`, `DescribeSecret`, `BatchGetSecretValue` via the AWS Secrets Manager provider.
**How to run:** Requires a Kubernetes cluster. Can test with kind + external-secrets Helm chart pointed at Rustack.
**What this validates:** Kubernetes-native secrets sync workflow, which is one of the primary use cases for Secrets Manager.

#### 13.4.5 Chamber (Segment)

**Source:** https://github.com/segmentio/chamber
**Coverage:** Chamber supports Secrets Manager as a backend (`chamber -b secretsmanager`). Tests cover write, read, list, history, delete operations.
**How to run:**
```bash
AWS_ENDPOINT_URL=http://localhost:4566 chamber -b secretsmanager write myservice db-password "s3cret"
AWS_ENDPOINT_URL=http://localhost:4566 chamber -b secretsmanager read myservice db-password
AWS_ENDPOINT_URL=http://localhost:4566 chamber -b secretsmanager list myservice
```
**What this validates:** The most common developer workflow -- storing and retrieving secrets by service/key hierarchy.

#### 13.4.6 Spring Cloud AWS

**Source:** https://github.com/awspring/spring-cloud-aws
**Coverage:** `spring-cloud-aws-starter-secrets-manager` reads secrets at application startup via `GetSecretValue` and `ListSecrets`.
**How to run:** Configure Spring Boot application with `spring.cloud.aws.secretsmanager.endpoint=http://localhost:4566`.
**What this validates:** Java/Spring Boot application integration, which represents a large segment of enterprise AWS users.

#### 13.4.7 AWS Secrets Manager Agent

**Source:** https://github.com/aws/aws-secretsmanager-agent
**Coverage:** The agent is a Rust-based sidecar that caches secrets locally. Tests `GetSecretValue` with caching, token-based auth, and version stage selection.
**How to run:** Build the agent and configure `SECRETS_MANAGER_ENDPOINT=http://localhost:4566`.
**What this validates:** The official AWS sidecar agent works correctly against Rustack.

### 13.5 CI Integration

```yaml
# .github/workflows/secretsmanager-ci.yml
name: Secrets Manager CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p rustack-secretsmanager-model
      - run: cargo test -p rustack-secretsmanager-core
      - run: cargo test -p rustack-secretsmanager-http

  integration:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
      - run: ./target/release/rustack &
      - run: sleep 2
      - run: |
          # AWS CLI smoke tests
          aws secretsmanager create-secret --name test-secret --secret-string "value" \
            --endpoint-url http://localhost:4566
          aws secretsmanager get-secret-value --secret-id test-secret \
            --endpoint-url http://localhost:4566
      - run: |
          # Python integration tests (LocalStack subset)
          pip install boto3 pytest
          pytest tests/integration/secretsmanager/ -v
```

---

## 14. Phased Implementation Plan

### Phase 0: Core CRUD (3-4 days)

**Goal:** Basic secret creation, retrieval, and deletion. Enough for `aws secretsmanager create-secret` and `get-secret-value` to work.

1. **Day 1: Model + Scaffolding**
   - Add Secrets Manager Smithy model to codegen
   - Generate `rustack-secretsmanager-model` crate
   - Create `rustack-secretsmanager-core` and `rustack-secretsmanager-http` crate scaffolding
   - Implement `SecretsManagerOperation` enum and router

2. **Day 2: Storage Engine + Core Operations**
   - Implement `SecretStore`, `SecretRecord`, `SecretVersion`
   - Implement version staging label management
   - Implement `CreateSecret`, `GetSecretValue`, `PutSecretValue`
   - Implement `DeleteSecret` (with `ForceDeleteWithoutRecovery`)
   - Implement `DescribeSecret`

3. **Day 3: Remaining Core + Gateway**
   - Implement `UpdateSecret`, `RestoreSecret`
   - Implement `ListSecrets` (basic, without filtering)
   - Implement `ListSecretVersionIds`
   - Implement `GetRandomPassword`
   - Integrate into gateway and health endpoint

4. **Day 4: Tests + Polish**
   - Unit tests for all storage operations
   - Integration tests with aws-sdk-rust
   - AWS CLI smoke tests
   - Fix edge cases from LocalStack test suite

**Deliverable:** AWS CLI, Terraform basic, CDK basic, Spring Cloud AWS all work.

### Phase 1: Tags and Resource Policies (1-2 days)

**Goal:** Full tag and policy management. Terraform `aws_secretsmanager_secret_policy` works.

5. **Day 5: Tags + Policies**
   - Implement `TagResource`, `UntagResource`
   - Implement `PutResourcePolicy`, `GetResourcePolicy`, `DeleteResourcePolicy`
   - Implement `ValidateResourcePolicy` (stub)
   - Add ListSecrets filter support (name, description, tag-key, tag-value)

**Deliverable:** Terraform full, external-secrets-operator basic, Chamber all work.

### Phase 2: Rotation and Batch (1-2 days)

**Goal:** Rotation lifecycle and batch operations.

6. **Day 6: Rotation + Batch**
   - Implement `RotateSecret` (metadata + AWSPENDING version creation)
   - Implement `CancelRotateSecret`
   - Implement `UpdateSecretVersionStage`
   - Implement `BatchGetSecretValue`

**Deliverable:** Full rotation lifecycle (minus Lambda invocation), batch retrieval works.

### Phase 3: Replication Stubs + CI (1 day)

**Goal:** Stub replication operations, CI pipeline, Docker image update.

7. **Day 7: Stubs + CI**
   - Implement `ReplicateSecretToRegions`, `RemoveRegionsFromReplication`, `StopReplicationToReplica` (accept-and-succeed stubs)
   - CI workflow for secretsmanager
   - Update Docker image, GitHub Action, README
   - Run LocalStack test suite subset, document pass/fail

**Deliverable:** All 23 operations implemented, CI green, Docker image updated.

---

## 15. Risk Analysis

### 15.1 Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Version staging label semantics are subtly wrong | Medium | High | Study LocalStack test suite thoroughly; the AWSCURRENT/AWSPREVIOUS auto-promotion logic has many edge cases |
| ListSecrets filter semantics differ from AWS | Medium | Medium | LocalStack tests cover filtering extensively (name prefix, description, negation); cross-reference with moto implementation |
| CreateSecret name collision during recovery window | Low | Medium | AWS reserves the name during the recovery window; our implementation must check pending-deletion secrets |
| ClientRequestToken idempotency handling wrong | Medium | Medium | If same token with same content: return success. Same token with different content: return error. Different token: new version. Test all three cases. |
| Partial ARN matching breaks with similar names | Low | Medium | ARN matching must handle the 6-char random suffix correctly; "MySecret" partial ARN should not match "MySecret2" |
| Error response format differs (capital M in "Message") | Low | Low | Secrets Manager uses `"Message"` (capital M); SSM uses `"message"`. AWS SDKs handle both, but exact format matters for snapshot tests |
| GetRandomPassword entropy insufficient | Low | Low | Use `rand::rngs::OsRng` for cryptographic randomness. Not security-critical for local dev, but should not be embarrassingly predictable |
| Rotation lifecycle incomplete without Lambda | Medium | Medium | Document clearly that `RotateSecret` stores metadata and creates AWSPENDING but does not invoke Lambda. Users must manually complete rotation via `UpdateSecretVersionStage`. This matches most testing scenarios. |

### 15.2 Dependencies

- `rustack-core` -- no changes needed
- `rustack-auth` -- no changes needed (SigV4 with service=`secretsmanager`)
- `dashmap` -- already in workspace
- `uuid` -- for version ID generation (already in workspace for SQS)
- `rand` -- for random password generation and ARN suffixes (already in workspace)
- `base64` -- for SecretBinary encoding/decoding (already in workspace)

### 15.3 Decision Log

| Decision | Rationale |
|----------|-----------|
| Use `HashMap<String, SecretVersion>` keyed by version UUID (not `BTreeMap<u64, _>` like SSM) | Secrets Manager versions are identified by UUID, not auto-incrementing integers. No natural ordering; versions are accessed by staging label or UUID. |
| Separate staging labels map from version struct | Staging labels are the authoritative mapping; version.version_stages is a cached derivation. This prevents label inconsistencies. |
| ForceDeleteWithoutRecovery immediately removes | Simplifies local dev. Real AWS allows 7-30 day window, but for testing, instant cleanup is preferred. |
| RotateSecret clones current value to AWSPENDING | Without Lambda, there is no mechanism to generate a new secret value. Cloning allows the rotation lifecycle (pending/promote/finish) to be tested without Lambda. |
| Store resource policies as JSON strings, not parsed | No policy enforcement engine. Storing as strings is sufficient for round-trip fidelity. |
| Replication operations are accept-and-succeed stubs | Multi-region replication requires multiple server instances. Not useful for single-instance local dev. |
